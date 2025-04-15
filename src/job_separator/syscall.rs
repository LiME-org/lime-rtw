//! Signature of blocking syscall-separated jobs.

use crate::events::{ClockId, EventData::*, TraceEvent};

use super::{JobSeparation, JobSeparator, Signature};

pub(crate) enum BlockingSyscall {
    Init,
    Body,
    Exit,
    Reject,
}

fn end_of_syscall(event: &TraceEvent) -> bool {
    matches!(event.ev, LimeEndOfTrace | ExitArrivalSite { .. })
}

fn filter_invalid_futex_ops(event: &TraceEvent) -> bool {
    // TODO clean up hard coded values
    match event.ev {
        EnterFutex { op, .. } => matches!(op & !(128 + 256), 0 | 9),
        _ => true,
    }
}

impl BlockingSyscall {
    fn get_curr_first_cycle(
        &self,
        is_back_to_back: bool,
        prev_end: u64,
        release: u64,
        arrival: &Option<u64>,
    ) -> u64 {
        if is_back_to_back {
            return prev_end;
        }

        if let Some(a) = arrival {
            if a >= &prev_end {
                return *a;
            }
        }

        release
    }

    fn get_curr_arrival(&self, events: &[TraceEvent]) -> Option<u64> {
        let event = events.first()?;

        match event.ev {
            EnterClockNanoSleep {
                abs_time,
                required_ns,
                clock_id: ClockId::ClockMonotonic,
                ..
            } if abs_time => Some(required_ns as u64),
            EnterClockNanoSleep {
                abs_time,
                required_ns,
                clock_id: ClockId::ClockMonotonic,
                ..
            } if !abs_time => Some(event.ts + required_ns as u64),
            _ => None,
        }
    }

    fn futex_separator(op: u32, uaddr: u64) -> JobSeparator {
        // TODO clean up hard coded values
        let clock_id = match op & 256 {
            0 => ClockId::ClockMonotonic,
            _ => ClockId::ClockRealtime,
        };

        match op & !(128 + 256) {
            0 => JobSeparator::FutexWait { uaddr, clock_id },
            9 => JobSeparator::FutexWaitBitSet { uaddr, clock_id },
            _ => unreachable!(),
        }
    }

    fn get_curr_release(
        &self,
        matched_events: &[TraceEvent],
        is_back_to_back: bool,
        arrival: &Option<u64>,
    ) -> u64 {
        let ret = matched_events.first().unwrap().ts;

        if !is_back_to_back {
            let mut first_sleep = false;

            for event in matched_events.iter() {
                match event.ev {
                    SchedSwitchedOut { .. } if event.is_suspension() => first_sleep = true,
                    SchedWakeUp { .. } if first_sleep => return event.ts,
                    LimeEndOfTrace => return 0,
                    _ => {}
                }
            }
        } else if let Some(a) = arrival {
            if *a > ret {
                return *a;
            }
        }

        ret
    }

    fn get_prev_end(&self, matched_events: &[TraceEvent], arrival: &Option<u64>) -> u64 {
        let mut job_end = matched_events.first().unwrap().ts;

        if let Some(a) = arrival {
            if job_end > *a {
                return job_end;
            }
        }

        for event in matched_events.iter().skip(1) {
            match event.ev {
                SchedSwitchedOut { .. } if event.is_suspension() => {
                    job_end = event.ts;

                    if let Some(a) = arrival {
                        if job_end > *a {
                            job_end = *a;
                        }
                    }

                    return job_end;
                }
                LimeEndOfTrace => return job_end,
                _ => {}
            }
        }

        if let Some(a) = arrival {
            if job_end < *a {
                return *a;
            }
        }

        job_end
    }

    fn is_back_to_back(&self, matched_events: &[TraceEvent], arrival: &Option<u64>) -> bool {
        let first = matched_events.first().unwrap().ts;

        if let Some(a) = arrival {
            if *a <= first {
                return true;
            }
        }

        for e in matched_events.iter().skip(1) {
            if e.is_suspension() {
                if let Some(a) = arrival {
                    if e.ts >= *a {
                        return true;
                    }
                }

                return false;
            }
        }

        true
    }
}

impl Signature for BlockingSyscall {
    fn next(&self, event: &TraceEvent) -> Self {
        match self {
            Self::Init if event.is_syscall_entry() && filter_invalid_futex_ops(event) => Self::Body,
            Self::Body if end_of_syscall(event) => Self::Exit,
            Self::Body => Self::Body,
            _ => Self::Reject,
        }
    }

    fn initial_state() -> Self {
        Self::Init
    }

    fn is_terminal_state(&self) -> bool {
        matches!(self, Self::Exit)
    }

    fn is_rejection_state(&self) -> bool {
        matches!(self, Self::Reject)
    }

