//! Thread to task mapping.

use std::fmt::Display;

use anyhow::{bail, Result};
use nom::{
    branch::alt,
    bytes::complete::{tag, take_while1},
    IResult,
};
use serde::{Deserialize, Serialize};

use crate::{events::SchedulingPolicy, utils::ThreadId};

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
    let (i, _) = alt((tag("events"), tag("infos"), tag("models"), tag("jobs")))(i)?;
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
    use sysinfo::{PidExt, System, SystemExt};

    use crate::{
        context::LimeContext,
        events::{EventData, SchedulingPolicy, TraceEvent},
    };

    use super::{AffinityMask, TaskId, TaskInfos};

    use anyhow::Result;

    type SysProcess = sysinfo::Process;

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
                let new_info = old_info.clone();

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
    }

    impl ThreadInfoSource {
        pub fn new() -> Self {
            Self {
                system: System::new(),
            }
        }

        fn sched_getattr(pid: u32) -> Result<SchedulingPolicy> {
            unsafe {
                let mut attrs = sched_attr_t::default();
                let size = std::mem::size_of::<sched_attr_t>() as u32;
                match nc::sched_getattr(pid as i32, &mut attrs, size, 0) {
                    Ok(()) => {
                        let policy = SchedulingPolicy::from(&attrs);

                        Ok(policy)
                    }
                    Err(e) => Err(anyhow::anyhow!("sched_getattr failed with code {}", e)),
                }
            }
        }

        fn get_comm<P: sysinfo::ProcessExt>(process: &P) -> String {
            process.name().to_string()
        }

        fn get_cmd<P: sysinfo::ProcessExt>(process: &P) -> String {
            process.cmd().join(" ")
        }

        fn get_ppid<P: sysinfo::ProcessExt>(process: &P) -> Option<u32> {
            process.parent().map(|ppid| ppid.as_u32())
        }

        pub fn get_sys_process(&mut self, pid: u32) -> Option<&SysProcess> {
            let sys_pid = sysinfo::Pid::from_u32(pid);
            if self.system.refresh_process(sys_pid) {
                return self.system.process(sys_pid);
            }

            None
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
            let p = self.get_sys_process(event.id.pid);

            let ppid = p.and_then(Self::get_ppid);
            let comm = p.map(Self::get_comm);
            let cmd = p.map(Self::get_cmd);

            let policy = self
                .get_sched_policy(event.id.pid)
                .unwrap_or(SchedulingPolicy::Unknown);

            let affinity_mask = self.get_affinity_mask(event.id.pid).ok();

            TaskInfos {
                id: event.id,
                ppid,
                comm,
                cmd,
                policy,
                affinity_mask,
            }
        }

        pub fn update_task_info(&mut self, task_info: &mut TaskInfos, event: &TraceEvent) {
            let mut refresh_cmd = task_info.cmd.as_ref().is_some_and(|s| s.is_empty());

            if task_info.id.tgid == 0 {
                task_info.id.tgid = event.id.tgid;
            }

            match event.ev {
                EventData::SchedulerChanged {
                    sched_policy: policy,
                } if policy == SchedulingPolicy::Unknown => {
                    let new_policy = self.get_sched_policy(event.id.pid);

                    task_info.policy = new_policy.unwrap_or(policy);
                }
                EventData::SchedulerChanged {
                    sched_policy: policy,
                } => {
                    task_info.policy = policy;
                }
                EventData::SchedProcessFork {
                    sched_policy: policy,
                } => {
                    task_info.policy = policy;
                }
                EventData::SchedProcessExec => {
                    refresh_cmd = true;
                }
                EventData::AffinityChanged => {
                    let affinity_mask = self.get_affinity_mask(event.id.pid).ok();

                    task_info.affinity_mask = affinity_mask;
                }
                _ => {}
            }

            if refresh_cmd {
                let p = self.get_sys_process(event.id.pid);

                task_info.comm = p.map(Self::get_comm);
                task_info.cmd = p.map(Self::get_cmd);
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
                EventData::SchedulerChange { sched_policy } => {
                    self.tracked_policies.contains(&sched_policy.policy_num())
                }
                EventData::SchedParamsChange { sched_policy }
                    if !self.allow_task_priority_change =>
                {
                    self.tracked_policies.contains(&sched_policy.policy_num())
                }
                EventData::AffinityChange if !self.allow_task_affinity_change => true,
                EventData::LimeThrottleRelease => true,
                _ => false,
            }
        }

        fn should_invalidate_mapping(&self, event: &TraceEvent) -> bool {
            match event.ev {
                EventData::SchedSwitchedOut { state, .. } => (state & 0x30) != 0,
                EventData::SchedulerChange { sched_policy } => {
                    !self.tracked_policies.contains(&sched_policy.policy_num())
                }
                EventData::SchedParamsChange { sched_policy }
                    if !self.allow_task_priority_change =>
                {
                    !self.tracked_policies.contains(&sched_policy.policy_num())
                }
                EventData::SchedulerChanged { sched_policy } => {
                    !self.tracked_policies.contains(&sched_policy.policy_num())
                }
                EventData::SchedParamsChanged { sched_policy }
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
