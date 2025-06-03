use nc::{SCHED_DEADLINE, SCHED_FIFO, SCHED_RR};
use serde::Serialize;

use crate::{
    events::{ClockId, EventData, SchedulingPolicy, TraceEvent},
    tracer::{EventType, RawEvent},
    utils::ThreadId,
};

use super::{RawClockId, SiCode};

impl Serialize for SiCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            SiCode::SI_USER => serializer.serialize_unit_variant("", 0, "SI_USER"),
            SiCode::SI_KERNEL => serializer.serialize_unit_variant("", 1, "SI_KERNEL"),
            SiCode::SI_QUEUE => serializer.serialize_unit_variant("", 2, "SI_QUEUE"),
            SiCode::SI_TIMER => serializer.serialize_unit_variant("", 3, "SI_TIMER"),
            SiCode::SI_MESGQ => serializer.serialize_unit_variant("", 4, "SI_MESGQ"),
            SiCode::SI_ASYNCIO => serializer.serialize_unit_variant("", 5, "SI_ASYNCIO"),
            SiCode::SI_SIGIO => serializer.serialize_unit_variant("", 6, "SI_SIGIO"),
            SiCode::SI_TKILL => serializer.serialize_unit_variant("", 7, "SI_TKILL"),
            SiCode::SI_DETHREAD => serializer.serialize_unit_variant("", 8, "SI_DETHREAD"),
            SiCode::SI_ASYNCNL => serializer.serialize_unit_variant("", 9, "SI_ASYNCNL"),
        }
    }
}

impl From<&RawEvent> for TraceEvent {
    fn from(event: &RawEvent) -> Self {
        let ts = event.ts;
        let ev = EventData::from(event);
        let id = ThreadId::from(event.pid_tgid);

        Self { id, ts, ev }
    }
}

impl From<RawClockId> for ClockId {
    fn from(variant: RawClockId) -> Self {
        unsafe { std::mem::transmute(variant as i32) }
    }
}

fn task_state_index(state: u32) -> u32 {
    let mut s = state & 0x7f;

    if state == 0x402 {
        s = 0x80;
    }

    32 - s.leading_zeros()
}

fn switch_prev_state(state: u32) -> u32 {
    let idx = task_state_index(state);

    if idx == 0 {
        return state;
    }

    1 << (idx - 1)
}

