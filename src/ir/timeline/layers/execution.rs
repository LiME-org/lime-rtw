use crate::{
    events::{EventData::*, TraceEvent},
    interval::{Interval, TimeInterval},
};

use super::{base::BaseLayer, Layer};

pub struct ExecutionLayer {
    recorder: BaseLayer,
}

impl Layer for ExecutionLayer {
    fn update(&mut self, event: &TraceEvent) {
        match event.ev {
            SchedSwitchedIn { .. } | SchedulerChanged { .. } => {
                self.recorder.push_lower_bound(event.ts);
            }

            SchedSwitchedOut { .. } => {
                self.recorder.push_upper_bound(event.ts);
            }

            _ => {}
        }
    }

    fn get_base(&self) -> &BaseLayer {
        &self.recorder
    }

    fn get_base_mut(&mut self) -> &mut BaseLayer {
        &mut self.recorder
    }
}

impl ExecutionLayer {
    pub fn new() -> Self {
        Self {
            recorder: BaseLayer::new(),
        }
    }

    pub fn plain_intervals<'a>(
        &'a mut self,
        range: &'a TimeInterval,
    ) -> impl Iterator<Item = Interval<u64>> + 'a {
        self.recorder.plain_intervals(range)
    }
}

impl Default for ExecutionLayer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::events::EventData::*;
    use crate::events::TraceEvent;
    use crate::interval::TimeInterval;
    use crate::ir::timeline::layers::execution::ExecutionLayer;
    use crate::ir::timeline::layers::Layer;
    use crate::utils::ThreadId;

    const ID: ThreadId = ThreadId { pid: 0, tgid: 0 };

    const EXAMPLE_1: &[TraceEvent] = &[
        TraceEvent {
            id: ID,
            ts: 0,
            ev: SchedSwitchedIn {
                cpu: 0,
                prio: 0,
                preempt: true,
            },
        },
        TraceEvent {
            id: ID,
            ts: 5,
            ev: SchedSwitchedOut {
                cpu: 0,
                prio: 0,
                state: 0,
            },
        },
        TraceEvent {
            id: ID,
            ts: 20,
            ev: SchedSwitchedIn {
                cpu: 0,
                prio: 0,
                preempt: true,
            },
        },
        TraceEvent {
            id: ID,
            ts: 25,
            ev: SchedSwitchedOut {
                cpu: 0,
                prio: 0,
                state: 0,
            },
        },
        TraceEvent {
            id: ID,
            ts: 30,
            ev: SchedSwitchedIn {
                cpu: 0,
                prio: 0,
                preempt: true,
            },
        },
        TraceEvent {
            id: ID,
            ts: 35,
            ev: SchedSwitchedOut {
                cpu: 0,
                prio: 0,
                state: 2,
            },
        },
        TraceEvent {
            id: ID,
            ts: 40,
            ev: SchedSwitchedIn {
                cpu: 0,
                prio: 0,
                preempt: true,
            },
        },
        TraceEvent {
            id: ID,
            ts: 45,
            ev: SchedSwitchedOut {
                cpu: 0,
                prio: 0,
                state: 0,
            },
        },
        TraceEvent {
            id: ID,
            ts: 50,
            ev: SchedSwitchedIn {
                cpu: 0,
                prio: 0,
                preempt: true,
            },
        },
        TraceEvent {
            id: ID,
            ts: 55,
            ev: SchedSwitchedOut {
                cpu: 0,
                prio: 0,
                state: 1,
            },
        },
        TraceEvent {
            id: ID,
            ts: 60,
            ev: SchedSwitchedIn {
                cpu: 0,
                prio: 0,
                preempt: true,
            },
        },
        TraceEvent {
            id: ID,
            ts: 65,
            ev: SchedSwitchedOut {
                cpu: 0,
                prio: 0,
                state: 0,
            },
        },
        TraceEvent {
            id: ID,
            ts: 70,
            ev: SchedSwitchedIn {
                cpu: 0,
                prio: 0,
                preempt: true,
            },
        },
        TraceEvent {
            id: ID,
            ts: 75,
            ev: SchedSwitchedOut {
                cpu: 0,
                prio: 0,
                state: 2,
            },
        },
    ];

    #[test]
    fn test_runtime_tracker() {
        let mut tracker = ExecutionLayer::new();
        let range = TimeInterval::range(20, 30);

        assert_eq!(tracker.time(&range), 0);

        for event in EXAMPLE_1[2..5].iter() {
            tracker.update(event);
        }

        assert_eq!(tracker.time(&range), 5)
    }
}
