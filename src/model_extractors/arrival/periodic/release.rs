use lime_model_extractors::{
    extractors::PeriodicExtractor as CratePeriodicExtractor, PeriodicConfig,
};

use crate::job::Job;

use super::super::{ArrivalModel, ArrivalModelExtractor, Periodic};

pub struct PeriodExtractor {
    extractor: CratePeriodicExtractor,
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

        let extractor = CratePeriodicExtractor::with_config(config)
            .expect("periodic extractor config is valid");

        Self { extractor }
    }

    pub fn update(&mut self, job: &Job) {
        self.extractor.feed([job.release]);
    }
}

impl Default for PeriodExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl ArrivalModelExtractor for PeriodExtractor {
    fn extract(&self) -> Option<ArrivalModel<'_>> {
        let model = self.extractor.current_model()?;

        if model.offset < 0 || model.jitter < 0 {
            return None;
        }

        Some(ArrivalModel::Periodic(Periodic::Release {
            period: model.period,
            offset: model.offset as u64,
            max_jitter: model.jitter as u64,
        }))
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
