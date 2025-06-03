//! Traced events definition
//!
//! Serialization from raw trace events is implemented by the tracer module

use serde::{Deserialize, Serialize};
use serde_hex::{CompactPfx, SerHex};

use crate::proto;
use crate::utils::ThreadId;

#[repr(i32)]
#[derive(PartialEq, Eq, Debug, Serialize, Deserialize, Clone, Copy, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ClockId {
    ClockRealtime = 0,
    ClockMonotonic = 1,
    ClockProcessCputimeId = 2,
    ClockThreadCputimeId = 3,
    ClockMonotonicRaw = 4,
    ClockRealtimeCoarse = 5,
    ClockMonotonicCoarse = 6,
    ClockBoottime = 7,
    ClockRealtimeAlarm = 8,
    ClockBoottimeAlarm = 9,
    ClockSgiCycle = 10,
    ClockTai = 11,
}

#[derive(PartialEq, Eq, Debug, Serialize, Deserialize, Clone, Copy)]
pub enum SchedulingPolicy {
    #[serde(rename = "SCHED_FIFO")]
    Fifo { prio: u32 },
    #[serde(rename = "SCHED_RR")]
    RoundRobin { prio: u32 },
    #[serde(rename = "SCHED_DEADLINE")]
    Deadline {
        runtime: u64,
        period: u64,
        deadline: u64,
    },
    #[serde(rename = "SCHED_OTHER")]
    Other,
    #[serde(rename = "UNKNOWN")]
    Unknown,
}

const SCHED_FIFO: i32 = 1;
const SCHED_RR: i32 = 2;
const SCHED_DEADLINE: i32 = 6;
const SCHED_OTHER: i32 = 0;
const SCHED_UNKNOWN: i32 = -1;

impl SchedulingPolicy {
    pub fn policy_num(&self) -> i32 {
        match self {
            SchedulingPolicy::Fifo { .. } => SCHED_FIFO,
            SchedulingPolicy::RoundRobin { .. } => SCHED_RR,
            SchedulingPolicy::Deadline { .. } => SCHED_DEADLINE,
            SchedulingPolicy::Other => SCHED_OTHER,
            SchedulingPolicy::Unknown => SCHED_UNKNOWN,
        }
    }

    pub fn is_rt_policy(&self) -> bool {
        matches!(
            self,
            SchedulingPolicy::Fifo { .. }
                | SchedulingPolicy::RoundRobin { .. }
                | SchedulingPolicy::Deadline { .. }
        )
    }

    pub fn same_policy(&self, other: &SchedulingPolicy) -> bool {
        match self {
            SchedulingPolicy::Fifo { .. } => matches!(other, SchedulingPolicy::Fifo { .. }),
            SchedulingPolicy::RoundRobin { .. } => {
                matches!(other, SchedulingPolicy::RoundRobin { .. })
            }
            SchedulingPolicy::Deadline { .. } => matches!(other, SchedulingPolicy::Deadline { .. }),
            _ => self.eq(other),
        }
    }
}

#[cfg(target_os = "linux")]
impl From<&nc::sched_attr_t> for SchedulingPolicy {
    fn from(attr: &nc::sched_attr_t) -> Self {
        match attr.sched_policy as i32 {
            SCHED_FIFO => SchedulingPolicy::Fifo {
                prio: attr.sched_priority,
            },
            SCHED_RR => SchedulingPolicy::RoundRobin {
                prio: attr.sched_priority,
            },
            SCHED_DEADLINE => SchedulingPolicy::Deadline {
                runtime: attr.sched_runtime,
                period: attr.sched_period,
                deadline: attr.sched_deadline,
            },
            _ => SchedulingPolicy::Other,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq, Clone)]
pub struct TraceEvent {
    pub ts: u64,
    #[serde(skip)]
    pub id: ThreadId,
    #[serde(flatten)]
    pub ev: EventData,
}

impl PartialOrd for TraceEvent {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TraceEvent {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.ts.cmp(&other.ts)
    }
}

impl TraceEvent {
    pub fn start_of_trace() -> Self {
        Self {
            ts: 0,
            id: ThreadId { pid: 0, tgid: 0 },
            ev: EventData::LimeStartOfTrace,
        }
    }

