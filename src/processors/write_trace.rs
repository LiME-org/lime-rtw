use crate::{
    cli::CLI,
    context::LimeContext,
    events::TraceEvent,
    io::LimeOutputDirectory,
    task::TaskId,
    trace::writer::{EventsFileFormat, TraceEventWriter},
    utils::Dispatcher,
    EventProcessor, EventSource,
};

use anyhow::{Ok, Result};
use std::collections::HashSet;

pub struct TraceWriter {
    output_dir: LimeOutputDirectory,
    thread_events_dumpers: Dispatcher<TaskId, TraceEventWriter>,
    format: EventsFileFormat,
    written_files: HashSet<String>,
}

impl TraceWriter {
    fn get_writer(&mut self, task_id: &TaskId) -> &mut TraceEventWriter {
        self.thread_events_dumpers.get_or_new(task_id, || {
            let f = self
                .output_dir
                .create_events_file(task_id, self.format)
                .unwrap();

            let filename = format!(
                "{}.events.{}",
                task_id,
                match self.format {
                    EventsFileFormat::Json => "json",
                    EventsFileFormat::Protobuf => "proto",
                }
            );
            self.written_files.insert(filename);

            TraceEventWriter::new(f, self.format)
        })
    }

    fn write_event(&mut self, task_id: &TaskId, event: &TraceEvent) {
        let writer = self.get_writer(task_id);

        writer.write(event).unwrap()
    }

    fn close<S: EventSource>(&mut self, src: &S) {
        // dump task metadata
        for task_id in self.thread_events_dumpers.keys() {
            if let Some(tinfo) = src.get_task_info(*task_id) {
                let f = self.output_dir.create_infos_file(task_id).unwrap();

                let filename = format!("{}.infos.json", task_id);
                self.written_files.insert(filename);

                serde_json::to_writer_pretty(f, &tinfo).unwrap();
            }
        }

        for writer in self.thread_events_dumpers.values_mut() {
            writer.close().unwrap();
        }
    }

    pub fn new(output_dir: LimeOutputDirectory, format: EventsFileFormat) -> Self {
        Self {
            output_dir,
            thread_events_dumpers: Dispatcher::new(),
            format,
            written_files: HashSet::new(),
        }
    }
}

impl TryFrom<&CLI> for TraceWriter {
    type Error = anyhow::Error;

    fn try_from(opts: &CLI) -> Result<Self, Self::Error> {
        let output_dir = LimeOutputDirectory::from(opts);
        let format = opts.output_format();
        Ok(TraceWriter::new(output_dir, format))
    }
}

impl TryFrom<&LimeContext> for TraceWriter {
    type Error = anyhow::Error;

    fn try_from(ctx: &LimeContext) -> Result<Self> {
        let format = ctx.output_format;
        Ok(TraceWriter::new(ctx.output_dir.clone(), format))
    }
}

impl EventProcessor for TraceWriter {
    fn pre_load_init(&mut self, _ctx: &LimeContext) -> Result<()> {
        self.output_dir.create_dir()?;

        Ok(())
    }

    fn post_load_init(&mut self, _ctx: &LimeContext) -> Result<()> {
        Ok(())
    }

    fn consume_event(&mut self, task_id: &TaskId, event: TraceEvent, _ctx: &LimeContext) {
        self.write_event(task_id, &event);
    }

    fn finalize<S: EventSource>(&mut self, src: &S, _ctx: &LimeContext) -> Result<()> {
        self.close(src);

        eprintln!("Results saved in {}.", self.output_dir.path());

        if self.written_files.is_empty() {
            eprintln!("WARNING: No trace files were generated. If you're trying to trace non-real-time processes,");
            eprintln!(
                "consider adding the --best-effort flag to trace best-effort (SCHED_OTHER) tasks."
            );
        }

        Ok(())
    }
}
