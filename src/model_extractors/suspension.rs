use std::collections::BTreeMap;

use lime_model_extractors::{extractors::SegmentedSuspensionModelExtractor, ObservedSegment};

use crate::{ir::timeline::Timeline, job::Job, utils::UpperBoundTracker};

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

pub struct BagOfSegmentedSelfSuspension {
    models: Option<BTreeMap<usize, SegmentedSuspensionModelExtractor>>,
    capacity: usize,
}

impl BagOfSegmentedSelfSuspension {
    pub fn new() -> Self {
        Self::with_capacity(16)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            models: Some(BTreeMap::new()),
            capacity,
        }
    }

    fn get_segments(timeline: &mut Timeline, job: &Job) -> Vec<ObservedSegment> {
        let range = job.boundaries();
        let mut execs = timeline.runtime.plain_intervals(&range).peekable();
        let jitter = job.release_jitter();
        // Positive release jitter already accounts for the first suspension interval.
        let suspensions = timeline
            .suspension
            .plain_intervals(&range)
            .skip(usize::from(jitter > 0));
        let mut segments = Vec::new();
        let mut delay = jitter;

        for suspension in suspensions {
            let mut execution = 0;
            while let Some(interval) =
                execs.next_if(|i| i.lower_bound() <= suspension.lower_bound())
            {
                execution += interval.duration_unwrap();
            }
            segments.push((delay, execution));
            delay = suspension.duration_unwrap();
        }

        let execution = execs.map(|i| i.duration_unwrap()).sum();
        if execution > 0 {
            segments.push((delay, execution));
        }
        segments
    }

    pub fn update(&mut self, timeline: &mut Timeline, job: &Job) {
        let Some(models) = &mut self.models else {
            return;
        };
        if timeline.has_compressed_intervals(&job.boundaries()) {
            // Compressed intervals no longer identify individual segment boundaries.
            self.models = None;
            return;
        }

        let segments = Self::get_segments(timeline, job);
        let count = segments.len();
        if models.len() == self.capacity && !models.contains_key(&count) {
            self.models = None;
            return;
        }

        let extractor = models.entry(count).or_insert_with(|| {
            SegmentedSuspensionModelExtractor::new(Some(1024)).expect("positive segment limit")
        });
        if segments.is_empty() {
            // Keep the entry so the report includes `"0": []`. feed() rejects empty jobs.
            return;
        }
        if extractor.feed([segments]).is_err() {
            self.models = None;
        }
    }

    pub fn extract(&self) -> Option<BTreeMap<usize, Vec<ObservedSegment>>> {
        let models = self.models.as_ref().filter(|m| !m.is_empty())?;
        models
            .iter()
            .map(|(&count, extractor)| {
                let segments = extractor
                    .current_model()?
                    .into_iter()
                    .map(|s| (s.max_suspension_time, s.max_execution_time))
                    .collect();
                Some((count, segments))
            })
            .collect()
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
            timeline.update(e);
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
