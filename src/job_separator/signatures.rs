use crate::events::{ClockId, EventData, TraceEvent};

use super::pattern::{filter_item, many_0, or, take_until};
use super::{JobSeparation, JobSeparator, Signature};

use crate::events::EventData::*;

use crate::job_separator::pattern::{item, then, MResult};

macro_rules! ev_preds {
    ($name:ident => $pat:pat) => {
        pub fn $name(input: &[TraceEvent]) -> MResult<TraceEvent>   {
            item(move |e: &TraceEvent| {
                match e.ev {
                    $pat => true,
                    _ => false
                }
            })(input)
        }
    };

    ($name:ident => $pat:pat, $($nname:ident => $npat:pat), +) => {
        ev_preds! {$name => $pat}
        ev_preds! { $($nname => $npat), +}
    };
}

ev_preds! {
    // enter_clock_nanosleep => EventData::EnterClockNanoSleep{..},
    exit_arrival_site => EventData::ExitArrivalSite{..},
    sched_switched_in => EventData::SchedSwitchedIn{..},
    sched_switched_out => EventData::SchedSwitchedOut{..},
    sched_wakeup => EventData::SchedWakeUp{..},
    sched_migrate_task => EventData::SchedMigrateTask { .. },
    // sched_wakeup_new => EventData::SchedWakeUpNew{..},
    sched_waking => EventData::SchedWaking{..},
    start_of_trace => EventData::LimeStartOfTrace,
    end_of_trace => EventData::LimeEndOfTrace
}

fn syscall_entry(input: &[TraceEvent]) -> MResult<TraceEvent> {
    item(|e: &TraceEvent| e.is_syscall_entry())(input)
}

fn suspension(input: &[TraceEvent]) -> MResult<'_, TraceEvent> {
    item(|e: &TraceEvent| e.is_suspension())(input)
}

fn dl_timer(input: &[TraceEvent]) -> MResult<'_, TraceEvent> {
    item(|e: &TraceEvent| matches!(e.ev, EventData::EnterDlTimer { .. }))(input)
}

fn sched_yield(input: &[TraceEvent]) -> MResult<'_, TraceEvent> {
    item(|e: &TraceEvent| matches!(e.ev, EventData::EnterSchedYield { .. }))(input)
}

fn not_running(input: &[TraceEvent]) -> MResult<'_, TraceEvent> {
    item(|e: &TraceEvent| {
        matches!(
            e.ev,
            EventData::AffinityChanged
                | EventData::SchedulerChanged { .. }
                | EventData::SchedMigrateTask { .. }
                | EventData::SchedWaking { .. }
        )
    })(input)
}

#[derive(Clone, Copy)]
pub struct BlockingSyscall {}

// We are only interested in futex waiting operations.
fn filter_invalid_futex_ops(event: &TraceEvent) -> bool {
    // TODO clean up hard coded values
    match event.ev {
        EnterFutex { op, .. } => matches!(op & !(128 + 256), 0 | 9),
        _ => true,
    }
}

fn end_of_syscall(event: &TraceEvent) -> bool {
    matches!(event.ev, LimeEndOfTrace | EventData::ExitArrivalSite { .. })
}

fn syscall_job(input: &[TraceEvent]) -> MResult<'_, TraceEvent> {
    seq![
        filter_item(syscall_entry, filter_invalid_futex_ops),
        take_until(end_of_syscall)
    ](input)
}

impl Signature for BlockingSyscall {
    fn match_pattern<'a>(&'a self, events: &'a [TraceEvent]) -> MResult<'a, TraceEvent> {
        syscall_job(events)
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

#[derive(Clone, Copy)]
pub struct Suspension {}

fn end_of_suspension(event: &TraceEvent) -> bool {
    matches!(
        event.ev,
        EventData::SchedWakeUp { .. } | EventData::LimeEndOfTrace
    )
}

fn non_self_suspending_job(input: &[TraceEvent]) -> MResult<'_, TraceEvent> {
    or(
        seq!(suspension, take_until(end_of_suspension)),
        seq!(start_of_trace, many_0(not_running), sched_wakeup),
    )(input)
}

impl Signature for Suspension {
    fn match_pattern<'a>(&'a self, events: &'a [TraceEvent]) -> MResult<'a, TraceEvent> {
        non_self_suspending_job(events)
    }

