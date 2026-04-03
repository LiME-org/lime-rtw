use std::borrow::Cow;

use lime_model_extractors::extractors::{DeltaMaxExtractor, DeltaMinExtractor};

use crate::job::Job;

use super::{ArrivalModel, ArrivalModelExtractor};

pub struct Sporadic {
    dmin_extractor: DeltaMinExtractor,
    dmax_extractor: DeltaMaxExtractor,
}

impl Sporadic {
    pub fn new(capacity: usize) -> Self {
        let nmax = (capacity > 0).then_some(capacity);

        Self {
            dmin_extractor: DeltaMinExtractor::new(nmax)
                .expect("arrival curve dmin config is valid"),
            dmax_extractor: DeltaMaxExtractor::new(nmax)
                .expect("arrival curve dmax config is valid"),
        }
    }

    pub fn max_jobs_in_prefix(&self) -> usize {
        self.dmin_extractor.current_model().len().saturating_sub(1)
    }

    pub fn min_separation(&self, n: usize) -> Option<u64> {
        match n {
            0 | 1 => Some(0),
            _ if n > self.max_jobs_in_prefix() => None,
            _ => self
                .dmin_extractor
                .current_model()
                .get(n)
                .copied()
                .map(|dmin| dmin.saturating_sub(1)),
        }
    }

    pub fn delta_mins(&self) -> impl Iterator<Item = u64> {
        Self::trim_and_shift_sub(self.dmin_extractor.current_model()).into_iter()
    }

    pub fn delta_maxs(&self) -> impl Iterator<Item = u64> {
        Self::trim_and_shift_add(self.dmax_extractor.current_model()).into_iter()
    }

    pub fn update(&mut self, job: &Job) {
        let arrival = job.arrival();
        self.dmin_extractor
            .feed([arrival])
            .expect("jobs are processed in arrival order");
        self.dmax_extractor
            .feed([arrival])
            .expect("jobs are processed in arrival order");
    }

    pub fn mit(&self) -> Option<u64> {
        self.dmin_extractor
            .current_model()
            .get(2)
            .copied()
            .map(|mit| mit - 1)
    }

    pub fn extract_sporadic(&self) -> Option<ArrivalModel<'_>> {
        match self.mit() {
            None | Some(0) => None,
            Some(mit) => Some(ArrivalModel::Sporadic { mit }),
        }
    }

    pub fn extract_arrival_curve(&self) -> Option<ArrivalModel<'_>> {
        let dmins = Self::trim_and_shift_sub(self.dmin_extractor.current_model());
        let dmaxs = Self::trim_and_shift_add(self.dmax_extractor.current_model());

        if dmaxs.iter().all(|&x| x == 0) {
            None
        } else {
            Some(ArrivalModel::ArrivalCurve {
                dmins: Cow::Owned(dmins),
                dmaxs: Cow::Owned(dmaxs),
            })
        }
    }

    // Keep the legacy Lime indexing semantics by dropping the first two
    // entries from the delta-min model and shifting the remaining values down.
    fn trim_and_shift_sub(mut values: Vec<u64>) -> Vec<u64> {
        values.drain(..2);
        for value in &mut values {
            *value = value.saturating_sub(1);
        }
        values
    }

    // Keep the legacy Lime indexing semantics by dropping the last two
    // entries from the delta-max model and shifting the remaining values up.
    fn trim_and_shift_add(mut values: Vec<u64>) -> Vec<u64> {
        values.truncate(values.len().saturating_sub(2));
        for value in &mut values {
            *value = value.saturating_add(1);
        }
        values
    }
}

impl ArrivalModelExtractor for Sporadic {
    fn extract(&self) -> Option<ArrivalModel<'_>> {
        self.extract_arrival_curve()
    }
}

#[cfg(test)]
mod tests {
    use crate::job::Job;

    use super::Sporadic;

    #[test]
    fn test_arrival_curve() {
        let example = [1, 4, 8, 9, 10, 16];
        let mut curve = Sporadic::new(4);

        for t in example.iter() {
            let job = Job {
                end: 0,
                first_cycle: *t,
                arrival: None,
                release: *t,
            };

            curve.update(&job);
        }

        assert_eq!(curve.max_jobs_in_prefix(), 4);

        assert_eq!(curve.min_separation(0), Some(0));
        assert_eq!(curve.min_separation(1), Some(0));
        assert_eq!(curve.min_separation(2), Some(1));
        assert_eq!(curve.min_separation(3), Some(2));
        assert_eq!(curve.min_separation(4), Some(6));
        assert_eq!(curve.min_separation(5), None);
        assert_eq!(curve.mit(), Some(1));

        let dmins = Sporadic::trim_and_shift_sub(curve.dmin_extractor.current_model());
        assert_eq!(dmins, [1, 2, 6]);

        let dmaxs = Sporadic::trim_and_shift_add(curve.dmax_extractor.current_model());
        assert_eq!(dmaxs, [6, 7, 8]);
    }
}
