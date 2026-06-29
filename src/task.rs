//! Thread to task mapping.

use std::fmt::Display;
use std::io::Write;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Iso8601;
use time::{Duration, OffsetDateTime};

use crate::{events::SchedulingPolicy, utils::ThreadId};

#[derive(Debug, Clone)]
pub struct TimeReference {
    pub start_time_iso8601: String,
    pub start_boottime_ns: u64,
    start_time: Result<OffsetDateTime, String>,
}

impl TimeReference {
    pub fn new(start_time_iso8601: String, start_boottime_ns: u64) -> Self {
        let start_time = OffsetDateTime::parse(&start_time_iso8601, &Iso8601::DEFAULT)
            .map_err(|e| format!("Failed to parse start time ISO8601: {}", e));

        Self {
            start_time_iso8601,
            start_boottime_ns,
            start_time,
        }
    }

    pub fn boottime_to_iso8601(&self, event_boottime_ns: u64) -> Result<String, String> {
        let start_dt = *self.start_time.as_ref().map_err(Clone::clone)?;

        let delta_ns = event_boottime_ns.saturating_sub(self.start_boottime_ns);
        let delta_seconds = delta_ns / 1_000_000_000;
        let remaining_nanos = delta_ns % 1_000_000_000;

        let duration =
            Duration::seconds(delta_seconds as i64) + Duration::nanoseconds(remaining_nanos as i64);

        (start_dt + duration)
            .format(&Iso8601::DEFAULT)
            .map_err(|e| format!("Failed to format as ISO8601: {}", e))
    }
}

/// Task identifier
#[derive(PartialEq, Eq, Hash, Copy, Clone, Debug, Ord, PartialOrd)]
pub struct TaskId {
    pid: u32,
    version: u32,
}

impl TaskId {
    pub fn new(pid: u32, version: u32) -> Self {
        Self { pid, version }
    }

    pub fn try_from_filename(fname: &str) -> Result<Self> {
        let stem = [
            ".events.json",
            ".events.pb",
            ".infos.json",
            ".models.json",
            ".jobs.json",
        ]
        .into_iter()
        .find_map(|suffix| fname.strip_suffix(suffix))
        .ok_or_else(|| anyhow::anyhow!("Error parsing {fname}"))?;
        let (pid, version) = stem
            .split_once('-')
            .ok_or_else(|| anyhow::anyhow!("Error parsing {fname}"))?;

        Ok(Self::new(pid.parse()?, version.parse()?))
    }
}

impl Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.pid, self.version)
    }
}

/// Task metadata.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TaskInfos {
    #[serde(flatten)]
    pub id: ThreadId,
    pub ppid: Option<u32>,
    pub comm: Option<String>,
    pub cmd: Option<String>,
    pub policy: SchedulingPolicy,
    pub affinity_mask: Option<AffinityMask>,
    #[serde(deserialize_with = "deserialize_event_time")]
    pub first_event_time: Option<u64>,
    #[serde(deserialize_with = "deserialize_event_time")]
    pub last_event_time: Option<u64>,
}

impl TaskInfos {
    pub fn write_pretty<W: Write>(
        &self,
        writer: W,
        time_reference: Option<&TimeReference>,
    ) -> serde_json::Result<()> {
        let json_event_time = |event_time: Option<u64>| {
            event_time.map(|boottime_ns| JsonEventTimeView {
                boottime_ns,
                iso8601: time_reference
                    .and_then(|time_reference| time_reference.boottime_to_iso8601(boottime_ns).ok())
                    .unwrap_or_default(),
            })
        };

        serde_json::to_writer_pretty(
            writer,
            &JsonTaskInfosView {
                id: self.id,
                ppid: self.ppid,
                comm: self.comm.as_deref(),
                cmd: self.cmd.as_deref(),
                policy: &self.policy,
                affinity_mask: self.affinity_mask.as_ref(),
                first_event_time: json_event_time(self.first_event_time),
                last_event_time: json_event_time(self.last_event_time),
            },
        )
    }
}

