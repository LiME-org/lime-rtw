use std::collections::HashMap;

use crate::events::{EventData, ProcessInfo, TraceEvent};
use crate::tracer::{EventType, RawEvent};
use crate::utils::ThreadId;

use super::types;

const MAX_CMD_LEN: usize = 512;
const AFFINITY_MASK_WORDS: usize = 16;
const AFFINITY_CHUNK_BYTES: usize = 32;
const AFFINITY_CHUNK_WORDS: usize = AFFINITY_CHUNK_BYTES / std::mem::size_of::<u64>();
const AFFINITY_MAX_CHUNKS: usize = AFFINITY_MASK_WORDS.div_ceil(AFFINITY_CHUNK_WORDS);

pub enum ProcessInfoAssemblerAction {
    NotHandled,
    Consumed,
    Completed(TraceEvent),
}

pub struct ProcessInfoAssembler {
    states: HashMap<u64, ProcessInfoState>,
}

impl ProcessInfoAssembler {
    pub fn new() -> Self {
        Self {
            states: HashMap::new(),
        }
    }

    pub fn handle(&mut self, event: &RawEvent) -> ProcessInfoAssemblerAction {
        match event.ev_type {
            EventType::PROCESS_INFO_START => {
                let start = unsafe { &event.evd.process_info_start };
                let state = ProcessInfoState::new(event, start);
                self.states.insert(event.pid_tgid, state);
                ProcessInfoAssemblerAction::Consumed
            }
            EventType::PROCESS_INFO_CMD_CHUNK => {
                if let Some(state) = self.states.get_mut(&event.pid_tgid) {
                    let chunk = unsafe { &event.evd.process_info_chunk };
                    state.append_chunk(chunk);
                }
                ProcessInfoAssemblerAction::Consumed
            }
            EventType::PROCESS_INFO_END => {
                if let Some(state) = self.states.remove(&event.pid_tgid) {
                    if let Some((id, ts, process_info)) = state.into() {
                        let trace_event = TraceEvent {
                            ts,
                            id,
                            ev: EventData::ProcessInfoUpdate {
                                process_info: Some(process_info),
                            },
                        };
                        return ProcessInfoAssemblerAction::Completed(trace_event);
                    }
                }

                ProcessInfoAssemblerAction::Consumed
            }
            _ => ProcessInfoAssemblerAction::NotHandled,
        }
    }
}

impl Default for ProcessInfoAssembler {
    fn default() -> Self {
        Self::new()
    }
}

struct ProcessInfoState {
    ts: u64,
    id: ThreadId,
    ppid: u32,
    comm: Option<String>,
    cmd: Vec<u8>,
    cmd_complete: bool,
}

impl ProcessInfoState {
    fn new(event: &RawEvent, start: &types::lime_process_info_start) -> ProcessInfoState {
        let id = ThreadId::from(event.pid_tgid);
        let comm = c_string_from_array(&start.comm);
        ProcessInfoState {
            ts: event.ts,
            id,
            ppid: start.ppid,
            comm,
            cmd: Vec::with_capacity(MAX_CMD_LEN),
            cmd_complete: false,
        }
    }

    fn append_chunk(&mut self, chunk: &types::lime_process_info_chunk) {
        if self.cmd_complete {
            return;
        }

        let len = chunk.chunk_len as usize;

        for &byte in chunk.chunk.iter().take(len) {
            if byte == 0 {
                self.cmd_complete = true;
                break;
            }
            self.cmd.push(byte as u8);
        }
    }
}

impl From<ProcessInfoState> for Option<(ThreadId, u64, ProcessInfo)> {
    fn from(state: ProcessInfoState) -> Self {
        let has_ppid = state.ppid != 0;
        let comm = state.comm;
        let cmd = if state.cmd.is_empty() {
            None
        } else {
            Some(String::from_utf8_lossy(&state.cmd).to_string())
        };

        if !has_ppid && comm.is_none() && cmd.is_none() {
            return None;
        }

        let process_info = ProcessInfo {
            ppid: if has_ppid { Some(state.ppid) } else { None },
            comm,
            cmd,
        };

        Some((state.id, state.ts, process_info))
    }
}

pub enum AffinityAssemblerAction {
    NotHandled,
    Consumed,
    Completed(TraceEvent),
}

