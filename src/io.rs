//! LiME saved output helper type.
//!
//! LiME has various outputs, each of which is saved in separate files. Since
//! managing all the paths to these files by hand would be quickly cumbersome,
//! this module provide a type handling all the output file management.

use std::{
    collections::BTreeSet,
    env::current_dir,
    fmt::Display,
    fs::File,
    path::{Path, PathBuf},
};

use anyhow::{bail, Ok, Result};
use glob::Paths;
use time::{macros::format_description, OffsetDateTime};

use crate::trace::writer::EventsFileFormat;
use crate::{cli::CLI, task::TaskId};

fn make_unique_result_dir_path<P: AsRef<Path>>(path: Option<P>) -> Option<PathBuf> {
    let now = OffsetDateTime::now_local().unwrap();

    let date_format = format_description!("[year]-[month]-[day]-[hour][minute][second]");
    let formatted_date = now.format(&date_format).unwrap();

    let name = format!("lime-{}", formatted_date);

    let p = path
        .as_ref()
        .map(|t| t.as_ref().join(&name))
        .unwrap_or_else(|| PathBuf::from(&name));

    if !Path::exists(&p) {
        return Some(p);
    }

    for c in 'a'..='z' {
        let name = format!("lime-{}-{}", formatted_date, c);
        let p = path
            .as_ref()
            .map(|p| p.as_ref().join(&name))
            .unwrap_or_else(|| PathBuf::from(&name));

        if !Path::exists(&p) {
            return Some(p);
        }
    }

    None
}

#[derive(Debug, PartialEq, Eq)]
enum LimeFile {
    Events(TaskId, EventsFileFormat),
    Infos(TaskId),
    Models(TaskId),
    Jobs(TaskId),
}

impl Display for LimeFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LimeFile::Events(t, format) => {
                let ext = match format {
                    EventsFileFormat::Json => "json",
                    EventsFileFormat::Protobuf => "pb",
                };
                write!(f, "{}.events.{}", t, ext)
            }
            LimeFile::Infos(t) => {
                write!(f, "{}.infos.json", t)
            }
            LimeFile::Models(t) => {
                write!(f, "{}.models.json", t)
            }
            LimeFile::Jobs(t) => {
                write!(f, "{}.jobs.json", t)
            }
        }
    }
}

impl TryFrom<&Path> for LimeFile {
    type Error = anyhow::Error;

    fn try_from(path: &Path) -> anyhow::Result<Self> {
        if let Some(fname) = path.file_name() {
            let fname_str = fname.to_str().unwrap();
            let task_id = TaskId::try_from_filename(fname_str)?;
            let tokens: Vec<&str> = fname.to_str().unwrap().split('.').collect();

            if let Some(t) = tokens.into_iter().rev().nth(1) {
                match t {
                    "events" => {
                        let tokens: Vec<&str> = fname.to_str().unwrap().split('.').collect();
                        let format = if tokens.iter().rev().nth(0) == Some(&"pb") {
                            EventsFileFormat::Protobuf
                        } else {
                            EventsFileFormat::Json
                        };
                        return Ok(LimeFile::Events(task_id, format));
                    }
                    "infos" => return Ok(LimeFile::Infos(task_id)),
                    "jobs" => return Ok(LimeFile::Jobs(task_id)),
                    "models" => return Ok(LimeFile::Models(task_id)),
                    _ => {}
                }
            }

            bail!("Invalid file format: {}", fname_str);
        }

        bail!("Path does not point to a file");
    }
}

/// Task id iterator.
pub struct TaskIds {
    paths: Paths,
    found: BTreeSet<TaskId>,
}

impl Iterator for TaskIds {
    type Item = TaskId;

    fn next(&mut self) -> Option<Self::Item> {
        for p in self.paths.by_ref() {
            if p.is_err() {
                continue;
            }

            if let Some(fname) = p.unwrap().file_name().and_then(|p| p.to_str()) {
                if let Result::Ok(task_id) = TaskId::try_from_filename(fname) {
                    if self.found.insert(task_id) {
                        return Some(task_id);
                    }
                }
            }
        }

        None
    }
}

impl TryFrom<&Path> for TaskIds {
    type Error = anyhow::Error;

    fn try_from(path: &Path) -> std::prelude::v1::Result<Self, Self::Error> {
        let pattern = format!("{}/*.json", path.to_string_lossy());
        let paths = glob::glob(pattern.as_str())?;

        Ok(Self {
            paths,
            found: BTreeSet::new(),
        })
    }
}

/// A LiME output directory.
///
/// This type provides the main I/O interface for files produced by LiME.
#[derive(Clone)]
pub struct LimeOutputDirectory {
    path: String,
}

impl LimeOutputDirectory {
    pub fn new(path: String) -> Self {
        Self { path }
    }

    /// Return an iterator of the task ids for which some output files exist.
    pub fn task_ids(&self) -> Result<TaskIds> {
        TaskIds::try_from(Path::new(self.path.as_str()))
    }