#[derive(Serialize)]
struct JsonEventTimeView {
    boottime_ns: u64,
    iso8601: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum JsonEventTime {
    BoottimeNs(u64),
    WithIso8601 { boottime_ns: u64 },
}

fn deserialize_event_time<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<JsonEventTime>::deserialize(deserializer).map(|maybe_json_event_time| {
        maybe_json_event_time.map(|json_event_time| match json_event_time {
            JsonEventTime::BoottimeNs(boottime_ns) | JsonEventTime::WithIso8601 { boottime_ns } => {
                boottime_ns
            }
        })
    })
}

#[derive(Serialize)]
struct JsonTaskInfosView<'a> {
    #[serde(flatten)]
    id: ThreadId,
    ppid: Option<u32>,
    comm: Option<&'a str>,
    cmd: Option<&'a str>,
    policy: &'a SchedulingPolicy,
    affinity_mask: Option<&'a AffinityMask>,
    first_event_time: Option<JsonEventTimeView>,
    last_event_time: Option<JsonEventTimeView>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AffinityMask {
    cpu_set: Vec<u32>,
}

#[cfg(target_os = "linux")]
pub mod mapper {
    //! Task id mapping from event streams.
    use nc::sched_attr_t;
    use nix::sched::CpuSet;
    use rustc_hash::FxHashMap;
    use sysinfo::{Pid, Process, ProcessesToUpdate, System};

    use crate::{
        context::LimeContext,
        events::{EventData, ProcessInfo, SchedulingPolicy, TraceEvent},
    };

    use super::{AffinityMask, TaskId, TaskInfos};

    use anyhow::Result;

    impl From<CpuSet> for AffinityMask {
        fn from(cpu_set: CpuSet) -> Self {
            AffinityMask {
                cpu_set: (0..CpuSet::count())
                    .filter(|&i| cpu_set.is_set(i).ok() == Some(true))
                    .map(|i| i as u32)
                    .collect(),
            }
        }
    }

    pub struct TaskMapper {
        pid_versions: FxHashMap<u32, u32>,
        infos: FxHashMap<TaskId, TaskInfos>,
        current_mappings: FxHashMap<u32, TaskId>,
        system: System,
        allow_task_priority_change: bool,
        allow_task_affinity_change: bool,
    }

    impl TaskMapper {
        pub fn new() -> Self {
            Self {
                pid_versions: FxHashMap::default(),
                infos: FxHashMap::default(),
                current_mappings: FxHashMap::default(),
                system: System::new(),
                allow_task_affinity_change: false,
                allow_task_priority_change: false,
            }
        }

        pub fn init(&mut self, ctx: &LimeContext) {
            self.allow_task_affinity_change = ctx.allow_task_affinity_change;
            self.allow_task_priority_change = ctx.allow_task_priority_change;
        }

        fn new_mapping(&mut self, pid: u32) -> TaskId {
            let version = self
                .pid_versions
                .entry(pid)
                .and_modify(|version| *version += 1)
                .or_insert(0);

            TaskId::new(pid, *version)
        }

        fn invalidate(&mut self, pid: u32) -> Option<TaskId> {
            self.current_mappings.remove(&pid)
        }

        fn remap(&mut self, pid: u32) -> TaskId {
            let mapping = self.new_mapping(pid);
            self.current_mappings.insert(pid, mapping);
            mapping
        }

        fn read_sched_policy(pid: u32) -> Result<SchedulingPolicy> {
            unsafe {
                let mut attrs = sched_attr_t::default();
                nc::sched_getattr(pid as i32, &mut attrs, 0)
                    .map(|()| SchedulingPolicy::from(&attrs))
                    .map_err(|e| anyhow::anyhow!("sched_getattr failed with code {}", e))
            }
        }

        fn read_affinity_mask(pid: u32) -> Result<AffinityMask> {
            let cpu_set = nix::sched::sched_getaffinity(nix::unistd::Pid::from_raw(pid as i32))?;
            Ok(AffinityMask::from(cpu_set))
        }

        fn refresh_process(system: &mut System, pid: u32) -> Option<&Process> {
            let sys_pid = Pid::from_u32(pid);
            (system.refresh_processes(ProcessesToUpdate::Some(&[sys_pid]), true) > 0)
                .then(|| system.process(sys_pid))
                .flatten()
        }

