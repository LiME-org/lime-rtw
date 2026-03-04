#include "vmlinux.h"

#include <bpf/bpf_core_read.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_tracing.h>
#include <linux/version.h>

#ifndef LIME_TARGET_KERNEL_VERSION_CODE
#ifdef LINUXKERNEL_VERSION_CODE
#define LIME_TARGET_KERNEL_VERSION_CODE LINUXKERNEL_VERSION_CODE
#else
#define LIME_TARGET_KERNEL_VERSION_CODE LINUX_VERSION_CODE
#endif
#endif

#if LIME_TARGET_KERNEL_VERSION_CODE >= KERNEL_VERSION(6, 12, 0)
#define HAS_BPF_TASK_KFUNC 1
#define __ksym __attribute__((section(".ksyms")))
extern struct task_struct *bpf_task_from_pid(s32 pid) __ksym;
extern void bpf_task_release(struct task_struct *task) __ksym;
#undef __ksym
#else
#define HAS_BPF_TASK_KFUNC 0
#endif

#include "lime_tracer.h"

/*
 * Kernel defines
 * Macro definitions are not dumped in vmlinux.h,
 * therefore needed macros are redefined here.
 */

#define SCHED_NORMAL 0
#define SCHED_FIFO 1
#define SCHED_RR 2
#define SCHED_BATCH 3
/* SCHED_ISO: reserved but not implemented yet */
#define SCHED_IDLE 5
#define SCHED_DEADLINE 6

#define ENOMEM 132

#define SIGRTMIN 32
#define SIGRTMAX 64

#define LIME_EVENT_BATCH_SIZE 1000

/* *** End of kernel defines *** */

// Dummy instance to get skeleton to generate definition for `struct event`
struct lime_event _event = {0};

volatile const u32 sched_policy_mask = 0x46;
volatile const int target_tgid = 0;

static inline bool test_bit(u8 bit, u32 word) { return (1 << bit) & word; }

struct {
  __uint(type, BPF_MAP_TYPE_HASH);
  __uint(max_entries, 10240);
  __type(key, pid_t);
  __type(value, u8);
} throttled SEC(".maps");

struct {
  __uint(type, BPF_MAP_TYPE_HASH);
  __uint(max_entries, 128 * 1024);
  __type(key, pid_t);
  __type(value, u8);
} filter_as SEC(".maps");

struct {
  __uint(type, BPF_MAP_TYPE_HASH);
  __uint(max_entries, 128);
  __type(key, pid_t);
  __type(value, u64);
} changing SEC(".maps");

struct {
  __uint(type, BPF_MAP_TYPE_HASH);
  __uint(max_entries, 1024);
  __type(key, u64);
  __type(value, u64);
} yield_timers SEC(".maps");

struct {
  __uint(type, BPF_MAP_TYPE_RINGBUF);
  __uint(max_entries, 64 * 1024 * 1024);
} events SEC(".maps");

// Kernel 5.14 changed the state field to __state
struct task_struct___pre_5_14 {
  long int state;
};

static inline bool is_rt_sched_policy(u32 policy) {
  return test_bit((u8)policy, sched_policy_mask);
}

static inline u32 get_sched_policy(struct task_struct *t) {
  u32 policy;

  bpf_core_read(&policy, sizeof(policy), &t->policy);

  return policy;
}

static inline u32 get_sched_priority(struct task_struct *t) {
  u32 prio;

  bpf_core_read(&prio, sizeof(prio), &t->rt_priority);

  return prio;
}

static inline u32 get_ppid(struct task_struct *t) {
  struct task_struct *p;
  u32 ppid;

  bpf_core_read(&p, sizeof(p), &t->real_parent);
  bpf_core_read(&ppid, sizeof(ppid), &p->pid);

  return ppid;
}

static inline u32 get_task_state(struct task_struct *t) {
  if (bpf_core_field_exists(t->__state))
    return t->__state;

  return ((struct task_struct___pre_5_14 *)t)->state;
}

static inline u32 get_task_cpu(struct task_struct *t) {
  u32 cpu;

#if LIME_TARGET_KERNEL_VERSION_CODE >= KERNEL_VERSION(5, 16, 0)
  bpf_core_read(&cpu, sizeof(cpu), &t->thread_info.cpu);
#else
  bpf_core_read(&cpu, sizeof(cpu), &t->on_cpu);
#endif

  return cpu;
}

static inline u64 get_pid_tgid(struct task_struct *t) {
  u64 ret;
  u32 pid;
  u32 tgid;

  bpf_core_read(&tgid, sizeof(tgid), &t->tgid);
  bpf_core_read(&pid, sizeof(pid), &t->pid);

  ret = tgid;
  ret <<= 32;
  ret += pid;

  return ret;
}

static inline void fill_sched_attr(struct task_struct *t,
                                   struct lime_sched_attr *attr) {
  u32 policy;

  if (!attr)
    return;

  __builtin_memset(attr, 0, sizeof(*attr));

  if (!t)
    return;

  policy = get_sched_policy(t);
  attr->policy = policy;

  switch (policy) {
  case SCHED_FIFO:
  case SCHED_RR:
    attr->attrs.rt.prio = t->rt_priority;
    break;

  case SCHED_DEADLINE:
    attr->attrs.dl.runtime = t->dl.dl_runtime;
    attr->attrs.dl.period = t->dl.dl_period;
    attr->attrs.dl.deadline = t->dl.dl_deadline;
    break;

  default:
    break;
  }
}

static inline void fill_affinity_mask(struct task_struct *t,
                                      __u64 mask[CPUMASK_U64_COUNT]) {
  if (!t || !mask)
    return;

  int cpu_count = 0;
  bpf_core_read(&cpu_count, sizeof(cpu_count), &t->nr_cpus_allowed);
  if (cpu_count <= 0 || cpu_count > (CPUMASK_U64_COUNT * 64))
    cpu_count = CPUMASK_U64_COUNT * 64;

#pragma unroll
  for (int i = 0; i < CPUMASK_U64_COUNT; i++) {
    unsigned long word = 0;
    int bit_base = i * 64;

    if (bit_base >= cpu_count) {
      mask[i] = 0;
      continue;
    }

    bpf_core_read(&word, sizeof(word), &t->cpus_mask.bits[i]);
    int bits_remaining = cpu_count - bit_base;
    if (bits_remaining < 64) {
      unsigned long mask = (1UL << bits_remaining) - 1;
      word &= mask;
    }

    mask[i] = word;
  }
}

static inline struct task_struct *
get_target_task(u32 target_pid, u64 current_pid_tgid,
                struct task_struct **ref_task) {
  *ref_task = NULL;

  if (!target_pid || target_pid == (u32)current_pid_tgid)
    return (struct task_struct *)bpf_get_current_task_btf();

#if HAS_BPF_TASK_KFUNC
  struct task_struct *task = bpf_task_from_pid((s32)target_pid);
  if (task)
    *ref_task = task;

  return task;
#else
  return NULL;
#endif
}

static inline void release_task(struct task_struct *ref_task) {
#if HAS_BPF_TASK_KFUNC
  if (ref_task)
    bpf_task_release(ref_task);
#else
  (void)ref_task;
#endif
}

struct sched_target_ctx {
  struct task_struct *task;
  struct task_struct *ref_task;
  bool same_task;
  bool target_is_real;
  u64 dst_pid_tgid;
};

static inline void prepare_sched_target(pid_t requested_pid, u64 pid_tgid,
                                        struct sched_target_ctx *ctx) {
  struct task_struct *target = get_target_task(
      requested_pid ? requested_pid : (u32)pid_tgid, pid_tgid, &ctx->ref_task);
  bool same_task = (!requested_pid || requested_pid == (u32)pid_tgid);

  if (!target && same_task)
    target = (struct task_struct *)bpf_get_current_task_btf();

  ctx->task = target;
  ctx->same_task = same_task;
  ctx->target_is_real = target && (ctx->ref_task || same_task);
  ctx->dst_pid_tgid =
      ctx->target_is_real ? get_pid_tgid(target)
                          : (requested_pid ? (u64)requested_pid : pid_tgid);
}

