//! Job separator detection.
//!
//! Job extraction has two consecutive steps:
//! 1. The stream of events is analysed to detect job separations. The different
//!    type of job separation are characterised by a __signature__.
//! 2. Event sequences matching a signature are analysed to extract a job
//!    separator (i.e, a job type identifier) and a job separation (i.e. timing
//!    informations).
//!
//! Signature matching is implemented with  deterministic finite automata (DFA).
//! A signature is defined by a type, which represents a DFA state and implements
//! the `Signature` trait. This trait contains the DFA transition function
//! and the analysis methods for job separator and separation extraction. Please
//! inspect the `suspension` and `syscall` modules for implementation examples.

use std::fmt::Display;

use serde::Serialize;

use crate::{
    events::{ClockId, TraceEvent},
    job::Job,
};

use self::{dfa::Dfa, suspension::SelfSuspension, syscall::BlockingSyscall};

mod dfa;

pub mod suspension;
pub mod syscall;

pub trait Signature {
    /// Constructs an initial state
    fn initial_state() -> Self;
    /// Inspects an event and constructs the next state.
    fn next(&self, event: &TraceEvent) -> Self;
    /// Returns true if the state is terminal, i.e., if the signature has been
    /// matched.
    fn is_terminal_state(&self) -> bool;
    /// Returns true if the state is a rejection state indicating that the DFA
    /// should be reset.
    fn is_rejection_state(&self) -> bool;

    /// Consume a sequence of events and return the last state.
    /// Transitioning to the rejection state causes a reset to the initial state.
    /// This function is mostly for testing purpose.
    fn check_seq(events: &[TraceEvent]) -> Self
    where
        Self: Sized,
    {
        let mut s = Self::initial_state();

        for event in events {
            s = s.next(event);
            if s.is_rejection_state() {
                s = Self::initial_state();
            }
        }

        s
    }

    /// Extract a job separator from a sequence of events matching this signature.
    fn job_separator(&self, matched_events: &[TraceEvent]) -> JobSeparator;
    /// Extract a job separation from a sequence of events matching this signature.
    fn job_separation(&self, matched_events: &[TraceEvent]) -> JobSeparation;
}

/// Wraps all supported signatures in a unique type.
/// Should be updated when adding new signatures.
pub struct SignatureMatcher {
    syscall: Dfa<BlockingSyscall>,
    suspension: Dfa<SelfSuspension>,
    in_progress: usize,
    buffer: Vec<TraceEvent>,
}

impl SignatureMatcher {
    pub fn new() -> Self {
        Self {
            syscall: Dfa::new(),
            suspension: Dfa::new(),
            in_progress: 0,
            buffer: Vec::new(),
        }
    }

    /// Initialize the matcher by feeding it a `LimeStartOfTrace` pseudo-event.
    pub fn init(&mut self) {
        self.buffer.push(TraceEvent::start_of_trace());
        self.syscall.update_no_extract(&self.buffer[0], 0);
        self.suspension.update_no_extract(&self.buffer[0], 0);
    }

    fn update_matchers(&mut self, pos: usize, result: &mut Vec<(JobSeparator, JobSeparation)>) {
        self.in_progress = 0;

        self.syscall.update(&self.buffer, pos, result);
        self.suspension.update(&self.buffer, pos, result);
    }

    /// Returns true if at least one signature matcher is in progress, i.e.,
    /// the underlying automaton is neither in the terminal state nor in the
    /// rejection state.
    pub fn ongoing_matching(&self) -> bool {
        self.suspension.is_matching() || self.syscall.is_matching()
    }

    /// Consume an event and update the DFA state. Write succesfully extracted
    /// job separators and separations in the `result` vector.
    pub fn consume_event(
        &mut self,
        event: TraceEvent,
        result: &mut Vec<(JobSeparator, JobSeparation)>,
    ) {
        self.buffer.push(event);
        let pos = self.buffer.len() - 1;

        self.update_matchers(pos, result);

        if !self.ongoing_matching() {
            self.buffer.clear();
        }
    }
}

impl Default for SignatureMatcher {
    fn default() -> Self {
        Self::new()
    }
}

/// Contains the timing information about the end of a job and the beginning of
/// the next one.
#[derive(PartialEq, Eq, Debug, Default)]
pub struct JobSeparation {
    /// Last cycle of the previously pending job.
    pub prev_end: u64,
    /// Arrival time of the currently pending job.
    pub curr_arrival: Option<u64>,
    /// Release time of the currently pending job.
    pub curr_release: u64,
    /// First cycle the currently pending job.
    pub curr_first_cycle: u64,
}

