use crate::{ir::timeline::Timeline, job::Job, utils::LowerBoundTracker};

use super::{ArrivalModel, ArrivalModelExtractor};

#[derive(Default)]
pub struct Sporadic {
    last_job: Option<Job>,
    mit: LowerBoundTracker<u64>,
}

impl Sporadic {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, _timeline: &Timeline, job: &Job) {
        if let Some(prev_job) = &self.last_job {
            assert!(prev_job.end <= job.first_cycle);

            let interarrival = job.arrival() - prev_job.arrival();

            self.mit.update(interarrival);
        }

        self.last_job.replace(job.clone());
    }

    pub fn get(&self) -> Option<u64> {
        self.mit.get()
    }
}

impl ArrivalModelExtractor for Sporadic {
    fn extract(&self) -> Option<ArrivalModel<'_>> {
        match self.get() {
            None | Some(0) => None,
            Some(mit) => Some(ArrivalModel::Sporadic { mit }),
        }
    }
}
