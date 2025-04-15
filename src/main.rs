use anyhow::{Error, Result};
use clap::Parser;
use lime_rtw::{
    context::LimeContext, processors::extract_job::JobExtractor,
    processors::extract_model::TaskModelExtractor, trace::reader::TraceReader, EventProcessor,
    EventSource,
};

#[cfg(target_os = "linux")]
use lime_rtw::processors::write_trace::TraceWriter;

use lime_rtw::cli::*;

#[cfg(target_os = "linux")]
use lime_rtw::tracer::BPFSource;

pub fn run<C: EventProcessor>(mut command: C, opts: &CLI, ctx: LimeContext) -> Result<()> {
    command.pre_load_init(&ctx)?;

    match opts.event_source_type() {
        #[cfg(target_os = "linux")]
        EventSourceType::BPFTracer => {
            let mut src = BPFSource::new().init(&ctx).start(&ctx)?;
            src.process_events(command, &ctx)?;

            std::process::exit(src.exit_status())
        }
        EventSourceType::TraceFolder(path) => {
            TraceReader::new(path)
                .start()
                .process_events(command, &ctx)?;
        }
    }

    Ok(())
}

fn main() -> Result<(), Error> {
    let opts = CLI::parse();
    let ctx = LimeContext::from(&opts);

    match &opts.command {
        #[cfg(target_os = "linux")]
        LimeSubCommand::Trace { .. } => {
            let processor = TraceWriter::try_from(&ctx)?;

            run(processor, &opts, ctx)
        }
        LimeSubCommand::Extract { .. } => {
            let processor = TaskModelExtractor::from(&ctx);

            run(processor, &opts, ctx)
        }
        LimeSubCommand::ExtractJobs { .. } => {
            let processor = JobExtractor::from(&ctx);

            run(processor, &opts, ctx)
        }
    }
}