        fn apply_event_metadata(task_info: &mut TaskInfos, event: &TraceEvent) {
            let pid = event.id.pid;

            match &event.ev {
                EventData::SchedulerChanged { sched_policy }
                | EventData::SchedParamsChanged { sched_policy }
                | EventData::SchedPolicyUpdate { sched_policy } => {
                    task_info.policy = if *sched_policy == SchedulingPolicy::Unknown {
                        Self::read_sched_policy(pid).unwrap_or(*sched_policy)
                    } else {
                        *sched_policy
                    };
                }
                EventData::AffinityChanged {} => {
                    task_info.affinity_mask = Self::read_affinity_mask(pid).ok();
                }
                EventData::AffinityUpdate { cpumask } => match cpumask {
                    Some(cpu_ids) => {
                        task_info.affinity_mask = Some(AffinityMask {
                            cpu_set: cpu_ids.clone(),
                        })
                    }
                    None => task_info.affinity_mask = Self::read_affinity_mask(pid).ok(),
                },
                EventData::ProcessInfoUpdate { process_info } => {
                    if let Some(ProcessInfo { ppid, comm, cmd }) = process_info.as_ref() {
                        if let Some(ppid) = ppid {
                            task_info.ppid = Some(*ppid);
                        }
                        if let Some(comm) = comm {
                            task_info.comm = Some(comm.clone());
                        }
                        if let Some(cmd) = cmd {
                            task_info.cmd = Some(cmd.clone());
                        }
                    }
                }
                _ => {}
            }
        }

        fn build_task_info(&mut self, event: &TraceEvent) -> TaskInfos {
            let mut task_info = TaskInfos {
                id: event.id,
                ppid: None,
                comm: None,
                cmd: None,
                policy: Self::read_sched_policy(event.id.pid).unwrap_or(SchedulingPolicy::Unknown),
                affinity_mask: Self::read_affinity_mask(event.id.pid).ok(),
                first_event_time: Some(event.ts),
                last_event_time: Some(event.ts),
            };
            Self::apply_event_metadata(&mut task_info, event);

            let needs_lookup = task_info.ppid.is_none()
                || task_info.comm.as_deref().is_none_or(str::is_empty)
                || task_info.cmd.as_deref().is_none_or(str::is_empty);

            if needs_lookup {
                let target_pid = event.id.tgid;
                if let Some(process) = Self::refresh_process(&mut self.system, target_pid) {
                    if task_info.ppid.is_none() {
                        task_info.ppid = process.parent().map(|ppid| ppid.as_u32());
                    }
                    if task_info.comm.as_deref().is_none_or(str::is_empty) {
                        task_info.comm = Some(process.name().to_string_lossy().into_owned());
                    }
                    if task_info.cmd.as_deref().is_none_or(str::is_empty) {
                        task_info.cmd = Some(
                            process
                                .cmd()
                                .iter()
                                .map(|arg| arg.to_string_lossy())
                                .collect::<Vec<_>>()
                                .join(" "),
                        );
                    }
                }
            }

            task_info
        }

        fn update_task_info(&mut self, task_id: TaskId, event: &TraceEvent) {
            if let Some(task_info) = self.infos.get_mut(&task_id) {
                if task_info.id.tgid == 0 {
                    task_info.id.tgid = event.id.tgid;
                }
                task_info.last_event_time = Some(event.ts);
                Self::apply_event_metadata(task_info, event);
            }
        }

        fn remap_task(&mut self, pid: u32, current_task_id: TaskId, event: &TraceEvent) -> TaskId {
            self.update_task_info(current_task_id, event);
            let new_mapping = self.remap(pid);
            if let Some(task_info) = self.infos.get(&current_task_id).cloned() {
                self.infos.insert(
                    new_mapping,
                    TaskInfos {
                        first_event_time: Some(event.ts),
                        last_event_time: Some(event.ts),
                        ..task_info
                    },
                );
            }
            current_task_id
        }

