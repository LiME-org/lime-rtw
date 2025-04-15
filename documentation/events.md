The event tracing is based on eBPF.

The following tables define the events being traced in the system.

# Scheduling events

| Event | Definition |
|-------|------------|
| `sched_wake_up_new` | First time the thread is woken up after its creation. |
| `sched_waking` | The thread is being woken up. Unlike `sched_wake_up`, this event is always emitted when `try_to_wakeup()` is invoked. |
| `sched_wake_up` | The thread is woken up: it becomes _ready_, i.e., it is available for dispatching. |
| `sched_switched_in` | The thread is scheduled: the thread starts being executed on a core. |
| `sched_switched_out` | The thread is descheduled: the thread ceases execution. There are two cases: (1) if the `state` attribute is `0`, then the thread was _preempted_ (and remains ready); (2) if the `state` attribute is `> 0`, then the thread suspended execution (and cannot be dispatched until the next `sched_wake_up` occurs). |
| `sched_migrate_task` | The thread is migrated to another processor. This does not mean that it immediately starts execution on another processor. It merely means that it now "belongs" to another core's queue, i.e., the thread's state is protected by the new core's runqueue lock. |
| `sched_set_scheduler` | The scheduling policy of this thread has been changed with `sched_set_scheduler`. |

# Arrival site events

| Event | Definition |
|-------|------------|
| `enter_accept` | The thread invokes `accept` or `accept4`. |
| `enter_clock_nano_sleep` | The thread invokes `clock_nanosleep`. |
| `enter_futex` | The thread invokes `futex`. This event should be broken down in several one depending on the futex operation. Only emitted upon waiting or waking operations. |
| `enter_poll` | The thread invokes `poll` |
| `enter_read_blk` | The thread invokes `read` on a block device. Only emitted on blocking reads. |
| `enter_read_chr` | The thread invokes `read` on a char device. Only emitted on blocking reads. |
| `enter_read_fifo` | The thread invokes `read` on a FIFO. Only emitted on blocking reads. |
| `enter_read_sock` | The thread invokes `read` on a socket. Only emitted on blocking reads. |
| `enter_select` | The thread invokes `select`. |
| `enter_sig_timed_wait` | The thread invokes `sigtimedwait`. |
| `exit_arrival_site` | The invocation of an arrival function completes. |

# Example Trace

Here is the beginning of a task invoking `clock_nanosleep()` to enact _relative_ sleeps (which does _not_ result in periodic behavior, e.g., [see here for an explanation](https://people.mpi-sws.org/\~bbb/writing/2020-09-Liu-and-Layland-and-Linux/)).

```json
  {
    "first_cpu": 1,
    "pid": 446,
    "tgid": 438,
    "comm": "multipathd",
    "cmd": "/sbin/multipathd -d -s",
    "first_policy": 2,
    "first_prio": 99,
    "events": [
      {
        "ts": 62185348876214,
        "event": "sched_wake_up",
        "cpu": 1
      },
      {
        "ts": 62185349090871,
        "event": "sched_switched_in",
        "cpu": 1,
        "prio": 99,
        "preempt": false
      },
      {
        "ts": 62185349146179,
        "event": "exit_arrival_site"
      },
      {
        "ts": 62185349230283,
        "event": "enter_clock_nano_sleep",
        "clock_id": 0,
        "abs_time": false,
        "required_ns": 1000000000
      },
      {
        "ts": 62185349313746,
        "event": "sched_switched_out",
        "cpu": 1,
        "prio": 99,
        "state": 1
      },
      {
        "ts": 62186384492632,
        "event": "sched_wake_up",
        "cpu": 1
      },
      {
        "ts": 62186384725520,
        "event": "sched_switched_in",
        "cpu": 1,
        "prio": 99,
        "preempt": false
      },
      {
        "ts": 62186384901042,
        "event": "exit_arrival_site"
      },
[...]
```