    fn job_separator(&self, matched_events: &[TraceEvent]) -> JobSeparator {
        match matched_events.first().unwrap().ev {
            EnterClockNanoSleep {
                clock_id, abs_time, ..
            } => JobSeparator::ClockNanosleep { clock_id, abs_time },
            EnterSelect { inp, .. } => JobSeparator::Select { inp },
            EnterPoll { pfds, .. } => JobSeparator::Poll { pfds },
            EnterReadSock { fd } => JobSeparator::ReadSock { fd },
            EnterReadFifo { fd } => JobSeparator::ReadFifo { fd },
            EnterReadBlk { fd } => JobSeparator::ReadBlk { fd },
            EnterReadChr { fd } => JobSeparator::ReadChr { fd },
            EnterAccept { sock_fd } => JobSeparator::Accept { sock_fd },
            EnterFutex { uaddr, op } => Self::futex_separator(op, uaddr),
            EnterSigTimedWait {} => JobSeparator::SigTimedWait,
            EnterEpollPWait { epfd } => JobSeparator::EpollPwait { epfd },
            EnterPselect6 { inp, .. } => JobSeparator::PSelect6 { inp },
            EnterNanosleep { .. } => JobSeparator::Nanosleep {},
            EnterPause {} => JobSeparator::Pause,
            EnterRtSigsuspend {} => JobSeparator::RtSigsuspend,
            EnterMqTimedreceive { mqd } => JobSeparator::MqTimedReceive { mqd },
            EnterRecv { sock_fd } => JobSeparator::Recv { sock_fd },
            EnterRecvfrom { sock_fd } => JobSeparator::Recvfrom { sock_fd },
            EnterRecvmsg { sock_fd } => JobSeparator::Recvmsg { sock_fd },
            EnterRecvmmsg { sock_fd } => JobSeparator::Recvmmsg { sock_fd },
            EnterMsgrcv { msqid } => JobSeparator::Msgrcv { msqid },
            EnterSemop { sem_id, timeout } => JobSeparator::Semop { sem_id, timeout },
            _ => unreachable!(),
        }
    }

    fn job_separation(&self, matched_events: &[TraceEvent]) -> JobSeparation {
        let curr_arrival = self.get_curr_arrival(matched_events);
        let is_back_to_back = self.is_back_to_back(matched_events, &curr_arrival);
        let mut curr_release =
            self.get_curr_release(matched_events, is_back_to_back, &curr_arrival);
        let prev_end = self.get_prev_end(matched_events, &curr_arrival);
        let curr_first_cycle =
            self.get_curr_first_cycle(is_back_to_back, prev_end, curr_release, &curr_arrival);

        if is_back_to_back {
            curr_release = curr_arrival.unwrap_or(curr_release);
        }

        JobSeparation {
            prev_end,
            curr_release,
            curr_arrival,
            curr_first_cycle,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        events::TraceEvent,
        job_separator::{syscall::BlockingSyscall, Signature},
    };

    const MIGRATE_BEFORE_SLEEP: &str = r#"[
    {"ts":0,"event":"enter_clock_nano_sleep","clock_id":"CLOCK_MONOTONIC","abs_time":true,"required_ns":1467100683809847},
    {"ts":1,"event":"sched_switched_out","cpu":3,"prio":76,"state":0},
    {"ts":1467100665172670,"event":"sched_migrate_task","dest_cpu":2},
    {"ts":1467100665179967,"event":"sched_switched_in","cpu":2,"prio":76,"preempt":false},
    {"ts":1467100665190281,"event":"sched_switched_out","cpu":2,"prio":76,"state":1},
    {"ts":1467100683814559,"event":"sched_waking","cpu":2},
    {"ts":1467100683821059,"event":"sched_wake_up","cpu":2},
    {"ts":1467100683831003,"event":"sched_switched_in","cpu":2,"prio":76,"preempt":false},
    {"ts":1467100683836318,"event":"exit_arrival_site","ret":0}
    ]
    "#;

    const CLOCK_NANOSLEEP: &str = r#"[
    {"ts":0,"event":"enter_clock_nano_sleep","clock_id":"CLOCK_MONOTONIC","abs_time":true,"required_ns":10},
    {"ts":1,"event":"sched_switched_out","cpu":0,"prio":2,"state":1},
    {"ts":11,"event":"sched_waking","cpu":0},
    {"ts":12,"event":"sched_wake_up","cpu":0},
    {"ts":14,"event":"sched_switched_in","cpu":0,"prio":2,"preempt":false},
    {"ts":16,"event":"exit_arrival_site","ret":0}
    ]
    "#;

    #[test]
    fn test_clock_nanosleep() {
        let values: Vec<TraceEvent> = serde_json::from_str(CLOCK_NANOSLEEP).unwrap();

        let state = BlockingSyscall::check_seq(&values[..]);
        assert!(state.is_terminal_state());

        let values: Vec<TraceEvent> = serde_json::from_str(MIGRATE_BEFORE_SLEEP).unwrap();
        assert_eq!(values.len(), 9);

        let state = BlockingSyscall::check_seq(&values[..]);
        assert!(state.is_terminal_state());
    }
}
