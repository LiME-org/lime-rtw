use std::borrow::Cow;

use crate::{job::Job, utils::RingBuffer};

use super::{ArrivalModel, ArrivalModelExtractor};

pub struct ArrivalCurve {
    window: RingBuffer<u64>,
    dmins: Vec<u64>,
    dmaxs: Vec<u64>,
}

impl ArrivalCurve {
    pub fn new(capacity: usize) -> Self {
        Self {
            window: RingBuffer::new(capacity - 1),
            dmins: Vec::with_capacity(capacity),
            dmaxs: Vec::with_capacity(capacity),
        }
    }

    fn push_arrival(&mut self, arrival: u64) {
        for (i, v) in self.window.as_slice().iter().rev().enumerate() {
            assert!(v <= &arrival);

            let observed_gap = arrival - v;

            if self.dmins.len() <= i {
                self.dmins.push(observed_gap);
                self.dmaxs.push(observed_gap);
            } else {
                self.dmins[i] = self.dmins[i].min(observed_gap);
                self.dmaxs[i] = self.dmaxs[i].max(observed_gap);
            }
        }

        self.window.push(arrival);
    }

    pub fn max_jobs_in_prefix(&self) -> usize {
        self.dmins.len() + 1
    }

    pub fn min_separation(&self, n: usize) -> Option<u64> {
        match n {
            0 | 1 => Some(0),
            _ if n > self.max_jobs_in_prefix() => None,
            _ => Some(self.dmins[n - 2]),
        }
    }

    pub fn delta_mins(&self) -> impl Iterator<Item = &u64> {
        self.dmins.iter()
    }

    pub fn delta_maxs(&self) -> impl Iterator<Item = &u64> {
        self.dmaxs.iter()
    }

    pub fn update(&mut self, job: &Job) {
        let arrival = job.arrival();

        self.push_arrival(arrival)
    }
}

impl ArrivalModelExtractor for ArrivalCurve {
    fn extract(&self) -> Option<ArrivalModel<'_>> {
        if self.dmaxs.iter().all(|&x| x == 0) {
            None
        } else {
            Some(ArrivalModel::ArrivalCurve {
                dmins: Cow::from(self.dmins.as_slice()),
                dmaxs: Cow::from(self.dmaxs.as_slice()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::job::Job;

    use super::ArrivalCurve;

    #[test]
    fn test_arrival_curve() {
        let example = [1, 4, 8, 9, 10, 16];
        let mut curve = ArrivalCurve::new(4);

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

        let dmins: Vec<u64> = curve.delta_mins().copied().collect();
        assert_eq!(dmins, [1, 2, 6]);

        let dmaxs: Vec<u64> = curve.delta_maxs().copied().collect();
        assert_eq!(dmaxs, [6, 7, 8]);
    }
}
