use crate::{
    job::Job,
    model_extractors::arrival::{ArrivalModel, ArrivalModelExtractor, Periodic},
    utils::UpperBoundTracker,
};

// A periodic arrival detector.
/// Suitable only if the arrivals can be determinded precisely and with certainty.
/// More precisely, this relies on the assumption that
/// `true_arrival == job.arrival - job.measured_jitter`.
/// This detector detects elided activations.
pub struct DiffPeriodicExtractor {
    offset: Option<u64>,
    last_arrival: Option<u64>,
    max_jitter: UpperBoundTracker<u64>,
    period: Option<u64>,
    should_update: bool,
}

impl DiffPeriodicExtractor {
    pub fn new() -> Self {
        Self {
            last_arrival: None,
            max_jitter: UpperBoundTracker::default(),
            period: None,
            offset: None,
            should_update: true,
        }
    }

    pub fn update(&mut self, job: &Job) {
        if !self.should_update {
            return;
        }

        // true arrival
        let arrival = job.arrival();
        let jitter = job.release_jitter();

        if let Some(last_arrival) = self.last_arrival {
            let delta = arrival - last_arrival;

            if let Some(period) = self.period {
                if delta == period {
                    self.max_jitter.update(jitter);
                } else {
                    self.should_update = false;
                    self.period = None;
                }
            } else {
                self.period.replace(delta);
                self.max_jitter.update(jitter);
            }
        } else {
            self.max_jitter.update(jitter);
            self.offset.replace(arrival);
        }

        self.last_arrival = Some(arrival);
    }
}

impl ArrivalModelExtractor for DiffPeriodicExtractor {
    fn extract(&self) -> Option<ArrivalModel<'_>> {
        self.period.map(|period| {
            ArrivalModel::Periodic(Periodic::Arrival {
                period,
                max_jitter: self.max_jitter.get().unwrap(),
                offset: self.offset.unwrap(),
            })
        })
    }
}
