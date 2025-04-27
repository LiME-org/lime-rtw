//! eBPF event tracer.
//!
//!

use crate::context::LimeContext;
use crate::task::TaskInfos;
use crate::task::{mapper::TaskMapper, TaskId};
use crate::utils::any_as_u8_slice_mut;
use crate::{EventProcessor, EventSource};
use anyhow::{Error, Result};
use libbpf_rs::skel::{OpenSkel, SkelBuilder};
use libbpf_rs::{MapFlags, MapHandle, RingBuffer, RingBufferBuilder};
use nix::sys;
use nix::sys::signal::Signal::{self, SIGCHLD};
use plain::Plain;

use crate::events::{EventData, SchedulingPolicy, TraceEvent};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::AtomicBool;
use std::thread::JoinHandle;
use std::time::Duration;

use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{mpsc, Arc, Barrier, Mutex};

use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::{fork, Pid};
use nix::{
    sys::signal::{
        SigSet,
        Signal::{SIGINT, SIGUSR1},
    },
    unistd::ForkResult,
};
use std::os::unix::process::CommandExt;

use std::cell::RefCell;

#[allow(clippy::all)]
mod skel {
    include!(concat!(env!("OUT_DIR"), "/lime_tracer.skel.rs"));
}
use skel::*;

pub type RawEvent = lime_tracer_bss_types::lime_event;
pub type EventType = lime_tracer_bss_types::event_type;
pub type BPFIter = libbpf_rs::Iter;
pub type RawClockId = lime_tracer_bss_types::clock_id;
pub type SiCode = lime_tracer_bss_types::si_code;

unsafe impl Plain for RawEvent {}

struct InnerRateLimiter {
    budget: usize,
    curr_budget: usize,
    remaining_throttled_period: usize,
    replenished: bool,
    throttled: bool,
}

impl InnerRateLimiter {
    pub fn new(budget: usize) -> Self {
        Self {
            budget,
            curr_budget: budget,
            remaining_throttled_period: 0,
            replenished: false,
            throttled: false,
        }
    }

    pub fn refresh(&mut self, pid: u32, throttle_map: &MapHandle) {
        if self.throttled {
            if self.remaining_throttled_period == 0 {
                self.curr_budget = self.budget;
                self.replenished = true;
                self.throttled = false;

                let mut pid = pid;
                let key = unsafe { any_as_u8_slice_mut(&mut pid) };
                let _ = throttle_map.delete(key);
            } else {
                self.remaining_throttled_period -= 1;
            }
        }
    }

    pub fn get_token(&mut self) -> RateLimiterResult {
        if self.curr_budget == 0 {
            if !self.throttled {
                self.throttled = true;
                self.remaining_throttled_period = 1;

                return RateLimiterResult::Throttling;
            }

            return RateLimiterResult::Throttled;
        }

        self.curr_budget -= 1;

        if self.replenished {
            self.replenished = false;

            return RateLimiterResult::ThrottleRelease;
        }

        RateLimiterResult::Ok
    }
}

struct RateLimiter {
    budget: usize,
    buckets: HashMap<u32, InnerRateLimiter>,
}

impl RateLimiter {
    pub fn new(budget: usize) -> Self {
        Self {
            budget,
            buckets: HashMap::new(),
        }
    }

    pub fn refresh(&mut self, throttle_map: &MapHandle) {
        for (k, v) in self.buckets.iter_mut() {
            v.refresh(*k, throttle_map);
        }
    }

    pub fn remove(&mut self, pid: u32) {
        self.buckets.remove(&pid);
    }

    pub fn get_token(&mut self, pid: u32) -> RateLimiterResult {
        match self.buckets.entry(pid) {
            std::collections::hash_map::Entry::Occupied(mut o) => o.get_mut().get_token(),
            std::collections::hash_map::Entry::Vacant(v) => {
                v.insert(InnerRateLimiter::new(self.budget)).get_token()
            }
        }
    }
}

pub enum SchedPolicy {
    SchedNormal = 0,
    SchedFifo = 1,
    SchedRr = 2,
    SchedBatch = 3,
    SchedIso = 4,
    SchedIdle = 5,
    SchedDeadline = 6,
}

struct TracedCmd {
    cmd_line: String,
    pid: Pid,
}

