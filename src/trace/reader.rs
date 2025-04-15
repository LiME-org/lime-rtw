//! Recorded trace reader.

use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, Lines, Read},
    path::Path,
    sync::mpsc::Receiver,
};

use crate::{
    context::LimeContext,
    events::{EventData, TraceEvent},
    io::LimeOutputDirectory,
    proto,
    task::{TaskId, TaskInfos},
    trace::writer::EventsFileFormat,
    utils::ThreadId,
    EventProcessor, EventSource,
};

use anyhow::Result;
use prost::Message;

struct TraceEvents {
    lines: Lines<BufReader<File>>,
}

impl TraceEvents {
    #[inline]
    fn is_start_line(line: &str) -> bool {
        line.trim() == "["
    }

    #[inline]
    fn is_end_line(line: &str) -> bool {
        line.trim() == "]"
    }

    fn parse_line(maybe_line: Result<String>) -> Result<TraceEvent> {
        let line = maybe_line?;
        let start = line
            .find('{')
            .ok_or_else(|| anyhow::anyhow!("Record start ('{{') not found in line: {}", line))?;
        let end = line
            .rfind('}')
            .ok_or_else(|| anyhow::anyhow!("Record end ('}}') not found in line: {}", line))?
            + 1;
        let event: TraceEvent = serde_json::from_str(&line[start..end])?;
        Ok(event)
    }

    fn next_line(&mut self) -> Option<Result<String>> {
        let res_line = self.lines.next()?;
        match res_line {
            Ok(ref line) if Self::is_start_line(line) => self.next_line(),
            Ok(ref line) if Self::is_end_line(line) => None,
            _ => Some(res_line.map_err(anyhow::Error::from)),
        }
    }
}

impl Iterator for TraceEvents {
    type Item = Result<TraceEvent>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_line().map(Self::parse_line)
    }
}

impl From<File> for TraceEvents {
    fn from(file: File) -> Self {
        Self {
            lines: BufReader::new(file).lines(),
        }
    }
}

struct ProtobufTraceEvents {
    reader: BufReader<File>,
}

impl ProtobufTraceEvents {
    fn read_next_event(&mut self) -> Option<Result<TraceEvent>> {
        let mut length_buf = [0u8; 10];
        let mut length_pos = 0;

        loop {
            match self
                .reader
                .read_exact(&mut length_buf[length_pos..length_pos + 1])
            {
                Ok(_) => {
                    if length_buf[length_pos] & 0x80 == 0 {
                        break;
                    }
                    length_pos += 1;
                }
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return None,
                Err(e) => return Some(Err(e.into())),
            }
        }

        let mut length: u64 = 0;
        for (i, &byte) in length_buf[..=length_pos].iter().enumerate() {
            length |= ((byte & 0x7F) as u64) << (i * 7);
        }

        let mut buf = vec![0u8; length as usize];
        match self.reader.read_exact(&mut buf) {
            Ok(_) => match proto::TraceEvent::decode(buf.as_slice()) {
                Ok(proto_event) => Some(Ok(TraceEvent {
                    ts: proto_event.ts,
                    id: ThreadId::new(
                        proto_event.id.as_ref().map(|id| id.pid).unwrap_or_default(),
                        proto_event
                            .id
                            .as_ref()
                            .map(|id| id.tgid)
                            .unwrap_or_default(),
                    ),
                    ev: EventData::from(&proto_event.event.unwrap_or_default()),
                })),
                Err(e) => Some(Err(e.into())),
            },
            Err(e) => Some(Err(e.into())),
        }
    }
}

impl Iterator for ProtobufTraceEvents {
    type Item = Result<TraceEvent>;

    fn next(&mut self) -> Option<Self::Item> {
        self.read_next_event()
    }
}

pub struct TraceReader {
    output_dir: LimeOutputDirectory,
    task_infos: HashMap<TaskId, TaskInfos>,
    rx: Option<Receiver<(TaskId, TraceEvent)>>,
    event_reader: Option<std::thread::JoinHandle<Result<()>>>,
}

impl TraceReader {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            output_dir: LimeOutputDirectory::new(path.as_ref().to_string_lossy().to_string()),
            task_infos: HashMap::new(),
            rx: None,
            event_reader: None,
        }
    }

    pub fn read_thread_infos<P: AsRef<Path>>(path: P) -> Result<TaskInfos> {
        let file = File::open(path)?;
        let tinfo: TaskInfos = serde_json::from_reader(file)?;
        Ok(tinfo)
    }

    fn populate_thread_infos(&mut self) -> Result<()> {
        let task_ids = self
            .output_dir
            .task_ids()
            .map_err(|e| anyhow::anyhow!("Failed to get task IDs: {}", e))?;
        for task_id in task_ids {
            if let Ok(file) = self.output_dir.open_infos_file(&task_id) {
                if let Ok(infos) = serde_json::from_reader(file) {
                    self.task_infos.insert(task_id, infos);
                }
            }
        }
        Ok(())
    }

    pub fn start(mut self) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<(TaskId, TraceEvent)>();

        let output_dir = self.output_dir.clone();

        self.rx = Some(rx);

        self.event_reader = Some(std::thread::spawn(move || -> Result<()> {
            let task_ids = output_dir
                .task_ids()
                .map_err(|e| anyhow::anyhow!("Failed to retrieve task IDs: {}", e))?;
            for task_id in task_ids {
                let format = if output_dir
                    .get_events_file_path(&task_id, EventsFileFormat::Protobuf)
                    .exists()
                {
                    EventsFileFormat::Protobuf
                } else {
                    EventsFileFormat::Json
                };

                let file = output_dir.open_events_file(&task_id, format)?;
                match format {
                    EventsFileFormat::Json => {
                        let events = TraceEvents::from(file);
                        for event in events {
                            let e = event?;
                            tx.send((task_id, e))?;
                        }
                    }
                    EventsFileFormat::Protobuf => {
                        let reader = ProtobufTraceEvents {
                            reader: BufReader::new(file),
                        };
                        for event in reader {
                            let e = event?;
                            tx.send((task_id, e))?;
                        }
                    }
                }
            }
            Ok(())
        }));

        self
    }
}

impl EventSource for TraceReader {
    fn event_loop<E: EventProcessor>(
        &mut self,
        processor: &mut E,
        ctx: &LimeContext,
    ) -> Result<()> {
        self.populate_thread_infos()?;

        let rx = self.rx.as_mut().unwrap();
        for (task_id, event) in rx.iter() {
            processor.consume_event(&task_id, event, ctx);
        }

        let handle = self.event_reader.take().unwrap();
        handle.join().unwrap()
    }

    fn get_task_info(&self, task_id: TaskId) -> Option<TaskInfos> {
        self.task_infos.get(&task_id).cloned()
    }
}
