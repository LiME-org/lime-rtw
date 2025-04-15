//! Model extraction output data definitions.
//!
//!

use std::collections::BTreeMap;

use serde::Serialize;

use crate::job_separator::JobSeparator;

use super::{arrival::ArrivalModel, rbf::Rbf, JobSeparatorExtractors, ThreadTaskModelExtractor};

/// Job-separator-specific extracted models.
#[derive(Serialize)]
pub struct JobSeparatorModels<'a> {
    separator: &'a JobSeparator,

    /// Number of jobs observed for this separator
    count: usize,

    /// WCET(n) for this separator. `wcet[n-1]` gives WCET(n).
    wcet_n: Vec<u64>,

    /// Dynamic self suspension bound
    dynamic_self_suspension: u64,

    /// Dictionary of segmented self-suspension bounds.
    /// The key is the number `m` of self-suspension segments.
    /// The value is a vector representing the sequence of segment bounds `C_1, S_1, ..., S_m, C_m+1`.
    /// The value of at index 2 * i is the i-th computation segment bound.
    /// The value at index (2 * i + 1) is the i-th suspension bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    segmented_self_suspensions: Option<BTreeMap<usize, Vec<(u64, u64)>>>,

    /// Job arrivals models extracted for this separator.
    arrival_models: Vec<ArrivalModel<'a>>,
}

impl<'a> JobSeparatorModels<'a> {
    fn new(separator: &'a JobSeparator, extractors: &'a JobSeparatorExtractors) -> Self {
        Self {
            separator,
            count: extractors.count.get(),
            wcet_n: extractors.wcet.wcets().collect(),
            dynamic_self_suspension: extractors.suspension.get(),
            segmented_self_suspensions: extractors.bsss.extract(),
            // segmented_self_suspension: e.segmented_self_suspension.get().map(|it| it.collect()),
            arrival_models: extractors.arrival_models(),
        }
    }
}

#[derive(Serialize)]
pub struct ThreadTaskModelReport<'a> {
    /// RBF extracted for this thread.
    rbf: Rbf,

    /// Request per second for this thread.
    rps: f64,

    /// Job-separator-specific models
    by_job_separator: Vec<JobSeparatorModels<'a>>,
}

impl<'a> ThreadTaskModelReport<'a> {
    pub fn new() -> Self {
        Self {
            rbf: Rbf::default(),
            rps: 0.0,
            by_job_separator: Vec::new(),
        }
    }

    pub fn from_extractor(extractor: &'a ThreadTaskModelExtractor) -> Self {
        let mut ret = Self::new();

        ret.rbf = extractor.rbf.to_rbf();

        ret.rps = extractor.event_rate.request_per_second();

        for (sep, ext) in extractor.job_separators_extractors.items() {
            let m = JobSeparatorModels::new(sep, ext);

            ret.by_job_separator.push(m);
        }

        ret
    }
}

impl Default for ThreadTaskModelReport<'_> {
    fn default() -> Self {
        Self::new()
    }
}
