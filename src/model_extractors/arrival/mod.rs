//! Job arrival model extractors

use std::borrow::Cow;

use serde::Serialize;

pub mod periodic;
pub mod sporadic;
pub type ArrivalCurve = sporadic::Sporadic;

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

    /// Arrival curves with release intervals represented with distance functions.
    ArrivalCurveReleaseIntervals {
        /// Lower bound minimum distance function.
        dmins_lo: Cow<'a, [u64]>,
        /// Lower bound maximum distance function.
        dmaxs_lo: Cow<'a, [u64]>,
        /// Upper bound minimum distance function.
        dmins_hi: Cow<'a, [u64]>,
        /// Upper bound maximum distance function.
        dmaxs_hi: Cow<'a, [u64]>,
    },

    /// Sporadic arrivals.
    Sporadic {
        /// Minimal interarrival time. Guaranteed to be strictly positive.
        mit: u64,
    },

    /// Sporadic arrivals with release intervals.
    SporadicReleaseIntervals {
        /// Lower bound of minimal interarrival time.
        mit_lo: u64,
        /// Upper bound of minimal interarrival time.
        mit_hi: u64,
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
        /// Offset of the best candidate
        offset: u64,
        /// Max jitter of the best candidate
        max_jitter: u64,
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
    /// Periodic model with release intervals. Provides both possibly-fit and certainly-fit models.
    ReleaseIntervals {
        /// Certainly-fitting model period
        period_certainly: u64,
        /// Certainly-fitting model offset
        offset_certainly: u64,
        /// Certainly-fitting model max jitter
        max_jitter_certainly: u64,
        /// Possibly-fitting model period
        period_possibly: u64,
        /// Possibly-fitting model offset
        offset_possibly: u64,
        /// Possibly-fitting model max jitter
        max_jitter_possibly: u64,
    },
}

/// Arrival model extractor interface.
pub trait ArrivalModelExtractor {
    fn extract(&self) -> Option<ArrivalModel<'_>>;
}
