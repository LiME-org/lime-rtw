use super::ArrivalModelExtractor;
use crate::{context::LimeContext, events::ClockId, job::Job, job_separator::JobSeparator};

mod arrival;
mod release;

enum InnerPeriodExtractor {
    OnArrival(arrival::DiffPeriodicExtractor),
    OnRelease(release::PeriodExtractor),
}

pub struct PeriodExtractor {
    inner: InnerPeriodExtractor,
}

impl PeriodExtractor {
    pub fn from_job_separator(sep: &JobSeparator, ctx: &LimeContext) -> Self {
        match sep {
            JobSeparator::ClockNanosleep {
                clock_id: ClockId::ClockMonotonic,
                abs_time: true,
            } => Self::new_on_arrival(ctx),
            _ => Self::new_on_release(ctx),
        }
    }

    fn new_on_arrival(_ctx: &LimeContext) -> Self {
        let extractor = arrival::DiffPeriodicExtractor::new();
        let inner = InnerPeriodExtractor::OnArrival(extractor);

        Self { inner }
    }

    fn new_on_release(ctx: &LimeContext) -> Self {
        let batch_size = ctx.period_extractor_batch_size;
        let extractor = release::PeriodExtractor::with_batch_size(batch_size);
        let inner = InnerPeriodExtractor::OnRelease(extractor);

        Self { inner }
    }

    pub fn update(&mut self, job: &Job) {
        match self.inner {
            InnerPeriodExtractor::OnArrival(ref mut e) => e.update(job),
            InnerPeriodExtractor::OnRelease(ref mut e) => e.update(job),
        }
    }

    pub fn flush(&mut self) {
        if let InnerPeriodExtractor::OnRelease(ref mut e) = self.inner {
            e.analyse_batch_and_clear()
        }
    }
}

impl ArrivalModelExtractor for PeriodExtractor {
    fn extract(&self) -> Option<super::ArrivalModel<'_>> {
        match &self.inner {
            InnerPeriodExtractor::OnArrival(e) => e.extract(),
            InnerPeriodExtractor::OnRelease(e) => e.extract(),
        }
    }
}
