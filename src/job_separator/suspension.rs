//! Signature of non-self-suspending jobs.

use crate::events::{EventData, TraceEvent};

use super::{JobSeparation, JobSeparator, Signature};

fn not_running(event: &TraceEvent) -> bool {
    matches!(
        event.ev,
        EventData::AffinityChanged
            | EventData::SchedulerChanged { .. }
            | EventData::SchedMigrateTask { .. }
            | EventData::SchedWaking { .. }
    )
}

fn start_of_trace(event: &TraceEvent) -> bool {
    matches!(event.ev, EventData::LimeStartOfTrace)
}

fn end_of_suspension(event: &TraceEvent) -> bool {
    matches!(
        event.ev,
        EventData::SchedWakeUp { .. } | EventData::LimeEndOfTrace
    )
}

pub enum SelfSuspension {
    Init,
    Suspended,
    NotRunning,
    Awake,
    Reject,
}

impl Signature for SelfSuspension {
    fn initial_state() -> Self {
        Self::Init
    }

    fn next(&self, event: &TraceEvent) -> Self {
        match self {
            Self::Init if event.is_suspension() => Self::Suspended,
            Self::Init if start_of_trace(event) => Self::NotRunning,
            Self::NotRunning => {
                if not_running(event) {
                    Self::NotRunning
                } else if event.is_wakeup() {
                    Self::Awake
                } else {
                    Self::Reject
                }
            }
            Self::Suspended => {
                if end_of_suspension(event) {
                    Self::Awake
                } else {
                    Self::Suspended
                }
            }
            _ => Self::Reject,
        }
    }

    fn is_terminal_state(&self) -> bool {
        matches!(self, Self::Awake)
    }

    fn is_rejection_state(&self) -> bool {
        matches!(self, Self::Reject)
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

#[cfg(test)]
mod tests {
    use crate::{
        events::TraceEvent,
        job_separator::{suspension::SelfSuspension, Signature},
    };

    const SELF_SUSPENDING: &str = r#"
    [{"ts":266655284184290,"pid":98218,"event":"sched_wake_up","cpu":0},
    {"ts":266655284184582,"pid":98218,"event":"sched_switched_in","cpu":0,"prio":99,"preempt":false},
    {"ts":266655284199789,"pid":98218,"event":"sched_switched_out","cpu":0,"prio":99,"state":1},
    {"ts":266655285181060,"pid":98218,"event":"sched_waking","cpu":0},
    {"ts":266655285183518,"pid":98218,"event":"sched_wake_up","cpu":1},
    {"ts":266655285188018,"pid":98218,"event":"sched_switched_in","cpu":1,"prio":99,"preempt":true},
    {"ts":266655285193434,"pid":98218,"event":"sched_switched_out","cpu":1,"prio":99,"state":1}]"#;

    const SELF_SUSPENDING_MIGRATE: &str = r#"
    [{"ts":266655284184290,"pid":98218,"event":"sched_wake_up","cpu":0},
    {"ts":266655284184582,"pid":98218,"event":"sched_switched_in","cpu":0,"prio":99,"preempt":false},
    {"ts":266655284199789,"pid":98218,"event":"sched_switched_out","cpu":0,"prio":99,"state":1},
    {"ts":266655285181060,"pid":98218,"event":"sched_waking","cpu":0},
    {"ts":266655285181727,"pid":98218,"event":"sched_migrate_task","dest_cpu":1},
    {"ts":266655285183518,"pid":98218,"event":"sched_wake_up","cpu":1},
    {"ts":266655285188018,"pid":98218,"event":"sched_switched_in","cpu":1,"prio":99,"preempt":true},
    {"ts":266655285193434,"pid":98218,"event":"sched_switched_out","cpu":1,"prio":99,"state":1}]"#;

    #[test]
    fn test_non_self_suspending_job() {
        let values: Vec<TraceEvent> = serde_json::from_str(SELF_SUSPENDING).unwrap();
        assert_eq!(values.len(), 7);

        let state = SelfSuspension::check_seq(&values[..5]);

        assert!(state.is_terminal_state());
    }

    #[test]
    fn test_non_self_suspending_job_migration() {
        let values: Vec<TraceEvent> = serde_json::from_str(SELF_SUSPENDING_MIGRATE).unwrap();

        let state = SelfSuspension::check_seq(&values[..6]);

        assert!(state.is_terminal_state());
    }
}
