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
    // writer: TaskModelJSONWriter,
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
    ) -> Result<()> {
        if let Some(infos) = infos {
            if let Ok(mut f_infos) = self.output_directory.create_infos_file(task_id) {
                _ = self.write_thread_infos(&mut f_infos, infos);
            }
        }

        let mut f_models = self.output_directory.create_models_file(task_id)?;
        self.write_thread_model(&mut f_models, extractor)?;

        Ok(())
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
        // Notifify end of trace to extractors
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

        for (task_id, extractor) in self.extractors.items() {
            let infos = src.get_task_info(*task_id);

            if !extractor.should_report(ctx, &infos) {
                continue;
            }

            self.report_task(task_id, extractor, &infos)?;
        }

        eprintln!("Results saved in {}.", ctx.output_dir.path());

        Ok(())
    }
}