impl TracedCmd {
    pub fn new(mut cmd: std::process::Command) -> Result<Self> {
        let mut sset = SigSet::empty();
        sset.add(sys::signal::Signal::SIGUSR1);
        sset.thread_block()?;

        match fork()? {
            ForkResult::Child => {
                sset.wait()?;
                sset.thread_unblock()?;
                let e = cmd.exec();

                Err(Error::new(e))
            }

            ForkResult::Parent { child, .. } => {
                let mut cmd_line = vec![cmd.get_program().to_string_lossy()];

                for arg in cmd.get_args() {
                    let sarg = arg.to_string_lossy();
                    cmd_line.push(sarg);
                }

                let l = cmd_line.join(" ");

                Ok(Self {
                    cmd_line: l,
                    pid: child,
                })
            }
        }
    }

    pub fn start(&self, verbose: bool) -> Result<()> {
        let mut sset = SigSet::empty();
        sset.add(sys::signal::Signal::SIGUSR1);

        if verbose {
            eprintln!("LiME: launching {:#?}", &self.cmd_line);
        }

        sys::signal::kill(self.pid, SIGUSR1)?;

        sset.thread_unblock().map_err(Error::new)
    }

    pub fn complete(&self, sig: i32, verbose: bool) -> Result<i32> {
        let mut s = 0;

        if let Ok(SIGINT) = Signal::try_from(sig) {
            sys::signal::kill(self.pid, SIGINT)?;
        }

        if let WaitStatus::Exited(_, status) = waitpid(self.pid, None)? {
            if verbose {
                eprintln!("LiME: thread {} exited with {}", self.pid, status);
            }
            s = status;
        }

        Ok(s)
    }
}

fn bump_memlock_rlimit() -> Result<()> {
    use anyhow::bail;

    let rlimit = libc::rlimit {
        rlim_cur: 128 << 20,
        rlim_max: 128 << 20,
    };

    if unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &rlimit) } != 0 {
        bail!("Failed to increase rlimit");
    }

    Ok(())
}

struct BPFTracer<'a> {
    rx: Option<Receiver<Vec<TraceEvent>>>,
    handles: Vec<JoinHandle<Result<i32>>>,
    trace_best_effort: bool,
    trace_all: bool,
    verbose: bool,
    tracer_term: Arc<AtomicBool>,
    cmd: Option<std::process::Command>,
    phantom: std::marker::PhantomData<&'a ()>,
    limiter_period: Duration,
    limiter_budget: usize,
    tx_batch_size: usize,
    poll_interval: Duration,
}

