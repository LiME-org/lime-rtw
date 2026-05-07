use lime_model_extractors::{
    extractors::{
        CertainFitPeriodicExtractor, PeriodicExtractor as ExactPeriodicExtractor,
        PossibleFitPeriodicExtractor,
    },
    time::ReleaseWindow,
    PeriodicConfig,
};

use crate::job::Job;

use super::super::{ArrivalModel, ArrivalModelExtractor, Periodic};

#[derive(Debug)]
enum Backend {
    Exact(ExactPeriodicExtractor),
    Intervals {
        certain: CertainFitPeriodicExtractor,
        possible: PossibleFitPeriodicExtractor,
    },
}

pub struct PeriodExtractor {
    config: PeriodicConfig,
    backend: Option<Backend>,
}

impl PeriodExtractor {
    pub fn new() -> Self {
        Self::with_batch_size(PeriodicConfig::default().batch_size)
    }

    pub fn with_batch_size(batch_size: usize) -> Self {
        let config = PeriodicConfig {
            batch_size: sanitize_batch_size(batch_size),
            negligible_jitter_threshold: 1_000_000,
            ..PeriodicConfig::default()
        };

        Self {
            config,
            backend: None,
        }
    }

    pub fn analyse_batch_and_clear(&mut self) {}

    pub fn update(&mut self, job: &Job) {
        match (&mut self.backend, job.release_lo) {
            (Some(Backend::Exact(exact)), None) => exact.feed([job.release]),
            (Some(Backend::Intervals { certain, possible }), Some(release_lo)) => {
                let window = ReleaseWindow::new(release_lo, job.release);
                certain.feed([window]);
                possible.feed([window]);
            }
            (None, None) => {
                let mut exact = ExactPeriodicExtractor::with_config(self.config.clone())
                    .expect("periodic extractor config is valid");
                exact.feed([job.release]);
                self.backend = Some(Backend::Exact(exact));
            }
            (None, Some(release_lo)) => {
                let mut certain = CertainFitPeriodicExtractor::with_config(self.config.clone())
                    .expect("certain-fit periodic extractor config is valid");
                let mut possible = PossibleFitPeriodicExtractor::with_config(self.config.clone())
                    .expect("possible-fit periodic extractor config is valid");
                let window = ReleaseWindow::new(release_lo, job.release);
                certain.feed([window]);
                possible.feed([window]);
                self.backend = Some(Backend::Intervals { certain, possible });
            }
            (Some(Backend::Exact(_)), Some(_)) | (Some(Backend::Intervals { .. }), None) => {
                panic!("mixed point-release and release-window jobs for one periodic extractor")
            }
        }
    }
}

impl Default for PeriodExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl ArrivalModelExtractor for PeriodExtractor {
    fn extract(&self) -> Option<ArrivalModel<'_>> {
        match &self.backend {
            Some(Backend::Intervals { certain, possible }) => {
                let certainly = certain.current_model()?;
                let possibly = possible.current_model()?;

                if certainly.offset < 0
                    || certainly.jitter < 0
                    || possibly.offset < 0
                    || possibly.jitter < 0
                {
                    return None;
                }

                Some(ArrivalModel::Periodic(Periodic::ReleaseIntervals {
                    period_certainly: certainly.period,
                    offset_certainly: certainly.offset as u64,
                    max_jitter_certainly: certainly.jitter as u64,
                    period_possibly: possibly.period,
                    offset_possibly: possibly.offset as u64,
                    max_jitter_possibly: possibly.jitter as u64,
                }))
            }
            Some(Backend::Exact(exact)) => {
                let exact = exact.current_model()?;

                if exact.offset < 0 || exact.jitter < 0 {
                    return None;
                }

                Some(ArrivalModel::Periodic(Periodic::Release {
                    period: exact.period,
                    offset: exact.offset as u64,
                    max_jitter: exact.jitter as u64,
                }))
            }
            None => None,
        }
    }
}

fn sanitize_batch_size(batch_size: usize) -> usize {
    if batch_size > 1 {
        batch_size
    } else {
        PeriodicConfig::default().batch_size
    }
}

#[cfg(test)]
mod tests {
    use crate::job::Job;

    use super::PeriodExtractor;
    use crate::model_extractors::arrival::{ArrivalModel, ArrivalModelExtractor, Periodic};

    fn point_job(release: u64) -> Job {
        Job {
            arrival: None,
            release_lo: None,
            release,
            end: release,
            first_cycle: release,
        }
    }

    #[test]
    fn extracts_point_release_model() {
        let mut extractor = PeriodExtractor::with_batch_size(3);
        for release in [10, 50, 90, 130] {
            extractor.update(&point_job(release));
        }

        assert_eq!(
            extractor.extract(),
            Some(ArrivalModel::Periodic(Periodic::Release {
                period: 40,
                offset: 10,
                max_jitter: 0,
            }))
        );
    }
}
