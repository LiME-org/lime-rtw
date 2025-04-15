use crate::interval::TimeInterval;
use crate::{events::TraceEvent, interval::Interval};

use crate::events::EventData::*;

use super::{base::BaseLayer, Layer};

pub struct SuspensionLayer {
    base: BaseLayer,
}

impl Layer for SuspensionLayer {
    fn update(&mut self, event: &TraceEvent) {
        match event.ev {
            SchedWakeUp { .. } => self.base.push_upper_bound(event.ts),

            SchedSwitchedOut { .. } if event.is_suspension() => {
                self.base.push_lower_bound(event.ts);
            }

            _ => {}
        }
    }

    fn get_base(&self) -> &BaseLayer {
        &self.base
    }

    fn get_base_mut(&mut self) -> &mut BaseLayer {
        &mut self.base
    }
}

impl SuspensionLayer {
    pub fn new() -> Self {
        Self {
            base: BaseLayer::new(),
        }
    }

    pub fn plain_intervals<'a>(
        &'a mut self,
        range: &'a TimeInterval,
    ) -> impl Iterator<Item = Interval<u64>> + 'a {
        self.base.plain_intervals(range)
    }
}

impl Default for SuspensionLayer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::events::EventData::*;
    use crate::events::TraceEvent;
    use crate::interval::TimeInterval;
    use crate::ir::timeline::layers::suspension::SuspensionLayer;
    use crate::ir::timeline::layers::Layer;
    use crate::utils::ThreadId;

    const ID: ThreadId = ThreadId { pid: 0, tgid: 0 };

    const EXAMPLE_2: &[TraceEvent] = &[
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
            ts: 15,
            ev: SchedWakeUp { cpu: 0 },
        },
        TraceEvent {
            id: ID,
            ts: 16,
            ev: SchedSwitchedIn {
                cpu: 0,
                prio: 0,
                preempt: true,
            },
        },
        TraceEvent {
            id: ID,
            ts: 20,
            ev: SchedSwitchedOut {
                cpu: 0,
                prio: 0,
                state: 1,
            },
        },
        TraceEvent {
            id: ID,
            ts: 30,
            ev: SchedWakeUp { cpu: 0 },
        },
        TraceEvent {
            id: ID,
            ts: 31,
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
                state: 0,
            },
        },
        TraceEvent {
            id: ID,
            ts: 45,
            ev: SchedWakeUp { cpu: 0 },
        },
        TraceEvent {
            id: ID,
            ts: 46,
            ev: SchedSwitchedIn {
                cpu: 0,
                prio: 0,
                preempt: true,
            },
        },
        TraceEvent {
            id: ID,
            ts: 50,
            ev: SchedSwitchedOut {
                cpu: 0,
                prio: 0,
                state: 2,
            },
        },
        TraceEvent {
            id: ID,
            ts: 60,
            ev: SchedWakeUp { cpu: 0 },
        },
        TraceEvent {
            id: ID,
            ts: 61,
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
                state: 128,
            },
        },
        TraceEvent {
            id: ID,
            ts: 75,
            ev: SchedWakeUp { cpu: 0 },
        },
    ];

    #[test]
    fn test_suspension_tracker() {
        let mut tracker = SuspensionLayer::new();

        let range = TimeInterval::range(20, 30);
        assert_eq!(tracker.time(&range), 0);

        for event in EXAMPLE_2.iter() {
            tracker.update(event);
        }

        let range = TimeInterval::range(0, 60);
        assert_eq!(tracker.time(&range), 20)
    }
}