pub struct AffinityUpdateAssembler {
    states: HashMap<u64, AffinityState>,
}

impl AffinityUpdateAssembler {
    pub fn new() -> Self {
        Self {
            states: HashMap::new(),
        }
    }

    pub fn handle(&mut self, event: &RawEvent) -> AffinityAssemblerAction {
        match event.ev_type {
            EventType::AFFINITY_UPDATE_START => {
                let start = unsafe { &event.evd.affinity_update_start };
                let expected_chunks = std::cmp::min(start.chunk_count, AFFINITY_MAX_CHUNKS as u32);
                let state = AffinityState::new(event, expected_chunks);
                self.states.insert(event.pid_tgid, state);
                AffinityAssemblerAction::Consumed
            }
            EventType::AFFINITY_UPDATE_CHUNK => {
                if let Some(state) = self.states.get_mut(&event.pid_tgid) {
                    let chunk = unsafe { &event.evd.affinity_update_chunk };
                    state.append_chunk(chunk);
                }
                AffinityAssemblerAction::Consumed
            }
            EventType::AFFINITY_UPDATE_CHUNK_END => {
                if let Some(mut state) = self.states.remove(&event.pid_tgid) {
                    let chunk = unsafe { &event.evd.affinity_update_chunk };
                    state.append_chunk(chunk);
                    if let Ok(trace_event) = state.try_into() {
                        return AffinityAssemblerAction::Completed(trace_event);
                    }
                }
                AffinityAssemblerAction::Consumed
            }
            _ => AffinityAssemblerAction::NotHandled,
        }
    }
}

impl Default for AffinityUpdateAssembler {
    fn default() -> Self {
        Self::new()
    }
}

struct AffinityState {
    ts: u64,
    id: ThreadId,
    expected_chunks: u32,
    received_chunks: u32,
    next_index: usize,
    mask: [u64; AFFINITY_MASK_WORDS],
}

impl AffinityState {
    fn new(event: &RawEvent, expected_chunks: u32) -> Self {
        Self {
            ts: event.ts,
            id: ThreadId::from(event.pid_tgid),
            expected_chunks,
            received_chunks: 0,
            next_index: 0,
            mask: [0; AFFINITY_MASK_WORDS],
        }
    }

    fn append_chunk(&mut self, chunk: &types::lime_affinity_update_chunk) {
        let bytes = chunk.chunk_len as usize;
        let words = bytes / std::mem::size_of::<u64>();
        let offset = self.next_index * AFFINITY_CHUNK_WORDS;
        for i in 0..words {
            self.mask[offset + i] = chunk.mask[i];
        }

        self.next_index += 1;
        self.received_chunks = self.received_chunks.saturating_add(1);
    }

    fn is_complete(&self) -> bool {
        self.expected_chunks == 0 || self.received_chunks >= self.expected_chunks
    }

    fn mask(&self) -> &[u64; AFFINITY_MASK_WORDS] {
        &self.mask
    }
}

impl TryFrom<AffinityState> for TraceEvent {
    type Error = ();

    fn try_from(state: AffinityState) -> Result<Self, Self::Error> {
        if !state.is_complete() {
            return Err(());
        }

        let cpumask = mask_to_cpuset(state.mask());
        Ok(TraceEvent {
            ts: state.ts,
            id: state.id,
            ev: EventData::AffinityUpdate { cpumask },
        })
    }
}

fn c_string_from_array(buf: &[i8]) -> Option<String> {
    let bytes: Vec<u8> = buf
        .iter()
        .map_while(|&c| if c == 0 { None } else { Some(c as u8) })
        .collect();
    if bytes.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }
}

fn mask_to_cpuset(mask: &[u64; AFFINITY_MASK_WORDS]) -> Option<Vec<u32>> {
    let mut cpu_ids = Vec::new();
    for (word_idx, &word) in mask.iter().enumerate() {
        if word == 0 {
            continue;
        }

        for bit_idx in 0..64 {
            if (word & (1u64 << bit_idx)) != 0 {
                cpu_ids.push((word_idx * 64 + bit_idx) as u32);
            }
        }
    }

    if cpu_ids.is_empty() {
        None
    } else {
        Some(cpu_ids)
    }
}
