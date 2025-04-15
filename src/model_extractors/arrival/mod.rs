//! Job arrival model extractors

use std::borrow::Cow;

use serde::Serialize;

mod arrival_curve;
pub mod periodic;
mod sporadic;

pub type ArrivalCurve = arrival_curve::ArrivalCurve;
pub type Sporadic = sporadic::Sporadic;

/// Arrival model extractors output.
///
/// Please update this enum when you add new arrival models.
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case", tag = "model")]
pub enum ArrivalModel<'a> {
    /// Arrival curves represented with distance functions.
    ArrivalCurve {
        /// Minimum distance function. dmins\[i\] gives the smallest time interval observed between (i+2) events.
        dmins: Cow<'a, [u64]>,
        /// Maximum distance function. dmaxs\[i\] gives the greatest time interval observed between (i+2) events.
        dmaxs: Cow<'a, [u64]>,
    },

    /// Sporadic arrivals.
    Sporadic {
        /// Minimal interarrival time. Guaranteed to be stricly positive.
        mit: u64,
    },

    /// Periodic arrivals.
    Periodic(Periodic),
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case", tag = "on")]
pub enum Periodic {
    /// Periodic model with releases only. Fails for bursty arrivals.
    Release {
        /// Period of the best candidate.w
        period: u64,
        /// Frequency of the best candidate
        frequency: f64,
        /// Offset of the best candidate
        offset: u64,
        /// Max jitter of the best candidate
        max_jitter: u64,
        /// Mean of all estimated periods, over all batches
        mean: u64,
        /// Variance of all estimated periods, over all batches
        variance: u64,
    },
    /// Periodic model extracted from arrival times
    Arrival {
        /// Detected period.
        period: u64,

        /// Biggest measured jitter.
        max_jitter: u64,

        /// Detected offset.
        offset: u64,
    },
}

/// Arrival model extractor interface.
pub trait ArrivalModelExtractor {
    fn extract(&self) -> Option<ArrivalModel<'_>>;
}
