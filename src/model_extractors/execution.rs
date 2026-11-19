use lime_model_extractors::extractors::MaxResourceUseExtractor;

use crate::{ir::timeline::Timeline, job::Job};

/// WCET(n) extractor.
pub struct ExecutionTime {
    tracker: Option<MaxResourceUseExtractor>,
}

impl ExecutionTime {
    /// Tracks execution time over windows of up to `n_max` consecutive jobs.
    pub fn new(n_max: usize) -> Self {
        Self {
            tracker: MaxResourceUseExtractor::new(Some(n_max)).ok(),
        }
    }

    pub fn update(&mut self, timeline: &mut Timeline, job: &Job) {
        if let Some(tracker) = &mut self.tracker {
            tracker.feed([job.processor_time(timeline)]);
        }
    }

    pub fn wcet(&self, n: usize) -> Option<u64> {
        // wcets() starts at WCET(1), so WCET(n) is at index n - 1 and n = 0 has no entry.
        self.wcets().nth(n.checked_sub(1)?)
    }

    /// Yields the observed execution-time bounds in order, starting with WCET(1).
    pub fn wcets(&self) -> impl Iterator<Item = u64> + '_ {
        self.tracker
            .as_ref()
            .map(MaxResourceUseExtractor::current_model)
            .unwrap_or_default()
            .into_iter()
            .skip(1)
    }
}

#[cfg(test)]
pub mod tests {
    use crate::{
        events::{ClockId::*, EventData::*, TraceEvent},
        ir::Ir,
        utils::ThreadId,
    };

    use super::ExecutionTime;

    const ID: ThreadId = ThreadId { pid: 0, tgid: 0 };

    const EXAMPLE_CLOCK_NANOSLEEP: &[TraceEvent] = &[
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
        TraceEvent {
            id: ID,
            ts: 40,
            ev: EnterClockNanoSleep {
                clock_id: ClockMonotonic,
                abs_time: true,
                required_ns: 10,
            },
        },
        TraceEvent {
            id: ID,
            ts: 41,
            ev: SchedSwitchedOut {
                cpu: 0,
                prio: 2,
                state: 1,
            },
        },
        TraceEvent {
            id: ID,
            ts: 51,
            ev: SchedWaking { cpu: 0 },
        },
        TraceEvent {
            id: ID,
            ts: 52,
            ev: SchedWakeUp { cpu: 0 },
        },
        TraceEvent {
            id: ID,
            ts: 54,
            ev: SchedSwitchedIn {
                cpu: 0,
                prio: 2,
                preempt: false,
            },
        },
        TraceEvent {
            id: ID,
            ts: 56,
            ev: ExitArrivalSite { ret: 0 },
        },
        TraceEvent {
            id: ID,
            ts: 80,
            ev: EnterClockNanoSleep {
                clock_id: ClockMonotonic,
                abs_time: true,
                required_ns: 10,
            },
        },
        TraceEvent {
            id: ID,
            ts: 81,
            ev: SchedSwitchedOut {
                cpu: 0,
                prio: 2,
                state: 1,
            },
        },
        TraceEvent {
            id: ID,
            ts: 91,
            ev: SchedWaking { cpu: 0 },
        },
        TraceEvent {
            id: ID,
            ts: 92,
            ev: SchedWakeUp { cpu: 0 },
        },
        TraceEvent {
            id: ID,
            ts: 94,
            ev: SchedSwitchedIn {
                cpu: 0,
                prio: 2,
                preempt: false,
            },
        },
        TraceEvent {
            id: ID,
            ts: 96,
            ev: ExitArrivalSite { ret: 0 },
        },
    ];

    #[test]
    fn test_wcet() {
        use crate::job_separator::JobSeparator;
        let mut ir = Ir::new();
        let mut jobs = Vec::new();

        for event in EXAMPLE_CLOCK_NANOSLEEP.iter().cloned() {
            ir.consume_event(event, &mut jobs);
        }

        let mut tracker = ExecutionTime::new(32);

        for (sep, job) in jobs.drain(..) {
            if let JobSeparator::ClockNanosleep { .. } = sep {
                tracker.update(&mut ir.timeline, &job);
            }
        }

        assert_eq!(tracker.wcet(1).unwrap(), 41 - 14);
    }

    #[test]
    fn test_max_sum_n() {
        let mut tracker = ExecutionTime::new(4);
        tracker.tracker.as_mut().unwrap().feed([1, 2, 3]);

        assert_eq!(tracker.wcet(1), Some(3));
        assert_eq!(tracker.wcet(2), Some(5));
        assert_eq!(tracker.wcet(3), Some(6));
        assert_eq!(tracker.wcet(4), None);

        tracker.tracker.as_mut().unwrap().feed([4, 5, 6]);

        assert_eq!(tracker.wcet(1), Some(6));
        assert_eq!(tracker.wcet(2), Some(11));
        assert_eq!(tracker.wcet(3), Some(15));
        assert_eq!(tracker.wcet(4), Some(18));

        tracker.tracker.as_mut().unwrap().feed([1]);

        assert_eq!(tracker.wcet(1), Some(6));
        assert_eq!(tracker.wcet(2), Some(11));
        assert_eq!(tracker.wcet(3), Some(15));
        assert_eq!(tracker.wcet(4), Some(18));
    }
}