static inline u64 now() { return bpf_ktime_get_boot_ns(); }

static inline void submit_event(struct lime_event *event) {
  long sz;
  int flags = 0;

  sz = bpf_ringbuf_query(&events, BPF_RB_AVAIL_DATA);
  flags =
      (sz >= LIME_EVENT_BATCH_SIZE) ? BPF_RB_FORCE_WAKEUP : BPF_RB_NO_WAKEUP;

  bpf_ringbuf_submit(event, flags);
}

static inline void emit_process_info_event(struct task_struct *t) {
  struct lime_event *event;
  struct mm_struct *mm = NULL;
  unsigned long arg_start = 0;
  unsigned long arg_end = 0;
  unsigned long total_bytes = 0;
  u64 pid_tgid;
  u64 ts;

  if (!t)
    return;

  pid_tgid = get_pid_tgid(t);
  ts = now();

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return;

  event->ev_type = PROCESS_INFO_START;
  event->pid_tgid = pid_tgid;
  event->ts = ts;
  event->evd.process_info_start.ppid = get_ppid(t);
  bpf_probe_read_kernel_str(event->evd.process_info_start.comm,
                            sizeof(event->evd.process_info_start.comm), t->comm);
  submit_event(event);

  bpf_core_read(&mm, sizeof(mm), &t->mm);
  if (!mm)
    goto end;

  bpf_core_read(&arg_start, sizeof(arg_start), &mm->arg_start);
  bpf_core_read(&arg_end, sizeof(arg_end), &mm->arg_end);
  if (arg_end <= arg_start)
    goto end;

  total_bytes = arg_end - arg_start;
  if (total_bytes > LIME_CMD_LEN)
    total_bytes = LIME_CMD_LEN;

#pragma unroll
  for (int idx = 0; idx < LIME_CMD_CHUNK_COUNT; idx++) {
    unsigned long offset = (__u64)idx * LIME_CMD_CHUNK_LEN;

    if (offset >= total_bytes)
      break;

    __u64 chunk_len = LIME_CMD_CHUNK_LEN;
    if (offset + chunk_len > total_bytes)
      chunk_len = (total_bytes - offset);

    // Hard clamp for verifier and helper argument bounds.
    if (chunk_len > sizeof(event->evd.process_info_chunk.chunk))
      chunk_len = sizeof(event->evd.process_info_chunk.chunk);

    event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
    if (!event)
      break;

    event->ev_type = PROCESS_INFO_CMD_CHUNK;
    event->pid_tgid = pid_tgid;
    event->ts = ts;
    event->evd.process_info_chunk.chunk_len = chunk_len;
    __builtin_memset(event->evd.process_info_chunk.chunk, 0,
                     sizeof(event->evd.process_info_chunk.chunk));
    if (chunk_len > 0) {
      bpf_probe_read_user(event->evd.process_info_chunk.chunk, chunk_len,
                          (void *)(arg_start + offset));
    }
    submit_event(event);
  }

end:
  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return;

  event->ev_type = PROCESS_INFO_END;
  event->pid_tgid = pid_tgid;
  event->ts = ts;
  submit_event(event);
}

static inline void emit_sched_policy_update_event(struct task_struct *t) {
  struct lime_event *event;

  if (!t)
    return;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return;

  event->ev_type = SCHED_POLICY_UPDATE;
  event->pid_tgid = get_pid_tgid(t);
  event->ts = now();
  fill_sched_attr(t, &event->evd.sched_attr);

  submit_event(event);
}

static inline void emit_affinity_update_event(struct task_struct *t) {
  struct lime_event *event;
  __u64 mask[CPUMASK_U64_COUNT] = {};
  __u64 pid_tgid;
  u64 ts;
  const __u32 chunk_count = LIME_AFFINITY_CHUNK_COUNT;

  if (!t)
    return;

  fill_affinity_mask(t, mask);
  pid_tgid = get_pid_tgid(t);
  ts = now();

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return;

  event->ev_type = AFFINITY_UPDATE_START;
  event->pid_tgid = pid_tgid;
  event->ts = ts;
  event->evd.affinity_update_start.chunk_count = chunk_count;
  submit_event(event);

#pragma unroll
  for (int chunk = 0; chunk < LIME_AFFINITY_CHUNK_COUNT; chunk++) {
    __u32 word_base = chunk * LIME_AFFINITY_CHUNK_WORDS;
    __u32 remaining_words = 0;
    __u32 copy_words = 0;

    if (word_base >= CPUMASK_U64_COUNT)
      break;

    remaining_words = CPUMASK_U64_COUNT - word_base;
    copy_words = remaining_words < LIME_AFFINITY_CHUNK_WORDS
                     ? remaining_words
                     : LIME_AFFINITY_CHUNK_WORDS;

    event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
    if (!event)
      break;

    event->ev_type = (chunk == (LIME_AFFINITY_CHUNK_COUNT - 1))
                         ? AFFINITY_UPDATE_CHUNK_END
                         : AFFINITY_UPDATE_CHUNK;
    event->pid_tgid = pid_tgid;
    event->ts = ts;
    event->evd.affinity_update_chunk.chunk_len =
        copy_words * sizeof(__u64);

    for (int i = 0; i < LIME_AFFINITY_CHUNK_WORDS; i++) {
      __u32 word_idx = word_base + i;
      if (i < copy_words && word_idx < CPUMASK_U64_COUNT) {
        event->evd.affinity_update_chunk.mask[i] = mask[word_idx];
      } else {
        event->evd.affinity_update_chunk.mask[i] = 0;
      }
    }

    submit_event(event);
  }

}

static inline void emit_full_process_update(struct task_struct *t) {
  if (!t)
    return;

  emit_process_info_event(t);
  emit_sched_policy_update_event(t);
  emit_affinity_update_event(t);
}

static inline int emit_sched_attr_event(struct task_struct *t,
                                        event_type_t type,
                                        u64 pid_tgid) {
  struct lime_event *event;

  if (!t)
    return 0;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return -ENOMEM;

  event->ts = now();
  event->ev_type = type;
  event->pid_tgid = pid_tgid;
  fill_sched_attr(t, &event->evd.sched_attr);

  submit_event(event);
  return 0;
}

static int filter_out_task(struct task_struct *t) {
  u32 policy;
  u64 pid_tgid;
  u32 tgid, ppid;
  u32 pid;

  if (!t)
    return 1;

  pid_tgid = get_pid_tgid(t);

  if (!pid_tgid)
    return 1;

  pid = (u32)pid_tgid;
  if (bpf_map_lookup_elem(&throttled, &pid)) {
    return 1;
  }

  if (target_tgid) {
    tgid = pid_tgid >> 32;
    ppid = get_ppid(t);

    if ((target_tgid != tgid) && (target_tgid != ppid))
      return 1;
  }

  policy = get_sched_policy(t);
  if (!is_rt_sched_policy(policy))
    return 1;

  return 0;
}

struct sched_wakeup_args {
  struct task_struct *t;
};

static int __handle_sched_wakeup(struct sched_wakeup_args *ctx,
                                 event_type_t event_type) {
  struct lime_event *event;
  u64 pid_tgid;

  if (filter_out_task(ctx->t))
    return 0;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);

  if (!event)
    return -ENOMEM;

  event->ev_type = event_type;
  event->ts = now();
  event->pid_tgid = get_pid_tgid(ctx->t);
  event->evd.sched_wakeup.cpu = get_task_cpu(ctx->t);

  submit_event(event);

  return 0;
}

SEC("tp_btf/sched_wakeup")
int handle__sched_wakeup(struct sched_wakeup_args *ctx) {
  return __handle_sched_wakeup(ctx, SCHED_WAKEUP);
}