    pub fn is_syscall_entry(&self) -> bool {
        matches!(
            self.ev,
            EventData::EnterAccept { .. }
                | EventData::EnterClockNanoSleep { .. }
                | EventData::EnterFutex { .. }
                | EventData::EnterPoll { .. }
                | EventData::EnterReadBlk { .. }
                | EventData::EnterReadChr { .. }
                | EventData::EnterReadFifo { .. }
                | EventData::EnterReadSock { .. }
                | EventData::EnterSelect { .. }
                | EventData::EnterPselect6 { .. }
                | EventData::EnterEpollPWait { .. }
                | EventData::EnterSigTimedWait { .. }
                | EventData::EnterNanosleep { .. }
                | EventData::EnterPause {}
                | EventData::EnterRtSigsuspend {}
                | EventData::EnterMqTimedreceive { .. }
                | EventData::EnterRecv { .. }
                | EventData::EnterRecvfrom { .. }
                | EventData::EnterRecvmsg { .. }
                | EventData::EnterRecvmmsg { .. }
                | EventData::EnterMsgrcv { .. }
                | EventData::EnterSemop { .. }
        )
    }

    pub fn is_suspension(&self) -> bool {
        match self.ev {
            EventData::SchedSwitchedOut { state, .. } => state != 0,
            _ => false,
        }
    }

    pub fn is_wakeup(&self) -> bool {
        matches!(self.ev, EventData::SchedWakeUp { .. })
    }

    pub fn is_rt_sigreturn(&self) -> bool {
        matches!(self.ev, EventData::EnterRtSigreturn {})
    }

    pub fn is_dl_timer(&self) -> bool {
        matches!(self.ev, EventData::EnterDlTimer { .. })
    }

    pub fn is_process_exit(&self) -> bool {
        matches!(self.ev, EventData::SchedProcessExit)
    }
}

/// Event-specific data.
#[derive(Debug, Serialize, Deserialize, Eq, PartialEq, Clone)]
#[serde(rename_all = "snake_case", tag = "event")]
pub enum EventData {
    SchedWakeUp {
        cpu: u32,
    },

    SchedWakeUpNew {
        cpu: u32,
    },

    SchedWaking {
        cpu: u32,
    },

    SchedSwitchedIn {
        cpu: u32,
        prio: u32,
        preempt: bool,
    },

    SchedSwitchedOut {
        cpu: u32,
        prio: u32,
        state: u32,
    },

    SchedMigrateTask {
        dest_cpu: u32,
    },

    /// Emitted on blockign system call exit. Always paired with a preceding
    /// system call entry.
    ExitArrivalSite {
        #[serde(default)]
        ret: i32,
    },

    EnterClockNanoSleep {
        clock_id: ClockId,
        abs_time: bool,
        required_ns: i64,
    },

    EnterSelect {
        #[serde(with = "SerHex::<CompactPfx>")]
        inp: u64,
        timeout_usec: i64,
        tvp_null: bool,
    },

    EnterPselect6 {
        #[serde(with = "SerHex::<CompactPfx>")]
        inp: u64,
        timeout_nsec: i64,
        tsp_null: bool,
    },

    EnterPoll {
        #[serde(with = "SerHex::<CompactPfx>")]
        pfds: u64,
        timeout_nsec: i64,
    },

    EnterEpollPWait {
        epfd: u32,
    },

    SchedSetScheduler {
        policy: u32,
        prio: u32,
    },

    EnterReadSock {
        fd: u32,
    },

    EnterReadFifo {
        fd: u32,
    },

    EnterReadBlk {
        fd: u32,
    },

    EnterReadChr {
        fd: u32,
    },

    EnterAccept {
        sock_fd: u32,
    },

    EnterFutex {
        uaddr: u64,
        op: u32,
    },

    EnterSigTimedWait {},

    EnterRtSigreturn {},

    EnterNanosleep {
        required_ns: i64,
    },

    EnterPause {},

    EnterRtSigsuspend {
        // TODO sigset
    },