        fn create_task(&mut self, pid: u32, event: &TraceEvent) -> TaskId {
            let task_id = self.remap(pid);
            let task_info = self.build_task_info(event);
            self.infos.insert(task_id, task_info);
            task_id
        }

        /// Determines the task id of the input event.
        ///
        /// If the event changes the task id, the state is updated and the old
        /// mapping is returned. If there is no such mapping, `None` is returned.
        pub fn find_task_id(&mut self, event: &TraceEvent) -> Option<TaskId> {
            let pid = event.id.pid;
            let current_mapping = self.current_mappings.get(&pid).copied();

            match &event.ev {
                EventData::EnterSetScheduler { .. } => {
                    return match current_mapping {
                        Some(task_id) => {
                            self.update_task_info(task_id, event);
                            Some(task_id)
                        }
                        None => Some(self.create_task(pid, event)),
                    };
                }
                EventData::SchedulerChange { .. } => {
                    return current_mapping.map(|task_id| self.remap_task(pid, task_id, event));
                }
                EventData::SchedParamsChange { .. } if !self.allow_task_priority_change => {
                    return current_mapping.map(|task_id| self.remap_task(pid, task_id, event));
                }
                EventData::SchedulerChanged { .. } => {
                    return match current_mapping {
                        Some(task_id) => {
                            self.update_task_info(task_id, event);
                            Some(task_id)
                        }
                        None => Some(self.create_task(pid, event)),
                    };
                }
                EventData::SchedParamsChanged { .. } if !self.allow_task_priority_change => {
                    return match current_mapping {
                        Some(task_id) => {
                            self.update_task_info(task_id, event);
                            Some(task_id)
                        }
                        None => Some(self.create_task(pid, event)),
                    };
                }
                EventData::AffinityChange {} if !self.allow_task_affinity_change => {
                    return current_mapping.map(|task_id| self.remap_task(pid, task_id, event));
                }
                EventData::LimeThrottleRelease => {
                    return current_mapping.map(|task_id| self.remap_task(pid, task_id, event));
                }
                EventData::SchedSwitchedOut { state, .. } if (state & 0x30) != 0 => {
                    return match current_mapping {
                        Some(task_id) => {
                            self.invalidate(pid);
                            Some(task_id)
                        }
                        None => None,
                    };
                }
                _ => {}
            }

            if let Some(task_id) = current_mapping {
                self.update_task_info(task_id, event);
                return Some(task_id);
            }

            Some(self.create_task(pid, event))
        }

        pub fn get_task_infos(&self, task_id: TaskId) -> Option<TaskInfos> {
            self.infos.get(&task_id).cloned()
        }
    }

    impl Default for TaskMapper {
        fn default() -> Self {
            Self::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_infos_deserializes_from_pretty_serialized_output() {
        let task_info = TaskInfos {
            id: ThreadId::new(123, 456),
            ppid: Some(1),
            comm: Some("cyclictest".to_string()),
            cmd: Some("cyclictest --priority 90".to_string()),
            policy: SchedulingPolicy::Fifo { prio: 90 },
            affinity_mask: Some(AffinityMask {
                cpu_set: vec![0, 2],
            }),
            first_event_time: Some(1_000),
            last_event_time: Some(2_000),
        };

        let mut serialized = Vec::new();
        task_info.write_pretty(&mut serialized, None).unwrap();

        let deserialized: TaskInfos = serde_json::from_slice(&serialized).unwrap();

        assert_eq!(deserialized.id, task_info.id);
        assert_eq!(deserialized.id.tgid, task_info.id.tgid);
        assert_eq!(deserialized.ppid, task_info.ppid);
        assert_eq!(deserialized.comm, task_info.comm);
        assert_eq!(deserialized.cmd, task_info.cmd);
        assert_eq!(deserialized.policy, task_info.policy);
        assert_eq!(
            deserialized.affinity_mask.unwrap().cpu_set,
            task_info.affinity_mask.unwrap().cpu_set
        );
        assert_eq!(deserialized.first_event_time, task_info.first_event_time);
        assert_eq!(deserialized.last_event_time, task_info.last_event_time);
    }
}
