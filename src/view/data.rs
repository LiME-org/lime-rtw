use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use anyhow::{Context, Result};
use glob::glob;
use serde_json::Value;

use crate::task::{TaskId, TaskInfos};

// Represents process information including task details and models
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub task_id: TaskId,
    pub info: TaskInfos,
    pub models: Option<Value>,
}

// Container for all view data loaded from trace files
#[derive(Debug, Clone)]
pub struct ViewData {
    processes: BTreeMap<TaskId, ProcessInfo>,
}

impl Default for ViewData {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewData {
    // Creates a new empty view data container
    pub fn new() -> Self {
        Self {
            processes: BTreeMap::new(),
        }
    }

    // Loads all trace data from the specified folder
    pub fn load_from_folder(&mut self, folder: &str) -> Result<()> {
        let folder_path = Path::new(folder);

        self.load_task_infos(folder_path)?;
        self.load_models(folder_path)?;

        Ok(())
    }

    // Loads task information files (.infos.json) from the folder
    fn load_task_infos(&mut self, folder_path: &Path) -> Result<()> {
        let pattern = folder_path.join("*-*.infos.json");
        let pattern_str = pattern.to_string_lossy();

        for entry in glob(&pattern_str)? {
            let path = entry?;
            let file_name = path.file_name().unwrap().to_string_lossy();
            let task_id = TaskId::try_from_filename(&file_name)?;

            let file = File::open(&path)?;
            let reader = BufReader::new(file);
            let task_info: TaskInfos = serde_json::from_reader(reader)
                .with_context(|| format!("Failed to parse task info from {}", path.display()))?;

            self.processes.insert(
                task_id,
                ProcessInfo {
                    task_id,
                    info: task_info,
                    models: None,
                },
            );
        }

        Ok(())
    }

    // Loads model files (.models.json) from the folder
    fn load_models(&mut self, folder_path: &Path) -> Result<()> {
        let pattern = folder_path.join("*-*.models.json");
        let pattern_str = pattern.to_string_lossy();

        for entry in glob(&pattern_str)? {
            let path = entry?;
            let file_name = path.file_name().unwrap().to_string_lossy();
            let task_id = TaskId::try_from_filename(&file_name)?;

            if let Some(process) = self.processes.get_mut(&task_id) {
                let file = File::open(&path)?;
                let reader = BufReader::new(file);
                let model: Value = serde_json::from_reader(reader)
                    .with_context(|| format!("Failed to parse model from {}", path.display()))?;

                process.models = Some(model);
            }
        }

        Ok(())
    }

    // Returns all loaded processes
    pub fn get_processes(&self) -> &BTreeMap<TaskId, ProcessInfo> {
        &self.processes
    }

    // Gets process information for a specific task ID
    pub fn get_process(&self, task_id: &TaskId) -> Option<&ProcessInfo> {
        self.processes.get(task_id)
    }
}