SEC("tp_btf/sched_wakeup_new")
int handle__sched_wakeup_new(struct sched_wakeup_args *ctx) {
  return __handle_sched_wakeup(ctx, SCHED_WAKEUP_NEW);
}

SEC("tp_btf/sched_waking")
int handle__sched_waking(struct sched_wakeup_args *ctx) {
  return __handle_sched_wakeup(ctx, SCHED_WAKING);
}

struct sched_switch_args {
  bool preempt;
  struct task_struct *prev;
  struct task_struct *next;
};

#define TASK_REPORT 0x7f
#define TASK_IDLE 0x402

unsigned int get_state_and_exit_state(struct task_struct *t) {
  unsigned int exit_state;
  unsigned int tsk_state = get_task_state(t);

  bpf_core_read(&exit_state, sizeof(exit_state), &t->exit_state);

  return (tsk_state | exit_state);
}

SEC("tp_btf/sched_switch")
int handle__sched_switch(struct sched_switch_args *args) {
  struct lime_event *e1, *e2;
  struct task_struct *t;

  u64 ts = now();

  if (!filter_out_task(args->next)) {
    t = args->next;

    e1 = bpf_ringbuf_reserve(&events, sizeof(*e1), 0);

    if (!e1)
      return -ENOMEM;

    e1->ev_type = SCHED_SCHEDULE;
    e1->pid_tgid = get_pid_tgid(t);
    e1->ts = ts;
    e1->evd.sched_schedule.prio = get_sched_priority(t);
    e1->evd.sched_schedule.cpu = get_task_cpu(t);
    e1->evd.sched_schedule.preempt = args->preempt;

    bpf_ringbuf_submit(e1, 0);
  }

  if (!filter_out_task(args->prev)) {
    t = args->prev;
    e2 = bpf_ringbuf_reserve(&events, sizeof(*e2), 0);

    if (!e2)
      return -ENOMEM;

    e2->ev_type = SCHED_DESCHEDULE;
    e2->pid_tgid = get_pid_tgid(t);
    e2->ts = ts;
    e2->evd.sched_deschedule.prio = get_sched_priority(t);
    e2->evd.sched_deschedule.cpu = get_task_cpu(t);
    e2->evd.sched_deschedule.state =
        (args->preempt) ? 0 : get_state_and_exit_state(t);

    bpf_ringbuf_submit(e2, 0);
  }

  return 0;
}

struct sched_migrate_task_args {
  struct task_struct *t;
  int dest_cpu;
};

SEC("tp_btf/sched_migrate_task")
int handle_sched_migrate_task(struct sched_migrate_task_args *ctx) {
  struct lime_event *event;

  if (filter_out_task(ctx->t))
    return 0;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return -ENOMEM;

  event->ts = now();
  event->ev_type = SCHED_MIGRATE_TASK;
  event->pid_tgid = get_pid_tgid(ctx->t);
  event->evd.sched_migrate_task.dest_cpu = ctx->dest_cpu;

  submit_event(event);

  return 0;
}

SEC("tp_btf/sched_process_exit")
int on_sched_process_exit(u64 *ctx) {
  struct task_struct *t = (struct task_struct *)ctx[0];
  u64 pid_tgid = get_pid_tgid(t);

  bpf_map_delete_elem(&throttled, &pid_tgid);

  if (filter_out_task(t))
    return 0;

  emit_full_process_update(t);
  return emit_sched_attr_event(t, SCHED_PROCESS_EXIT, pid_tgid);
}

SEC("tp_btf/sched_process_exec")
int on_sched_process_exec(u64 *ctx) {
  struct task_struct *t = (struct task_struct *)ctx[0];
  u64 pid_tgid;

  if (filter_out_task(t))
    return 0;

  pid_tgid = get_pid_tgid(t);
  emit_full_process_update(t);
  emit_sched_attr_event(t, SCHED_PROCESS_EXEC, pid_tgid);

  return 0;
}

SEC("tp_btf/sched_process_fork")
int on_sched_process_fork(u64 *ctx) {
  struct task_struct *t = (struct task_struct *)ctx[1];
  u64 pid_tgid;

  if (filter_out_task(t))
    return 0;

  pid_tgid = get_pid_tgid(t);
  emit_full_process_update(t);
  emit_sched_attr_event(t, SCHED_PROCESS_FORK, pid_tgid);

  return 0;
}

struct rt_thread_info _info = {};

/*

SEC("iter/task")
int dump_rt_threads(struct bpf_iter__task_file *ctx) {
    pid_t pid;
    u8 val = 1;
    struct rt_thread_info info = {0};
    struct seq_file *seq = ctx->meta->seq;
    struct task_struct *task = ctx->task;

    if (filter_out_task(task))
        return 0;

    bpf_core_read(&pid, sizeof(pid), &task->pid);
    bpf_map_update_elem(&known_pids, &pid, &val, 0);

    info.pid_tgid = get_pid_tgid(task);
    info.cpu = get_task_cpu(task);
    info.policy = get_sched_policy(task);
    info.prio = get_sched_priority(task);

    bpf_seq_write(seq, &info, sizeof(info));

    return 0;
}
*/

struct enter_clock_nanosleep_args {
  unsigned short common_type;
  unsigned char common_flags;
  unsigned char common_preempt_count;
  int common_pid;
  int __syscall_nr;
  u64 which_clock;
  u64 flags;
  struct __kernel_timespec *rqtp;
  struct __kernel_timespec *rmtp;
};

struct exit_ar_args {
  u64 unused[2];
  u64 ret;
};

static int handle_exit_arrival_site(struct exit_ar_args *ctx) {
  u64 ts;
  u64 pid_tgid;
  struct lime_event *event;
  struct task_struct *t;

  t = (struct task_struct *)bpf_get_current_task();

  if (filter_out_task(t))
    return 0;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);

  if (!event)
    return -ENOMEM;

  ts = now();
  pid_tgid = bpf_get_current_pid_tgid();

  event->ev_type = EXIT_AS;
  event->pid_tgid = pid_tgid;
  event->ts = ts;

  event->evd.exit_ar.ret = (int)ctx->ret;

  submit_event(event);

  return 0;
}
struct sched_setscheduler_args {
  u64 __padding[2];
  u64 pid;
  u64 policy;
  struct sched_param *params;
};

SEC("tracepoint/syscalls/sys_enter_sched_setscheduler")
int handle_sys_enter_sched_setscheduler(struct sched_setscheduler_args *ctx) {
  struct lime_event *event;
  u64 pid_tgid, dst_pid_tgid;
  u32 pid = ctx->pid;
  int prio = 0;
  struct sched_param *p;
  long err;
  struct sched_target_ctx target_ctx = {};

  if (!ctx->params)
    return 0;

  pid_tgid = bpf_get_current_pid_tgid();
  prepare_sched_target(pid, pid_tgid, &target_ctx);

  if (target_ctx.same_task && target_ctx.task && filter_out_task(target_ctx.task)) {
    release_task(target_ctx.ref_task);
    return 0;
  }

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event) {
    release_task(target_ctx.ref_task);
    return -ENOMEM;
  }

  event->ts = now();
  dst_pid_tgid = target_ctx.dst_pid_tgid;
  event->pid_tgid = dst_pid_tgid;

  err = bpf_map_update_elem(&changing, &pid_tgid, &dst_pid_tgid, BPF_NOEXIST);
  if (err < 0) {
    bpf_ringbuf_discard(event, 0);
    release_task(target_ctx.ref_task);
    return err;
  }

  event->ev_type = ENTER_SCHED_SETSCHEDULER;
  event->evd.sched_attr.policy = ctx->policy;
  if (target_ctx.task)
    event->evd.sched_attr.old_policy = get_sched_policy(target_ctx.task);
  else
    event->evd.sched_attr.old_policy =
        get_sched_policy((struct task_struct *)bpf_get_current_task_btf());

  switch (ctx->policy) {
  case SCHED_FIFO:
  case SCHED_RR:
    p = ctx->params;
    bpf_core_read_user(&prio, sizeof(prio), &p->sched_priority);
    event->evd.sched_attr.attrs.rt.prio = (__u32)prio;
    break;

  default:
    break;
  }

  submit_event(event);

  release_task(target_ctx.ref_task);

  return 0;
}

