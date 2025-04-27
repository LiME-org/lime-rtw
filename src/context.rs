//! Lime runtime parameters.
//!
//! This module defines the `LimeContext` struct containing all the parameters
//! needed by time at runtime. The LimeContext struct is meant to be built from
//! command line parameters.
//! ```no_run
//! use lime_rtw::{cli::CLI, context::LimeContext};
//! use clap::Parser;
//!
//! let args = CLI::parse();
//! let ctx = LimeContext::from(&args);
//! ```
//! Please note that  LiME actual default parameters are defined in the `cli`
//! module.

use std::time::Duration;

use crate::cli::CLI;
use crate::io::LimeOutputDirectory;
use crate::trace::writer::EventsFileFormat;

/// Contains all LiME paramaters
pub struct LimeContext {
    /// Output directory.
    pub output_dir: LimeOutputDirectory,
    pub verbose: bool,
    #[allow(unused)]
    cmd_vec: Vec<String>,

    /// If true, all threads running under the tracked scheduling policies are
    /// traced. Otherwise, only trace thread launched by LiME and their children.
    pub trace_all: bool,
    /// If true, add CFS to the set of tracked scheduling policies.
    pub trace_best_effort: bool,
    /// Rate limiter period.
    pub limiter_period: Duration,
    /// Rate limiter budget.
    pub limiter_budget: usize,

    /// If true, reports model extracted for all traced threads, even if there
    /// are none.
    pub all_threads: bool,

    /// EBPF ring buffer poll interval
    pub ebpf_poll_interval: Duration,

    /// If true, do not update RBFs until syscall job separator is detected.
    pub no_init_rbf_update: bool,

    /// Only extract RBF when the flag is set.
    pub enable_rbf: bool,
    /// Maximum number of step of RBF extracted by the variable-sized-steps
    /// RBF extractor.
    pub rbf_max_steps: usize,
    /// RBF horizon.
    pub rbf_horizon: u64,
    /// Minimum distance between consecutive RBF steps.
    pub rbf_min_sep: u64,
    /// Maximum number of consecutive jobs tracked by the WCET(n) extractor.
    pub wcet_n_max_len: usize,
    /// Maximum number of consecutive jobs tracked by arrival curve extractors.
    pub arrival_curve_max_len: usize,
    /// Release period extractor batch size
    pub period_extractor_batch_size: usize,

    pub allow_task_priority_change: bool,
    pub allow_task_affinity_change: bool,

    pub tx_batch_size: usize,

    pub output_format: EventsFileFormat,
}

impl LimeContext {
    #[cfg(target_os = "linux")]
    fn get_cmd_vec(&self) -> Option<&Vec<String>> {
        match self.cmd_vec.len() {
            0 => None,
            _ => Some(&self.cmd_vec),
        }
    }

    /// Returns process parameters for the command specified by the user, if any.
    /// Returns `None` otherwise.
    #[cfg(target_os = "linux")]
    pub fn get_cmd(&self) -> Option<std::process::Command> {
        if let Some(cmd_vec) = self.get_cmd_vec() {
            let (cmd, args) = cmd_vec.split_at(1);

            let mut command = std::process::Command::new(&cmd[0]);

            command.args(args);

            return Some(command);
        }

        None
    }
}

impl From<&CLI> for LimeContext {
    fn from(cli_opts: &CLI) -> Self {
        Self {
            wcet_n_max_len: cli_opts.wcet_n_max_len().unwrap_or_default(),
            arrival_curve_max_len: cli_opts.arrival_curve_max_len().unwrap_or_default(),
            enable_rbf: cli_opts.enable_rbf(),
            rbf_max_steps: cli_opts.rbf_max_steps().unwrap_or_default(),
            rbf_horizon: cli_opts.rbf_horizon().unwrap_or_default(),
            rbf_min_sep: cli_opts.rbf_min_sep().unwrap_or_default(),
            trace_all: cli_opts.trace_all(),
            trace_best_effort: cli_opts.trace_best_effort(),
            all_threads: cli_opts.all_threads(),
            cmd_vec: cli_opts.get_cmd_vec().cloned().unwrap_or_default(),
            limiter_period: cli_opts.rate_limiter_period().unwrap_or_default(),
            limiter_budget: cli_opts.rate_limiter_budget().unwrap_or_default(),
            output_dir: LimeOutputDirectory::from(cli_opts),
            no_init_rbf_update: cli_opts.rbf_no_update_init().unwrap_or_default(),
            verbose: cli_opts.verbose,
            period_extractor_batch_size: cli_opts.period_extractor_batch_size(),
            allow_task_priority_change: cli_opts.allow_task_priority_change(),
            allow_task_affinity_change: cli_opts.allow_task_affinity_change(),
            tx_batch_size: cli_opts.tx_batch_size(),
            ebpf_poll_interval: cli_opts.ebpf_poll_interval(),
            output_format: cli_opts.output_format(),
        }
    }
}
