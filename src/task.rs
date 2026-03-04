//! Thread to task mapping.

use std::fmt::Display;

use anyhow::{bail, Result};
use nom::{
    branch::alt,
    bytes::complete::{tag, take_while1},
    IResult, Parser,
};
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Iso8601;
use time::{Duration, OffsetDateTime};

use crate::{events::SchedulingPolicy, utils::ThreadId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventTime {
    pub boottime_ns: u64,
    pub iso8601: String,
}

impl EventTime {
    pub fn new(boottime_ns: u64, iso8601: String) -> Self {
        Self {
            boottime_ns,
            iso8601,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TimeReference {
    pub start_time_iso8601: String,
    pub start_boottime_ns: u64,
}

impl TimeReference {
    pub fn new(start_time_iso8601: String, start_boottime_ns: u64) -> Self {
        Self {
            start_time_iso8601,
            start_boottime_ns,
        }
    }

    pub fn boottime_to_iso8601(&self, event_boottime_ns: u64) -> Result<String, String> {
        let start_dt = OffsetDateTime::parse(&self.start_time_iso8601, &Iso8601::DEFAULT)
            .map_err(|e| format!("Failed to parse start time ISO8601: {}", e))?;

        let delta_ns = event_boottime_ns.saturating_sub(self.start_boottime_ns);

        let delta_seconds = delta_ns / 1_000_000_000;
        let remaining_nanos = delta_ns % 1_000_000_000;

        let duration =
            Duration::seconds(delta_seconds as i64) + Duration::nanoseconds(remaining_nanos as i64);

        let event_time = start_dt + duration;

        event_time
            .format(&Iso8601::DEFAULT)
            .map_err(|e| format!("Failed to format as ISO8601: {}", e))
    }

    pub fn create_event_time(&self, event_boottime_ns: u64) -> Result<EventTime, String> {
        let iso8601_time = self.boottime_to_iso8601(event_boottime_ns)?;
        Ok(EventTime::new(event_boottime_ns, iso8601_time))
    }
}

/// Task identifier
#[derive(PartialEq, Eq, Hash, Copy, Clone, Debug)]
pub struct TaskId {
    pid: u32,
    version: u32,
}

impl Ord for TaskId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.pid.cmp(&other.pid) {
            std::cmp::Ordering::Equal => self.version.cmp(&other.version),
            ord => ord,
        }
    }
}

impl PartialOrd for TaskId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Parse the taskid from a lime output file.
fn parse_taskid(input: &str) -> IResult<&str, TaskId> {
    let (i, spid) = take_while1(|c: char| c.is_ascii_digit())(input)?;
    let (i, _) = tag("-")(i)?;
    let (i, sversion) = take_while1(|c: char| c.is_ascii_digit())(i)?;
    let (i, _) = tag(".")(i)?;
    let (i, _) = alt((tag("events"), tag("infos"), tag("models"), tag("jobs"))).parse(i)?;
    let (i, _) = tag(".json")(i)?;

    let pid = spid.parse::<u32>().unwrap();
    let version = sversion.parse::<u32>().unwrap();

    Ok((i, TaskId { pid, version }))
}

impl TaskId {
    pub fn new(pid: u32, version: u32) -> Self {
        Self { pid, version }
    }

    pub fn try_from_filename(fname: &str) -> Result<Self> {
        match parse_taskid(fname) {
            Ok((_, task_id)) => Ok(task_id),
            Err(_) => bail!("Error parsing {}", fname),
        }
    }
}

impl Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{}-{}", self.pid, self.version))
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
    pub first_event_time: Option<EventTime>,
    pub last_event_time: Option<EventTime>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AffinityMask {
    cpu_set: Vec<u32>,
}

#[cfg(target_os = "linux")]
pub mod mapper {
    //! Task id mapping from event streams.
    use std::collections::{BTreeSet, HashMap};

    use libc::SCHED_OTHER;
    use nc::{sched_attr_t, SCHED_BATCH, SCHED_DEADLINE, SCHED_FIFO, SCHED_IDLE, SCHED_RR};
    use nix::sched::CpuSet;
    use sysinfo::{Pid, Process, ProcessesToUpdate, System};

    use crate::{
        context::LimeContext,
        events::{EventData, ProcessInfo, SchedulingPolicy, TraceEvent},
    };

    use super::{AffinityMask, TaskId, TaskInfos, TimeReference};

    use anyhow::Result;

    type SysProcess = Process;

    impl From<CpuSet> for AffinityMask {
        fn from(cpu_set: CpuSet) -> Self {
            let mut new_cpu_set = Vec::new();
            for i in 0..CpuSet::count() {
                if let Ok(set) = cpu_set.is_set(i) {
                    if set {
                        new_cpu_set.push(i as u32);
                    }
                }
            }
            AffinityMask {
                cpu_set: new_cpu_set,
            }
        }
    }

    struct ThreadInfoMap {
        builder: ThreadInfoSource,
        thread_infos: HashMap<TaskId, TaskInfos>,
    }

    impl ThreadInfoMap {
        pub fn new() -> Self {
            Self {
                builder: ThreadInfoSource::new(),
                thread_infos: HashMap::new(),
            }
        }

        pub fn get_thread_info(&self, task_id: &TaskId) -> Option<&TaskInfos> {
            self.thread_infos.get(task_id)
        }

        fn new_entry(&mut self, task_id: TaskId, event: &TraceEvent) {
            let mut tinfo = self.builder.build_new_task_info(event);
            self.builder.update_task_info(&mut tinfo, event);
            self.thread_infos.insert(task_id, tinfo);
        }

        fn duplicate_entry(&mut self, old_mapping: TaskId, new_mapping: TaskId) {
            if let Some(old_info) = self.get(old_mapping) {
                let mut new_info = old_info.clone();

                new_info.first_event_time = None;

                self.thread_infos.insert(new_mapping, new_info);
            }
        }

        fn get(&self, task_id: TaskId) -> Option<&TaskInfos> {
            self.get_thread_info(&task_id)
        }

        fn update_entry(&mut self, task_id: TaskId, event: &TraceEvent) {
            if let Some(info) = self.thread_infos.get_mut(&task_id) {
                self.builder.update_task_info(info, event);
            }
        }
    }

    impl Default for ThreadInfoMap {
        fn default() -> Self {
            Self::new()
        }
    }
    struct ThreadInfoSource {
        system: System,
        time_reference: Option<TimeReference>,
    }

    impl ThreadInfoSource {
        pub fn new() -> Self {
            Self {
                system: System::new(),
                time_reference: None,
            }
        }

        pub fn set_time_reference(&mut self, time_reference: Option<TimeReference>) {
            self.time_reference = time_reference;
        }

        fn extract_process_info(event: &TraceEvent) -> Option<&ProcessInfo> {
            match &event.ev {
                EventData::ProcessInfoUpdate { process_info } => process_info.as_ref(),
                _ => None,
            }
        }

        fn apply_process_info(task_info: &mut TaskInfos, process_info: Option<&ProcessInfo>) {
            if let Some(info) = process_info {
                if let Some(ppid) = info.ppid {
                    task_info.ppid = Some(ppid);
                }

                if let Some(comm) = &info.comm {
                    task_info.comm = Some(comm.clone());
                }

                if let Some(cmd) = &info.cmd {
                    task_info.cmd = Some(cmd.clone());
                }
            }
        }

        fn resolve_policy(&mut self, pid: u32, policy: SchedulingPolicy) -> SchedulingPolicy {
            if policy == SchedulingPolicy::Unknown {
                self.get_sched_policy(pid).unwrap_or(policy)
            } else {
                policy
            }
        }

        fn apply_policy_update(
            &mut self,
            task_info: &mut TaskInfos,
            pid: u32,
            policy: SchedulingPolicy,
        ) {
            task_info.policy = self.resolve_policy(pid, policy);
        }

        fn sched_getattr(pid: u32) -> Result<SchedulingPolicy> {
            unsafe {
                let mut attrs = sched_attr_t::default();
                match nc::sched_getattr(pid as i32, &mut attrs, 0) {
                    Ok(()) => {
                        let policy = SchedulingPolicy::from(&attrs);

                        Ok(policy)
                    }
                    Err(e) => Err(anyhow::anyhow!("sched_getattr failed with code {}", e)),
                }
            }
        }

        fn get_comm(process: &Process) -> String {
            process.name().to_string_lossy().into_owned()
        }

        fn get_cmd(process: &Process) -> String {
            process
                .cmd()
                .iter()
                .map(|s| s.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        }

        fn get_ppid(process: &Process) -> Option<u32> {
            process.parent().map(|ppid| ppid.as_u32())
        }

        pub fn get_sys_process(&mut self, pid: u32) -> Option<&SysProcess> {
            let sys_pid = Pid::from_u32(pid);
            let updated = self
                .system
                .refresh_processes(ProcessesToUpdate::Some(&[sys_pid]), true);

            if updated > 0 {
                return self.system.process(sys_pid);
            }

            None
        }

        fn string_missing(value: &Option<String>) -> bool {
            value.as_ref().map(|s| s.is_empty()).unwrap_or(true)
        }

        fn maybe_populate_from_sysinfo(&mut self, task_info: &mut TaskInfos, target_pid: u32) {
            let need_ppid = task_info.ppid.is_none();
            let need_comm = Self::string_missing(&task_info.comm);
            let need_cmd = Self::string_missing(&task_info.cmd);

            if !need_ppid && !need_comm && !need_cmd {
                return;
            }

            let proc_info = self.get_sys_process(target_pid);

            if let Some(process) = proc_info {
                if need_ppid {
                    task_info.ppid = Self::get_ppid(process);
                }
                if need_comm {
                    task_info.comm = Some(Self::get_comm(process));
                }
                if need_cmd {
                    task_info.cmd = Some(Self::get_cmd(process));
                }
            } else {
                if need_comm {
                    task_info.comm = None;
                }
                if need_cmd {
                    task_info.cmd = None;
                }
            }
        }

        pub fn get_sched_policy(&mut self, pid: u32) -> Result<SchedulingPolicy> {
            Self::sched_getattr(pid)
        }

        fn get_sched_affinity(&mut self, pid: u32) -> Result<CpuSet> {
            let p = nix::unistd::Pid::from_raw(pid as i32);
            let ret = nix::sched::sched_getaffinity(p)?;

            Ok(ret)
        }

        pub fn get_affinity_mask(&mut self, pid: u32) -> Result<AffinityMask> {
            let cpu_set = self.get_sched_affinity(pid)?;

            Ok(AffinityMask::from(cpu_set))
        }

        pub fn build_new_task_info(&mut self, event: &TraceEvent) -> TaskInfos {
            let policy = self
                .get_sched_policy(event.id.pid)
                .unwrap_or(SchedulingPolicy::Unknown);

            let affinity_mask = self.get_affinity_mask(event.id.pid).ok();

            let event_time = self
                .time_reference
                .as_ref()
                .and_then(|time_ref| time_ref.create_event_time(event.ts).ok());

            let mut task_info = TaskInfos {
                id: event.id,
                ppid: None,
                comm: None,
                cmd: None,
                policy,
                affinity_mask,
                first_event_time: event_time.clone(),
                last_event_time: event_time,
            };

            let event_process_info = Self::extract_process_info(event);
            Self::apply_process_info(&mut task_info, event_process_info);

            let needs_initial_lookup = task_info.ppid.is_none()
                || Self::string_missing(&task_info.comm)
                || Self::string_missing(&task_info.cmd);

            if needs_initial_lookup {
                self.maybe_populate_from_sysinfo(&mut task_info, event.id.tgid);
            }

            task_info
        }

        pub fn update_task_info(&mut self, task_info: &mut TaskInfos, event: &TraceEvent) {
            let mut need_sys_lookup = false;

            if task_info.cmd.as_ref().is_some_and(|s| s.is_empty()) {
                task_info.cmd = None;
                need_sys_lookup = true;
            }

            if task_info.id.tgid == 0 {
                task_info.id.tgid = event.id.tgid;
            }

            match event.ev {
                EventData::SchedulerChanged {
                    sched_policy: ref policy,
                }
                | EventData::SchedParamsChanged {
                    sched_policy: ref policy,
                } => {
                    self.apply_policy_update(task_info, event.id.pid, *policy);
                }
                EventData::SchedPolicyUpdate {
                    sched_policy: ref policy,
                } => {
                    self.apply_policy_update(task_info, event.id.pid, *policy);
                }
                EventData::SchedProcessExec {} => {}
                EventData::EnterSchedSetAffinity {} => {}
                EventData::AffinityChange {} => {}
                EventData::AffinityChanged {} => {
                    task_info.affinity_mask = self.get_affinity_mask(event.id.pid).ok();
                }
                EventData::AffinityUpdate { ref cpumask } => {
                    task_info.affinity_mask = match cpumask {
                        Some(cpu_ids) => Some(AffinityMask {
                            cpu_set: cpu_ids.clone(),
                        }),
                        None => self.get_affinity_mask(event.id.pid).ok(),
                    };
                }
                EventData::ProcessInfoUpdate { ref process_info } => {
                    Self::apply_process_info(task_info, process_info.as_ref());
                }
                _ => {}
            }

            if need_sys_lookup {
                self.maybe_populate_from_sysinfo(task_info, event.id.tgid);
            }

            if let Some(time_ref) = &self.time_reference {
                if let Ok(event_time) = time_ref.create_event_time(event.ts) {
                    if task_info.first_event_time.is_none() {
                        task_info.first_event_time = Some(event_time.clone());
                    }
                    task_info.last_event_time = Some(event_time);
                }
            }
        }
    }

    impl Default for ThreadInfoSource {
        fn default() -> Self {
            Self::new()
        }
    }

    /// Maintains a pid to taskid mapping.
    pub struct TaskMapper {
        tracked_policies: BTreeSet<i32>,
        versions: HashMap<u32, u32>,
        task_infos: ThreadInfoMap,
        inner: HashMap<u32, TaskId>,
        allow_task_priority_change: bool,
        allow_task_affinity_change: bool,
    }

    impl TaskMapper {
        pub fn new() -> Self {
            Self {
                task_infos: ThreadInfoMap::new(),
                tracked_policies: BTreeSet::new(),
                versions: HashMap::new(),
                inner: HashMap::new(),
                allow_task_affinity_change: false,
                allow_task_priority_change: false,
            }
        }

        pub fn init(&mut self, ctx: &LimeContext) {
            self.tracked_policies.insert(SCHED_FIFO);
            self.tracked_policies.insert(SCHED_RR);
            self.tracked_policies.insert(SCHED_DEADLINE);

            if ctx.trace_best_effort {
                self.tracked_policies.insert(SCHED_BATCH);
                self.tracked_policies.insert(SCHED_OTHER);
                self.tracked_policies.insert(SCHED_IDLE);
            }
            self.allow_task_affinity_change = ctx.allow_task_affinity_change;
            self.allow_task_priority_change = ctx.allow_task_priority_change;

            self.task_infos
                .builder
                .set_time_reference(ctx.time_reference.clone());
        }

        fn new_mapping(&mut self, pid: u32) -> TaskId {
            let version = match self.versions.entry(pid) {
                std::collections::hash_map::Entry::Occupied(mut o) => {
                    let new_version = *o.get() + 1;
                    o.insert(new_version);
                    new_version
                }
                std::collections::hash_map::Entry::Vacant(v) => *v.insert(0),
            };

            TaskId { pid, version }
        }

        fn invalidate(&mut self, pid: u32) -> Option<TaskId> {
            self.inner.remove(&pid)
        }

        fn update(&mut self, pid: u32) -> TaskId {
            let mapping = self.new_mapping(pid);

            self.inner.insert(pid, mapping);

            mapping
        }

        fn should_update_mapping(&self, event: &TraceEvent) -> bool {
            match event.ev {
                EventData::SchedulerChange { sched_policy, .. } => {
                    self.tracked_policies.contains(&sched_policy.policy_num())
                }
                EventData::SchedParamsChange { sched_policy, .. }
                    if !self.allow_task_priority_change =>
                {
                    self.tracked_policies.contains(&sched_policy.policy_num())
                }
                EventData::AffinityChange { .. } if !self.allow_task_affinity_change => true,
                EventData::LimeThrottleRelease => true,
                _ => false,
            }
        }

        fn should_invalidate_mapping(&self, event: &TraceEvent) -> bool {
            match event.ev {
                EventData::SchedSwitchedOut { state, .. } => (state & 0x30) != 0,
                EventData::SchedulerChange { sched_policy, .. } => {
                    !self.tracked_policies.contains(&sched_policy.policy_num())
                }
                EventData::SchedParamsChange { sched_policy, .. }
                    if !self.allow_task_priority_change =>
                {
                    !self.tracked_policies.contains(&sched_policy.policy_num())
                }
                EventData::SchedulerChanged { sched_policy, .. } => {
                    !self.tracked_policies.contains(&sched_policy.policy_num())
                }
                EventData::SchedParamsChanged { sched_policy, .. }
                    if !self.allow_task_priority_change =>
                {
                    !self.tracked_policies.contains(&sched_policy.policy_num())
                }

                _ => false,
            }
        }

        fn extract_pid(&self, event: &TraceEvent) -> u32 {
            event.id.pid
        }

        /// Determines the task id of the input event.
        ///
        /// If the event changes the task id, the state is updated and the old
        /// mapping is returned. If there is no such mapping, `None` is returned.
        pub fn find_task_id(&mut self, event: &TraceEvent) -> Option<TaskId> {
            let pid = self.extract_pid(event);

            let m = self.inner.get(&pid).cloned();

            match m {
                Some(old_mapping) => {
                    if self.should_invalidate_mapping(event) {
                        self.invalidate(pid);

                        return Some(old_mapping);
                    }

                    if self.should_update_mapping(event) {
                        self.invalidate(pid);

                        let new_mapping = self.update(pid);

                        self.task_infos.duplicate_entry(old_mapping, new_mapping);
                        self.task_infos.update_entry(new_mapping, event);

                        return Some(old_mapping);
                    }

                    self.task_infos.update_entry(old_mapping, event);

                    m
                }

                None => {
                    /* If nothing has been observed drop the event */
                    if self.should_invalidate_mapping(event) || self.should_update_mapping(event) {
                        return None;
                    }

                    let new_mapping = self.update(pid);
                    self.task_infos.new_entry(new_mapping, event);
                    Some(new_mapping)
                }
            }
        }

        pub fn get_task_infos(&self, task_id: TaskId) -> Option<&TaskInfos> {
            self.task_infos.get(task_id)
        }
    }

    impl Default for TaskMapper {
        fn default() -> Self {
            Self::new()
        }
    }
}