static inline int on_ret_change_scheduler(void *ctx, int retval) {
  struct lime_event *event;
  __u64 pidtgid = bpf_get_current_pid_tgid();
  struct task_struct *target = NULL;
  struct task_struct *ref_task = NULL;
  u32 target_pid = 0;

  __u64 *dst_pidtgid = bpf_map_lookup_elem(&changing, &pidtgid);
  if (!dst_pidtgid) {
    return -1;
  }

  target_pid = (u32)(*dst_pidtgid);
  target = get_target_task(target_pid, pidtgid, &ref_task);

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);

  if (!event) {
    release_task(ref_task);
    return -ENOMEM;
  }

  event->ts = now();
  event->pid_tgid = *dst_pidtgid;

  if (!retval) {
    event->ev_type = SCHED_SCHEDULER_CHANGE;
  } else {
    event->ev_type = SCHED_SCHEDULER_CHANGE_FAILED;
  }

  if (target)
    fill_sched_attr(target, &event->evd.sched_attr);
  else
    __builtin_memset(&event->evd.sched_attr, 0, sizeof(event->evd.sched_attr));

  bpf_map_delete_elem(&changing, &pidtgid);

  submit_event(event);

  if (!retval) {
    emit_process_info_event(target);
    emit_affinity_update_event(target);
  }

  release_task(ref_task);

  return 0;
}

SEC("tracepoint/syscalls/sys_exit_sched_setscheduler")
int handle_sys_exit_sched_setscheduler(struct exit_ar_args *ctx) {
  int retval = ctx->ret;

  return on_ret_change_scheduler(ctx, retval);
}

struct sched_setattr_args {
  u64 __padding[2];
  pid_t pid;
  struct sched_attr *attrs;
  u64 flags;
};

SEC("tracepoint/syscalls/sys_enter_sched_setattr")
int handle_sys_enter_sched_setattr(struct sched_setattr_args *ctx) {
  struct lime_event *event;
  u64 pid_tgid, dst_pid_tgid;
  u32 pid = ctx->pid;
  u32 prio = 0;
  int policy;
  u64 runtime, period, deadline;
  long err;
  struct sched_target_ctx target_ctx = {};

  struct sched_attr *attrs = ctx->attrs;
  if (!attrs)
    return 0;

  pid_tgid = bpf_get_current_pid_tgid();
  prepare_sched_target(pid, pid_tgid, &target_ctx);

  if (target_ctx.same_task && target_ctx.task && filter_out_task(target_ctx.task)) {
    release_task(target_ctx.ref_task);
    return 0;
  }

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);

  if (!event) {
    release_task(target_ctx.ref_task);
    return -ENOMEM;
  }

  event->ts = now();
  event->ev_type = ENTER_SCHED_SETATTR;

  dst_pid_tgid = target_ctx.dst_pid_tgid;
  event->pid_tgid = dst_pid_tgid;

  err = bpf_map_update_elem(&changing, &pid_tgid, &dst_pid_tgid, BPF_NOEXIST);
  if (err < 0) {
    bpf_ringbuf_discard(event, 0);
    release_task(target_ctx.ref_task);
    return err;
  }

  bpf_core_read_user(&policy, sizeof(policy), &attrs->sched_policy);
  event->evd.sched_attr.policy = policy;
  if (target_ctx.task)
    event->evd.sched_attr.old_policy = get_sched_policy(target_ctx.task);
  else
    event->evd.sched_attr.old_policy =
        get_sched_policy((struct task_struct *)bpf_get_current_task_btf());

  switch (policy) {
  case SCHED_FIFO:
  case SCHED_RR:
    bpf_core_read_user(&prio, sizeof(prio), &attrs->sched_priority);
    event->evd.sched_attr.attrs.rt.prio = prio;
    break;

  case SCHED_DEADLINE:
    bpf_core_read_user(&runtime, sizeof(runtime), &attrs->sched_runtime);
    event->evd.sched_attr.attrs.dl.runtime = runtime;

    bpf_core_read_user(&period, sizeof(period), &attrs->sched_period);
    event->evd.sched_attr.attrs.dl.period = period;

    bpf_core_read_user(&deadline, sizeof(deadline), &attrs->sched_deadline);
    event->evd.sched_attr.attrs.dl.deadline = deadline;

    break;
  default:
    break;
  }

  submit_event(event);
  release_task(target_ctx.ref_task);

  return 0;
}

SEC("tracepoint/syscalls/sys_exit_sched_setattr")
int handle_sys_exit_sched_setattr(struct exit_ar_args *ctx) {
  int retval = ctx->ret;

  return on_ret_change_scheduler(ctx, retval);
}

struct enter_setaffinity_args {
  __u64 unused[2];
  pid_t pid;           // First syscall arg
  size_t cpusetsize;   // Second syscall arg
  const unsigned long *user_mask_ptr;  // Third syscall arg
};

SEC("tracepoint/syscalls/sys_enter_sched_setaffinity")
int handle_sys_enter_sched_setaffinity(struct enter_setaffinity_args *ctx) {
  struct lime_event *event;
  u64 pid_tgid, dst_pid_tgid;
  long err;
  struct sched_target_ctx target_ctx = {};

  pid_tgid = bpf_get_current_pid_tgid();
  prepare_sched_target(ctx->pid, pid_tgid, &target_ctx);

  if (target_ctx.same_task && target_ctx.task && filter_out_task(target_ctx.task)) {
    release_task(target_ctx.ref_task);
    return 0;
  }

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event) {
    release_task(target_ctx.ref_task);
    return -ENOMEM;
  }

  event->ts = now();
  event->ev_type = ENTER_SCHED_SETAFFINITY;
  dst_pid_tgid = target_ctx.dst_pid_tgid;
  event->pid_tgid = dst_pid_tgid;

  err = bpf_map_update_elem(&changing, &pid_tgid, &dst_pid_tgid, BPF_NOEXIST);
  if (err < 0) {
    bpf_ringbuf_discard(event, 0);
    release_task(target_ctx.ref_task);
    return err;
  }

  submit_event(event);
  release_task(target_ctx.ref_task);

  return 0;
}

SEC("tracepoint/syscalls/sys_exit_sched_setaffinity")
int handle_sys_exit_sched_setaffinity(struct exit_ar_args *ctx) {
  int retval = ctx->ret;
  u64 pid_tgid;
  struct lime_event *event;
  struct task_struct *target = NULL;
  struct task_struct *ref_task = NULL;
  u32 target_pid = 0;

  pid_tgid = bpf_get_current_pid_tgid();

  u64 *dst_pid_tgid = bpf_map_lookup_elem(&changing, &pid_tgid);
  if (!dst_pid_tgid) {
    return -1;
  }

  target_pid = (u32)(*dst_pid_tgid);
  target = get_target_task(target_pid, pid_tgid, &ref_task);

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event) {
    bpf_map_delete_elem(&changing, &pid_tgid);
    release_task(ref_task);
    return -ENOMEM;
  }

  event->ts = now();
  event->pid_tgid = *dst_pid_tgid;  

  if (!retval) {
    event->ev_type = SCHED_AFFINITY_CHANGE;
  } else {
    event->ev_type = SCHED_AFFINITY_CHANGE_FAILED;
  }

  if (target)
    fill_sched_attr(target, &event->evd.sched_attr);
  else
    __builtin_memset(&event->evd.sched_attr, 0, sizeof(event->evd.sched_attr));

  submit_event(event);

  if (!retval)
    emit_affinity_update_event(target);

  release_task(ref_task);

  bpf_map_delete_elem(&changing, &pid_tgid);

  return 0;
}

