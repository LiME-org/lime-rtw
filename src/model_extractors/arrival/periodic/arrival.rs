use crate::{
    job::Job,
    model_extractors::arrival::{ArrivalModel, ArrivalModelExtractor, Periodic},
};

/// Uses the first gap between absolute timer arrivals as the period.
/// A later gap of a different length rules out an exact period for this trace.
#[derive(Default)]
pub struct DiffPeriodicExtractor {
    period: Option<u64>,
    first_arrival: u64,
    last_arrival: Option<u64>,
    max_jitter: u64,
    rejected: bool,
}

impl DiffPeriodicExtractor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, job: &Job) {
        if self.rejected {
            return;
        }
        let arrival = job.arrival();
        if let Some(last) = self.last_arrival {
            let gap = arrival
                .checked_sub(last)
                .expect("jobs are processed in arrival order");
            if self.period.is_some_and(|period| period != gap) {
                self.rejected = true;
                self.period = None;
                return;
            }
            self.period = Some(gap);
        } else {
            self.first_arrival = arrival;
        }
        self.last_arrival = Some(arrival);
        self.max_jitter = self.max_jitter.max(job.release_jitter());
    }
}

impl ArrivalModelExtractor for DiffPeriodicExtractor {
    fn extract(&self) -> Option<ArrivalModel<'_>> {
        Some(ArrivalModel::Periodic(Periodic::Arrival {
            period: self.period?,
            max_jitter: self.max_jitter,
            offset: self.first_arrival,
        }))
    }
}