impl BPFTracer<'_> {
    pub fn new() -> Self {
        Self {
            rx: None,
            cmd: None,
            trace_best_effort: false,
            verbose: false,
            trace_all: false,
            tracer_term: Arc::new(AtomicBool::new(false)),
            handles: Vec::with_capacity(2),
            phantom: std::marker::PhantomData,
            limiter_period: Duration::from_secs(0),
            limiter_budget: 0,
            tx_batch_size: 4 * 1024,
            poll_interval: Duration::from_millis(10),
        }
    }

    pub fn init(mut self, ctx: &LimeContext) -> Self {
        self.verbose = ctx.verbose;
        self.trace_all = ctx.trace_all;
        self.trace_best_effort = ctx.trace_best_effort;
        self.cmd = ctx.get_cmd();
        self.limiter_budget = ctx.limiter_budget;
        self.limiter_period = ctx.limiter_period;
        self.tx_batch_size = ctx.tx_batch_size;
        self.poll_interval = ctx.ebpf_poll_interval;

        self
    }

    fn start_tracer_thread(
        &self,
        cmd_pid: i32,
        tx: SyncSender<Vec<TraceEvent>>,
        barrier: Arc<Barrier>,
    ) -> JoinHandle<Result<i32>> {
        let trace_all = self.trace_all;
        let trace_best_effort = self.trace_best_effort;
        let tracer_term = self.tracer_term.clone();
        let limiter_period = self.limiter_period;
        let limiter_budget = self.limiter_budget;
        let tx_batch_size = self.tx_batch_size;
        let poll_interval = self.poll_interval;
        let verbose = self.verbose;

        std::thread::spawn(move || {
            let skel_builder = LimeTracerSkelBuilder::default();

            let mut open_skel = skel_builder.open().map_err(Error::new)?;

            if !trace_all && (cmd_pid != 0) {
                open_skel.rodata().target_tgid = cmd_pid;
            }

            if trace_best_effort {
                open_skel.rodata().sched_policy_mask = 0x47;
            }

            // Begin tracing
            let mut skel = open_skel.load().map_err(Error::new)?;

            let mut attached = 0;
            let mut links = vec![];
            let mut failed = vec![];

            let prev = libbpf_rs::set_print(None);

            for prog in skel.obj.progs_iter_mut() {
                match prog.attach() {
                    Ok(link) => {
                        links.push(link);
                        attached += 1;
                    }
                    Err(_) => {
                        failed.push(String::from(prog.name()));
                    }
                }
            }
            libbpf_rs::set_print(prev);

            if verbose {
                eprintln!(
                    "LiME: {} eBPF programs succesfully attached ({} failed)",
                    attached,
                    failed.len()
                );
            }

            // event batching
            let event_batch = RefCell::new(Vec::with_capacity(tx_batch_size));
            let send_batched = |ev: TraceEvent| {
                let mut batch = event_batch.borrow_mut();
                batch.push(ev);
                if batch.len() >= tx_batch_size {
                    tx.send(std::mem::take(&mut *batch)).unwrap();
                }
            };

            let m = skel.maps();
            let throttled_map = m.throttled();
            let throttled = MapHandle::try_clone(throttled_map)?;

            let mut limiter = BPFSourceRateLimiter::new(limiter_period, limiter_budget, throttled);
            let _ = limiter.start();

            let mut mm = skel.maps_mut();

            let mut builder = RingBufferBuilder::new();
            _ = builder
                .add(mm.events(), move |d: &[u8]| {
                    let raw_event = Self::parse_raw_event(d);
                    let event = TraceEvent::from(&raw_event);

                    match limiter.is_block_listed(&event) {
                        RateLimiterResult::Ok => send_batched(event),
                        RateLimiterResult::Throttled => {}
                        RateLimiterResult::ThrottleRelease => {
                            if verbose {
                                eprintln!("LiME: Releasing throttle of thread {}", event.id.pid);
                            }
                            let mut e2 = event.clone();
                            e2.ev = EventData::LimeThrottleRelease;

                            send_batched(e2);
                            send_batched(event);
                        }
                        RateLimiterResult::Throttling => {
                            if verbose {
                                eprintln!("LiME: Dropping events of thread {}", event.id.pid);
                            }
                            limiter.throttle(event.id.pid);
                            let mut e2 = event.clone();
                            e2.ev = EventData::LimeThrottle;

                            send_batched(event);
                            send_batched(e2);
                        }
                    }

                    0
                })
                .unwrap();

            let rb = builder.build().map_err(Error::new)?;

            if cmd_pid == 0 {
                if verbose {
                    eprintln!("LiME: tracing started (press Ctrl+C to stop)");
                }
            } else if verbose {
                eprintln!("LiME: tracing started");
            }

            // signal that we are ready to trace
            barrier.wait();

            // Paired with the store in `start_coordination_thread`
            while !tracer_term.load(Ordering::Acquire) {
                Self::poll_ringbuf_signal(&rb, Duration::from_millis(250), poll_interval).unwrap();
            }

            if verbose {
                eprintln!("\nLiME: tracing stopped");
            }

            let mut batch = event_batch.borrow_mut();
            if !batch.is_empty() {
                tx.send(std::mem::take(&mut *batch)).unwrap();
            }

            // limiter.stop();

            if !failed.is_empty() && verbose {
                eprint!("\nLiME: note: failed to attach ");
                for p in failed.iter() {
                    eprint!(" {}", p)
                }
                eprintln!()
            }

            Ok(0)
        })
    }

    fn start_coordination_thread(
        &self,
        mut cmd: Option<TracedCmd>,
        barrier: Arc<Barrier>,
    ) -> JoinHandle<Result<i32>> {
        let tracer_term = self.tracer_term.clone();
        let verbose = self.verbose;

        std::thread::spawn(move || {
            let mut signals = signal_hook::iterator::Signals::new([SIGCHLD as i32, SIGINT as i32])?;
            let mut ret = Ok(0);

            // wait for the tracer
            barrier.wait();

            cmd.as_mut().map(|c| c.start(verbose));

            // wait for the launched command to complete or sigint.
            if let Some(signal) = signals.wait().next() {
                match Signal::try_from(signal) {
                    Ok(SIGCHLD) | Ok(SIGINT) => {
                        if let Some(c) = cmd {
                            ret = c.complete(signal, verbose);
                        }
                    }
                    _ => unreachable!(),
                }
            }

            // Shut the tracer down.
            // Paired with the store in `start_tracer_thread`
            tracer_term.store(true, Ordering::Release);

            ret
        })
    }

    pub fn start(mut self, ctx: &LimeContext) -> Result<Self> {
        bump_memlock_rlimit()?;

        let barrier = Arc::new(Barrier::new(2));
        let (tx, rx) = mpsc::sync_channel::<Vec<TraceEvent>>(128);

        self.rx = Some(rx);

        // Since this function forks it must be called before spawing threads.
        let cmd = ctx.get_cmd().map(TracedCmd::new);

        if let Some(c) = cmd {
            let c1 = c?;

            self.handles
                .push(self.start_tracer_thread(c1.pid.as_raw(), tx, barrier.clone()));

            self.handles
                .push(self.start_coordination_thread(Some(c1), barrier));
        } else {
            self.handles
                .push(self.start_tracer_thread(0, tx, barrier.clone()));

            self.handles
                .push(self.start_coordination_thread(None, barrier));
        }

        Ok(self)
    }

    fn poll_ringbuf_signal(
        perf: &RingBuffer,
        timeout: Duration,
        poll_interval: Duration,
    ) -> Result<(), Error> {
        if let Err(libbpf_rs::Error::System(libc::EINTR)) = perf.poll(timeout) {
            perf.poll(Duration::from_millis(0)).map_err(Error::new)
        } else {
            std::thread::sleep(poll_interval);
            Ok(())
        }
    }

    pub fn join_children(&mut self) -> Result<i32> {
        self.handles
            .drain(..)
            .map(|handle| handle.join().expect("Thread panicked"))
            .try_fold(0, |acc, result| {
                let res = result?;
                Ok(if res != 0 { res } else { acc })
            })
    }

    fn parse_raw_event(data: &[u8]) -> RawEvent {
        let mut event = RawEvent::default();
        plain::copy_from_bytes(&mut event, data).expect("Data buffer was too short");

        event
    }

    pub fn events(&self) -> impl Iterator<Item = TraceEvent> + '_ {
        TraceEvents::new(self.rx.as_ref().unwrap().iter(), self.tx_batch_size)
    }
}

