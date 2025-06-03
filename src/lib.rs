//! A real-time task model extraction tool.
//!
//! LiME is real-time task model extractor for Linux. It extracts models from
//! traces gathered using an eBPF tracer. The tracer can be consumed directly or
//! saved on disk to be processed later.
//!
//! Lime's main components are either __event sources__ or __event processors__:
//! - An event source produces a stream of events. It implements the `EventSource` trait.
//!   Currently, events sources are trace files and the eBPF tracer.
//! - An event processor consumes a stream of events. It implements the `EventProcessor`
//!   trait. Lime currently has currently three processors: a trace recorder, a
//!   job extractor, and a task model extractor.

pub mod cli;
pub mod utils;

pub mod task;
pub mod trace;

pub mod events;

#[cfg(target_os = "linux")]
pub mod tracer;

pub mod ir;
pub mod job_separator;
pub mod model_extractors;

pub mod context;
pub mod interval;
pub mod processors;

pub mod job;

pub mod io;

pub mod view;

// Include the generated protobuf code
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/lime.rs"));
}

use anyhow::Result;

use crate::{
    context::LimeContext,
    events::TraceEvent,
    task::{TaskId, TaskInfos},
};

/// Feeds an `EventProcessor` with a stream of events.
pub trait EventSource: Sized {
    /// Consume and feed all events to the supplied processor.
    fn event_loop<P: EventProcessor>(&mut self, processor: &mut P, ctx: &LimeContext)
        -> Result<()>;

    /// Process the events with the supplied `EventProcessor`.
    fn process_events<P: EventProcessor>(
        &mut self,
        mut processor: P,
        ctx: &LimeContext,
    ) -> Result<()> {
        processor.post_load_init(ctx)?;

        self.event_loop(&mut processor, ctx)?;

        processor.finalize(self, ctx)
    }

    fn get_task_info(&self, task_id: TaskId) -> Option<TaskInfos>;
}

/// Consumes a stream of events.
pub trait EventProcessor {
    /// Initialize the processor before supplying it to an event source.
    fn pre_load_init(&mut self, ctx: &LimeContext) -> Result<()>;

    /// Initialize the processor after it has been supplied to an event source.
    fn post_load_init(&mut self, ctx: &LimeContext) -> Result<()>;

    /// Process an event
    fn consume_event(&mut self, task_id: &TaskId, event: TraceEvent, ctx: &LimeContext);

    /// Destructor function
    fn finalize<S: EventSource>(&mut self, src: &S, ctx: &LimeContext) -> Result<()>;
}
