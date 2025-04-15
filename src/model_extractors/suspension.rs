use std::collections::BTreeMap;

use crate::job::Job;
use crate::utils::InterleaveBy;
use crate::{
    interval::{Bound, Interval},
    ir::timeline::Timeline,
    utils::UpperBoundTracker,
};

#[derive(Default)]
pub struct DynamicSelfSuspension {
    tracker: UpperBoundTracker<u64>,
}

impl DynamicSelfSuspension {
    pub fn new() -> Self {
        Self {
            tracker: UpperBoundTracker::default(),
        }
    }

    pub fn update(&mut self, timeline: &mut Timeline, job: &Job) -> u64 {
        let t = job.suspension_time(timeline);

        self.tracker.update(t)
    }

    pub fn get(&self) -> u64 {
        self.tracker.get().unwrap_or(0)
    }

    pub fn report(&self) {
        println!(" - Max. SS (dynamic): {}", self.get());
    }
}

#[derive(PartialEq, Eq)]
enum Segment {
    Execution(Interval<u64>),
    SelfSuspension(Interval<u64>),
}

impl Segment {
    fn start(&self) -> Bound<u64> {
        match self {
            Segment::Execution(a) | Segment::SelfSuspension(a) => a.lower_bound(),
        }
    }
}

impl PartialOrd for Segment {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Segment {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.start().cmp(&other.start())
    }
}

pub struct BagOfSegmentedSelfSuspension {
    segments: BTreeMap<usize, Vec<(u64, u64)>>,
    capacity: usize,
    should_extract: bool,
}

impl BagOfSegmentedSelfSuspension {
    pub fn new() -> Self {
        Self::with_capacity(16)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity,
            segments: BTreeMap::new(),
            should_extract: true,
        }
    }

    fn get_segments(timeline: &mut Timeline, job: &Job) -> Vec<(u64, u64)> {
        let range = job.boundaries();
        let execs = timeline
            .runtime
            .plain_intervals(&range)
            .map(Segment::Execution);

        let mut skip = 0;

        if job.release_jitter() > 0 {
            skip = 1;
        }

        let suspensions = timeline
            .suspension
            .plain_intervals(&range)
            .skip(skip)
            .map(Segment::SelfSuspension);

        let segments = InterleaveBy::new(execs, suspensions, |a, b| a.cmp(b));

        let mut prev_exec = 0;
        let mut prev_susp = job.release_jitter();
        let mut new_segments = Vec::with_capacity(64);

        for segment in segments {
            match segment {
                Segment::Execution(i) => prev_exec += i.duration_unwrap(),
                Segment::SelfSuspension(i) => {
                    let segment_pairs = (prev_susp, prev_exec);
                    new_segments.push(segment_pairs);
                    prev_exec = 0;
                    prev_susp = i.duration_unwrap();
                }
            }
        }

        if prev_exec > 0 {
            let segment_pairs = (prev_susp, prev_exec);
            new_segments.push(segment_pairs);
        }

        new_segments.shrink_to_fit();

        new_segments
    }

    fn update_segments(old: &mut [(u64, u64)], new: &[(u64, u64)]) {
        let n = old.len().min(new.len());

        for i in 0..n {
            old[i].0 = old[i].0.max(new[i].0);
            old[i].1 = old[i].1.max(new[i].1);
        }
    }

    pub fn update(&mut self, timeline: &mut Timeline, job: &Job) {
        if self.should_extract {
            if timeline.has_compressed_intervals(&job.boundaries()) {
                self.should_extract = false;

                return;
            }

            let segments = Self::get_segments(timeline, job);
            let m = segments.len();

            if m > 1024 {
                self.should_extract = false;

                return;
            }

            let is_full = self.segments.len() == self.capacity;

            match self.segments.entry(m) {
                std::collections::btree_map::Entry::Vacant(v) => {
                    if is_full {
                        self.should_extract = false;
                        self.segments.clear();

                        return;
                    }

                    v.insert(segments);
                }

                std::collections::btree_map::Entry::Occupied(mut o) => {
                    let old = o.get_mut();

                    Self::update_segments(old, &segments);
                }
            }
        }
    }

    pub fn extract(&self) -> Option<BTreeMap<usize, Vec<(u64, u64)>>> {
        if self.should_extract && !self.segments.is_empty() {
            return Some(self.segments.clone());
        }

        None
    }
}

impl Default for BagOfSegmentedSelfSuspension {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::{
        events::{ClockId::*, EventData::*, TraceEvent},
        ir::timeline::Timeline,
        job::Job,
        utils::ThreadId,
    };

    use super::BagOfSegmentedSelfSuspension;