impl JobSeparation {
    /// Returns true if there is no waiting time between the previously and the
    /// currently pending job.
    pub fn is_back_to_back(&self) -> bool {
        self.curr_first_cycle == self.prev_end
    }

    /// Combine two consecutive job separations to build a new job.
    pub fn make_job(&self, next_boundary: &Self) -> Job {
        Job {
            arrival: self.curr_arrival,
            release: self.curr_release,
            end: next_boundary.prev_end,
            first_cycle: self.curr_first_cycle,
        }
    }
}

/// Extracted job type identifier.
///
/// Job separators identifies specific job arrival sites.
/// Job-specific model extractions is performed for each detected job separators.
#[derive(PartialEq, Eq, Hash, Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JobSeparator {
    ClockNanosleep { clock_id: ClockId, abs_time: bool },
    Select { inp: u64 },
    Poll { pfds: u64 },
    ReadSock { fd: u32 },
    ReadFifo { fd: u32 },
    ReadBlk { fd: u32 },
    ReadChr { fd: u32 },
    Accept { sock_fd: u32 },
    FutexWait { uaddr: u64, clock_id: ClockId },
    FutexWaitBitSet { uaddr: u64, clock_id: ClockId },
    EpollPwait { epfd: u32 },
    PSelect6 { inp: u64 },
    Nanosleep {},
    Pause,
    RtSigsuspend,
    MqTimedReceive { mqd: i32 },
    Recv { sock_fd: i32 },
    Recvfrom { sock_fd: i32 },
    Recvmsg { sock_fd: i32 },
    Recvmmsg { sock_fd: i32 },
    Msgrcv { msqid: i32 },
    Semop { sem_id: i32, timeout: u64 },
    SigTimedWait,

    Suspension,

    YieldSchedDeadline,
}

impl Display for JobSeparator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            // TODO handle abs_time field
            JobSeparator::ClockNanosleep { clock_id, .. } => {
                format!("clock_nanosleep @{:?}", clock_id)
            }
            JobSeparator::Select { inp } => format!("select @:{:x}", inp),
            JobSeparator::Poll { pfds } => format!("poll @:{:x}", pfds),
            JobSeparator::ReadSock { fd } => format!("read_sock fd:{}", fd),
            JobSeparator::ReadFifo { fd } => format!("read_fifo fd:{}", fd),
            JobSeparator::ReadBlk { fd } => format!("read_blk fd:{}", fd),
            JobSeparator::ReadChr { fd } => format!("read_chr fd:{}", fd),
            JobSeparator::Accept { sock_fd } => format!("accept fd:{}", sock_fd),
            JobSeparator::FutexWait { uaddr, clock_id } => {
                format!("futex_wait @:{:x} on {:?}", uaddr, clock_id)
            }
            JobSeparator::FutexWaitBitSet { uaddr, clock_id } => {
                format!("futex_wait_bitset @:{:x} on {:?}", uaddr, clock_id)
            }
            JobSeparator::SigTimedWait => "sigtimedwait".to_string(),
            JobSeparator::Suspension => "suspension".to_string(),
            JobSeparator::EpollPwait { epfd } => format!("epoll_pwait fd:{}", epfd),
            JobSeparator::PSelect6 { inp } => format!("pselect6 @:{:x}", inp),
            JobSeparator::YieldSchedDeadline => "yield_sched_deadline".to_string(),
            JobSeparator::Nanosleep {} => "nanosleep".to_string(),
            JobSeparator::Pause => "pause".to_string(),
            JobSeparator::RtSigsuspend => "rt_sigsuspend".to_string(),
            JobSeparator::MqTimedReceive { mqd } => format!("mq_timed_receive mqd:{}", mqd),
            JobSeparator::Recv { sock_fd } => format!("recv sock_fd:{}", sock_fd),
            JobSeparator::Recvfrom { sock_fd } => format!("recvfrom sock_fd:{}", sock_fd),
            JobSeparator::Recvmsg { sock_fd } => format!("recvmsg sock_fd:{}", sock_fd),
            JobSeparator::Recvmmsg { sock_fd } => format!("recvmmsg sock_fd:{}", sock_fd),
            JobSeparator::Msgrcv { msqid } => format!("recv msqid:{}", msqid),
            JobSeparator::Semop { sem_id, timeout } => {
                format!("semop semid:{} timeout:{}", sem_id, timeout)
            }
        };

        write!(f, "{}", s)
    }
}