impl From<&RawEvent> for EventData {
    fn from(event: &RawEvent) -> EventData {
        match event.ev_type {
            EventType::EXIT_AS => EventData::ExitArrivalSite {
                ret: unsafe { event.evd.exit_ar.ret },
            },

            EventType::ENTER_CLOCK_NANOSLEEP => {
                let mut required_ns = 0;

                required_ns += unsafe { event.evd.clock_nanosleep.rq_sec * 1_000_000_000 };
                required_ns += unsafe { event.evd.clock_nanosleep.rq_nsec };

                EventData::EnterClockNanoSleep {
                    clock_id: unsafe { ClockId::from(event.evd.clock_nanosleep.clock_id) },
                    abs_time: unsafe { event.evd.clock_nanosleep.abs_time },
                    required_ns,
                }
            }
            EventType::ENTER_SELECT => {
                let mut timeout_usec: i64 = unsafe { event.evd.enter_select.tv_sec * 1_000_000 };
                timeout_usec += unsafe { event.evd.enter_select.tv_usec };

                EventData::EnterSelect {
                    inp: unsafe { event.evd.enter_select.inp },
                    timeout_usec,
                    tvp_null: unsafe { event.evd.enter_select.tvp_null },
                }
            }

            EventType::ENTER_PSELECT6 => {
                let mut timeout_nsec: i64 =
                    unsafe { event.evd.enter_pselect6.tv_sec * 1_000_000_000 };
                timeout_nsec += unsafe { event.evd.enter_pselect6.tv_nsec };

                EventData::EnterPselect6 {
                    inp: unsafe { event.evd.enter_select.inp },
                    timeout_nsec,
                    tsp_null: unsafe { event.evd.enter_pselect6.tsp_null },
                }
            }

            EventType::SCHED_WAKEUP => EventData::SchedWakeUp {
                cpu: unsafe { event.evd.sched_wakeup.cpu },
            },

            EventType::SCHED_WAKEUP_NEW => EventData::SchedWakeUpNew {
                cpu: unsafe { event.evd.sched_wakeup.cpu },
            },

            EventType::SCHED_WAKING => EventData::SchedWaking {
                cpu: unsafe { event.evd.sched_wakeup.cpu },
            },

            EventType::SCHED_SCHEDULE => EventData::SchedSwitchedIn {
                cpu: unsafe { event.evd.sched_schedule.cpu },
                prio: unsafe { event.evd.sched_schedule.prio },
                preempt: unsafe { event.evd.sched_schedule.preempt != 0 },
            },

            EventType::SCHED_DESCHEDULE => EventData::SchedSwitchedOut {
                cpu: unsafe { event.evd.sched_deschedule.cpu },
                prio: unsafe { event.evd.sched_deschedule.prio },
                state: switch_prev_state(unsafe { event.evd.sched_deschedule.state }),
            },

            EventType::SCHED_MIGRATE_TASK => EventData::SchedMigrateTask {
                dest_cpu: unsafe { event.evd.sched_migrate_task.dest_cpu },
            },

            EventType::ENTER_POLL => EventData::EnterPoll {
                pfds: unsafe { event.evd.enter_poll.pfds },
                timeout_nsec: unsafe {
                    let t = event.evd.enter_poll.timeout_msecs as i64;
                    if t < 0 {
                        t
                    } else {
                        t * 1_000_000
                    }
                },
            },

            EventType::ENTER_PPOLL => {
                let mut timeout_nsec = 0;

                timeout_nsec += unsafe { event.evd.enter_ppoll.tv_sec * 1_000_000_000 };
                timeout_nsec += unsafe { event.evd.enter_ppoll.tv_nsec };

                EventData::EnterPoll {
                    pfds: unsafe { event.evd.enter_poll.pfds },
                    timeout_nsec,
                }
            }

            EventType::ENTER_EPOLL_PWAIT => EventData::EnterEpollPWait {
                epfd: unsafe { event.evd.enter_epoll_pwait.epfd.try_into().unwrap() },
            },

            EventType::ENTER_READ_SOCK => EventData::EnterReadSock {
                fd: unsafe { event.evd.enter_read.fd as u32 },
            },
            EventType::ENTER_READ_FIFO => EventData::EnterReadFifo {
                fd: unsafe { event.evd.enter_read.fd as u32 },
            },
            EventType::ENTER_READ_BLK => EventData::EnterReadBlk {
                fd: unsafe { event.evd.enter_read.fd as u32 },
            },
            EventType::ENTER_READ_CHR => EventData::EnterReadChr {
                fd: unsafe { event.evd.enter_read.fd as u32 },
            },
            EventType::ENTER_ACCEPT => EventData::EnterAccept {
                sock_fd: unsafe { event.evd.enter_accept.sock_fd as u32 },
            },
            EventType::ENTER_FUTEX => EventData::EnterFutex {
                uaddr: unsafe { event.evd.enter_futex.word_addr },
                op: unsafe { event.evd.enter_futex.op },
            },
            EventType::ENTER_RT_SIGTIMEDWAIT => EventData::EnterSigTimedWait {},

            EventType::ENTER_RT_SIGRETURN => EventData::EnterRtSigreturn {},

            EventType::DELIVER_RT_SIGNAL => EventData::RtSigDelivered {
                signo: unsafe { event.evd.deliver_rt_sig.signo },
                si_code: unsafe { event.evd.deliver_rt_sig.si_code as i32 },
            },
            EventType::ENTER_NANOSLEEP => {
                let mut required_ns = 0;

                required_ns += unsafe { event.evd.clock_nanosleep.rq_sec * 1_000_000_000 };
                required_ns += unsafe { event.evd.clock_nanosleep.rq_nsec };

                EventData::EnterNanosleep { required_ns }
            }

            EventType::ENTER_PAUSE => EventData::EnterPause {},

            EventType::ENTER_RT_SIGSUSPEND => EventData::EnterRtSigsuspend {},

            EventType::ENTER_MQ_TIMEDRECEIVE => EventData::EnterMqTimedreceive {
                mqd: unsafe { event.evd.enter_mq_timedreceive.mqd },
            },

            EventType::ENTER_RECV => EventData::EnterRecv {
                sock_fd: unsafe { event.evd.enter_recv.sock_fd },
            },

            EventType::ENTER_RECVFROM => EventData::EnterRecvfrom {
                sock_fd: unsafe { event.evd.enter_recv.sock_fd },
            },

            EventType::ENTER_RECVMSG => EventData::EnterRecvmsg {
                sock_fd: unsafe { event.evd.enter_recv.sock_fd },
            },

            EventType::ENTER_RECVMMSG => EventData::EnterRecvmmsg {
                sock_fd: unsafe { event.evd.enter_recv.sock_fd },
            },

            EventType::ENTER_MSGRCV => EventData::EnterMsgrcv {
                msqid: unsafe { event.evd.enter_msgrcv.msqid },
            },

            EventType::ENTER_SCHED_SETATTR | EventType::ENTER_SCHED_SETSCHEDULER => {
                EventData::EnterSetScheduler {
                    old_policy_num: unsafe { event.evd.sched_attr.old_policy as i32 },
                    sched_policy: unsafe {
                        let attrs = event.evd.sched_attr.attrs;

                        match event.evd.sched_attr.policy as i32 {
                            SCHED_DEADLINE => SchedulingPolicy::Deadline {
                                runtime: attrs.dl.runtime,
                                period: attrs.dl.period,
                                deadline: attrs.dl.deadline,
                            },

                            SCHED_FIFO => SchedulingPolicy::Fifo {
                                prio: attrs.rt.prio,
                            },

                            SCHED_RR => SchedulingPolicy::RoundRobin {
                                prio: attrs.rt.prio,
                            },

                            _ => SchedulingPolicy::Other,
                        }
                    },
                }
            }

            EventType::ENTER_SCHED_YIELD => EventData::EnterSchedYield {},

            EventType::ENTER_DL_TIMER => EventData::EnterDlTimer {
                expires: unsafe { event.evd.dl_timer.expires },
            },

            EventType::ENTER_SEMOP => EventData::EnterSemop {
                sem_id: unsafe { event.evd.enter_semop.sem_id },
                timeout: unsafe { event.evd.enter_semop.timeout },
            },

            EventType::SCHED_PROCESS_FORK => EventData::SchedProcessFork {
                sched_policy: unsafe {
                    let attrs = event.evd.sched_attr.attrs;

                    match event.evd.sched_attr.policy as i32 {
                        SCHED_DEADLINE => SchedulingPolicy::Deadline {
                            runtime: attrs.dl.runtime,
                            period: attrs.dl.period,
                            deadline: attrs.dl.deadline,
                        },

                        SCHED_FIFO => SchedulingPolicy::Fifo {
                            prio: attrs.rt.prio,
                        },

                        SCHED_RR => SchedulingPolicy::RoundRobin {
                            prio: attrs.rt.prio,
                        },

                        _ => SchedulingPolicy::Other,
                    }
                },
            },
            EventType::SCHED_PROCESS_EXIT => EventData::SchedProcessExit,
            EventType::SCHED_PROCESS_EXEC => EventData::SchedProcessExec,

            EventType::ENTER_SCHED_SETAFFINITY => EventData::EnterSchedSetAffinity,
            EventType::SCHED_AFFINITY_CHANGE => EventData::AffinityChange,
            EventType::SCHED_AFFINITY_CHANGE_FAILED => EventData::AffinityChangeFailed,

            EventType::SCHED_SCHEDULER_CHANGE => EventData::RawSchedulerChange,
            EventType::SCHED_SCHEDULER_CHANGE_FAILED => EventData::SchedulerChangeFailed,
        }
    }
}