struct TraceEvents<I> {
    iter: I,
    backlog: VecDeque<TraceEvent>,
    pending_policy: HashMap<u32, (i32, SchedulingPolicy)>,
}

impl<I> TraceEvents<I> {
    pub fn new(iter: I, capacity: usize) -> Self {
        Self {
            iter,
            backlog: VecDeque::with_capacity(capacity),
            pending_policy: HashMap::new(),
        }
    }

    fn handle_event(&mut self, event: TraceEvent) -> TraceEvent {
        match event.ev {
            EventData::AffinityChange => {
                let next = TraceEvent {
                    ts: event.ts,
                    id: event.id,
                    ev: EventData::AffinityChanged,
                };

                self.backlog.push_back(next);

                event
            }

            EventData::EnterSetScheduler {
                old_policy_num,
                sched_policy,
            } => {
                self.pending_policy
                    .insert(event.id.pid, (old_policy_num, sched_policy));

                event
            }

            EventData::RawSchedulerChange => {
                if let Some((old_policy, sched_policy)) = self.pending_policy.remove(&event.id.pid)
                {
                    let next = TraceEvent {
                        ts: event.ts,
                        id: event.id,
                        ev: if sched_policy.policy_num() == old_policy {
                            EventData::SchedParamsChanged { sched_policy }
                        } else {
                            EventData::SchedulerChanged { sched_policy }
                        },
                    };

                    self.backlog.push_back(next);

                    let mut ret = event;
                    ret.ev = if sched_policy.policy_num() == old_policy {
                        EventData::SchedParamsChange { sched_policy }
                    } else {
                        EventData::SchedulerChange { sched_policy }
                    };

                    return ret;
                }

                let sched_policy = SchedulingPolicy::Unknown;

                TraceEvent {
                    ts: event.ts,
                    id: event.id,
                    ev: EventData::SchedulerChanged { sched_policy },
                }
            }

            EventData::SchedulerChangeFailed => {
                self.pending_policy.remove(&event.id.pid);

                event
            }

            _ => event,
        }
    }
}

