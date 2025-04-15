//! Intermediate representation (IR).

use self::{
    job::JobTracker,
    timeline::{layers::Layer, Timeline},
};
use crate::{events::TraceEvent, interval::TimeInterval, job::Job, job_separator::JobSeparator};

mod job;
pub mod timeline;

/// Intermediate representation.
///
/// This struct provides a high-level view of a task activity that used by the
/// model extractors. It is in charge of tracking the application state over
/// time and to detected job boundaries.
pub struct Ir {
    pub timeline: Timeline,
    job_tracker: JobTracker,
}

impl Ir {
    /// Create a new intermediate representation.
    pub fn new() -> Self {
        let mut job_tracker = JobTracker::new();

        job_tracker.init();

        Self {
            timeline: Timeline::new(),
            job_tracker,
        }
    }

    /// Update the timeline and attempt to extract jobs.
    /// Extracted jobs are stored in the `completed_jobs` vector.
    pub fn consume_event(
        &mut self,
        event: TraceEvent,
        completed_jobs: &mut Vec<(JobSeparator, Job)>,
    ) {
        self.timeline.update(&event);
        self.job_tracker
            .consume_event(event.clone(), completed_jobs);
    }

    /// Clean the IR. This is the main IR garbage collection interface.
    /// This method is meant to be called by event processors after each update
    /// once each extracted jobs has been handled. This method changes the IR's
    /// active range, which is the time interval during which the timeline is
    /// guaranteed to be valid. Therefore, all extracted jobs must have been
    /// handled before invoking this method. Querying the timeline outside of
    /// the active range is UB.
    pub fn clean(&mut self) {
        self.job_tracker.clean();
        self.timeline.try_clean(&self.job_tracker);
    }

    pub fn cpu_time(&mut self, range: &TimeInterval) -> u64 {
        self.timeline.runtime.get_base_mut().coverage(range)
    }
}

impl Default for Ir {
    fn default() -> Self {
        Self::new()
    }
}