SEC("tp_btf/hrtimer_expire_entry")
int on_hrtimer_expire_entry(u64 *ctx) {
  u64 *pid;
  u64 ts;
  u64 pid_tgid;
  struct lime_event *event;
  struct hrtimer *timer;
  u64 expires;

  timer = (struct hrtimer *)ctx[0];

  pid = bpf_map_lookup_elem(&yield_timers, (u64 *)&timer);
  if (pid) {
    bpf_map_delete_elem(&yield_timers, (u64 *)&timer);
    event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
    if (!event)
      return -ENOMEM;

    ts = now();
    pid_tgid = *pid;

    event->ev_type = ENTER_DL_TIMER;
    event->pid_tgid = pid_tgid;
    event->ts = ts;

    bpf_core_read(&expires, sizeof(expires), &timer->_softexpires);
    event->evd.dl_timer.expires = expires;

    submit_event(event);
  }

  return 0;
}

struct sched_yield_args {};

SEC("tracepoint/syscalls/sys_enter_sched_yield")
int on_sched_yield(struct sched_yield_args *ctx) {
  u64 ts;
  u64 pid_tgid;
  struct lime_event *event;
  struct task_struct *t;
  struct sched_dl_entity *se;
  u64 hrt;
  int err;
  int policy;

  t = (struct task_struct *)bpf_get_current_task_btf();

  policy = t->policy;

  if (policy != SCHED_DEADLINE) {
    return 0;
  }

  if (filter_out_task(t))
    return 0;

  ts = now();
  pid_tgid = bpf_get_current_pid_tgid();

  bpf_core_read(&se, sizeof(se), &t->dl);

  hrt = (u64)&t->dl.dl_timer;

  err = bpf_map_update_elem(&yield_timers, &hrt, &pid_tgid, BPF_ANY);
  if (err < 0)
    return err;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return -ENOMEM;

  event->ev_type = ENTER_SCHED_YIELD;
  event->pid_tgid = pid_tgid;
  event->ts = ts;

  submit_event(event);

  return 0;
}

SEC("tracepoint/syscalls/sys_exit_sched_yield")
int on_sys_exit_sched_yield(void *ctx) {
  struct task_struct *t;
  int policy;

  t = (struct task_struct *)bpf_get_current_task_btf();
  policy = t->policy;

  if (policy != SCHED_DEADLINE) {
    return 0;
  }

  return handle_exit_arrival_site(ctx);
}

SEC("tracepoint/syscalls/sys_enter_clock_nanosleep")
int on_sys_enter_clock_nanosleep(struct enter_clock_nanosleep_args *ctx) {
  u64 ts, secs = 1, nsecs = 1;
  u64 pid;
  u32 policy;

  struct task_struct *t = NULL;
  struct lime_event *event;
  clockid_t which_clock = (clockid_t)ctx->which_clock;
  int flags = (int)ctx->flags;
  struct __kernel_timespec *rqtp = ctx->rqtp;

  t = (struct task_struct *)bpf_get_current_task();

  if (filter_out_task(t))
    return 0;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);

  if (!event)
    return -ENOMEM;

  pid = bpf_get_current_pid_tgid();

  event->ev_type = ENTER_CLOCK_NANOSLEEP;
  event->pid_tgid = pid;
  event->ts = now();
  event->evd.clock_nanosleep.clock_id = which_clock;
  event->evd.clock_nanosleep.abs_time = (flags == 1);

  bpf_core_read_user(&secs, sizeof(secs), &rqtp->tv_sec);

  bpf_core_read_user(&nsecs, sizeof(nsecs), &rqtp->tv_nsec);

  event->evd.clock_nanosleep.rq_sec = secs;
  event->evd.clock_nanosleep.rq_nsec = nsecs;

  submit_event(event);

  return 0;
}

SEC("tracepoint/syscalls/sys_exit_clock_nanosleep")
int on_sys_exit_clock_nanosleep(void *ctx) {
  return handle_exit_arrival_site(ctx);
}

struct enter_nanosleep_args {
  u64 __unused[2];
  struct __kernel_timespec *rqtp;
  struct __kernel_timespec *rmtp;
};

SEC("tracepoint/syscalls/sys_enter_nanosleep")
int on_sys_enter_nanosleep(struct enter_nanosleep_args *ctx) {
  u64 ts, secs = 1, nsecs = 1;
  u64 pid;
  u32 policy;

  struct task_struct *t = NULL;
  struct lime_event *event;
  struct __kernel_timespec *rqtp = ctx->rqtp;

  t = (struct task_struct *)bpf_get_current_task();

  if (filter_out_task(t))
    return 0;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);

  if (!event)
    return -ENOMEM;

  pid = bpf_get_current_pid_tgid();

  event->ev_type = ENTER_NANOSLEEP;
  event->pid_tgid = pid;
  event->ts = now();

  bpf_core_read_user(&secs, sizeof(secs), &rqtp->tv_sec);

  bpf_core_read_user(&nsecs, sizeof(nsecs), &rqtp->tv_nsec);

  event->evd.enter_nanosleep.rq_sec = secs;
  event->evd.enter_nanosleep.rq_nsec = nsecs;

  submit_event(event);

  return 0;
}

SEC("tracepoint/syscalls/sys_exit_nanosleep")
int on_sys_exit_nanosleep(void *ctx) { return handle_exit_arrival_site(ctx); }

struct select_args {
  u64 unused[2];
  u64 n;
  struct fd_set *inp;
  struct fd_set *outp;
  struct fd_set *exp;
  struct __kernel_old_timeval *tvp;
};

SEC("tracepoint/syscalls/sys_enter_select")
int on_sys_enter_select(struct select_args *ctx) {
  struct task_struct *t = NULL;
  struct lime_event *event;
  struct __kernel_old_timeval *tvp = ctx->tvp;

  t = (struct task_struct *)bpf_get_current_task();

  if (filter_out_task(t))
    return 0;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return -ENOMEM;

  event->ev_type = ENTER_SELECT;
  event->ts = now();
  event->pid_tgid = bpf_get_current_pid_tgid();
  event->evd.enter_select.inp = (u64)ctx->inp;

  if (tvp == NULL) {
    event->evd.enter_select.tvp_null = true;
    event->evd.enter_select.tv_sec = 0;
    event->evd.enter_select.tv_usec = 0;
  } else {
    event->evd.enter_select.tvp_null = false;
    bpf_core_read_user(&event->evd.enter_select.tv_sec, sizeof(long int),
                       &tvp->tv_sec);

    bpf_core_read_user(&event->evd.enter_select.tv_usec, sizeof(long int),
                       &tvp->tv_usec);
  }

  submit_event(event);

  return 0;
}

struct pselect6_args {
  u64 unused[2];
  u64 n;
  struct fd_set *inp;
  struct fd_set *outp;
  struct fd_set *exp;
  struct __kernel_timespec *tsp;
};

SEC("tracepoint/syscalls/sys_enter_pselect6")
int on_sys_enter_pselect6(struct pselect6_args *ctx) {
  struct task_struct *t = NULL;
  struct lime_event *event;
  struct __kernel_timespec *tsp = ctx->tsp;

  t = (struct task_struct *)bpf_get_current_task();

  if (filter_out_task(t))
    return 0;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return -ENOMEM;

  event->ev_type = ENTER_PSELECT6;
  event->ts = now();
  event->pid_tgid = bpf_get_current_pid_tgid();
  event->evd.enter_select.inp = (u64)ctx->inp;

  if (tsp == NULL) {
    event->evd.enter_pselect6.tsp_null = true;
    event->evd.enter_pselect6.tv_sec = 0;
    event->evd.enter_pselect6.tv_nsec = 0;
  } else {
    event->evd.enter_pselect6.tsp_null = false;
    bpf_core_read_user(&event->evd.enter_pselect6.tv_sec, sizeof(long int),
                       &tsp->tv_sec);

    bpf_core_read_user(&event->evd.enter_pselect6.tv_nsec, sizeof(long int),
                       &tsp->tv_nsec);
  }

  submit_event(event);

  return 0;
}

