//! Event processors.
//!
//! This module contains the front-end of LiME's event processors. Each of these
//! processors are invoked by a different CLI subcommand.

pub mod extract_job;
pub mod extract_model;
#[cfg(target_os = "linux")]
pub mod write_trace;
