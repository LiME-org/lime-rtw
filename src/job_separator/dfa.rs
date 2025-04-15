use crate::events::TraceEvent;

use super::{JobSeparation, JobSeparator, Signature};

#[derive(PartialEq, Eq, Debug)]
pub(crate) enum AResult {
    Accept { start: usize, end: usize },
    Reject,
    Matching { start: usize },
}

pub(crate) struct Dfa<S> {
    start: Option<usize>,
    curr_state: S,
    matching: bool,
}

impl<S: Signature> Dfa<S> {
    pub fn new() -> Self {
        Self {
            start: None,
            curr_state: S::initial_state(),
            matching: false,
        }
    }

    fn reset(&mut self) {
        self.start = None;
        self.curr_state = S::initial_state();
        self.matching = false;
    }

    fn job_separator(&self, matched_events: &[TraceEvent]) -> JobSeparator {
        self.curr_state.job_separator(matched_events)
    }

    fn job_separation(&self, matched_events: &[TraceEvent]) -> JobSeparation {
        self.curr_state.job_separation(matched_events)
    }

    pub fn update(
        &mut self,
        events: &[TraceEvent],
        pos: usize,
        extracted: &mut Vec<(JobSeparator, JobSeparation)>,
    ) -> AResult {
        let event = &events[pos];

        let res = self.update_no_extract(event, pos);

        if let AResult::Accept { start, end } = res {
            let matched_events = &events[start..end];
            let separator = self.job_separator(matched_events);
            let separation = self.job_separation(matched_events);

            extracted.push((separator, separation));
        }

        res
    }

    pub fn update_no_extract(&mut self, event: &TraceEvent, pos: usize) -> AResult {
        let mut new_state = self.curr_state.next(event);

        if new_state.is_rejection_state() {
            self.reset();

            new_state = self.curr_state.next(event);

            if new_state.is_rejection_state() {
                return AResult::Reject;
            }
        };

        self.curr_state = new_state;

        if self.start.is_none() {
            self.start = Some(pos);
        }

        let start = unsafe { self.start.unwrap_unchecked() };

        if self.curr_state.is_terminal_state() {
            self.reset();

            return AResult::Accept {
                start,
                end: pos + 1,
            };
        }

        self.matching = true;

        AResult::Matching { start }
    }

    pub fn is_matching(&self) -> bool {
        self.matching
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        events::{ClockId, TraceEvent},
        job_separator::JobSeparator,
    };

    use crate::job_separator::SignatureMatcher;

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

    #[test]
    fn test_job_separator_extractor() {
        let mut matchers = SignatureMatcher::new();
        let mut extracted = Vec::new();

        let values: Vec<TraceEvent> = serde_json::from_str(MIGRATE_BEFORE_SLEEP).unwrap();
        assert_eq!(values.len(), 9);

        for event in values.iter() {
            matchers.consume_event(event.clone(), &mut extracted);
        }

        assert_eq!(extracted.len(), 2);

        assert_ne!(extracted[0].0, extracted[1].0);

        let nanosleep_sep = JobSeparator::ClockNanosleep {
            clock_id: ClockId::ClockMonotonic,
            abs_time: true,
        };
        let suspension_sep = JobSeparator::Suspension;

        if extracted[0].0 == nanosleep_sep {
            assert_eq!(extracted[1].0, suspension_sep);
        } else {
            assert_eq!(extracted[0].0, suspension_sep);
            assert_eq!(extracted[1].0, nanosleep_sep);
        }
    }
}