    fn job_separator(&self, _matched_events: &[TraceEvent]) -> JobSeparator {
        JobSeparator::Suspension
    }

    fn job_separation(&self, matched_events: &[TraceEvent]) -> JobSeparation {
        let prev_end = matched_events.first().unwrap().ts;
        let curr_release = matched_events.last().unwrap().ts;

        JobSeparation {
            prev_end,
            curr_release,
            curr_first_cycle: curr_release,
            curr_arrival: None,
        }
    }
}

#[derive(Clone, Copy)]
pub struct SchedYieldDl {}

impl SchedYieldDl {
    fn get_curr_release(&self, matched_events: &[TraceEvent]) -> u64 {
        match matched_events.iter().find(|e| e.is_dl_timer()) {
            Some(e) => e.ts,
            _ => unreachable!(),
        }
    }

    fn end(&self, matched_events: &[TraceEvent]) -> u64 {
        matched_events.first().unwrap().ts
    }

    fn get_curr_arrival(&self, matched_events: &[TraceEvent]) -> Option<u64> {
        match matched_events.iter().find(|e| e.is_dl_timer()) {
            Some(e) => match e.ev {
                EventData::EnterDlTimer { expires } => Some(expires),
                _ => unreachable!(),
            },
            _ => unreachable!(),
        }
    }
}

impl Signature for SchedYieldDl {
    fn match_pattern<'a>(&'a self, events: &'a [TraceEvent]) -> MResult<'a, TraceEvent> {
        seq![sched_yield, sched_switched_out, dl_timer](events)
    }

    fn job_separator(&self, _matched_events: &[TraceEvent]) -> JobSeparator {
        JobSeparator::YieldSchedDeadline
    }

    fn job_separation(&self, matched_events: &[TraceEvent]) -> super::JobSeparation {
        let curr_release = self.get_curr_release(matched_events);
        let curr_arrival = self.get_curr_arrival(matched_events);
        let curr_first_cycle = curr_arrival.unwrap();

        JobSeparation {
            prev_end: self.end(matched_events),
            curr_release,
            curr_arrival,
            curr_first_cycle,
        }
    }
}

/// The JOB_SEPARATOR_SIGNATURES array contains the signatures
/// that job trackers evaluate for job separator extraction.
/// Any new signature must be appended to this array to be activated.
pub const JOB_SEPARATOR_SIGNATURES: &[&dyn Signature] =
    &[&Suspension {}, &BlockingSyscall {}, &SchedYieldDl {}];

#[cfg(test)]
mod tests {
    use crate::{
        events::{ClockId::*, EventData::*, TraceEvent},
        job_separator::pattern::MResult,
        job_separator::signatures::{non_self_suspending_job, syscall_job, BlockingSyscall},
        job_separator::Signature,
        utils::ThreadId,
    };
    const ID: ThreadId = ThreadId { pid: 0, tgid: 0 };

    const EXAMPLE_CLOCK_NANOSLEEP: [TraceEvent; 6] = [
        TraceEvent {
            id: ID,
            ts: 0,
            ev: EnterClockNanoSleep {
                clock_id: ClockMonotonic,
                abs_time: true,
                required_ns: 10,
            },
        },
        TraceEvent {
            id: ID,
            ts: 1,
            ev: SchedSwitchedOut {
                cpu: 0,
                prio: 2,
                state: 1,
            },
        },
        TraceEvent {
            id: ID,
            ts: 11,
            ev: SchedWaking { cpu: 0 },
        },
        TraceEvent {
            id: ID,
            ts: 12,
            ev: SchedWakeUp { cpu: 0 },
        },
        TraceEvent {
            id: ID,
            ts: 14,
            ev: SchedSwitchedIn {
                cpu: 0,
                prio: 2,
                preempt: false,
            },
        },
        TraceEvent {
            id: ID,
            ts: 16,
            ev: ExitArrivalSite { ret: 0 },
        },
    ];