    EnterMqTimedreceive {
        mqd: i32,
    },

    RtSigDelivered {
        signo: i32,
        si_code: i32, // TODO fancy enum for serialization
    },

    EnterRecv {
        sock_fd: i32,
    },

    EnterRecvfrom {
        sock_fd: i32,
    },

    EnterRecvmsg {
        sock_fd: i32,
    },

    EnterRecvmmsg {
        sock_fd: i32,
    },

    EnterMsgrcv {
        msqid: i32,
    },

    EnterSchedYield {},

    EnterDlTimer {
        expires: u64,
    },

    EnterSetScheduler {
        #[serde(default)]
        old_policy_num: i32,
        sched_policy: SchedulingPolicy,
    },

    EnterSemop {
        sem_id: i32,
        timeout: u64,
    },

    /// Emitted when exiting a scheduler change system call. On success, the
    /// current task will end with `SchedulerChange` event, and new one will
    /// start with `SchedulerChanged` event. This event is a pseudo event. It
    /// should not be observed in regular LiME's output.
    RawSchedulerChange,
    /// Last event emitted by a task on scheduler change.
    SchedulerChange {
        sched_policy: SchedulingPolicy,
    },
    /// Emitted on scheduler change failure.
    SchedulerChangeFailed,
    /// First event emitted by a task created after succesful scheduler change.
    SchedulerChanged {
        sched_policy: SchedulingPolicy,
    },

    SchedParamsChange {
        sched_policy: SchedulingPolicy,
    },
    /// First event emitted by a task created after succesful scheduler change.
    SchedParamsChanged {
        sched_policy: SchedulingPolicy,
    },

    SchedProcessExit,
    SchedProcessFork {
        sched_policy: SchedulingPolicy,
    },
    SchedProcessExec,

    EnterSchedSetAffinity,

    /// Last event emitted by a task on processor affinity change.
    AffinityChange,
    AffinityChangeFailed,
    /// First event emitted by a task created after succesful processor affinity
    /// change.
    AffinityChanged,

