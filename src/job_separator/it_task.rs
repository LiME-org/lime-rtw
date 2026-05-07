//! Direct job extraction for ItTask events.

use crate::{
    events::{EventData, TraceEvent},
    job::Job,
};
use std::collections::HashMap;

use super::JobSeparator;

/// Simple state tracker for ItTask jobs.
/// Tracks enter/exit events to create jobs directly.
#[derive(Debug, Clone, Default)]
pub struct ItTaskTracker {
    active_tasks: HashMap<String, ItTaskState>,
    last_sched_switch_in: Option<u64>,
    preempted: bool,
}

#[derive(Debug, Clone)]
struct ItTaskState {
    release_time: u64,
}

impl ItTaskTracker {
    pub fn new() -> Self {
        Self {
            active_tasks: HashMap::new(),
            last_sched_switch_in: None,
            preempted: false,
        }
    }

    /// Process an event and return any completed jobs.
    pub fn process_event(&mut self, event: &TraceEvent) -> Vec<(JobSeparator, Job)> {
        let mut jobs = Vec::new();

        match &event.ev {
            EventData::SchedSwitchedOut { state, .. } => {
                self.preempted = *state != 1;
            }
            EventData::SchedSwitchedIn { .. }
                if !self.preempted && self.active_tasks.is_empty() =>
            {
                self.last_sched_switch_in = Some(event.ts);
            }
            EventData::EnterItTask { it_task_id } => {
                let task_state = ItTaskState {
                    release_time: event.ts,
                };
                self.active_tasks.insert(it_task_id.clone(), task_state);
            }
            EventData::ExitItTask { it_task_id } => {
                if let Some(task_state) = self.active_tasks.remove(it_task_id) {
                    let separator = JobSeparator::ItTask {
                        it_task_id: it_task_id.clone(),
                    };

                    let job = Job {
                        arrival: None,
                        release: task_state.release_time,
                        release_lo: self.last_sched_switch_in,
                        end: event.ts,
                        first_cycle: task_state.release_time,
                    };

                    jobs.push((separator, job));
                }
            }
            EventData::LimeStartOfTrace if self.last_sched_switch_in.is_none() => {
                self.last_sched_switch_in = Some(event.ts);
            }
            _ => {}
        }

        jobs
    }

    pub fn is_tracking(&self) -> bool {
        !self.active_tasks.is_empty()
    }
}
