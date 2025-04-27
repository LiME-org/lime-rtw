//! Trace recorder.

use std::{
    fs::File,
    io::{BufWriter, Write},
    str::FromStr,
};

use crate::{events::TraceEvent, proto};
use anyhow::Result;
use prost::Message;

/// Output format for trace events
#[derive(Debug, Clone, Copy, clap::ValueEnum, PartialEq, Eq)]
pub enum EventsFileFormat {
    Json,
    Protobuf,
}

impl EventsFileFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            EventsFileFormat::Json => "json",
            EventsFileFormat::Protobuf => "proto",
        }
    }
}

impl FromStr for EventsFileFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("json") {
            Ok(EventsFileFormat::Json)
        } else if s.eq_ignore_ascii_case("protobuf") {
            Ok(EventsFileFormat::Protobuf)
        } else {
            Err(format!("Unknown format: {}", s))
        }
    }
}

pub struct TraceEventWriter {
    file: BufWriter<File>,
    count: usize,
    format: EventsFileFormat,
}

impl TraceEventWriter {
    pub fn new(file: File, format: EventsFileFormat) -> Self {
        Self {
            file: std::io::BufWriter::with_capacity(64 * 1024, file),
            count: 0,
            format,
        }
    }

    pub fn write(&mut self, event: &TraceEvent) -> Result<()> {
        match self.format {
            EventsFileFormat::Json => self.write_json(event),
            EventsFileFormat::Protobuf => self.write_protobuf(event),
        }
    }

    #[inline]
    fn write_json(&mut self, event: &TraceEvent) -> Result<()> {
        if self.count == 0 {
            self.file.write_all(b"[\n    ")?;
        } else {
            self.file.write_all(b",\n    ")?;
        }

        serde_json::to_writer(&mut self.file, event)?;
        self.count += 1;
        Ok(())
    }

    #[inline]
    fn write_protobuf(&mut self, event: &TraceEvent) -> Result<()> {
        let proto_event = proto::TraceEvent::from(event);

        let mut buf = Vec::with_capacity(proto_event.encoded_len());
        proto_event.encode(&mut buf)?;

        let len = buf.len();
        let mut length_buf = Vec::with_capacity(10);
        prost::encoding::encode_varint(len as u64, &mut length_buf);

        self.file.write_all(&length_buf)?;
        self.file.write_all(&buf)?;

        self.count += 1;
        Ok(())
    }

    pub fn close(&mut self) -> Result<()> {
        match self.format {
            EventsFileFormat::Json => {
                self.file.write_all(b"\n]")?;
                self.file.flush()?;
            }
            EventsFileFormat::Protobuf => {
                self.file.flush()?;
            }
        }
        Ok(())
    }
}
