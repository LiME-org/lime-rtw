use crate::job::Job;

/// Counts the number of detected job for a specific separator.
#[derive(Default)]
pub struct JobCounter {
    count: usize,
}

impl JobCounter {
    pub fn new() -> Self {
        Self { count: 0 }
    }

    pub fn update(&mut self, _job: &Job) {
        self.count += 1
    }

    pub fn get(&self) -> usize {
        self.count
    }
}

#[cfg(test)]
pub mod tests {

    use crate::{
        events::{ClockId::*, EventData::*, TraceEvent},
        ir::Ir,
        job_separator::JobSeparator,
        utils::ThreadId,
    };

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
                required_ns: 50,
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
                required_ns: 90,
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
    pub fn test_job_extraction() {
        let mut ir = Ir::new();
        let mut jobs = Vec::new();

        for event in EXAMPLE_CLOCK_NANOSLEEP.iter().cloned() {
            ir.consume_event(event, &mut jobs);
        }

        let mut count = 0;

        for (sep, job) in jobs.drain(..) {
            match sep {
                JobSeparator::ClockNanosleep { .. } => {
                    assert_eq!(job.response_time(), 31);
                    assert_eq!(job.processor_time(&mut ir.timeline), 41 - 14);
                    assert_eq!(job.end % 40, 1);
                    assert_eq!(job.release % 40, 12);
                    assert_eq!((job.release - job.release_jitter()) % 40, 10);
                }
                JobSeparator::Suspension => {
                    assert_eq!(job.response_time(), 29);
                    assert_eq!(job.processor_time(&mut ir.timeline), 41 - 14);
                    assert_eq!(job.end % 40, 1);
                    assert_eq!(job.release % 40, 12);
                    assert_eq!((job.release - job.release_jitter()) % 40, 12);
                }
                _ => unreachable!(),
            }
            count += 1;
        }
        assert_eq!(count, 4);
    }
}
