use std::ops::{Add, AddAssign};

use crate::{ir::timeline::Timeline, job::Job, utils::RingBuffer};

/// WCET(n) extractor.
pub struct ExecutionTime {
    tracker: MaxSumN<u64>,
}

// TODO user should be able to modify this
const WCET_N_MAX: usize = 32;

impl ExecutionTime {
    pub fn new(n_max: usize) -> Self {
        Self {
            tracker: MaxSumN::new(n_max),
        }
    }

    pub fn update(&mut self, timeline: &mut Timeline, job: &Job) {
        let t = job.processor_time(timeline);

        self.tracker.update(t);
    }

    pub fn wcet(&self, n: usize) -> Option<u64> {
        self.tracker.max_sum_n(n).copied()
    }

    pub fn wcets(&self) -> impl Iterator<Item = u64> + '_ {
        self.tracker.max_sums()
    }
}

struct MaxSumN<T> {
    buff: RingBuffer<T>,
    max_sums: Vec<T>,
}

impl<T> Default for MaxSumN<T>
where
    T: Ord + Add<Output = T> + AddAssign + Copy + Default,
{
    fn default() -> Self {
        Self::new(WCET_N_MAX)
    }
}

impl<T> MaxSumN<T>
where
    T: Ord + Add<Output = T> + AddAssign + Copy + Default,
{
    pub fn new(n: usize) -> Self {
        Self {
            buff: RingBuffer::new(n),
            max_sums: Vec::with_capacity(n),
        }
    }

    pub fn update(&mut self, new_element: T) {
        let old = self.buff.push(new_element);

        if old.is_none() {
            self.max_sums.push(new_element);

            let last = self.max_sums.len();

            if last > 1 {
                let v = self.max_sums[last - 2];
                self.max_sums[last - 1] += v;
            }
        }

        let mut sum = T::default();

        for (i, v) in self.buff.as_slice().iter().rev().enumerate() {
            sum += *v;

            self.max_sums[i] = self.max_sums[i].max(sum);
        }
    }

    pub fn max_sum_n(&self, n: usize) -> Option<&T> {
        self.max_sums.get(n - 1)
    }

    pub fn max_sums(&self) -> impl Iterator<Item = T> + '_ {
        self.max_sums.iter().copied()
    }
}

#[cfg(test)]
pub mod tests {
    use crate::{
        events::{ClockId::*, EventData::*, TraceEvent},
        ir::Ir,
        utils::ThreadId,
    };

    use super::MaxSumN;

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
        use crate::{job_separator::JobSeparator, model_extractors::execution::ExecutionTime};
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
        let example = &[1, 2, 3, 4, 5, 6];

        let mut tracker = MaxSumN::<i32>::new(4);

        for i in example[..3].iter() {
            tracker.update(*i);
        }

        assert_eq!(tracker.max_sum_n(1), Some(&3));
        assert_eq!(tracker.max_sum_n(2), Some(&5));
        assert_eq!(tracker.max_sum_n(3), Some(&6));
        assert_eq!(tracker.max_sum_n(4), None);

        for i in example[3..].iter() {
            tracker.update(*i);
        }

        assert_eq!(tracker.max_sum_n(1), Some(&6));
        assert_eq!(tracker.max_sum_n(2), Some(&11));
        assert_eq!(tracker.max_sum_n(3), Some(&15));
        assert_eq!(tracker.max_sum_n(4), Some(&18));

        tracker.update(1);

        assert_eq!(tracker.max_sum_n(1), Some(&6));
        assert_eq!(tracker.max_sum_n(2), Some(&11));
        assert_eq!(tracker.max_sum_n(3), Some(&15));
        assert_eq!(tracker.max_sum_n(4), Some(&18));
    }
}
