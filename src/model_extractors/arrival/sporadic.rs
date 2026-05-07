use std::borrow::Cow;

use lime_model_extractors::{
    extractors::{
        DeltaMaxExtractor, DeltaMaxHiExtractor, DeltaMaxLoExtractor, DeltaMinExtractor,
        DeltaMinHiExtractor, DeltaMinLoExtractor,
    },
    time::ReleaseWindow,
};

use crate::job::Job;

use super::{ArrivalModel, ArrivalModelExtractor};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArrivalMode {
    Exact,
    Intervals,
}

pub struct Sporadic {
    nmax: Option<usize>,
    exact_dmins: DeltaMinExtractor,
    exact_dmaxs: DeltaMaxExtractor,
    interval_dmins_lo: DeltaMinHiExtractor,
    interval_dmaxs_lo: DeltaMaxHiExtractor,
    interval_dmins_hi: DeltaMinLoExtractor,
    interval_dmaxs_hi: DeltaMaxLoExtractor,
    mode: Option<ArrivalMode>,
    is_valid: bool,
}

impl Sporadic {
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

    pub fn new(capacity: usize) -> Self {
        let nmax = (capacity > 0).then_some(capacity);

        Self {
            nmax,
            exact_dmins: DeltaMinExtractor::new(nmax).expect("arrival curve dmin config is valid"),
            exact_dmaxs: DeltaMaxExtractor::new(nmax).expect("arrival curve dmax config is valid"),
            interval_dmins_lo: DeltaMinHiExtractor::new(nmax)
                .expect("arrival curve interval dmin-lo config is valid"),
            interval_dmaxs_lo: DeltaMaxHiExtractor::new(nmax)
                .expect("arrival curve interval dmax-lo config is valid"),
            interval_dmins_hi: DeltaMinLoExtractor::new(nmax)
                .expect("arrival curve interval dmin-hi config is valid"),
            interval_dmaxs_hi: DeltaMaxLoExtractor::new(nmax)
                .expect("arrival curve interval dmax-hi config is valid"),
            mode: None,
            is_valid: true,
        }
    }

    pub fn max_jobs_in_prefix(&self) -> usize {
        self.nmax.unwrap_or_else(|| match self.mode {
            Some(ArrivalMode::Intervals) => self.interval_dmins_lo.current_model().len(),
            _ => self.exact_dmins.current_model().len().saturating_sub(1),
        })
    }

    pub fn min_separation(&self, n: usize) -> Option<u64> {
        let dmins = match self.mode {
            Some(ArrivalMode::Intervals) => {
                Self::trim_and_shift_sub(self.interval_dmins_lo.current_model())
            }
            _ => Self::trim_and_shift_sub(self.exact_dmins.current_model()),
        };

        match n {
            0 | 1 => Some(0),
            _ => dmins.get(n.saturating_sub(2)).copied(),
        }
    }

    pub fn delta_mins(&self) -> impl Iterator<Item = u64> {
        match self.mode {
            Some(ArrivalMode::Intervals) => {
                Self::trim_and_shift_sub(self.interval_dmins_lo.current_model()).into_iter()
            }
            _ => Self::trim_and_shift_sub(self.exact_dmins.current_model()).into_iter(),
        }
    }

    pub fn delta_maxs(&self) -> impl Iterator<Item = u64> {
        match self.mode {
            Some(ArrivalMode::Intervals) => {
                Self::trim_and_shift_add(self.interval_dmaxs_hi.current_model()).into_iter()
            }
            _ => Self::trim_and_shift_add(self.exact_dmaxs.current_model()).into_iter(),
        }
    }

