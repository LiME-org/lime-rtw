//! Signature of sched_yield-separated deadline jobs.

use crate::events::{EventData, TraceEvent};

use super::{JobSeparation, JobSeparator, Signature};

pub enum SchedYieldDeadline {
    Init,
    Yielding,
    SwitchedOut,
    Timer,
    Reject,
}

impl SchedYieldDeadline {
    fn is_sched_yield(event: &TraceEvent) -> bool {
        matches!(event.ev, EventData::EnterSchedYield {})
    }

    fn is_switched_out(event: &TraceEvent) -> bool {
        matches!(event.ev, EventData::SchedSwitchedOut { .. })
    }

    fn dl_timer_event(events: &[TraceEvent]) -> &TraceEvent {
        events
            .iter()
            .find(|e| matches!(e.ev, EventData::EnterDlTimer { .. }))
            .unwrap()
    }
}

impl Signature for SchedYieldDeadline {
    fn initial_state() -> Self {
        Self::Init
    }

    fn next(&self, event: &TraceEvent) -> Self {
        match self {
            Self::Init if Self::is_sched_yield(event) => Self::Yielding,
            Self::Yielding if Self::is_switched_out(event) => Self::SwitchedOut,
            Self::SwitchedOut if event.is_dl_timer() => Self::Timer,
            _ => Self::Reject,
        }
    }

    fn is_terminal_state(&self) -> bool {
        matches!(self, Self::Timer)
    }

    fn is_rejection_state(&self) -> bool {
        matches!(self, Self::Reject)
    }

    fn job_separator(&self, _matched_events: &[TraceEvent]) -> JobSeparator {
        JobSeparator::YieldSchedDeadline
    }

    fn job_separation(&self, matched_events: &[TraceEvent]) -> JobSeparation {
        let prev_end = matched_events.first().unwrap().ts;
        let dl_timer = Self::dl_timer_event(matched_events);
        let curr_release = dl_timer.ts;

        let curr_arrival = match dl_timer.ev {
            EventData::EnterDlTimer { expires } => Some(expires),
            _ => unreachable!(),
        };

        JobSeparation {
            prev_end,
            curr_release,
            curr_first_cycle: curr_arrival.unwrap(),
            curr_arrival,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        events::TraceEvent,
        job_separator::{
            sched_yield::SchedYieldDeadline, JobSeparator, Signature, SignatureMatcher,
        },
    };

    const SCHED_YIELD_DL: &str = r#"[
    {"ts":10,"event":"enter_sched_yield"},
    {"ts":11,"event":"sched_switched_out","cpu":0,"prio":99,"state":0},
    {"ts":20,"event":"enter_dl_timer","expires":25}
    ]"#;

    const INTERRUPTED_SEQUENCE: &str = r#"[
    {"ts":5,"event":"sched_switched_out","cpu":0,"prio":99,"state":1},
    {"ts":10,"event":"enter_sched_yield"},
    {"ts":11,"event":"sched_switched_out","cpu":0,"prio":99,"state":0},
    {"ts":15,"event":"sched_switched_in","cpu":0,"prio":99,"preempt":false},
    {"ts":20,"event":"enter_dl_timer","expires":25}
    ]"#;

    #[test]
    fn sched_yield_deadline_signature() {
        let events: Vec<TraceEvent> = serde_json::from_str(SCHED_YIELD_DL).unwrap();

        let state = SchedYieldDeadline::check_seq(&events);
        assert!(state.is_terminal_state());

        let sep = state.job_separation(&events);
        assert_eq!(sep.prev_end, 10);
        assert_eq!(sep.curr_release, 20);
        assert_eq!(sep.curr_arrival, Some(25));
        assert_eq!(sep.curr_first_cycle, 25);

        assert_eq!(
            state.job_separator(&events),
            JobSeparator::YieldSchedDeadline
        );
    }

    #[test]
    fn matcher_extracts_sched_yield_deadline() {
        let events: Vec<TraceEvent> = serde_json::from_str(SCHED_YIELD_DL).unwrap();
        let mut matcher = SignatureMatcher::new();
        let mut extracted = Vec::new();

        for event in events {
            matcher.consume_event(event, &mut extracted);
        }

        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].0, JobSeparator::YieldSchedDeadline);
    }

    #[test]
    fn interrupted_sequence_rejects() {
        let events: Vec<TraceEvent> = serde_json::from_str(INTERRUPTED_SEQUENCE).unwrap();

        let state = SchedYieldDeadline::check_seq(&events);
        assert!(!state.is_terminal_state());
    }
}
