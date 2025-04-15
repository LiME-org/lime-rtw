use crate::{
    events::TraceEvent,
    job::Job,
    job_separator::{JobSeparation, JobSeparator, SignatureMatcher},
    utils::Dispatcher,
};

pub struct JobTracker {
    signature_matcher: SignatureMatcher,
    last_separation: Dispatcher<JobSeparator, Option<JobSeparation>>,
}

impl JobTracker {
    pub fn new() -> Self {
        Self {
            signature_matcher: SignatureMatcher::new(),
            last_separation: Dispatcher::new(),
        }
    }

    pub fn init(&mut self) {
        self.signature_matcher.init()
    }

    pub fn consume_event(
        &mut self,
        event: TraceEvent,
        detected_jobs: &mut Vec<(JobSeparator, Job)>,
    ) -> usize {
        let mut ret = 0;
        let mut separations = Vec::new();

        self.signature_matcher
            .consume_event(event, &mut separations);

        for (separator, separation) in separations.drain(..) {
            let mut s = separation;
            let last_separation = self.last_separation.get_or_default(&separator);

            if let Some(ref prev) = last_separation {
                if s.is_back_to_back() && s.curr_arrival.is_none() {
                    s.curr_release = prev.curr_release;
                }

                let job = prev.make_job(&s);
                detected_jobs.push((separator, job));
            }

            last_separation.replace(s);

            ret += 1
        }

        ret
    }

    pub fn job_separators(&self) -> impl Iterator<Item = &JobSeparator> {
        self.last_separation.keys()
    }

    pub fn clean(&mut self) {}

    pub fn pendings_first_cycle(&self) -> Vec<u64> {
        let mut ret = self
            .last_separation
            .values()
            .filter_map(|v| v.as_ref().map(|e| e.curr_first_cycle))
            .collect::<Vec<u64>>();

        ret.sort();

        ret
    }

    pub fn oldest_job_first_cycle(&self) -> u64 {
        self.last_separation
            .values()
            .filter_map(|v| v.as_ref().map(|e| e.curr_first_cycle))
            .min()
            .unwrap_or(u64::MAX)
    }

    pub fn ongoing_matching(&self) -> bool {
        self.signature_matcher.ongoing_matching()
    }
}

impl Default for JobTracker {
    fn default() -> Self {
        Self::new()
    }
}