    pub fn update(&mut self, job: &Job) {
        if !self.is_valid {
            return;
        }

        match (self.mode, job.release_lo) {
            (Some(ArrivalMode::Exact), Some(_)) | (Some(ArrivalMode::Intervals), None) => {
                self.is_valid = false;
            }
            (_, Some(release_lo)) => {
                self.mode = Some(ArrivalMode::Intervals);
                let window = ReleaseWindow::new(release_lo, job.arrival());
                self.is_valid = self.interval_dmins_lo.feed([window]).is_ok()
                    && self.interval_dmaxs_lo.feed([window]).is_ok()
                    && self.interval_dmins_hi.feed([window]).is_ok()
                    && self.interval_dmaxs_hi.feed([window]).is_ok();
            }
            (_, None) => {
                self.mode = Some(ArrivalMode::Exact);
                let arrival = job.arrival();
                self.is_valid = self.exact_dmins.feed([arrival]).is_ok()
                    && self.exact_dmaxs.feed([arrival]).is_ok();
            }
        }
    }

    pub fn mit(&self) -> Option<u64> {
        if !self.is_valid || self.mode != Some(ArrivalMode::Exact) {
            return None;
        }

        self.exact_dmins
            .current_model()
            .get(2)
            .copied()
            .map(|mit| mit.saturating_sub(1))
    }

    pub fn get_release_intervals(&self) -> Option<(u64, u64)> {
        if !self.is_valid || self.mode != Some(ArrivalMode::Intervals) {
            return None;
        }

        let mit_lo = self.interval_dmins_lo.current_model().get(2).copied()?;
        let mit_hi = self.interval_dmins_hi.current_model().get(2).copied()?;

        Some((mit_lo.saturating_sub(1), mit_hi.saturating_sub(1)))
    }

    pub fn extract_sporadic(&self) -> Option<ArrivalModel<'_>> {
        if let Some((mit_lo, mit_hi)) = self.get_release_intervals() {
            if mit_lo == 0 && mit_hi == 0 {
                return None;
            }

            return Some(ArrivalModel::SporadicReleaseIntervals { mit_lo, mit_hi });
        }

        match self.mit() {
            None | Some(0) => None,
            Some(mit) => Some(ArrivalModel::Sporadic { mit }),
        }
    }

    pub fn extract_arrival_curve(&self) -> Option<ArrivalModel<'_>> {
        if !self.is_valid {
            return None;
        }

        match self.mode {
            Some(ArrivalMode::Intervals) => {
                let dmins_lo = Self::trim_and_shift_sub(self.interval_dmins_lo.current_model());
                let dmaxs_lo = Self::trim_and_shift_add(self.interval_dmaxs_lo.current_model());
                let dmins_hi = Self::trim_and_shift_sub(self.interval_dmins_hi.current_model());
                let dmaxs_hi = Self::trim_and_shift_add(self.interval_dmaxs_hi.current_model());

                if dmaxs_lo.iter().all(|&x| x == 0) && dmaxs_hi.iter().all(|&x| x == 0) {
                    None
                } else {
                    Some(ArrivalModel::ArrivalCurveReleaseIntervals {
                        dmins_lo: Cow::Owned(dmins_lo),
                        dmaxs_lo: Cow::Owned(dmaxs_lo),
                        dmins_hi: Cow::Owned(dmins_hi),
                        dmaxs_hi: Cow::Owned(dmaxs_hi),
                    })
                }
            }
            Some(ArrivalMode::Exact) => {
                let dmins = Self::trim_and_shift_sub(self.exact_dmins.current_model());
                let dmaxs = Self::trim_and_shift_add(self.exact_dmaxs.current_model());

                if dmaxs.iter().all(|&x| x == 0) {
                    None
                } else {
                    Some(ArrivalModel::ArrivalCurve {
                        dmins: Cow::Owned(dmins),
                        dmaxs: Cow::Owned(dmaxs),
                    })
                }
            }
            None => None,
        }
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
                release_lo: None,
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

        let dmins = Sporadic::trim_and_shift_sub(curve.exact_dmins.current_model());
        assert_eq!(dmins, [1, 2, 6]);

        let dmaxs = Sporadic::trim_and_shift_add(curve.exact_dmaxs.current_model());
        assert_eq!(dmaxs, [6, 7, 8]);
    }
}
