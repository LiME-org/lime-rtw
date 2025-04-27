use anyhow::Result;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::PathBuf,
    collections::HashSet,
};

use crate::{
    cli::LimeSubCommand::Extract,
    cli::CLI,
    processors::extract_model::TaskModelExtractor,
    context::LimeContext,
    task::{TaskId, TaskInfos},
    utils::make_unique_result_dir_path,
    EventSource,
};

use super::{dd, ThreadTaskModelExtractor};

pub struct TaskModelOutputDirectory(pub String);

impl TaskModelOutputDirectory {
    pub fn make_unique_path() -> Self {
        let s = make_unique_result_dir_path::<String>(None)
            .unwrap()
            .to_string_lossy()
            .to_string();

        TaskModelOutputDirectory(s)
    }
}

impl TryFrom<&CLI> for TaskModelOutputDirectory {
    type Error = anyhow::Error;

    fn try_from(opts: &CLI) -> std::result::Result<Self, Self::Error> {
        match &opts.command {
            Extract {
                output: Some(o), ..
            } => Ok(TaskModelOutputDirectory(o.clone())),
            Extract {
                output: None,
                from: Some(f),
                inplace: true,
                ..
            } => Ok(TaskModelOutputDirectory(f.clone())),
            Extract { .. } => Ok(TaskModelOutputDirectory::make_unique_path()),

            _ => anyhow::bail!("Can only be extracted from extract command parameters"),
        }
    }
}

pub fn open_task_model_file(base: &str, task_id: &TaskId) -> Result<File> {
    let mut b = PathBuf::from(base);
    let filename = format!("{}.models.json", task_id);

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

pub struct TaskModelJSONWriter {
    base: String,
    written_files: HashSet<String>,
}

impl TaskModelJSONWriter {
    pub fn new(base: String) -> Self {
        Self { 
            base,
            written_files: HashSet::new(),
        }
    }

    fn write_thread_infos(&mut self, task_id: &TaskId, tinfo: &TaskInfos) -> Result<()> {
        let mut b = PathBuf::from(self.base.clone());

        let filename = format!("{}.infos.json", task_id);
        self.written_files.insert(filename.clone());

        b.push(filename);

        if !b.exists() {
            let f = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(b.as_path())?;

            serde_json::to_writer_pretty(f, &tinfo)?;
        }

        Ok(())
    }

    fn write_thread_model<W>(
        thread_extractor: &ThreadTaskModelExtractor,
        writer: &mut W,
    ) -> Result<()>
    where
        W: Write,
    {
        let m = dd::ThreadTaskModelReport::from_extractor(thread_extractor);

        serde_json::to_writer_pretty(writer, &m).map_err(anyhow::Error::new)
    }

    fn report_thread(
        &mut self,
        thread_extractor: &ThreadTaskModelExtractor,
        task_id: &TaskId,
    ) -> Result<()> {
        let filename = format!("{}.models.json", task_id);
        self.written_files.insert(filename);
        
        let mut f = open_task_model_file(self.base.as_str(), task_id)?;
        TaskModelJSONWriter::write_thread_model(thread_extractor, &mut f)
    }

    pub fn write<S: EventSource>(
        &mut self,
        src: &S,
        extractor: &TaskModelExtractor,
        ctx: &LimeContext,
    ) -> Result<()> {
        for (task_id, e) in extractor.thread_extractor() {
            if !ctx.all_threads && e.is_empty() {
                // This is some short-lived thread for which no model
                // whatsoever could be inferred. Nothing interesting
                // can be deduced from it. Let's not bother the user
                // with it.
                continue;
            }

            if let Some(tinfo) = src.get_task_info(*task_id) {
                if !ctx.trace_best_effort && !tinfo.policy.is_rt_policy() {
                    // This is not a real-time task and we want to
                    // report only real-time tasks.
                    continue;
                }
                self.write_thread_infos(task_id, &tinfo)?;
            } else if !ctx.trace_best_effort {
                // We don't know what it is, and we want to report
                // only (confirmed) real-time tasks => let's ignore
                // this one.
                continue;
            }

            self.report_thread(e, task_id)?;
        }

        if self.written_files.is_empty() {
            eprintln!("WARNING: No model files were generated. If you're trying to analyze non-real-time processes,");
            eprintln!("consider adding the --best-effort flag to include best-effort (SCHED_OTHER) tasks.");
        }

        Ok(())
    }

    pub fn init(&mut self) -> Result<()> {
        if !std::path::Path::new(self.base.as_str()).exists() {
            std::fs::create_dir(self.base.as_str())?;
        }

        Ok(())
    }
}