    LimeThrottle,
    LimeThrottleRelease,
    LimeStartOfTrace,
    LimeEndOfTrace,
}

// Implement conversion from TraceEvent to protobuf message
impl From<&TraceEvent> for proto::TraceEvent {
    fn from(event: &TraceEvent) -> Self {
        proto::TraceEvent {
            ts: event.ts,
            id: Some(proto::ThreadId {
                pid: event.id.pid,
                tgid: event.id.tgid,
            }),
            event: Some((&event.ev).into()),
        }
    }
}

impl From<&EventData> for proto::EventData {
    fn from(event: &EventData) -> Self {
        use proto::event_data::Event;

        let event = match event {
            EventData::SchedWakeUp { cpu } => Event::SchedWakeUp(proto::SchedWakeUp { cpu: *cpu }),
            EventData::SchedWakeUpNew { cpu } => {
                Event::SchedWakeUpNew(proto::SchedWakeUpNew { cpu: *cpu })
            }
            EventData::SchedWaking { cpu } => Event::SchedWaking(proto::SchedWaking { cpu: *cpu }),
            EventData::SchedSwitchedIn { cpu, prio, preempt } => {
                Event::SchedSwitchedIn(proto::SchedSwitchedIn {
                    cpu: *cpu,
                    prio: *prio,
                    preempt: *preempt,
                })
            }
            EventData::SchedSwitchedOut { cpu, prio, state } => {
                Event::SchedSwitchedOut(proto::SchedSwitchedOut {
                    cpu: *cpu,
                    prio: *prio,
                    state: *state,
                })
            }
            EventData::SchedMigrateTask { dest_cpu } => {
                Event::SchedMigrateTask(proto::SchedMigrateTask {
                    dest_cpu: *dest_cpu,
                })
            }
            EventData::ExitArrivalSite { ret } => {
                Event::ExitArrivalSite(proto::ExitArrivalSite { ret: *ret })
            }
            EventData::EnterClockNanoSleep {
                clock_id,
                abs_time,
                required_ns,
            } => Event::EnterClockNanoSleep(proto::EnterClockNanoSleep {
                clock_id: *clock_id as i32,
                abs_time: *abs_time,
                required_ns: *required_ns,
            }),
            EventData::EnterSelect {
                inp,
                timeout_usec,
                tvp_null,
            } => Event::EnterSelect(proto::EnterSelect {
                inp: *inp,
                timeout_usec: *timeout_usec,
                tvp_null: *tvp_null,
            }),
            EventData::EnterPselect6 {
                inp,
                timeout_nsec,
                tsp_null,
            } => Event::EnterPselect6(proto::EnterPselect6 {
                inp: *inp,
                timeout_nsec: *timeout_nsec,
                tsp_null: *tsp_null,
            }),
            EventData::EnterPoll { pfds, timeout_nsec } => Event::EnterPoll(proto::EnterPoll {
                pfds: *pfds,
                timeout_nsec: *timeout_nsec,
            }),
            EventData::EnterEpollPWait { epfd } => {
                Event::EnterEpollPwait(proto::EnterEpollPWait { epfd: *epfd })
            }
            EventData::SchedSetScheduler { policy, prio } => {
                Event::SchedSetScheduler(proto::SchedSetScheduler {
                    policy: *policy,
                    prio: *prio,
                })
            }
            EventData::EnterReadSock { fd } => {
                Event::EnterReadSock(proto::EnterReadSock { fd: *fd })
            }
            EventData::EnterReadFifo { fd } => {
                Event::EnterReadFifo(proto::EnterReadFifo { fd: *fd })
            }
            EventData::EnterReadBlk { fd } => Event::EnterReadBlk(proto::EnterReadBlk { fd: *fd }),
            EventData::EnterReadChr { fd } => Event::EnterReadChr(proto::EnterReadChr { fd: *fd }),
            EventData::EnterAccept { sock_fd } => {
                Event::EnterAccept(proto::EnterAccept { sock_fd: *sock_fd })
            }
            EventData::EnterFutex { uaddr, op } => Event::EnterFutex(proto::EnterFutex {
                uaddr: *uaddr,
                op: *op,
            }),
            EventData::EnterSigTimedWait {} => {
                Event::EnterSigTimedWait(proto::EnterSigTimedWait {})
            }
            EventData::EnterRtSigreturn {} => Event::EnterRtSigreturn(proto::EnterRtSigreturn {}),
            EventData::EnterNanosleep { required_ns } => {
                Event::EnterNanosleep(proto::EnterNanosleep {
                    required_ns: *required_ns,
                })
            }
            EventData::EnterPause {} => Event::EnterPause(proto::EnterPause {}),
            EventData::EnterRtSigsuspend {} => {
                Event::EnterRtSigsuspend(proto::EnterRtSigsuspend {})
            }
            EventData::EnterMqTimedreceive { mqd } => {
                Event::EnterMqTimedreceive(proto::EnterMqTimedreceive { mqd: *mqd })
            }
            EventData::RtSigDelivered { signo, si_code } => {
                Event::RtSigDelivered(proto::RtSigDelivered {
                    signo: *signo,
                    si_code: *si_code,
                })
            }
            EventData::EnterRecv { sock_fd } => {
                Event::EnterRecv(proto::EnterRecv { sock_fd: *sock_fd })
            }
            EventData::EnterRecvfrom { sock_fd } => {
                Event::EnterRecvfrom(proto::EnterRecvfrom { sock_fd: *sock_fd })
            }
            EventData::EnterRecvmsg { sock_fd } => {
                Event::EnterRecvmsg(proto::EnterRecvmsg { sock_fd: *sock_fd })
            }
            EventData::EnterRecvmmsg { sock_fd } => {
                Event::EnterRecvmmsg(proto::EnterRecvmmsg { sock_fd: *sock_fd })
            }
            EventData::EnterMsgrcv { msqid } => {
                Event::EnterMsgrcv(proto::EnterMsgrcv { msqid: *msqid })
            }
            EventData::EnterSchedYield {} => Event::EnterSchedYield(proto::EnterSchedYield {}),
            EventData::EnterDlTimer { expires } => {
                Event::EnterDlTimer(proto::EnterDlTimer { expires: *expires })
            }
            EventData::EnterSetScheduler {
                old_policy_num,
                sched_policy,
            } => Event::EnterSetScheduler(proto::EnterSetScheduler {
                old_policy_num: *old_policy_num,
                sched_policy: Some(sched_policy.into()),
            }),
            EventData::EnterSemop { sem_id, timeout } => Event::EnterSemop(proto::EnterSemop {
                sem_id: *sem_id,
                timeout: *timeout,
            }),
            EventData::RawSchedulerChange => {
                Event::RawSchedulerChange(proto::RawSchedulerChange {})
            }
            EventData::SchedulerChange { sched_policy } => {
                Event::SchedulerChange(proto::SchedulerChange {
                    sched_policy: Some(sched_policy.into()),
                })
            }
            EventData::SchedulerChangeFailed => {
                Event::SchedulerChangeFailed(proto::SchedulerChangeFailed {})
            }
            EventData::SchedulerChanged { sched_policy } => {
                Event::SchedulerChanged(proto::SchedulerChanged {
                    sched_policy: Some(sched_policy.into()),
                })
            }
            EventData::SchedParamsChange { sched_policy } => {
                Event::SchedParamsChange(proto::SchedParamsChange {
                    sched_policy: Some(sched_policy.into()),
                })
            }
            EventData::SchedParamsChanged { sched_policy } => {
                Event::SchedParamsChanged(proto::SchedParamsChanged {
                    sched_policy: Some(sched_policy.into()),
                })
            }
            EventData::SchedProcessExit => Event::SchedProcessExit(proto::SchedProcessExit {}),
            EventData::SchedProcessFork { sched_policy } => {
                Event::SchedProcessFork(proto::SchedProcessFork {
                    sched_policy: Some(sched_policy.into()),
                })
            }
            EventData::SchedProcessExec => Event::SchedProcessExec(proto::SchedProcessExec {}),
            EventData::EnterSchedSetAffinity => {
                Event::EnterSchedSetAffinity(proto::EnterSchedSetAffinity {})
            }
            EventData::AffinityChange => Event::AffinityChange(proto::AffinityChange {}),
            EventData::AffinityChangeFailed => {
                Event::AffinityChangeFailed(proto::AffinityChangeFailed {})
            }
            EventData::AffinityChanged => Event::AffinityChanged(proto::AffinityChanged {}),
            EventData::LimeThrottle => Event::LimeThrottle(proto::LimeThrottle {}),
            EventData::LimeThrottleRelease => {
                Event::LimeThrottleRelease(proto::LimeThrottleRelease {})
            }
            EventData::LimeStartOfTrace => Event::LimeStartOfTrace(proto::LimeStartOfTrace {}),
            EventData::LimeEndOfTrace => Event::LimeEndOfTrace(proto::LimeEndOfTrace {}),
        };

        proto::EventData { event: Some(event) }
    }
}

impl From<&proto::EventData> for EventData {
    fn from(proto_event: &proto::EventData) -> Self {
        match &proto_event.event {
            Some(event) => event.into(),
            None => EventData::LimeEndOfTrace,
        }
    }
}

// Implement conversion for SchedulingPolicy
impl From<&SchedulingPolicy> for proto::SchedulingPolicy {
    fn from(policy: &SchedulingPolicy) -> Self {
        use proto::scheduling_policy::Policy;

        let policy = match policy {
            SchedulingPolicy::Fifo { prio } => Policy::Fifo(proto::FifoPolicy { prio: *prio }),
            SchedulingPolicy::RoundRobin { prio } => {
                Policy::RoundRobin(proto::RoundRobinPolicy { prio: *prio })
            }
            SchedulingPolicy::Deadline {
                runtime,
                period,
                deadline,
            } => Policy::Deadline(proto::DeadlinePolicy {
                runtime: *runtime,
                period: *period,
                deadline: *deadline,
            }),
            SchedulingPolicy::Other => Policy::Other(proto::OtherPolicy {}),
            SchedulingPolicy::Unknown => Policy::Unknown(proto::UnknownPolicy {}),
        };

        proto::SchedulingPolicy {
            policy: Some(policy),
        }
    }
}

impl From<&proto::event_data::Event> for EventData {
    fn from(event: &proto::event_data::Event) -> Self {
        use proto::event_data::Event;
        match event {
            Event::SchedWakeUp(e) => EventData::SchedWakeUp { cpu: e.cpu },
            Event::SchedWakeUpNew(e) => EventData::SchedWakeUpNew { cpu: e.cpu },
            Event::SchedWaking(e) => EventData::SchedWaking { cpu: e.cpu },
            Event::SchedSwitchedIn(e) => EventData::SchedSwitchedIn {
                cpu: e.cpu,
                prio: e.prio,
                preempt: e.preempt,
            },
            Event::SchedSwitchedOut(e) => EventData::SchedSwitchedOut {
                cpu: e.cpu,
                prio: e.prio,
                state: e.state,
            },
            Event::SchedMigrateTask(e) => EventData::SchedMigrateTask {
                dest_cpu: e.dest_cpu,
            },
            Event::ExitArrivalSite(e) => EventData::ExitArrivalSite { ret: e.ret },
            Event::EnterClockNanoSleep(e) => EventData::EnterClockNanoSleep {
                clock_id: ClockId::from(e.clock_id),
                abs_time: e.abs_time,
                required_ns: e.required_ns,
            },
            Event::EnterSelect(e) => EventData::EnterSelect {
                inp: e.inp,
                timeout_usec: e.timeout_usec,
                tvp_null: e.tvp_null,
            },
            Event::EnterPselect6(e) => EventData::EnterPselect6 {
                inp: e.inp,
                timeout_nsec: e.timeout_nsec,
                tsp_null: e.tsp_null,
            },
            Event::EnterPoll(e) => EventData::EnterPoll {
                pfds: e.pfds,
                timeout_nsec: e.timeout_nsec,
            },
            Event::EnterEpollPwait(e) => EventData::EnterEpollPWait { epfd: e.epfd },
            Event::SchedSetScheduler(e) => EventData::SchedSetScheduler {
                policy: e.policy,
                prio: e.prio,
            },
            Event::EnterReadSock(e) => EventData::EnterReadSock { fd: e.fd },
            Event::EnterReadFifo(e) => EventData::EnterReadFifo { fd: e.fd },
            Event::EnterReadBlk(e) => EventData::EnterReadBlk { fd: e.fd },
            Event::EnterReadChr(e) => EventData::EnterReadChr { fd: e.fd },
            Event::EnterAccept(e) => EventData::EnterAccept { sock_fd: e.sock_fd },
            Event::EnterFutex(e) => EventData::EnterFutex {
                uaddr: e.uaddr,
                op: e.op,
            },
            Event::EnterSigTimedWait(_) => EventData::EnterSigTimedWait {},
            Event::EnterRtSigreturn(_) => EventData::EnterRtSigreturn {},
            Event::EnterNanosleep(e) => EventData::EnterNanosleep {
                required_ns: e.required_ns,
            },
            Event::EnterPause(_) => EventData::EnterPause {},
            Event::EnterRtSigsuspend(_) => EventData::EnterRtSigsuspend {},
            Event::EnterMqTimedreceive(e) => EventData::EnterMqTimedreceive { mqd: e.mqd },
            Event::RtSigDelivered(e) => EventData::RtSigDelivered {
                signo: e.signo,
                si_code: e.si_code,
            },
            Event::EnterRecv(e) => EventData::EnterRecv { sock_fd: e.sock_fd },
            Event::EnterRecvfrom(e) => EventData::EnterRecvfrom { sock_fd: e.sock_fd },
            Event::EnterRecvmsg(e) => EventData::EnterRecvmsg { sock_fd: e.sock_fd },
            Event::EnterRecvmmsg(e) => EventData::EnterRecvmmsg { sock_fd: e.sock_fd },
            Event::EnterMsgrcv(e) => EventData::EnterMsgrcv { msqid: e.msqid },
            Event::EnterSchedYield(_) => EventData::EnterSchedYield {},
            Event::EnterDlTimer(e) => EventData::EnterDlTimer { expires: e.expires },
            Event::EnterSetScheduler(e) => EventData::EnterSetScheduler {
                old_policy_num: e.old_policy_num,
                sched_policy: e.sched_policy.into(),
            },
            Event::EnterSemop(e) => EventData::EnterSemop {
                sem_id: e.sem_id,
                timeout: e.timeout,
            },
            Event::RawSchedulerChange(_) => EventData::RawSchedulerChange,
            Event::SchedulerChange(e) => EventData::SchedulerChange {
                sched_policy: e.sched_policy.into(),
            },
            Event::SchedulerChangeFailed(_) => EventData::SchedulerChangeFailed,
            Event::SchedulerChanged(e) => EventData::SchedulerChanged {
                sched_policy: e.sched_policy.into(),
            },
            Event::SchedParamsChange(e) => EventData::SchedParamsChange {
                sched_policy: e.sched_policy.into(),
            },
            Event::SchedParamsChanged(e) => EventData::SchedParamsChanged {
                sched_policy: e.sched_policy.into(),
            },
            Event::SchedProcessExit(_) => EventData::SchedProcessExit,
            Event::SchedProcessFork(e) => EventData::SchedProcessFork {
                sched_policy: e.sched_policy.into(),
            },
            Event::SchedProcessExec(_) => EventData::SchedProcessExec,
            Event::EnterSchedSetAffinity(_) => EventData::EnterSchedSetAffinity,
            Event::AffinityChange(_) => EventData::AffinityChange,
            Event::AffinityChangeFailed(_) => EventData::AffinityChangeFailed,
            Event::AffinityChanged(_) => EventData::AffinityChanged,
            Event::LimeThrottle(_) => EventData::LimeThrottle,
            Event::LimeThrottleRelease(_) => EventData::LimeThrottleRelease,
            Event::LimeStartOfTrace(_) => EventData::LimeStartOfTrace,
            Event::LimeEndOfTrace(_) => EventData::LimeEndOfTrace,
        }
    }
}

impl From<i32> for ClockId {
    fn from(value: i32) -> Self {
        match value {
            0 => ClockId::ClockRealtime,
            1 => ClockId::ClockMonotonic,
            2 => ClockId::ClockProcessCputimeId,
            3 => ClockId::ClockThreadCputimeId,
            4 => ClockId::ClockMonotonicRaw,
            5 => ClockId::ClockRealtimeCoarse,
            6 => ClockId::ClockMonotonicCoarse,
            7 => ClockId::ClockBoottime,
            8 => ClockId::ClockRealtimeAlarm,
            9 => ClockId::ClockBoottimeAlarm,
            10 => ClockId::ClockSgiCycle,
            11 => ClockId::ClockTai,
            _ => ClockId::ClockRealtime,
        }
    }
}

impl From<Option<proto::SchedulingPolicy>> for SchedulingPolicy {
    fn from(policy: Option<proto::SchedulingPolicy>) -> Self {
        match policy {
            Some(p) => (&p).into(),
            None => SchedulingPolicy::Unknown,
        }
    }
}

impl From<&proto::SchedulingPolicy> for SchedulingPolicy {
    fn from(policy: &proto::SchedulingPolicy) -> Self {
        match &policy.policy {
            Some(proto::scheduling_policy::Policy::Fifo(p)) => {
                SchedulingPolicy::Fifo { prio: p.prio }
            }
            Some(proto::scheduling_policy::Policy::RoundRobin(p)) => {
                SchedulingPolicy::RoundRobin { prio: p.prio }
            }
            Some(proto::scheduling_policy::Policy::Deadline(p)) => SchedulingPolicy::Deadline {
                runtime: p.runtime,
                period: p.period,
                deadline: p.deadline,
            },
            Some(proto::scheduling_policy::Policy::Other(_)) => SchedulingPolicy::Other,
            Some(proto::scheduling_policy::Policy::Unknown(_)) => SchedulingPolicy::Unknown,
            None => SchedulingPolicy::Unknown,
        }
    }
}