    /// Constructs a `LimeOutputDirectory` pointing to a directory in `root`
    /// with a unique name. This method merely constructs the struct and does not
    /// create the directory in itself. One must invoke the `create_dir()` method
    /// before any other operations.
    pub fn with_default_name(root: String) -> Result<Self> {
        let path_name = make_unique_result_dir_path(Some(root.clone()));

        if let Some(p) = path_name {
            let s = p.to_string_lossy().to_string();

            return Ok(Self::new(s));
        }

        bail!(
            "Could not create a default lime output directory in {}",
            root
        );
    }

    fn lime_file_path(&self, f: &LimeFile) -> PathBuf {
        let ret = PathBuf::from(self.path.as_str());
        let filename = format!("{}", f);

        ret.join(filename)
    }

    fn open_lime_file(&self, f: &LimeFile) -> Result<File> {
        let path = self.lime_file_path(f);

        let f = File::open(path)?;

        Ok(f)
    }

    fn create_lime_file(&self, f: &LimeFile) -> Result<File> {
        let path = self.lime_file_path(f);

        let f = File::create(path)?;

        Ok(f)
    }

    /// Open `task_id`'s `.infos` files in read-only mode.
    /// The file must already exists.
    pub fn open_infos_file(&self, task_id: &TaskId) -> Result<File> {
        self.open_lime_file(&LimeFile::Infos(*task_id))
    }

    /// Open `task_id`'s `.events` files in read-only mode.
    /// The file must already exists.
    pub fn open_events_file(&self, task_id: &TaskId, format: EventsFileFormat) -> Result<File> {
        self.open_lime_file(&LimeFile::Events(*task_id, format))
    }

    /// Create and open a `.infos` file for `task_id`.
    /// The file is opened with write permission.
    /// Fails if `task_id` already has a `.infos` file.
    pub fn create_infos_file(&self, task_id: &TaskId) -> Result<File> {
        self.create_lime_file(&LimeFile::Infos(*task_id))
    }

    /// Create and open a `.events` file for `task_id`.
    /// The file is opened with write permission.
    /// Fails if `task_id` already has a `.events` file.
    pub fn create_events_file(&self, task_id: &TaskId, format: EventsFileFormat) -> Result<File> {
        self.create_lime_file(&LimeFile::Events(*task_id, format))
    }

    /// Create and open a `.jobs` file for `task_id`.
    /// The file is opened with write permission.
    /// Fails if `task_id` already has a `.jobs` file.
    pub fn create_jobs_file(&self, task_id: &TaskId) -> Result<File> {
        self.create_lime_file(&LimeFile::Jobs(*task_id))
    }

    /// Create and open a `.models` file for `task_id`.
    /// The file is opened with write permission.
    /// Fails if `task_id` already has a `.models` file.
    pub fn create_models_file(&self, task_id: &TaskId) -> Result<File> {
        self.create_lime_file(&LimeFile::Models(*task_id))
    }

    /// Create the directory pointed by `self.path()` if it does not already
    /// exists.
    pub fn create_dir(&self) -> Result<()> {
        if !std::path::Path::new(self.path()).exists() {
            std::fs::create_dir(self.path())?;
        }

        Ok(())
    }

    /// Returns a reference to the path of the represented output directory.
    pub fn path(&self) -> &str {
        self.path.as_str()
    }

    pub fn get_events_file_path(&self, task_id: &TaskId, format: EventsFileFormat) -> PathBuf {
        PathBuf::from(&self.path).join(LimeFile::Events(*task_id, format).to_string())
    }
}

impl From<&CLI> for LimeOutputDirectory {
    fn from(cli: &CLI) -> Self {
        match cli.output_dir() {
            Some(p) => LimeOutputDirectory::new(p),
            None => {
                let cwd = current_dir().unwrap().to_string_lossy().to_string();

                LimeOutputDirectory::with_default_name(cwd).unwrap()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::task::TaskId;
    use crate::trace::writer::EventsFileFormat;
    use std::path::Path;

    #[test]
    fn test_lime_file_parsing() {
        use super::LimeFile;

        let id = TaskId::new(123, 2);

        let example = Path::new("123-2.events.json");
        let expected = LimeFile::Events(id, EventsFileFormat::Json);
        assert_eq!(LimeFile::try_from(example).unwrap(), expected);

        let example = Path::new("123-2.jobs.json");
        let expected = LimeFile::Jobs(id);
        assert_eq!(LimeFile::try_from(example).unwrap(), expected);

        let example = Path::new("123-2.models.json");
        let expected = LimeFile::Models(id);
        assert_eq!(LimeFile::try_from(example).unwrap(), expected);

        let example = Path::new("123-2.infos.json");
        let expected = LimeFile::Infos(id);
        assert_eq!(LimeFile::try_from(example).unwrap(), expected);
    }
}
