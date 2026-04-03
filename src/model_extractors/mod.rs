use self::arrival::periodic::PeriodExtractor;
use self::event_rate::EventRate;
use self::rbf::RbfExtractor;
use self::suspension::BagOfSegmentedSelfSuspension;
use self::{
    arrival::{sporadic::Sporadic, ArrivalModel, ArrivalModelExtractor},
    execution::ExecutionTime,
    job_count::JobCounter,
    suspension::DynamicSelfSuspension,
};
use crate::ir::Ir;
use crate::job::Job;
use crate::task::TaskInfos;
use crate::{
    context::LimeContext, events::TraceEvent, ir::timeline::Timeline, job_separator::JobSeparator,
    utils::Dispatcher,
};

pub mod arrival;
pub mod dd;
pub mod execution;
pub mod job_count;
pub mod rbf;
pub mod suspension;

mod event_rate;

struct JobSeparatorExtractors {
    pub wcet: ExecutionTime,
    pub suspension: DynamicSelfSuspension,
    pub bsss: BagOfSegmentedSelfSuspension,
    // pub segmented_self_suspension: SegementedSelfSuspension,
    pub count: JobCounter,
    pub periodic: PeriodExtractor,
    pub sporadic: Sporadic,
}

impl JobSeparatorExtractors {
    pub fn new(sep: &JobSeparator, ctx: &LimeContext) -> Self {
        Self {
            wcet: ExecutionTime::new(ctx.wcet_n_max_len),
            suspension: DynamicSelfSuspension::new(),
            bsss: BagOfSegmentedSelfSuspension::new(),
            // segmented_self_suspension: SegementedSelfSuspension::new(),
            count: JobCounter::new(),
            sporadic: Sporadic::new(ctx.arrival_curve_max_len),
            periodic: PeriodExtractor::from_job_separator(sep, ctx),
        }
    }

    pub fn update(&mut self, job: &Job, timeline: &mut Timeline) {
        self.wcet.update(timeline, job);
        self.suspension.update(timeline, job);
        self.bsss.update(timeline, job);
        self.count.update(job);
        self.sporadic.update(job);
        self.periodic.update(job);
    }

    pub fn arrival_models(&self) -> Vec<ArrivalModel<'_>> {
        let mut ret = vec![];

        if let Some(c) = self.sporadic.extract_arrival_curve() {
            ret.push(c)
        }

        if let Some(m) = self.sporadic.extract_sporadic() {
            ret.push(m)
        }

        if let Some(m) = self.periodic.extract() {
            ret.push(m)
        }

        ret
    }
}

/// Task-level model extraction facade.
///
/// Any new model extractor must be interfaced in this structure.
pub struct ThreadTaskModelExtractor<'a> {
    event_rate: EventRate,
    rbf: RbfExtractor<'a>,
    // rbf: RbfExtractor,
    job_separators_extractors: Dispatcher<JobSeparator, JobSeparatorExtractors>,
    ir: Ir,
    detected_jobs: Vec<(JobSeparator, Job)>,
}

impl ThreadTaskModelExtractor<'_> {
    /// Create a new model extractor for a task.
    pub fn new(ctx: &LimeContext) -> Self {
        Self {
            event_rate: EventRate::new(),
            rbf: RbfExtractor::from_lime_context(ctx),
            job_separators_extractors: Dispatcher::new(),
            ir: Ir::new(),
            detected_jobs: Vec::new(),
        }
    }

    fn update_ir(&mut self, event: TraceEvent) {
        self.event_rate.update(&event);
        self.rbf.update(&event);
        self.ir.consume_event(event, &mut self.detected_jobs);
    }

    fn clean_ir(&mut self) {
        self.ir.clean();
    }

    fn send_new_jobs_to_extractors(&mut self, ctx: &LimeContext) {
        for (sep, job) in self.detected_jobs.drain(..) {
            self.job_separators_extractors
                .get_or_new(&sep, || JobSeparatorExtractors::new(&sep, ctx))
                .update(&job, &mut self.ir.timeline)
        }
    }

    /// Update the IR, and update contained extractors state with newly detected
    /// jobs.
    pub fn consume_event(&mut self, event: TraceEvent, ctx: &LimeContext) {
        self.update_ir(event);
        self.send_new_jobs_to_extractors(ctx);
        self.clean_ir()
    }

    /// Returns true if no models have been extracted so far.
    pub fn is_empty(&self) -> bool {
        self.rbf.is_empty() && self.job_separators_extractors.is_empty()
    }

    /// Flush buffered extractors.
    pub fn flush(&mut self) {
        self.rbf.flush();
    }

    /// Returns true if models should be outputted for this task.
    pub fn should_report(&self, ctx: &LimeContext, infos: &Option<TaskInfos>) -> bool {
        if !ctx.all_threads && self.is_empty() {
            // This is some short-lived thread for which no model
            // whatsoever could be inferred. Nothing interesting
            // can be deduced from it. Let's not bother the user
            // with it.
            return false;
        }

        if let Some(infos) = infos {
            if !ctx.trace_best_effort && !infos.policy.is_rt_policy() {
                // This is not a real-time task and we want to
                // report only real-time tasks.
                return false;
            }
        } else if !ctx.trace_best_effort {
            // We don't know what it is, and we want to report
            // only (confirmed) real-time tasks => let's ignore
            // this one.
            return false;
        }

        true
    }
}
