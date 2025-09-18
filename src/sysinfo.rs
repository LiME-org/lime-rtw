//! System information collection for tracing metadata.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::process::Command;

use crate::task::TimeReference;

#[derive(Debug, Serialize, Deserialize)]
pub struct SystemInfo {
    pub start_time: String,
    pub boottime_ns: u64,
    pub system: SystemConfig,
    pub lime: LimeInfo,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SystemConfig {
    pub kernel_version: String,
    pub kernel_cmdline: Option<String>,
    pub sched_rt_period_us: Option<u64>,
    pub sched_rt_runtime_us: Option<u64>,
    pub sched_rr_timeslice_ms: Option<u64>,
    pub sched_deadline_period_max_us: Option<u64>,
    pub sched_deadline_period_min_us: Option<u64>,
    pub sched_cfs_bandwidth_slice_us: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LimeInfo {
    pub version: String,
    pub command: String,
    pub args: Vec<String>,
}

impl SystemInfo {
    pub fn collect(command: &str, args: &[String], time_ref: &TimeReference) -> Result<Self> {
        let start_time = time_ref.start_time_iso8601.clone();
        let boottime_ns = time_ref.start_boottime_ns;

        let system = SystemConfig::collect()?;
        let lime = LimeInfo {
            version: env!("CARGO_PKG_VERSION").to_string(),
            command: command.to_string(),
            args: args.to_vec(),
        };

        Ok(SystemInfo {
            start_time,
            boottime_ns,
            system,
            lime,
        })
    }
}

impl SystemConfig {
    pub fn collect() -> Result<Self> {
        let kernel_version = Self::get_kernel_version()?;

        Ok(SystemConfig {
            kernel_version,
            kernel_cmdline: Self::read_proc_string("/proc/cmdline"),
            sched_rt_period_us: Self::read_proc_value("/proc/sys/kernel/sched_rt_period_us"),
            sched_rt_runtime_us: Self::read_proc_value("/proc/sys/kernel/sched_rt_runtime_us"),
            sched_rr_timeslice_ms: Self::read_proc_value("/proc/sys/kernel/sched_rr_timeslice_ms"),
            sched_deadline_period_max_us: Self::read_proc_value(
                "/proc/sys/kernel/sched_deadline_period_max_us",
            ),
            sched_deadline_period_min_us: Self::read_proc_value(
                "/proc/sys/kernel/sched_deadline_period_min_us",
            ),
            sched_cfs_bandwidth_slice_us: Self::read_proc_value(
                "/proc/sys/kernel/sched_cfs_bandwidth_slice_us",
            ),
        })
    }

    fn get_kernel_version() -> Result<String> {
        let output = Command::new("uname")
            .arg("-r")
            .output()
            .context("Failed to execute uname -r")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!("uname -r failed"));
        }

        let version = String::from_utf8(output.stdout)
            .context("Failed to parse uname output")?
            .trim()
            .to_string();

        Ok(version)
    }

    fn read_proc_string(path: &str) -> Option<String> {
        fs::read_to_string(path).ok().map(|s| s.trim().to_string())
    }

    fn read_proc_value(path: &str) -> Option<u64> {
        fs::read_to_string(path).ok()?.trim().parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_info_collection() {
        let args = vec!["--best-effort".to_string(), "mycommand".to_string()];
        let time_ref = TimeReference::new("2023-01-01T00:00:00Z".to_string(), 1000000000);
        let info = SystemInfo::collect("extract", &args, &time_ref);

        assert!(info.is_ok());
        let info = info.unwrap();
        assert!(!info.start_time.is_empty());
        assert!(!info.lime.version.is_empty());
        assert_eq!(info.lime.command, "extract");
        assert_eq!(info.lime.args, args);
        assert_eq!(info.start_time, "2023-01-01T00:00:00Z");
        assert_eq!(info.boottime_ns, 1000000000);
    }

    #[test]
    fn test_system_config_collection() {
        let config = SystemConfig::collect();
        assert!(config.is_ok());
        let config = config.unwrap();
        assert!(!config.kernel_version.is_empty());
    }
}