    #[test]
    fn test_clock_nanosleep() {
        let res = syscall_job(&EXAMPLE_CLOCK_NANOSLEEP[..]);

        assert_eq!(res, MResult::Matched(&EXAMPLE_CLOCK_NANOSLEEP, &[]));
    }

    const SELF_SUSPENDING: &str = "
    [{\"ts\":266655284184290,\"pid\":98218,\"event\":\"sched_wake_up\",\"cpu\":0},
    {\"ts\":266655284184582,\"pid\":98218,\"event\":\"sched_switched_in\",\"cpu\":0,\"prio\":99,\"preempt\":false},
    {\"ts\":266655284199789,\"pid\":98218,\"event\":\"sched_switched_out\",\"cpu\":0,\"prio\":99,\"state\":1},
    {\"ts\":266655285181060,\"pid\":98218,\"event\":\"sched_waking\",\"cpu\":0},
    {\"ts\":266655285183518,\"pid\":98218,\"event\":\"sched_wake_up\",\"cpu\":1},
    {\"ts\":266655285188018,\"pid\":98218,\"event\":\"sched_switched_in\",\"cpu\":1,\"prio\":99,\"preempt\":true},
    {\"ts\":266655285193434,\"pid\":98218,\"event\":\"sched_switched_out\",\"cpu\":1,\"prio\":99,\"state\":1}]";

    const SELF_SUSPENDING_MIGRATE: &str = "
    [{\"ts\":266655284184290,\"pid\":98218,\"event\":\"sched_wake_up\",\"cpu\":0},
    {\"ts\":266655284184582,\"pid\":98218,\"event\":\"sched_switched_in\",\"cpu\":0,\"prio\":99,\"preempt\":false},
    {\"ts\":266655284199789,\"pid\":98218,\"event\":\"sched_switched_out\",\"cpu\":0,\"prio\":99,\"state\":1},
    {\"ts\":266655285181060,\"pid\":98218,\"event\":\"sched_waking\",\"cpu\":0},
    {\"ts\":266655285181727,\"pid\":98218,\"event\":\"sched_migrate_task\",\"dest_cpu\":1},
    {\"ts\":266655285183518,\"pid\":98218,\"event\":\"sched_wake_up\",\"cpu\":1},
    {\"ts\":266655285188018,\"pid\":98218,\"event\":\"sched_switched_in\",\"cpu\":1,\"prio\":99,\"preempt\":true},
    {\"ts\":266655285193434,\"pid\":98218,\"event\":\"sched_switched_out\",\"cpu\":1,\"prio\":99,\"state\":1}]";

    /*
    const RT_SIG_HANDLER: &str = "
    [{\"ts\":81066147459635,\"pid\":129597,\"event\":\"rt_sig_delivered\",\"signo\":35,\"si_code\":-2},
    {\"ts\":81066147565231,\"pid\":129597,\"event\":\"enter_futex\",\"uaddr\":187650449342896,\"op\":1},
    {\"ts\":81066147601277,\"pid\":129597,\"event\":\"exit_arrival_site\"},
    {\"ts\":81066147606528,\"pid\":129597,\"event\":\"enter_rt_sigreturn\"}]";
    */

    const MIGRATE_AFTER_SLEEP: &str = r#"[
    {"ts":1467082285857386,"event":"enter_clock_nano_sleep","clock_id":"CLOCK_MONOTONIC","abs_time":true,"required_ns":1467082307743766},
    {"ts":1467082285887848,"event":"sched_switched_out","cpu":0,"prio":76,"state":1},
    {"ts":1467082307749908,"event":"sched_waking","cpu":0},
    {"ts":1467082307757315,"event":"sched_wake_up","cpu":0},
    {"ts":1467082307773148,"event":"sched_switched_in","cpu":0,"prio":76,"preempt":false},
    {"ts":1467082307793870,"event":"sched_switched_out","cpu":0,"prio":76,"state":0},
    {"ts":1467082307807314,"event":"sched_migrate_task","dest_cpu":1},
    {"ts":1467082307843943,"event":"sched_migrate_task","dest_cpu":3},
    {"ts":1467082307861054,"event":"sched_switched_in","cpu":3,"prio":76,"preempt":false},
    {"ts":1467082307869202,"event":"exit_arrival_site","ret":0}
    ]"#;

    const MIGRATE_BEFORE_SLEEP: &str = r#"[
    {"ts":1467100665150912,"event":"enter_clock_nano_sleep","clock_id":"CLOCK_MONOTONIC","abs_time":true,"required_ns":1467100683809847},
    {"ts":1467100665163541,"event":"sched_switched_out","cpu":3,"prio":76,"state":0},
    {"ts":1467100665172670,"event":"sched_migrate_task","dest_cpu":2},
    {"ts":1467100665179967,"event":"sched_switched_in","cpu":2,"prio":76,"preempt":false},
    {"ts":1467100665190281,"event":"sched_switched_out","cpu":2,"prio":76,"state":1},
    {"ts":1467100683814559,"event":"sched_waking","cpu":2},
    {"ts":1467100683821059,"event":"sched_wake_up","cpu":2},
    {"ts":1467100683831003,"event":"sched_switched_in","cpu":2,"prio":76,"preempt":false},
    {"ts":1467100683836318,"event":"exit_arrival_site","ret":0}
    ]
    "#;

    #[test]
    fn test_non_self_suspending_job_migration() {
        let values: Vec<TraceEvent> = serde_json::from_str(SELF_SUSPENDING_MIGRATE).unwrap();
        assert_eq!(values.len(), 8);

        assert!(matches!(
            non_self_suspending_job(values.as_slice()),
            MResult::Rejected
        ));

        assert!(matches!(
            non_self_suspending_job(&values[2..]),
            MResult::Matched(_, _)
        ));

        assert!(matches!(
            non_self_suspending_job(&values[2..]),
            MResult::Matched(_, _)
        ));
    }

    #[test]
    fn test_non_self_suspending_job() {
        let values: Vec<TraceEvent> = serde_json::from_str(SELF_SUSPENDING).unwrap();
        assert_eq!(values.len(), 7);

        assert!(matches!(
            non_self_suspending_job(values.as_slice()),
            MResult::Rejected
        ));

        assert!(matches!(
            non_self_suspending_job(&values[2..]),
            MResult::Matched(_, _)
        ));

        assert!(matches!(
            non_self_suspending_job(&values[2..]),
            MResult::Matched(_, _)
        ));
    }

    #[test]
    fn test_migrate_before_exit() {
        let values: Vec<TraceEvent> = serde_json::from_str(MIGRATE_AFTER_SLEEP).unwrap();
        assert_eq!(values.len(), 10);

        let m = BlockingSyscall {};
        assert!(matches!(m.match_pattern(&values), MResult::Matched(_, _)));

        let values: Vec<TraceEvent> = serde_json::from_str(MIGRATE_BEFORE_SLEEP).unwrap();
        assert_eq!(values.len(), 9);

        let m = BlockingSyscall {};
        assert!(matches!(m.match_pattern(&values), MResult::Matched(_, _)));
    }

    /*
    #[test]
    fn test_rt_signal_handler() {
        let values: Vec<TraceEvent> = serde_json::from_str(RT_SIG_HANDLER).unwrap();
        assert_eq!(values.len(), 4);

        assert!(matches!(
            rt_sig_handler(values.as_slice()),
            MResult::Matched(_, _)
        ));
    }
    */
}