SEC("tracepoint/syscalls/sys_exit_select")
int on_sys_exit_select(void *ctx) { return handle_exit_arrival_site(ctx); }

SEC("tracepoint/syscalls/sys_exit_pselect6")
int on_sys_exit_pselect6(void *ctx) { return handle_exit_arrival_site(ctx); }

struct poll_args {
  u64 unused[2];
  struct pollfd *pfds;
  u64 nfds;
  u64 timeout_msecs;
};

SEC("tracepoint/syscalls/sys_enter_poll")
int on_sys_enter_poll(struct poll_args *ctx) {
  struct task_struct *t = NULL;
  struct lime_event *event;

  t = (struct task_struct *)bpf_get_current_task();

  if (filter_out_task(t))
    return 0;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return -ENOMEM;

  event->ev_type = ENTER_POLL;
  event->ts = now();
  event->pid_tgid = bpf_get_current_pid_tgid();
  event->evd.enter_poll.pfds = (u64)ctx->pfds;
  event->evd.enter_poll.timeout_msecs = (int)ctx->timeout_msecs;

  submit_event(event);

  return 0;
}

struct ppoll_args {
  u64 unused[2];
  struct pollfd *pfds;
  u64 nfds;
  struct __kernel_timespec *tsp;
};

SEC("tracepoint/syscalls/sys_enter_ppoll")
int on_sys_enter_ppoll(struct ppoll_args *ctx) {
  struct task_struct *t = NULL;
  struct lime_event *event;
  struct __kernel_timespec *tsp = ctx->tsp;

  t = (struct task_struct *)bpf_get_current_task();

  if (filter_out_task(t))
    return 0;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return -ENOMEM;

  event->ev_type = ENTER_PPOLL;
  event->ts = now();
  event->pid_tgid = bpf_get_current_pid_tgid();
  event->evd.enter_ppoll.pfds = (u64)ctx->pfds;

  bpf_core_read_user(&event->evd.enter_ppoll.tv_sec,
                     sizeof(event->evd.enter_ppoll.tv_sec), &tsp->tv_sec);

  bpf_core_read_user(&event->evd.enter_ppoll.tv_nsec,
                     sizeof(event->evd.enter_ppoll.tv_nsec), &tsp->tv_nsec);

  submit_event(event);

  return 0;
}

static inline struct file *get_struct_file(struct task_struct *t, int fd) {
  struct files_struct *f;
  struct fdtable *fdt;
  struct file **fdd;
  struct file *file;

  if (fd < 0) {
    return NULL;
  }

  // get files_struct
  bpf_core_read(&f, sizeof(f), &t->files);

  // get fdt table
  bpf_probe_read(&fdt, sizeof(fdt), (void *)&f->fdt);

  // get files table
  long ret = bpf_probe_read(&fdd, sizeof(fdd), (void *)&fdt->fd);

  if (ret) {
    bpf_printk("bpf_probe_read failed: %d\n", ret);
    return NULL;
  }

  bpf_probe_read(&file, sizeof(file), (void *)&fdd[fd]);

  return file;
}

struct read_args {
  u64 unused[2];
  int fd;
};

#define S_IFMT 00170000
#define S_IFSOCK 0140000
#define S_IFBLK 0060000
#define S_IFCHR 0020000
#define S_IFIFO 0010000

#define O_NONBLOCK 00004000

SEC("tracepoint/syscalls/sys_enter_read")
int on_sys_enter_read(struct read_args *ctx) {
  struct task_struct *t = NULL;
  struct lime_event *event;
  u64 pid_tgid;
  struct file *f;
  struct inode *inode;
  umode_t mode;
  unsigned int flags;
  u8 val = 0;
  enum event_type ev_type = 0;
  int fd;

  t = (struct task_struct *)bpf_get_current_task();

  if (filter_out_task(t))
    return 0;

  f = get_struct_file(t, ctx->fd);

  // Check file is not in non-blocking mode
  bpf_core_read(&flags, sizeof(flags), &f->f_flags);
  if (flags & O_NONBLOCK)
    return 0;

  // Get file mode
  bpf_core_read(&inode, sizeof(inode), &f->f_inode);
  bpf_core_read(&mode, sizeof(mode), &inode->i_mode);

  switch (mode & S_IFMT) {
  case S_IFSOCK:
    ev_type = ENTER_READ_SOCK;
    break;

  case S_IFIFO:
    ev_type = ENTER_READ_FIFO;
    break;

  case S_IFBLK:
    ev_type = ENTER_READ_BLK;
    break;

  case S_IFCHR:
    ev_type = ENTER_READ_CHR;
    break;

  default:
    return 0;
  }

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return -ENOMEM;

  event->ts = now();
  event->ev_type = ev_type;

  pid_tgid = bpf_get_current_pid_tgid();

  event->pid_tgid = pid_tgid;
  event->evd.enter_read.fd = ctx->fd;

  bpf_map_update_elem(&filter_as, &pid_tgid, &val, BPF_NOEXIST);

  submit_event(event);

  return 0;
}

SEC("tracepoint/syscalls/sys_exit_poll")
int on_sys_exit_poll(void *ctx) { return handle_exit_arrival_site(ctx); }

SEC("tracepoint/syscalls/sys_exit_ppoll")
int on_sys_exit_ppoll(void *ctx) { return handle_exit_arrival_site(ctx); }

SEC("tracepoint/syscalls/sys_exit_read")
int on_sys_exit_read(void *ctx) {
  u64 key = bpf_get_current_pid_tgid();

  if (bpf_map_lookup_elem(&filter_as, &key)) {
    bpf_map_delete_elem(&filter_as, &key);

    return handle_exit_arrival_site(ctx);
  }

  return 0;
}

struct accept_args {
  u64 unused[2];
  int fd;
};

int __on_enter_accept(struct accept_args *ctx) {
  struct task_struct *t = NULL;
  struct lime_event *event;

  t = (struct task_struct *)bpf_get_current_task();

  if (filter_out_task(t))
    return 0;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return -ENOMEM;

  event->ev_type = ENTER_ACCEPT;
  event->ts = now();
  event->pid_tgid = bpf_get_current_pid_tgid();
  event->evd.enter_accept.sock_fd = ctx->fd;

  submit_event(event);

  return 0;
}

SEC("tracepoint/syscalls/sys_enter_accept")
int on_enter_accept(struct accept_args *ctx) { return __on_enter_accept(ctx); }

SEC("tracepoint/syscalls/sys_enter_accept4")
int on_enter_accept4(struct accept_args *ctx) { return __on_enter_accept(ctx); }

SEC("tracepoint/syscalls/sys_exit_accept")
int on_sys_exit_accept(void *ctx) { return handle_exit_arrival_site(ctx); }
SEC("tracepoint/syscalls/sys_exit_accept4")
int on_sys_exit_accept4(void *ctx) { return handle_exit_arrival_site(ctx); }

#define FUTEX_WAIT 0
#define FUTEX_WAKE 1
#define FUTEX_FD 2
#define FUTEX_REQUEUE 3
#define FUTEX_CMP_REQUEUE 4
#define FUTEX_WAKE_OP 5
#define FUTEX_LOCK_PI 6
#define FUTEX_UNLOCK_PI 7
#define FUTEX_TRYLOCK_PI 8
#define FUTEX_WAIT_BITSET 9
#define FUTEX_WAKE_BITSET 10
#define FUTEX_WAIT_REQUEUE_PI 11
#define FUTEX_CMP_REQUEUE_PI 12
#define FUTEX_LOCK_PI2 13
#define FUTEX_PRIVATE_FLAG 128
#define FUTEX_CLOCK_REALTIME 256

struct futex_args {
  u64 unused[2];
  u32 *wordaddr;
  u64 op;
  u64 val;
};

