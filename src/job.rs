//! Job definition.

use crate::{
    interval::TimeInterval,
    ir::timeline::{layers::Layer, Timeline},
};
use serde::Serialize;

/// Job timing data.
///
/// This struct contains all the important time in the lifetime of a job.
/// It can also query the associated timeline to gather more informations for
/// model extractors.
#[derive(Debug, Clone, Serialize)]
pub struct Job {
    /// Time at which is supposed
    /// to arrive in the system.
    pub arrival: Option<u64>,
    /// First point in time the job is present in the system.
    pub release: u64,
    /// Last point in time
    /// the job is scheduled
    pub end: u64,
    /// First point in time at which the job
    /// is the last pending job.
    pub first_cycle: u64,
}

impl Job {
    /// Returns the amount of processor time used by the job.
    pub fn processor_time(&self, timeline: &mut Timeline) -> u64 {
        timeline.runtime.time(&self.boundaries())
    }

    /// Returns the amount of time the job has been suspended.
    pub fn suspension_time(&self, timeline: &mut Timeline) -> u64 {
        timeline.suspension.time(&self.boundaries())
    }

    /// Returns the amount of time the job has been preempted.
    pub fn preemption_time(&self, timeline: &mut Timeline) -> u64 {
        self.response_time() - self.processor_time(timeline) - self.suspension_time(timeline)
    }

    /// Returns the job's response time.
    pub fn response_time(&self) -> u64 {
        self.end - self.first_cycle
    }

    /// Returns the job's release jitter, i.e, the time between its arrival and
    /// its release. Always equal to 0 if no arrival time is available.
    pub fn release_jitter(&self) -> u64 {
        if let Some(expected_arrival) = self.arrival {
            return self.release.saturating_sub(expected_arrival);
        }

        0
    }

    /// Returns the job's arrival time. If no arrival time is available, then
    /// the release time is returned.
    pub fn arrival(&self) -> u64 {
        self.release - self.release_jitter()
    }

    /// Returns the time intervals spanning from the job's first to its last cycle.
    pub fn boundaries(&self) -> TimeInterval {
        TimeInterval::range(self.first_cycle, self.end)
    }
}