    const ID: ThreadId = ThreadId { pid: 0, tgid: 0 };

    const EXAMPLE_CLOCK_NANOSLEEP_1: &[TraceEvent] = &[
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
            ts: 21,
            ev: SchedSwitchedOut {
                cpu: 0,
                prio: 2,
                state: 0,
            },
        },
        TraceEvent {
            id: ID,
            ts: 25,
            ev: SchedWaking { cpu: 0 },
        },
        TraceEvent {
            id: ID,
            ts: 26,
            ev: SchedWakeUp { cpu: 0 },
        },
        TraceEvent {
            id: ID,
            ts: 27,
            ev: SchedSwitchedIn {
                cpu: 0,
                prio: 2,
                preempt: false,
            },
        },
        TraceEvent {
            id: ID,
            ts: 31,
            ev: SchedSwitchedOut {
                cpu: 0,
                prio: 2,
                state: 1,
            },
        },
        TraceEvent {
            id: ID,
            ts: 41,
            ev: SchedWaking { cpu: 0 },
        },
        TraceEvent {
            id: ID,
            ts: 42,
            ev: SchedWakeUp { cpu: 0 },
        },
        TraceEvent {
            id: ID,
            ts: 44,
            ev: SchedSwitchedIn {
                cpu: 0,
                prio: 2,
                preempt: false,
            },
        },
        TraceEvent {
            id: ID,
            ts: 50,
            ev: EnterClockNanoSleep {
                clock_id: ClockMonotonic,
                abs_time: true,
                required_ns: 60,
            },
        },
        TraceEvent {
            id: ID,
            ts: 51,
            ev: SchedSwitchedOut {
                cpu: 0,
                prio: 2,
                state: 1,
            },
        },
        TraceEvent {
            id: ID,
            ts: 61,
            ev: SchedWaking { cpu: 0 },
        },
        TraceEvent {
            id: ID,
            ts: 62,
            ev: SchedWakeUp { cpu: 0 },
        },
        TraceEvent {
            id: ID,
            ts: 64,
            ev: SchedSwitchedIn {
                cpu: 0,
                prio: 2,
                preempt: false,
            },
        },
        TraceEvent {
            id: ID,
            ts: 66,
            ev: ExitArrivalSite { ret: 0 },
        },
        TraceEvent {
            id: ID,
            ts: 71,
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
            ts: 100,
            ev: EnterClockNanoSleep {
                clock_id: ClockMonotonic,
                abs_time: true,
                required_ns: 110,
            },
        },
        /*

        TraceEvent { id: ID, ts: 101, ev: SchedSwitchedOut { cpu: 0, prio: 2, state: 1 }},
        TraceEvent { id: ID, ts: 111, ev: SchedWaking { cpu: 0 }},
        TraceEvent { id: ID, ts: 112, ev: SchedWakeUp { cpu: 0 }},
        TraceEvent { id: ID, ts: 114, ev: SchedSwitchedIn { cpu: 0, prio: 2, preempt: false }},
        */
        TraceEvent {
            id: ID,
            ts: 116,
            ev: ExitArrivalSite { ret: 0 },
        },
    ];

    #[test]
    fn test_bag_of_self_suspensions() {
        let mut bsss = BagOfSegmentedSelfSuspension::with_capacity(2);
        let mut timeline = Timeline::new();

        for e in EXAMPLE_CLOCK_NANOSLEEP_1.iter() {
            timeline.update(&e);
        }

        let j = Job {
            release: 12,
            arrival: None,
            first_cycle: 12,
            end: 50,
        };

        bsss.update(&mut timeline, &j);

        let mut expected = BTreeMap::new();
        expected.insert(2, vec![(0, 11), (11, 6)]);

        let segments = bsss.extract().unwrap();
        assert_eq!(segments, expected);

        let j = Job {
            release: 62,
            arrival: None,
            first_cycle: 62,
            end: 100,
        };

        bsss.update(&mut timeline, &j);
        expected.insert(2, vec![(0, 11), (21, 6)]);

        let segments = bsss.extract().unwrap();
        assert_eq!(segments, expected);

        let j = Job {
            release: 12,
            first_cycle: 12,
            arrival: None,
            end: 100,
        };

        bsss.update(&mut timeline, &j);
        let segments = bsss.extract().unwrap();
        expected.insert(4, vec![(0, 11), (11, 7), (11, 7), (21, 6)]);
        assert_eq!(segments, expected);

        let j = Job {
            release: 0,
            first_cycle: 0,
            arrival: None,
            end: 100,
        };

        bsss.update(&mut timeline, &j);
        let segments = bsss.extract();
        assert_eq!(segments, None);
    }
}
