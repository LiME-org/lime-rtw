//! Task model extractor.

use std::io::Write;

use crate::{
    context::LimeContext,
    events::{EventData, TraceEvent},
    io::LimeOutputDirectory,
    model_extractors::{dd, ThreadTaskModelExtractor},
    task::{TaskId, TaskInfos},
    utils::{Dispatcher, ThreadId},
    EventProcessor, EventSource,
};

use anyhow::Result;

pub struct TaskModelExtractor {
    extractors: Dispatcher<TaskId, ThreadTaskModelExtractor<'static>>,
    output_directory: LimeOutputDirectory,
}

impl TaskModelExtractor {
    pub fn new(output_directory: LimeOutputDirectory) -> Self {
        Self {
            extractors: Dispatcher::new(),
            output_directory,
        }
    }

    pub fn thread_extractor(&self) -> impl Iterator<Item = (&TaskId, &ThreadTaskModelExtractor)> {
        self.extractors.items()
    }

    fn write_thread_infos<W: Write>(&self, writer: &mut W, tinfo: &TaskInfos) -> Result<()> {
        serde_json::to_writer_pretty(writer, &tinfo)?;

        Ok(())
    }

    fn write_thread_model<W>(
        &self,
        writer: &mut W,
        thread_extractor: &ThreadTaskModelExtractor,
    ) -> Result<()>
    where
        W: Write,
    {
        let m = dd::ThreadTaskModelReport::from_extractor(thread_extractor);

        serde_json::to_writer_pretty(writer, &m).map_err(anyhow::Error::new)
    }

    fn report_task(
        &self,
        task_id: &TaskId,
        extractor: &ThreadTaskModelExtractor,
        infos: &Option<TaskInfos>,
    ) -> Result<(bool, bool)> {
        let mut info_file_created = false;
        let mut model_file_created = false;

        if let Some(infos) = infos {
            if let Ok(mut f_infos) = self.output_directory.create_infos_file(task_id) {
                info_file_created = true;
                _ = self.write_thread_infos(&mut f_infos, infos);
            }
        }

        if let Ok(mut f_models) = self.output_directory.create_models_file(task_id) {
            model_file_created = true;
            self.write_thread_model(&mut f_models, extractor)?;
        }

        Ok((info_file_created, model_file_created))
    }
}

impl From<&LimeContext> for TaskModelExtractor {
    fn from(ctx: &LimeContext) -> Self {
        let output_dir = ctx.output_dir.clone();

        TaskModelExtractor::new(output_dir)
    }
}

impl EventProcessor for TaskModelExtractor {
    fn pre_load_init(&mut self, _ctx: &LimeContext) -> anyhow::Result<()> {
        self.output_directory.create_dir()?;

        Ok(())
    }

    fn post_load_init(&mut self, _ctx: &LimeContext) -> anyhow::Result<()> {
        Ok(())
    }

    fn consume_event(&mut self, task_id: &TaskId, event: TraceEvent, ctx: &LimeContext) {
        self.extractors
            .get_or_new(task_id, || ThreadTaskModelExtractor::new(ctx))
            .consume_event(event, ctx)
    }

    fn finalize<S: EventSource>(&mut self, src: &S, ctx: &LimeContext) -> Result<()> {
        // Notify end of trace to extractors
        let id = ThreadId::new(0, 0);
        let e = TraceEvent {
            ts: 0,
            id,
            ev: EventData::LimeEndOfTrace,
        };

        for extractor in self.extractors.values_mut() {
            extractor.consume_event(e.clone(), ctx);
            extractor.flush();
        }

        // Process all tasks
        let mut total_files = 0;

        // First, collect all task IDs that should be reported
        let reportable_tasks: Vec<(TaskId, bool)> = self
            .extractors
            .items()
            .filter_map(|(task_id, extractor)| {
                let infos = src.get_task_info(*task_id);
                if extractor.should_report(ctx, &infos) {
                    Some((*task_id, infos.is_some()))
                } else {
                    None
                }
            })
            .collect();

        // Then, report each task
        for (task_id, has_info) in reportable_tasks {
            let extractor = self.extractors.get(&task_id).unwrap();
            let infos = if has_info {
                src.get_task_info(task_id)
            } else {
                None
            };

            if let Ok((info_created, model_created)) = self.report_task(&task_id, extractor, &infos)
            {
                if info_created || model_created {
                    total_files += 1;
                }
            }
        }

        eprintln!("Results saved in {}.", ctx.output_dir.path());

        if total_files == 0 {
            eprintln!("WARNING: No model files were generated. If you're trying to analyze non-real-time processes,");
            eprintln!("consider adding the --best-effort flag to include best-effort (SCHED_OTHER) tasks.");
        }

        Ok(())
    }
}