SEC("tracepoint/syscalls/sys_enter_futex")
int on_sys_enter_futex(struct futex_args *ctx) {
  struct task_struct *t = NULL;
  struct lime_event *event;
  u64 pid_tgid;
  u8 val = 0;

  t = (struct task_struct *)bpf_get_current_task();

  if (filter_out_task(t))
    return 0;

  switch (ctx->op & ~(FUTEX_PRIVATE_FLAG | FUTEX_CLOCK_REALTIME)) {
  case FUTEX_WAIT:
  case FUTEX_WAKE:
  case FUTEX_WAKE_BITSET:
  case FUTEX_WAIT_BITSET:
    break;

  default:
    return 0;
  }

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return -ENOMEM;

  event->ts = now();
  pid_tgid = bpf_get_current_pid_tgid();
  event->pid_tgid = pid_tgid;

  bpf_map_update_elem(&filter_as, &pid_tgid, &val, BPF_NOEXIST);

  event->ev_type = ENTER_FUTEX;
  event->evd.enter_futex.op = ctx->op;
  event->evd.enter_futex.word_addr = (u64)ctx->wordaddr;

  submit_event(event);

  return 0;
}

SEC("tracepoint/syscalls/sys_exit_futex")
int on_sys_exit_futex(void *ctx) {
  u64 key = bpf_get_current_pid_tgid();

  if (bpf_map_lookup_elem(&filter_as, &key)) {
    bpf_map_delete_elem(&filter_as, &key);

    return handle_exit_arrival_site(ctx);
  }

  return 0;
}

SEC("tracepoint/syscalls/sys_enter_rt_sigtimedwait")
int on_sys_enter_rt_sigtimedwait(void *ctx) {
  struct task_struct *t = NULL;
  struct lime_event *event;

  t = (struct task_struct *)bpf_get_current_task();

  if (filter_out_task(t))
    return 0;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return -ENOMEM;

  event->ev_type = ENTER_RT_SIGTIMEDWAIT;
  event->ts = now();
  event->pid_tgid = bpf_get_current_pid_tgid();

  submit_event(event);

  return 0;
}

SEC("tracepoint/syscalls/sys_exit_rt_sigtimedwait")
int on_sys_exit_rt_sigtimedwait(void *ctx) {
  return handle_exit_arrival_site(ctx);
}

SEC("tracepoint/syscalls/sys_enter_rt_sigsuspend")
int on_sys_enter_rt_sigsuspend(void *ctx) {
  struct task_struct *t = NULL;
  struct lime_event *event;

  t = (struct task_struct *)bpf_get_current_task();

  if (filter_out_task(t))
    return 0;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return -ENOMEM;

  event->ev_type = ENTER_RT_SIGSUSPEND;
  event->ts = now();
  event->pid_tgid = bpf_get_current_pid_tgid();

  submit_event(event);

  return 0;
}

SEC("tracepoint/syscalls/sys_exit_rt_sigsuspend")
int on_sys_exit_rt_sigsuspend(void *ctx) {
  return handle_exit_arrival_site(ctx);
}

struct epoll_pwait_args {
  u64 unused[2];
  u64 epfd;
};

SEC("tracepoint/syscalls/sys_enter_epoll_pwait")
int on_sys_enter_epoll_wait(struct epoll_pwait_args *ctx) {
  struct task_struct *t = NULL;
  struct lime_event *event;

  t = (struct task_struct *)bpf_get_current_task();

  if (filter_out_task(t))
    return 0;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return -ENOMEM;

  event->ev_type = ENTER_EPOLL_PWAIT;
  event->ts = now();
  event->pid_tgid = bpf_get_current_pid_tgid();
  event->evd.enter_epoll_pwait.epfd = ctx->epfd;

  submit_event(event);

  return 0;
}

SEC("tracepoint/syscalls/sys_exit_epoll_pwait")
int on_sys_exit_epoll_pwait(void *ctx) { return handle_exit_arrival_site(ctx); }

SEC("tracepoint/syscalls/sys_enter_epoll_pwait2")
int on_sys_enter_epoll_wait2(struct epoll_pwait_args *ctx) {
  struct task_struct *t = NULL;
  struct lime_event *event;

  t = (struct task_struct *)bpf_get_current_task();

  if (filter_out_task(t))
    return 0;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return -ENOMEM;

  event->ev_type = ENTER_EPOLL_PWAIT;
  event->ts = now();
  event->pid_tgid = bpf_get_current_pid_tgid();
  event->evd.enter_epoll_pwait.epfd = ctx->epfd;

  submit_event(event);

  return 0;
}

SEC("tracepoint/syscalls/sys_exit_epoll_pwait2")
int on_sys_exit_epoll_pwait2(void *ctx) {
  return handle_exit_arrival_site(ctx);
}

SEC("tp_btf/signal_deliver")
int on_signal_deliver(u64 *ctx) {
  struct task_struct *t = NULL;
  struct lime_event *event;
  int sig = 0;
  struct kernel_siginfo *siginfo = NULL;

  sig = (int)ctx[0];

  // Ignore non-real-time-signals
  if ((sig < SIGRTMIN) || (sig > SIGRTMAX))
    return 0;

  t = (struct task_struct *)bpf_get_current_task();

  if (filter_out_task(t))
    return 0;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return -ENOMEM;

  event->ev_type = DELIVER_RT_SIGNAL;
  event->ts = now();
  event->pid_tgid = bpf_get_current_pid_tgid();

  siginfo = (struct kernel_siginfo *)ctx[1];

  event->evd.deliver_rt_sig.signo = sig;
  event->evd.deliver_rt_sig.si_code = siginfo->si_code;

  submit_event(event);

  return 0;
}

SEC("tracepoint/syscalls/sys_enter_rt_sigreturn")
int on_sys_exit_rt_sigreturn(void *ctx) {
  struct task_struct *t = NULL;
  struct lime_event *event;

  t = (struct task_struct *)bpf_get_current_task();

  if (filter_out_task(t))
    return 0;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return -ENOMEM;

  event->ev_type = ENTER_RT_SIGRETURN;
  event->ts = now();
  event->pid_tgid = bpf_get_current_pid_tgid();

  submit_event(event);

  return 0;
}

SEC("tracepoint/syscalls/sys_enter_pause")
int on_sys_enter_pause(void *ctx) {
  struct task_struct *t = NULL;
  struct lime_event *event;

  t = (struct task_struct *)bpf_get_current_task();

  if (filter_out_task(t))
    return 0;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return -ENOMEM;

  event->ev_type = ENTER_PAUSE;
  event->ts = now();
  event->pid_tgid = bpf_get_current_pid_tgid();

  submit_event(event);

  return 0;
}

SEC("tracepoint/syscalls/sys_exit_pause")
int on_sys_exit_pause(void *ctx) { return handle_exit_arrival_site(ctx); }

struct enter_mq_timedreceive_args {
  u64 unused[2];

  u64 mqdes;
  u64 u_msg_ptr;
  u64 msg_len;
  u64 u_msg_prio;
};

SEC("tracepoint/syscalls/sys_enter_mq_timedreceive")
int on_sys_enter_mq_timedreceive(struct enter_mq_timedreceive_args *ctx) {
  struct task_struct *t = NULL;
  struct lime_event *event;

  t = (struct task_struct *)bpf_get_current_task();

  if (filter_out_task(t))
    return 0;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return -ENOMEM;

  event->ev_type = ENTER_MQ_TIMEDRECEIVE;
  event->ts = now();
  event->pid_tgid = bpf_get_current_pid_tgid();

  event->evd.enter_mq_timedreceive.mqd = ctx->mqdes;

  submit_event(event);

  return 0;
}

SEC("tracepoint/syscalls/sys_exit_mq_timedreceive")
int on_sys_exit_mq_timedreceive(void *ctx) {
  return handle_exit_arrival_site(ctx);
}

