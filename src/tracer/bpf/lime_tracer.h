#ifndef __LIME_TRACER_H
#define __LIME_TRACER_H

#include "vmlinux.h"

typedef enum clock_id {
    CLOCK_REALTIME,
    CLOCK_MONOTONIC,			    
    CLOCK_PROCESS_CPUTIME_ID,
    CLOCK_THREAD_CPUTIME_ID,
    CLOCK_MONOTONIC_RAW,    
    CLOCK_REALTIME_COARSE,		
    CLOCK_MONOTONIC_COARSE,		
    CLOCK_BOOTTIME,			    
    CLOCK_REALTIME_ALARM,		
    CLOCK_BOOTTIME_ALARM,		
    CLOCK_SGI_CYCLE,			
    CLOCK_TAI,			
} clock_id;


typedef enum event_type {
    SCHED_SCHEDULE,
    SCHED_DESCHEDULE,
    SCHED_WAKEUP,
    SCHED_WAKING,
    SCHED_WAKEUP_NEW,
    SCHED_MIGRATE_TASK,
    ENTER_CLOCK_NANOSLEEP,
    ENTER_NANOSLEEP,
    ENTER_SELECT,
    ENTER_PSELECT6,
    ENTER_POLL,
    ENTER_PPOLL,
    ENTER_EPOLL_PWAIT,
    ENTER_ACCEPT,
    ENTER_READ_SOCK,
    ENTER_READ_BLK,
    ENTER_READ_FIFO,
    ENTER_READ_CHR,
    ENTER_FUTEX,
    ENTER_RT_SIGTIMEDWAIT,
    EXIT_AS,
    ENTER_RT_SIGRETURN,
    ENTER_RT_SIGSUSPEND,
    ENTER_MQ_TIMEDRECEIVE,
    ENTER_RECV,
    ENTER_RECVFROM,
    ENTER_RECVMSG,
    ENTER_RECVMMSG,
    ENTER_MSGRCV,
    ENTER_SEMOP,
    DELIVER_RT_SIGNAL,
    ENTER_PAUSE,
    SCHED_PROCESS_EXIT,
    SCHED_PROCESS_FORK,
    SCHED_PROCESS_EXEC,

    ENTER_SCHED_SETAFFINITY,
    SCHED_AFFINITY_CHANGE,
    SCHED_AFFINITY_CHANGE_FAILED,

    ENTER_SCHED_SETSCHEDULER,
    ENTER_SCHED_SETATTR,
    SCHED_SCHEDULER_CHANGE,
    SCHED_SCHEDULER_CHANGE_FAILED,
    ENTER_SCHED_YIELD,
    ENTER_DL_TIMER,
} event_type_t;

struct sched_switch {
    __u64 prev;
    __u32 prev_prio;
    __u32 prev_policy;
    __u64 next;
    __u32 next_prio;
    __u32 next_policy;
    __u32 preempt;
    __u32 prev_state;
};

struct sched_deschedule {
    __u32 cpu;
    __u32 prio;
    __u32 state;
};

struct sched_schedule {
    __u32 prio;
    __u32 cpu;
    __u32 preempt;
};

struct sched_wakeup {
    __u32 cpu;
};

struct sched_migrate_task {
    __u32 dest_cpu;
};

struct clock_nanosleep {
    clock_id clock_id;
    bool abs_time;
    __kernel_time64_t rq_sec;
    long long int rq_nsec;
};

struct sched_new_fp_task {
    u32 priority;
};

struct sched_new_deadline_task {
    u32 flags;
    u64 deadline;
    u64 period;
    u64 runtime;
};

struct lime_thread_info {
    u32 cpu;
    u32 policy;
    u32 prio;
};

struct enter_select {
    u64 inp;
    long int tv_sec;
    long int tv_usec;
};

struct enter_pselect6 {
    u64 inp;
    long int tv_sec;
    long int tv_nsec;
};

struct enter_poll {
    u64 pfds;
    int timeout_msecs;
};

struct enter_ppoll {
    u64 pfds;
    long int tv_sec;
    long int tv_nsec;
};

struct enter_read {
  int fd;
};

struct enter_accept {
  int sock_fd;
};

struct enter_futex {
    u32 op;
    u64 word_addr;
};

struct enter_epoll_pwait {
    int epfd;
};

struct enter_nanosleep {
    __kernel_time64_t rq_sec;
    long long int rq_nsec;
};

struct enter_mq_timedreceive {
    int mqd;
};

struct exit_ar {
    int ret;
};

// Define several structs for the whole `recv` family.
struct enter_recv {
    int sock_fd;
};

struct enter_msgrcv {
    int msqid;
};

struct lime_sched_attr {
    __u32 old_policy;
    __u32 policy;

    union {
        struct { 
            __u32 prio;
        } rt;

        struct {
            __u64 runtime;
            __u64 deadline;
            __u64 period;
        } dl;
    } attrs;
};

enum si_code {
    SI_USER = 0,		/* sent by kill, sigsend, raise */
    SI_KERNEL = 0x80,		/* sent by the kernel from somewhere */
    SI_QUEUE = -1,		/* sent by sigqueue */
    SI_TIMER = -2,		/* sent by timer expiration */
    SI_MESGQ = -3,		/* sent by real time mesq state change */
    SI_ASYNCIO = -4,		/* sent by AIO completion */
    SI_SIGIO = -5,		/* sent by queued SIGIO */
    SI_TKILL = -6,		/* sent by tkill system call */
    SI_DETHREAD = -7,		/* sent by execve() killing subsidiary threads */
    SI_ASYNCNL = -60	/* sent by glibc async name lookup completion */
};

struct deliver_rt_sig {
    int signo;
    enum si_code si_code;
};

struct dl_timer {
    u64 expires;
};

struct enter_semop {
    int sem_id;
    u64 timeout;
};

struct lime_event {
    event_type_t ev_type;
	__u64 pid_tgid;
    __u64 ts;

    union {
        // scheduling events
        struct sched_schedule sched_schedule;
        struct sched_deschedule sched_deschedule;
        struct sched_wakeup sched_wakeup;
        struct sched_migrate_task sched_migrate_task;
        struct sched_new_deadline_task sched_new_deadline_task;
        struct sched_new_fp_task sched_new_fp_task;

        // Thread info
        struct lime_thread_info thread_info;

        // arrival sites
        struct clock_nanosleep clock_nanosleep;
        struct enter_nanosleep enter_nanosleep;
        struct enter_select enter_select;
        struct enter_pselect6 enter_pselect6;
        struct enter_poll enter_poll;
        struct enter_ppoll enter_ppoll;
        struct enter_epoll_pwait enter_epoll_pwait;
        struct enter_read enter_read;
        struct enter_accept enter_accept;
        struct enter_futex enter_futex;
        struct enter_recv enter_recv;
        struct enter_mq_timedreceive enter_mq_timedreceive;
        struct enter_msgrcv enter_msgrcv;
        struct enter_semop enter_semop;
        struct deliver_rt_sig deliver_rt_sig;
        struct lime_sched_attr sched_attr;
        struct dl_timer dl_timer;


        struct exit_ar exit_ar;
    } evd;
};

struct rt_thread_info {
    __u64 pid_tgid;
    __u32 policy;
    __u32 prio;
    __u32 cpu;
};

#endif