impl<I> Iterator for TraceEvents<I>
where
    I: Iterator<Item = Vec<TraceEvent>>,
{
    type Item = TraceEvent;

    fn next(&mut self) -> Option<Self::Item> {
        let event = self.backlog.pop_front();

        if event.is_some() {
            return event;
        }

        self.iter.next().and_then(|batch| {
            let mut iter = batch.into_iter();
            let event = iter.next().map(|e| self.handle_event(e));
            self.backlog.extend(iter);
            event
        })
    }
}

impl Default for BPFTracer<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// Event source for the eBPF tracer backend.
pub struct BPFSource<'a> {
    exit_status: i32,
    tracer: BPFTracer<'a>,
    task_mapper: TaskMapper,
}

impl BPFSource<'_> {
    pub fn new() -> Self {
        Self {
            exit_status: 0,
            tracer: BPFTracer::new(),
            task_mapper: TaskMapper::new(),
        }
    }

    pub fn init(mut self, ctx: &LimeContext) -> Self {
        self.task_mapper.init(ctx);
        self.tracer = self.tracer.init(ctx);

        self
    }

    pub fn start(mut self, ctx: &LimeContext) -> Result<Self> {
        self.tracer = self.tracer.start(ctx)?;

        Ok(self)
    }

    pub fn exit_status(&self) -> i32 {
        self.exit_status
    }
}

enum RateLimiterResult {
    /// The event can be processed
    Ok,
    /// The thread is already throttled
    Throttled,
    /// The event exhausts the thread budget.
    Throttling,
    ThrottleRelease,
}

struct BPFSourceRateLimiter {
    period: Duration,
    limiter: Arc<Mutex<RateLimiter>>,
    stop: Arc<AtomicBool>,
    refresher_handle: Option<JoinHandle<Result<()>>>,
    throttled_map: libbpf_rs::MapHandle,
    enabled: bool,
}

impl BPFSourceRateLimiter {
    pub fn new(period: Duration, budget: usize, throttled_map: libbpf_rs::MapHandle) -> Self {
        Self {
            period,
            limiter: Arc::new(Mutex::new(RateLimiter::new(budget))),
            stop: Arc::new(AtomicBool::new(false)),
            refresher_handle: None,
            throttled_map,
            enabled: !period.is_zero(),
        }
    }

    pub fn start(&mut self) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        let stop = Arc::clone(&self.stop);
        let limiter = Arc::clone(&self.limiter);
        let period = self.period;
        let throttled_map = MapHandle::try_clone(&self.throttled_map)?;

        self.refresher_handle = Some(std::thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                if let Ok(mut guard) = limiter.lock() {
                    guard.refresh(&throttled_map);
                }
                std::thread::park_timeout(period);
            }
            Ok(())
        }));

        Ok(())
    }

    fn get_token(&self, pid: u32) -> RateLimiterResult {
        self.limiter.lock().unwrap().get_token(pid)
    }

    pub fn is_block_listed(&mut self, event: &TraceEvent) -> RateLimiterResult {
        if !self.enabled {
            return RateLimiterResult::Ok;
        }

        let pid = event.id.pid;

        if event.is_process_exit() {
            self.limiter.lock().unwrap().remove(pid);

            return RateLimiterResult::Ok;
        }

        self.get_token(pid)
    }

    pub fn throttle(&mut self, pid: u32) {
        let mut pid = pid;
        let key = unsafe { any_as_u8_slice_mut(&mut pid) };

        let _ = self.throttled_map.update(key, &[0], MapFlags::ANY);
    }
}

impl Default for BPFSource<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSource for BPFSource<'_> {
    fn event_loop<P: EventProcessor>(
        &mut self,
        processor: &mut P,
        ctx: &LimeContext,
    ) -> Result<()> {
        for event in self.tracer.events() {
            if let Some(task_id) = self.task_mapper.find_task_id(&event) {
                processor.consume_event(&task_id, event, ctx);
            }
        }

        self.exit_status = self.tracer.join_children()?;
        Ok(())
    }

    fn get_task_info(&self, task_id: TaskId) -> Option<TaskInfos> {
        self.task_mapper.get_task_infos(task_id).cloned()
    }
}

pub mod dd;
