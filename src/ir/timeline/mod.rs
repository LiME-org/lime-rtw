use crate::{
    events::TraceEvent,
    interval::{Interval, TimeInterval},
};

use self::{layers::execution::ExecutionLayer, layers::suspension::SuspensionLayer, layers::Layer};

use super::job::JobTracker;

pub mod layers;

/// Tracks the state of a task over time.
///
/// A timeline consists of different layers. A layer is an ordered sequence of
/// time intervals during which the task is in a particular state.
/// Each layer can be queried using the methods of the `Layer`. In addition to
/// these methods, each layer implements a `plain_intervals` method to iterate
/// over its contained intervals.
pub struct Timeline {
    /// Tracks the times at which a task is running on a processor.
    pub runtime: ExecutionLayer,
    /// Tracks the times at which a task is suspended.
    pub suspension: SuspensionLayer,

    update_before_clean: usize,
    compression_point: Option<u64>,
    compression_thresh: usize,
}

const CLEAN_PERIOD: usize = 32;
const COMPRESSION_DEFAULT_THRESH: usize = 1024;

impl Timeline {
    pub fn new() -> Self {
        Self {
            runtime: ExecutionLayer::new(),
            suspension: SuspensionLayer::new(),
            update_before_clean: CLEAN_PERIOD,
            compression_point: None,
            compression_thresh: COMPRESSION_DEFAULT_THRESH,
        }
    }

    fn trimming_point(&self, job_tracker: &JobTracker) -> u64 {
        job_tracker.oldest_job_first_cycle()
    }

    pub fn has_compressed_intervals(&self, range: &Interval<u64>) -> bool {
        self.runtime.has_compressed_intervals(range)
            || self.suspension.has_compressed_intervals(range)
    }

    fn plain_trimming_point(
        &self,
        compression_point: Option<u64>,
        job_tracker: &JobTracker,
    ) -> Option<u64> {
        if let Some(c) = compression_point {
            let mut candidates = job_tracker.pendings_first_cycle();
            candidates.retain(|arr| arr >= &c);

            return candidates.iter().min().copied();
        }

        None
    }

    pub fn try_clean(&mut self, job_tracker: &JobTracker) {
        self.update_before_clean = self.update_before_clean.saturating_sub(1);

        if self.update_before_clean == 0 && !job_tracker.ongoing_matching() {
            self.update_before_clean = CLEAN_PERIOD;

            self.clean(job_tracker);
        }
    }

    fn do_compress(&mut self, until: u64, new_block: bool) {
        self.runtime.compress(until, new_block);
        self.suspension.compress(until, new_block);
    }

    fn do_compress_all(&mut self, new_block: bool) {
        self.runtime.compress_all(new_block);
        self.suspension.compress_all(new_block);
    }

    fn trim(&mut self, job_tracker: &JobTracker) {
        let trimming_point = self.trimming_point(job_tracker);

        if let Some(c) = self.compression_point {
            if trimming_point <= c {
                let t_p = self.plain_trimming_point(self.compression_point, job_tracker);

                match t_p {
                    Some(p) => self.do_compress(p, false),
                    None => self.do_compress_all(false),
                }

                if trimming_point < c {
                    self.compression_point = Some(trimming_point)
                }
            }
        }

        let range = TimeInterval::infinite_right(trimming_point);

        self.runtime.trim(&range);
        self.suspension.trim(&range);
    }

    fn clean(&mut self, job_tracker: &JobTracker) {
        self.trim(job_tracker);

        if self.weight() > self.compression_thresh {
            self.compress(job_tracker);
        }
    }

    fn next_compression_point(&self, prev: Option<u64>, job_tracker: &JobTracker) -> Option<u64> {
        match prev {
            Some(p) => {
                let arrivals = &job_tracker.pendings_first_cycle()[1..];

                for arrival in arrivals {
                    if arrival > &p {
                        return Some(*arrival);
                    }
                }

                None
            }
            None => job_tracker.pendings_first_cycle()[1..].first().copied(),
        }
    }

    pub fn processor_time(&mut self, range: &Interval<u64>) -> u64 {
        self.runtime.time(range)
    }

    pub fn suspension_time(&mut self, range: &Interval<u64>) -> u64 {
        self.suspension.time(range)
    }

    fn compress(&mut self, job_tracker: &JobTracker) {
        let next_compression_point =
            self.next_compression_point(self.compression_point, job_tracker);

        if next_compression_point.is_some() {
            self.compression_point = next_compression_point;
            match self.plain_trimming_point(next_compression_point, job_tracker) {
                Some(c) => self.do_compress(c, true),
                None => self.do_compress_all(true),
            }
        }
    }

    pub fn update(&mut self, event: &TraceEvent) {
        self.runtime.update(event);
        self.suspension.update(event);
    }

    /// Used to determine whether the timeline needs to be compressed.
    fn weight(&self) -> usize {
        let r = self.runtime.get_base().plain_interval_nb();
        let s = self.suspension.get_base().plain_interval_nb();

        r.max(s)
    }
}

impl Default for Timeline {
    fn default() -> Self {
        Self::new()
    }
}