#define MSG_DONTWAIT 0x40

struct enter_recv_args {
  u64 unused[2];
  u64 sock_fd;
  u64 ubuff;
  u64 flags;
};

// TODO write separate handlers for every function of the recv family
static inline int on_enter_recv_common(struct enter_recv_args *ctx,
                                       event_type_t ev_type, int no_wait) {
  struct task_struct *t = NULL;
  struct lime_event *event;
  struct file *f;
  u8 val = 0;
  u64 pid_tgid;
  u32 flags;

  if (no_wait) {
    return 0;
  }

  t = (struct task_struct *)bpf_get_current_task();

  if (filter_out_task(t))
    return 0;

  f = get_struct_file(t, ctx->sock_fd);
  bpf_core_read(&flags, sizeof(flags), &f->f_flags);

  if (flags & O_NONBLOCK)
    return 0;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return -ENOMEM;

  event->ev_type = ev_type;
  event->ts = now();
  pid_tgid = bpf_get_current_pid_tgid();

  bpf_map_update_elem(&filter_as, &pid_tgid, &val, BPF_NOEXIST);

  event->pid_tgid = pid_tgid;
  event->evd.enter_recv.sock_fd = (int)ctx->sock_fd;

  submit_event(event);

  return 0;
}

SEC("tracepoint/syscalls/sys_enter_recv")
int on_sys_enter_recv(struct enter_recv_args *ctx) {
  int no_wait = ctx->flags & 0x40;

  return on_enter_recv_common(ctx, ENTER_RECV, no_wait);
}

SEC("tracepoint/syscalls/sys_exit_recv")
int on_sys_exit_recv(void *ctx) {
  u64 key = bpf_get_current_pid_tgid();

  if (bpf_map_lookup_elem(&filter_as, &key)) {
    bpf_map_delete_elem(&filter_as, &key);

    return handle_exit_arrival_site(ctx);
  }

  return 0;
}

SEC("tracepoint/syscalls/sys_enter_recvfrom")
int on_sys_enter_recvfrom(struct enter_recv_args *ctx) {
  int no_wait = ctx->flags & 0x40;

  return on_enter_recv_common(ctx, ENTER_RECVFROM, no_wait);
}

SEC("tracepoint/syscalls/sys_exit_recvfrom")
int on_sys_exit_recvfrom(void *ctx) {
  u64 key = bpf_get_current_pid_tgid();

  if (bpf_map_lookup_elem(&filter_as, &key)) {
    bpf_map_delete_elem(&filter_as, &key);

    return handle_exit_arrival_site(ctx);
  }

  return 0;
}

struct enter_recvmsg_args {
  u64 unused[2];
  u64 sock_fd;
  u64 msghdr;
  u64 flags;
};

SEC("tracepoint/syscalls/sys_enter_recvmsg")
int on_sys_enter_recvmsg(struct enter_recv_args *ctx) {
  int no_wait = ctx->flags & MSG_DONTWAIT;

  return on_enter_recv_common(ctx, ENTER_RECVMSG, no_wait);
}

SEC("tracepoint/syscalls/sys_exit_recvmsg")
int on_sys_exit_recvmsg(void *ctx) {
  u64 key = bpf_get_current_pid_tgid();

  if (bpf_map_lookup_elem(&filter_as, &key)) {
    bpf_map_delete_elem(&filter_as, &key);

    return handle_exit_arrival_site(ctx);
  }

  return 0;
}

SEC("tracepoint/syscalls/sys_enter_recvmmsg")
int on_sys_enter_recvmmsg(struct enter_recv_args *ctx) {
  int no_wait = ctx->flags & MSG_DONTWAIT;

  return on_enter_recv_common(ctx, ENTER_RECVMMSG, no_wait);
}

SEC("tracepoint/syscalls/sys_exit_recvmmsg")
int on_sys_exit_recvmmsg(void *ctx) {
  u64 key = bpf_get_current_pid_tgid();

  if (bpf_map_lookup_elem(&filter_as, &key)) {
    bpf_map_delete_elem(&filter_as, &key);

    return handle_exit_arrival_site(ctx);
  }

  return 0;
}

struct msgrcv_args {
  u64 unused[2];
  u64 msqid;
};

SEC("tracepoint/syscalls/sys_enter_msgrcv")
int on_sys_enter_msgrcv(struct msgrcv_args *ctx) {
  struct task_struct *t = NULL;
  struct lime_event *event;

  t = (struct task_struct *)bpf_get_current_task();

  if (filter_out_task(t))
    return 0;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return -ENOMEM;

  event->ev_type = ENTER_MSGRCV;
  event->ts = now();
  event->pid_tgid = bpf_get_current_pid_tgid();

  event->evd.enter_msgrcv.msqid = ctx->msqid;

  submit_event(event);

  return 0;
}

SEC("tracepoint/syscalls/sys_exit_msgrcv")
int on_sys_exit_msgrcv(void *ctx) { return handle_exit_arrival_site(ctx); }

#define IPC_NOWAIT 00004000
#define MAX_TSOPS 16

struct semop_args {
  u64 unused[2];
  u64 semid;
  u64 tsops;
  u64 nsops;
  u64 timeout;
};

static inline int do_on_sys_enter_semop(struct semop_args *ctx, int timed) {
  struct task_struct *t = NULL;
  struct lime_event *event;
  struct sembuf *tsops, *tsop;
  int sem_op;
  int sem_flg;
  u64 ts;
  u64 pid_tgid;
  bool blocking = false;
  u64 timeout = 0;
  struct __kernel_timespec *timeout_ts;

  int n = MAX_TSOPS;

  t = (struct task_struct *)bpf_get_current_task();

  if (filter_out_task(t))
    return 0;

  ts = now();
  pid_tgid = bpf_get_current_pid_tgid();

  if (ctx->nsops <= n) {
    n = ctx->nsops;
  }

  tsops = (struct sembuf *)ctx->tsops;

  for (int i = 0; i < n; i++) {
    bpf_core_read_user(&tsop, sizeof(tsop), &tsops[i]);
    bpf_core_read_user(&sem_op, sizeof(sem_op), &tsop->sem_op);
    bpf_core_read_user(&sem_flg, sizeof(sem_flg), &tsop->sem_flg);

    if ((sem_op <= 0) && !(sem_flg & IPC_NOWAIT)) {
      blocking = true;
      break;
    }
  }

  if (!blocking)
    return 0;

  event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
  if (!event)
    return -ENOMEM;

  event->ev_type = ENTER_SEMOP;
  event->ts = ts;
  event->pid_tgid = pid_tgid;
  event->evd.enter_semop.sem_id = (int)ctx->semid;
  u64 tmp;

  if (timed && ctx->timeout) {
    struct __kernel_timespec *timeout_ts = (void *)ctx->timeout;

    bpf_core_read_user(&tmp, sizeof(tmp), &timeout_ts->tv_sec);
    timeout += tmp * 1000000000;

    bpf_core_read_user(&tmp, sizeof(tmp), &timeout_ts->tv_nsec);
    timeout += tmp;
  }

  event->evd.enter_semop.timeout = timeout;

  submit_event(event);

  return 0;
}

SEC("tracepoint/syscalls/sys_enter_semop")
int on_sys_enter_semop(struct semop_args *ctx) {
  return do_on_sys_enter_semop(ctx, 0);
}

SEC("tracepoint/syscalls/sys_enter_semtimedop")
int on_sys_enter_semtimedop(struct semop_args *ctx) {
  return do_on_sys_enter_semop(ctx, 1);
}

SEC("tracepoint/syscalls/sys_exit_semtimedop")
int on_sys_exit_semtimedop(void *ctx) { return handle_exit_arrival_site(ctx); }

SEC("tracepoint/syscalls/sys_exit_semop")
int on_sys_exit_semop(void *ctx) { return handle_exit_arrival_site(ctx); }

const char LICENSE[] SEC("license") = "GPL";
