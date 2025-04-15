//! Job extractor.

use std::{
    fs::{File, OpenOptions},
    io::BufWriter,
    path::PathBuf,
};

use crate::{
    context::LimeContext,
    events::{EventData, TraceEvent},
    io::LimeOutputDirectory,
    ir::Ir,
    job::Job,
    job_separator::JobSeparator,
    task::TaskId,
    utils::{Dispatcher, ThreadId},
    EventProcessor,
};
use anyhow::Result;
use serde::Serialize;

#[derive(Serialize)]
pub struct JobSummary {
    #[serde(flatten)]
    job: Job,
    execution_time: u64,
    suspension_time: u64,
}

#[derive(Serialize)]
struct SeparatorJobSummaries<'a> {
    separator: &'a JobSeparator,
    jobs: &'a [JobSummary],
}

impl JobSummary {
    pub fn new(job: Job, execution_time: u64, suspension_time: u64) -> Self {
        Self {
            job,
            execution_time,
            suspension_time,
        }
    }
}

fn open_job_summary_file(base: &str, task_id: &TaskId) -> Result<File> {
    let mut b = PathBuf::from(base);
    let filename = format!("{}.jobs.json", task_id);

    b.push(filename);

    if !b.exists() {
        let ret = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(b.as_path())?;

        return Ok(ret);
    }

    anyhow::bail!("File {} already exists", b.as_os_str().to_string_lossy())
}

struct TaskJobExtractor {
    ir: Ir,
    job_summaries: Dispatcher<JobSeparator, Vec<JobSummary>>,
    detected_jobs: Vec<(JobSeparator, Job)>,
}

impl TaskJobExtractor {
    fn new() -> Self {
        Self {
            ir: Ir::new(),
            job_summaries: Dispatcher::new(),
            detected_jobs: Vec::new(),
        }
    }

    fn separator_job_summaries(&self) -> Vec<SeparatorJobSummaries<'_>> {
        self.job_summaries
            .items()
            .map(|(s, j)| SeparatorJobSummaries {
                separator: s,
                jobs: j.as_slice(),
            })
            .collect()
    }

    fn consume_event(&mut self, event: TraceEvent) {
        self.ir.consume_event(event, &mut self.detected_jobs);

        for (sep, job) in self.detected_jobs.drain(..) {
            let job_summaries = self.job_summaries.get_or_default(&sep);
            let exec = job.processor_time(&mut self.ir.timeline);
            let susp = job.suspension_time(&mut self.ir.timeline);
            let summary = JobSummary::new(job, exec, susp);
            job_summaries.push(summary);
        }

        self.ir.clean();
    }

    pub fn write(&self, output_dir: &str, task_id: &TaskId) -> Result<()> {
        let f = open_job_summary_file(output_dir, task_id)?;
        let w = BufWriter::new(f);
        let data = self.separator_job_summaries();

        serde_json::to_writer_pretty(w, &data)?;

        Ok(())
    }
}

pub struct JobExtractor {
    output_dir: LimeOutputDirectory,
    task_job_extractors: Dispatcher<TaskId, TaskJobExtractor>,
}

impl JobExtractor {
    pub fn new(output_directory: LimeOutputDirectory) -> Self {
        Self {
            output_dir: output_directory,
            task_job_extractors: Dispatcher::new(),
        }
    }

    fn write(&self) -> Result<()> {
        for (task_id, j) in self.task_job_extractors.items() {
            j.write(self.output_dir.path(), task_id)?;
        }

        Ok(())
    }
}

impl From<&LimeContext> for JobExtractor {
    fn from(ctx: &LimeContext) -> Self {
        Self::new(ctx.output_dir.clone())
    }
}

impl EventProcessor for JobExtractor {
    fn pre_load_init(&mut self, _ctx: &LimeContext) -> Result<()> {
        Ok(())
    }

    fn post_load_init(&mut self, _ctx: &LimeContext) -> Result<()> {
        self.output_dir.create_dir()?;

        Ok(())
    }

    fn consume_event(&mut self, task_id: &TaskId, event: TraceEvent, _ctx: &LimeContext) {
        let e = self
            .task_job_extractors
            .get_or_new(task_id, TaskJobExtractor::new);
        e.consume_event(event);
    }

    fn finalize<S: crate::EventSource>(&mut self, _src: &S, _ctx: &LimeContext) -> Result<()> {
        // Notifify end of trace to extractors
        let id = ThreadId::new(0, 0);
        let e = TraceEvent {
            ts: 0,
            id,
            ev: EventData::LimeEndOfTrace,
        };

        for extractor in self.task_job_extractors.values_mut() {
            extractor.consume_event(e.clone())
        }

        self.write()?;

        eprintln!("Results saved in {}.", self.output_dir.path());

        Ok(())
    }
}
