use std::{cmp::Ordering, collections::VecDeque};

use index_list::{IndexList, ListIndex};
use serde::Serialize;

use crate::{
    context::LimeContext,
    events::{EventData, TraceEvent},
};

#[derive(Clone, PartialEq, Eq, Debug)]
struct Demand {
    pub arrival: u64,
    pub work: u64,
}

struct DemandTracker {
    preempted: u64,
    was_preempted: bool,
    arrival: Option<u64>,
    last_switched_in: Option<u64>,
    last_preempted: Option<u64>,
    work: u64,
    sleeping: bool,
}

impl DemandTracker {
    pub fn new() -> Self {
        Self {
            was_preempted: false,
            preempted: 0,
            arrival: None,
            last_switched_in: None,
            last_preempted: None,
            work: 0,
            sleeping: false,
        }
    }

    fn handle_wake_up(&mut self, event: &TraceEvent) {
        if self.sleeping {
            self.arrival.replace(event.ts - self.preempted);
            self.last_preempted.replace(event.ts);
            self.work = 0;
            self.sleeping = false;
        }
    }

    fn handle_switched_in(&mut self, event: &TraceEvent) {
        self.last_switched_in.replace(event.ts);

        if self.arrival.is_none() {
            self.arrival.replace(event.ts - self.preempted);
        }

        if let Some(last_preempted) = self.last_preempted.take() {
            self.preempted += event.ts - last_preempted;
        }
    }

    fn handle_preemption(&mut self, event: &TraceEvent) {
        if self.arrival.is_none() {
            return;
        }

        self.was_preempted = true;
        self.last_preempted.replace(event.ts);

        if let Some(last_switched_in) = self.last_switched_in {
            self.work += event.ts - last_switched_in;
        }
    }

    fn handle_suspension(&mut self, event: &TraceEvent) -> Option<Demand> {
        self.sleeping = true;
        let arrival = self.arrival?;

        if let Some(last_switched_in) = self.last_switched_in {
            self.work += event.ts - last_switched_in;

            let demand = Demand {
                arrival,
                work: self.work,
            };

            return Some(demand);
        }
        None
    }

    pub fn update(&mut self, event: &TraceEvent) -> Option<Demand> {
        match event.ev {
            EventData::SchedSwitchedIn { .. } => {
                self.handle_switched_in(event);
                None
            }
            EventData::SchedSwitchedOut { .. } if !event.is_suspension() => {
                self.handle_preemption(event);
                None
            }
            EventData::SchedSwitchedOut { .. } if event.is_suspension() => {
                self.handle_suspension(event)
            }
            EventData::SchedWakeUp { .. } => {
                self.handle_wake_up(event);
                None
            }
            _ => None,
        }
    }
}

struct DemandAggregator {
    should_shift: bool,
    last_demand: Option<Demand>,
    min_work: Option<u64>,
    time_granularity_ns: Option<u64>,
}

impl DemandAggregator {
    pub fn new(min_work: Option<u64>, time_granularity_ns: Option<u64>) -> Self {
        Self {
            should_shift: true,
            last_demand: None,
            min_work,
            time_granularity_ns
        }
    }

    pub fn should_aggregate(&self, d1: &Demand, d2: &Demand) -> bool {
        let mut ret = d1.arrival == d2.arrival;

        if let Some(min_work) = self.min_work {
            ret = ret || ((d2.work + d1.work) < min_work)
        }

        ret
    }

    fn round_arrival(&self, arrival: u64) -> u64 {
        if let Some(t) = self.time_granularity_ns {
            return arrival.saturating_sub(arrival % t);
        }
        arrival
    }

    pub fn flush(&mut self) -> Option<Demand> {
        self.last_demand.take()
    }

    pub fn push(&mut self, demand: Demand) -> Option<Demand> {
        let arrival = self.round_arrival(demand.arrival);

        let demand = Demand {
            arrival,
            work: demand.work,
        };

        match self.last_demand.take() {
            Some(d) if self.should_aggregate(&d, &demand) =>{
                self.last_demand = Some(Demand {
                    arrival: d.arrival,
                    work: d.work + demand.work,
                });

                None
            },

            Some(d) => {
                self.last_demand = Some(demand);

                Some(d)
            },

            None => {
                self.last_demand = Some(demand); 
                None
            }
        }
    }
}

/// An RBF step.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub struct Step {
    pub delta: u64,
    pub work: u64,
}

impl PartialOrd for Step {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Step {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.delta.cmp(&other.delta) {
            Ordering::Equal => self.work.cmp(&other.work).reverse(),
            ord => ord,
        }
    }
}

enum InnerRbfExtractor<'a> {
    Exact(DynamicRbfExtractor),
    FixedSteps(FixedRbfExtractor<'a>),
}

impl<'a> InnerRbfExtractor<'a> {
    pub fn is_empty(&self) -> bool {
        match self {
            InnerRbfExtractor::Exact(t) => t.is_empty(),
            InnerRbfExtractor::FixedSteps(t) => t.is_empty(),
        }
    }

    #[inline]
    pub fn on_window_update(&mut self, new_window: &[Demand]) {
        match self {
            InnerRbfExtractor::Exact(t) => t.on_window_update(new_window),
            InnerRbfExtractor::FixedSteps(t) => t.on_window_update(new_window),
        }
    }

    pub fn trim_window(&mut self, window: &mut VecDeque<Demand>) {
        match self {
            InnerRbfExtractor::Exact(t) => t.trim_window(window),
            InnerRbfExtractor::FixedSteps(t) => t.trim_window(window),
        }
    }
}

const RBF_UPDATE_PERIOD: u64 = 10_000_000_000;
const RBF_UPDATE_BUDGET: usize = 10_000;

struct RbfUpdateRateMonitor {
    period: u64,
    budget: usize,
    count: usize,
    period_start: Option<u64>,
}

impl RbfUpdateRateMonitor {
    pub fn new(period: u64, budget: usize) -> Self {
        Self {
            period,
            budget,
            count: 0,
            period_start: None,
        }
    }

    /// Consume one unit of budget and returns a boolean indicating whether the
    /// budget is exceeded or not.
    pub fn update(&mut self, ts: u64) -> bool {
        if self.period_start.is_none() {
            self.count = 0;
            self.period_start.replace(ts);

            return false;
        }

        if let Some(period_start) = self.period_start {
            if (ts - period_start) >= self.period {
                self.count = 0;
                self.period_start.replace(ts);

                return false;
            }

            self.count += 1;

            if self.count >= self.budget {
                return true;
            }
        }

        false
    }
}

/// RBF extractor.
///
/// By default this extractor tries to build exact RBFs, i.e, with
/// variable-sized steps. If the update rate becomes too high, then it extracts
/// RBFs with predefined step-sized. This is less precise but much cheaper to
/// update. The switch is a destructive operation. Once done, there is no coming
/// back.
pub struct RbfExtractor<'a> {
    step_widths: &'a [u64],
    demand_tracker: DemandTracker,
    should_update: bool,
    window: VecDeque<Demand>,
    rbf_tracker: InnerRbfExtractor<'a>,
    limiter: RbfUpdateRateMonitor,
    demand_aggregator: DemandAggregator,
}

impl<'a> RbfExtractor<'a> {
    /// Build an extractor using the parameters stored in a Lime context.
    /// Newly constructed extractors extracts exact RBF.
    pub fn from_lime_context(ctx: &LimeContext) -> Self {
        let rbf = DynamicRbfExtractor::new(
            ctx.rbf_max_steps,
            match ctx.rbf_horizon {
                0 => None,
                h => Some(h),
            },
        );

        let should_update = !ctx.no_init_rbf_update;

        let rbf_tracker = InnerRbfExtractor::Exact(rbf);

        Self {
            step_widths: RBF_DEFAULT_STEP_WIDTHS,
            demand_tracker: DemandTracker::new(),
            should_update,
            window: VecDeque::new(),
            rbf_tracker,
            limiter: RbfUpdateRateMonitor::new(RBF_UPDATE_PERIOD, RBF_UPDATE_BUDGET),
            demand_aggregator: DemandAggregator::new(Some(100_000), Some(10_000_000)),
        }
    }

    #[allow(unused)]
    fn from_exact_rbf_extractor(
        e: DynamicRbfExtractor,
        should_update: bool,
        step_widths: &'a [u64],
    ) -> Self {
        Self {
            step_widths,
            demand_tracker: DemandTracker::new(),
            should_update,
            window: VecDeque::new(),
            rbf_tracker: InnerRbfExtractor::Exact(e),
            limiter: RbfUpdateRateMonitor::new(RBF_UPDATE_PERIOD, RBF_UPDATE_BUDGET),
            demand_aggregator: DemandAggregator::new(Some(100_000), Some(10_000_000)),
        }
    }

    #[allow(unused)]
    fn from_fixed_steps_rbf_extractor(e: FixedRbfExtractor<'a>, should_update: bool) -> Self {
        Self {
            step_widths: RBF_DEFAULT_STEP_WIDTHS,
            demand_tracker: DemandTracker::new(),
            should_update,
            window: VecDeque::new(),
            rbf_tracker: InnerRbfExtractor::FixedSteps(e),
            limiter: RbfUpdateRateMonitor::new(RBF_UPDATE_PERIOD, RBF_UPDATE_BUDGET),
            demand_aggregator: DemandAggregator::new(Some(100_000), Some(10_000_000)),
        }
    }

    fn switch_to_fixed_steps_extractor(&mut self) {
        if let InnerRbfExtractor::Exact(rbf) = &self.rbf_tracker {
            let w = self.window.make_contiguous();
            let new_rbf = FixedRbfExtractor::from_exact_rbf_extractor(rbf, self.step_widths, w);
            self.rbf_tracker = InnerRbfExtractor::FixedSteps(new_rbf);
        }
    }

    fn should_switch(&mut self, ts: u64) -> bool {
        if matches!(self.rbf_tracker, InnerRbfExtractor::Exact(_)) {
            return self.limiter.update(ts);
        }

        false
    }

    pub fn flush(&mut self) {
        if let Some(demand) = self.demand_aggregator.flush() {
            self.consume_demand(demand);
        }
    }

    /// Update the extractor internal state.
    pub fn update(&mut self, event: &TraceEvent) {
        if event.is_syscall_entry() {
            self.should_update = true;
        }

        if !self.should_update {
            return;
        }

        if let Some(demand) = self.demand_tracker.update(event) {
            if let Some(demand) = self.demand_aggregator.push(demand) {
                if self.should_switch(event.ts) {
                    self.switch_to_fixed_steps_extractor();
                }

                self.consume_demand(demand)
            } 
        }
    }

    /// Returns true if no demand has been detected so far.
    pub fn is_empty(&self) -> bool {
        self.rbf_tracker.is_empty()
    }

    #[allow(unused)]
    fn steps(&self) -> Vec<Step> {
        match &self.rbf_tracker {
            InnerRbfExtractor::Exact(t) => t.steps().copied().collect(),
            InnerRbfExtractor::FixedSteps(t) => t.steps().collect(),
        }
    }

    fn step_pairs(&self) -> Vec<(u64, u64)> {
        match &self.rbf_tracker {
            InnerRbfExtractor::Exact(t) => { 
                t.steps()
                 .zip(t.steps().skip(1))
                 .map(|(s1, s2)| (s1.delta, s2.work))
                 .collect()
            },
            InnerRbfExtractor::FixedSteps(t) => t.steps().map(|s| (s.delta, s.work)).collect(),
        }
    }

    fn consume_demand(&mut self, demand: Demand) {
        self.window.push_back(demand);
        let window = self.window.make_contiguous();

        self.rbf_tracker.on_window_update(window);
        self.rbf_tracker.trim_window(&mut self.window);
    }

    pub fn to_rbf(&self) -> Rbf {
        Rbf::new(self.step_pairs())
    }
}

/// An extracted RBF.
///
/// Can be queried and serialized.
#[derive(Default, Serialize)]
pub struct Rbf {
    steps: Vec<(u64, u64)>,
}

impl Rbf {
    pub fn new(steps: Vec<(u64, u64)>) -> Self {
        Self { steps }
    }

    /// Returns an upper bound of the processor demanded in a time interval of
    /// duration `delta`. Returns None if `delta` is not covered.
    pub fn request_bound(&self, delta: u64) -> Option<u64> {
        if delta == 0 {
            return Some(0);
        }

        if self.is_delta_covered(delta) {
            return self
                .steps
                .iter()
                .rev()
                .find(|step| delta >= step.0)
                .map(|found| found.1);
        }

        None
    }

    /// Returns the biggest delta covered by this RBF. Returns none if the RBF
    /// is empty.
    pub fn max_delta_covered(&self) -> Option<u64> {
        self.steps.last().map(|step| step.0)
    }

    /// Returns true if `delta` is covered by the RBF.
    ///
    /// ```
    /// use lime_rtw::model_extractors::rbf::Rbf;
    ///
    /// let rbf = Rbf::default();
    ///
    /// assert_eq!(rbf.is_delta_covered(42), rbf.request_bound(42).is_some());
    ///
    /// ```
    pub fn is_delta_covered(&self, delta: u64) -> bool {
        self.max_delta_covered()
            .filter(|max_delta| delta <= *max_delta)
            .is_some()
    }
}
// Fixed RBF extractor
const FIXED_RBF_TRIM_PERIOD: usize = 64;

const RBF_DEFAULT_STEP_WIDTHS: &[u64] = &[
    1_000_000,
    1_000_000,
    1_000_000,
    1_000_000,
    1_000_000,
    5_000_000, // 10 ms
    10_000_000,
    10_000_000,
    10_000_000,
    10_000_000,
    10_000_000,
    10_000_000,
    10_000_000,
    10_000_000,
    10_000_000, // 100 ms
    100_000_000,
    100_000_000,
    100_000_000,
    100_000_000,
    100_000_000,
    100_000_000,
    100_000_000,
    100_000_000,
    100_000_000, // 1 s
    500_000_000,
    500_000_000,
    500_000_000,
    500_000_000,
    500_000_000,
    500_000_000,
    500_000_000,
    500_000_000,
    500_000_000, // 5.5 s
    500_000_000,
    500_000_000,
    500_000_000,
    500_000_000,
    500_000_000,
    500_000_000,
    500_000_000,
    500_000_000,
    500_000_000, // 10.5 s
];
struct StepTracker {
    delta: u64,
    first: usize,
    curr_demand: u64,
    max_demand: u64,
}

impl StepTracker {
    pub fn new(delta: u64) -> Self {
        Self {
            delta,
            first: 0,
            curr_demand: 0,
            max_demand: 0,
        }
    }

    fn window_length(&self, window: &[Demand]) -> u64 {
        let first_demand = &window[self.first];
        let last_demand = window.last().unwrap();

        last_demand.arrival - first_demand.arrival + 1
    }

    fn is_overflowing(&self, window: &[Demand]) -> bool {
        let delta = self.window_length(window);

        delta >= self.delta
    }

    /// push a new demand and update the tracker state
    /// Returns a boolean indicating if the internal window overflowed.
    pub fn push_demand(&mut self, window: &[Demand]) -> bool {
        assert!(!window.is_empty());

        let last_demand = window.last().unwrap();
        self.curr_demand += last_demand.work;

        let mut overflow = false;

        while self.is_overflowing(window) {
            overflow = true;
            self.curr_demand -= &window[self.first].work;
            self.first += 1;
        }

        self.max_demand = self.max_demand.max(self.curr_demand);

        overflow
    }

    pub fn next_tracker(&self, window: &[Demand], next_width: u64) -> Self {
        let mut ret = Self {
            delta: self.delta + next_width,
            first: self.first,
            curr_demand: self.curr_demand,
            max_demand: self.max_demand,
        };

        while ret.first > 0 {
            ret.first -= 1;

            if ret.is_overflowing(window) {
                ret.first += 1;

                break;
            }

            ret.curr_demand += window[ret.first].work;
        }

        ret.max_demand = ret.max_demand.max(ret.curr_demand);

        ret
    }
}

pub struct Steps<'a> {
    pos: usize,
    last_delta: u64,
    trackers: &'a [StepTracker],
}

impl<'a> Iterator for Steps<'a> {
    type Item = Step;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.trackers.len() {
            return None;
        }

        let t = &self.trackers[self.pos];

        let ret = Step {
            delta: self.last_delta,
            work: t.max_demand,
        };

        self.last_delta = t.delta;
        self.pos += 1;

        Some(ret)
    }
}

impl<'a> Steps<'a> {
    fn new(trackers: &'a [StepTracker]) -> Self {
        Self {
            pos: 0,
            trackers,
            last_delta: 1,
        }
    }
}

pub struct FixedRbfExtractor<'a> {
    next_step_width: usize,
    step_widths: &'a [u64],
    trackers: Vec<StepTracker>,
    trim_count: usize,
}

impl<'a> FixedRbfExtractor<'a> {
    pub fn new(step_widths: &'a [u64]) -> Self {
        assert!(!step_widths.is_empty());

        Self {
            next_step_width: 0,
            step_widths,
            trackers: Vec::with_capacity(step_widths.len()),
            trim_count: 0,
        }
    }

    fn on_window_update(&mut self, window: &[Demand]) {
        if self.trackers.is_empty() {
            self.trackers
                .push(StepTracker::new(1 + self.step_widths[0]));
            self.next_step_width += 1;
        }

        let mut last_overflow = false;
        for tracker in self.trackers.iter_mut() {
            last_overflow = tracker.push_demand(window);
        }

        while last_overflow && (self.next_step_width < self.step_widths.len()) {
            let prev_tracker = self.trackers.last().unwrap();
            let next_width = self.step_widths[self.next_step_width];
            let new_tracker = prev_tracker.next_tracker(window, next_width);

            self.trackers.push(new_tracker);
            self.next_step_width += 1;
        }
    }

    pub fn steps(&'a self) -> Steps<'a> {
        Steps::new(self.trackers.as_slice())
    }

    pub fn is_empty(&self) -> bool {
        self.trackers.is_empty()
    }

    fn trim_window(&mut self, window: &mut VecDeque<Demand>) {
        if self.trackers.len() < self.step_widths.len() {
            return;
        }

        self.trim_count += 1;

        if self.trim_count < FIXED_RBF_TRIM_PERIOD {
            return;
        }

        self.trim_count = 0;

        let to_pop = self.trackers.last().unwrap().first;

        if to_pop == 0 {
            return;
        }

        for _ in 0..to_pop {
            window.pop_front();
        }

        for tracker in self.trackers.iter_mut() {
            tracker.first -= to_pop;
        }
    }

    fn from_exact_rbf_extractor(
        e: &DynamicRbfExtractor,
        step_widths: &'a [u64],
        window: &[Demand],
    ) -> Self {
        let mut ret = Self::new(step_widths);

        if window.is_empty() {
            return ret;
        }

        let mut delta = 1;
        let last_demand = window.last().unwrap();
        let delta_max = last_demand.arrival - window[0].arrival + 1;

        for w in step_widths.iter() {
            delta += w;
            if delta > delta_max {
                let tracker = StepTracker::new(delta);
                ret.trackers.push(tracker);

                break;
            }

            let tracker = StepTracker::new(delta);
            ret.trackers.push(tracker);
        }

        let mut work: u64 = window.iter().map(|d| d.work).sum();
        let mut i = 0;

        for tracker in ret.trackers.iter_mut().rev() {
            while i < window.len() {
                let delta = last_demand.arrival - window[i].arrival + 1;
                if delta >= tracker.delta {
                    work = work.saturating_sub(window[i].work);
                    i += 1;
                } else {
                    break;
                }
            }

            tracker.curr_demand = work;
            tracker.max_demand = work;
            tracker.first = i;
        }

        let mut j = 0;
        for step in e.steps() {
            while j < ret.trackers.len() {
                let delta = ret.trackers[j].delta - step_widths[j];

                if step.delta <= delta {
                    ret.trackers[j].max_demand = ret.trackers[j].max_demand.max(step.work);
                    j += 1;
                } else {
                    break;
                }
            }
        }

        ret
    }
}

// Dynamic RBF extraction

// The Min Sep compressor was used to avoid deltas to be small.
// It is not used right now because its usefulness is not clear.
#[allow(dead_code)]
struct MinSepCompressor {
    min_sep: u64,
}

#[allow(dead_code)]
impl MinSepCompressor {
    pub fn new(min_sep: u64) -> Self {
        Self { min_sep }
    }

    fn delete(step: &mut Step) {
        step.delta = 0;
        step.work = 0;
    }

    fn is_deleted(step: &Step) -> bool {
        step.delta == 0
    }

    fn merge_steps(steps: &mut [Step], l: usize, r: usize) {
        steps[l].work = steps[r].work;
        Self::delete(&mut steps[r]);
    }

    fn find_mid(steps: &[Step]) -> usize {
        assert!(!steps.is_empty());

        if steps.len() <= 2 {
            return steps.len() / 2;
        }

        let mut mid = steps.len() / 2;

        if (mid % 2) != 0 {
            mid += 1;
        }

        mid
    }

    fn merge(
        &self,
        steps: &mut [Step],
        mid: usize,
        deleted_left: usize,
        deleted_right: usize,
    ) -> usize {
        let mut deleted = deleted_left + deleted_right;

        let mut rh = mid;
        let mut lh = mid - deleted_left - 1;

        while rh < steps.len() {
            assert!(!Self::is_deleted(&steps[lh]));
            // every remaining element of the left slices are deleted
            if Self::is_deleted(&steps[rh]) {
                break;
            }

            if (steps[rh].delta - steps[lh].delta) < self.min_sep {
                Self::merge_steps(steps, lh, rh);
                rh += 1;
                deleted += 1;
            } else {
                steps.swap(lh + 1, rh);
                lh += 1;
                rh += 1;
            }

            if rh == (lh + 1) {
                break;
            }
        }

        deleted
    }

    fn do_compress(&self, steps: &mut [Step]) -> usize {
        if steps.len() < 2 {
            return 0;
        }

        let mid = Self::find_mid(steps);

        let deleted_left = self.do_compress(&mut steps[..mid]);
        let deleted_right = self.do_compress(&mut steps[mid..]);

        self.merge(steps, mid, deleted_left, deleted_right)
    }

    pub fn compress(&self, steps: &mut Vec<Step>) {
        let deleted = self.do_compress(steps.as_mut_slice());
        let new_len = steps.len() - deleted;
        steps.truncate(new_len);
    }
}

pub struct DynamicRbfExtractor {
    horizon: Option<u64>,
    max_steps: Option<usize>,
    steps: IndexList<Step>,
}

impl DynamicRbfExtractor {
    pub fn new(max_steps: usize, horizon: Option<u64>) -> Self {
        Self {
            horizon,
            max_steps: Some(max_steps),
            steps: IndexList::with_capacity(2 * max_steps),
        }
    }

    fn cost_li(&self, index: ListIndex) -> Option<u64> {
        let curr = self.steps.get(index)?;
        let prev = self.steps.get(self.steps.prev_index(index))?;

        let dist = curr.delta - prev.delta;
        let work = curr.work - prev.work;

        Some(dist.saturating_mul(work))
    }

    fn compress_1(&mut self) {
        let mut min_cost = u64::MAX;
        let mut min_cost_index = None;

        let mut pos = self.steps.next_index(self.steps.first_index());

        while pos.is_some() {
            let cost = self.cost_li(pos).unwrap();

            if cost <= min_cost {
                min_cost_index.replace(pos);
                min_cost = cost;
            }

            pos = self.steps.next_index(pos);
        }

        if let Some(min_idx) = min_cost_index {
            let s = unsafe { self.steps.get(min_idx).unwrap_unchecked() };
            let work = s.work;
            let prev_index = self.steps.prev_index(min_idx);
            self.steps.remove(min_idx);
            let s = unsafe { self.steps.get_mut(prev_index).unwrap_unchecked() };
            s.work = work;
        }
    }

    fn compress_res(&mut self, time_granularity: u64, work_granularity: u64) -> usize {
        let mut ret = 0;

        let mut pos = self.steps.first_index();

        while pos.is_some() {
            let next = self.steps.next_index(pos);
            if next.is_none() {
                break;
            }

            let d_pos = unsafe { self.steps.get(pos).unwrap_unchecked() };
            let d_next = unsafe { self.steps.get(next).unwrap_unchecked() };

            let d_next_work = d_next.work;
            let time_delta = d_next.delta - d_pos.delta;

            if (time_delta < time_granularity) || (work_granularity < work_granularity) {
                let d_pos = unsafe { self.steps.get_mut(pos).unwrap_unchecked() };
                d_pos.work = d_next_work;
                self.steps.remove(next);
                ret += 1;
            }

            pos = self.steps.next_index(pos);
        }

        ret
    }

    fn compress(&mut self) {
        if let Some(max_steps) = self.max_steps {
            let mut time_granularity = 15_000_000;
            let mut work_granularity = 100_000;

             while  self.steps.len() > max_steps {
                // self.compress_1();
                self.compress_res(time_granularity, work_granularity);
                time_granularity *= 2;
                work_granularity *= 2;
            }
        }
    }

    /// Returns true if no demand has been detected so far.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    fn insert_step(&mut self, index: ListIndex, delta: u64, work: u64) -> ListIndex {
        let mut pos = index;
        let mut same_delta = false;

        while pos.is_some() {
            let s = unsafe { self.steps.get_mut(pos).unwrap_unchecked() };

            if s.delta < delta {
                if s.work >= work {
                    return pos;
                }
            } else {
                if s.delta == delta {
                    same_delta = true;
                }

                break;
            }

            pos = self.steps.next_index(pos);
        }

        if pos.is_none() {
            self.steps.insert_last(Step { delta, work });

            return self.steps.last_index();
        }

        if same_delta {
            let s = unsafe { self.steps.get_mut(pos).unwrap_unchecked() };

            if s.work >= work {
                return pos;
            }

            s.work = work;
            pos = self.steps.next_index(pos);
        } else {
            self.steps.insert_before(pos, Step { delta, work });
        }

        while pos.is_some() {
            let s = unsafe { self.steps.get(pos).unwrap_unchecked() };

            if s.work > work {
                break;
            }

            let next = self.steps.next_index(pos);

            self.steps.remove(pos);

            pos = next;
        }

        pos
    }

    fn do_trim_steps(&mut self) {
        if let Some(horizon) = self.horizon {
            let mut pos = self.steps.last_index();

            while pos.is_some() {
                let s = unsafe { self.steps.get(pos).unwrap_unchecked() };

                if s.delta > horizon {
                    let prev = self.steps.prev_index(pos);
                    self.steps.remove(pos);
                    pos = prev;
                } else {
                    break;
                }
            }
        }
    }

    fn do_update(&mut self, window: &[Demand]) {
        if window.is_empty() {
            return;
        }

        let mut work = 0;

        let last_arrival = unsafe { window.last().unwrap_unchecked().arrival };

        let mut steps_idx = self.steps.first_index();

        for w in window.iter().rev() {
            let delta = last_arrival - w.arrival + 1;
            work += w.work;

            steps_idx = self.insert_step(steps_idx, delta, work);
        }
    }

    fn do_trim_window(window: &mut VecDeque<Demand>, horizon: u64, last_arrival: u64) {
        let mut to_pop = 0;

        for s in window.make_contiguous().iter() {
            if (last_arrival - s.arrival + 1) < horizon {
                break;
            }

            to_pop += 1;
        }

        for _ in 0..to_pop {
            window.pop_front();
        }
    }

    fn trim_window(&mut self, window: &mut VecDeque<Demand>) {
        if let (Some(h), Some(d)) = (self.horizon, window.back()) {
            Self::do_trim_window(window, h, d.arrival)
        }
    }

    fn on_window_update(&mut self, window: &[Demand]) {
        self.do_update(window);
        self.do_trim_steps();
        self.compress();
    }

    pub fn steps(&self) -> impl Iterator<Item = &Step> {
        self.steps.iter()
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        events::{EventData::*, TraceEvent},
        model_extractors::rbf_old::{
            DynamicRbfExtractor, FixedRbfExtractor, MinSepCompressor, RbfExtractor, Step,
        },
        utils::ThreadId,
    };

    use super::{Demand, DemandTracker};

    #[test]
    fn test_demand_tracker() {
        let mut tracker = DemandTracker::new();
        let id = ThreadId::new(0, 0);

        let e = TraceEvent {
            ts: 0,
            id,
            ev: SchedSwitchedOut {
                cpu: 0,
                prio: 0,
                state: 1,
            },
        };
        let ret = tracker.update(&e);
        assert_eq!(ret, None);

        let e = TraceEvent {
            ts: 5,
            id,
            ev: SchedWakeUp { cpu: 0 },
        };
        let ret = tracker.update(&e);
        assert_eq!(ret, None);

        let e = TraceEvent {
            ts: 10,
            id,
            ev: SchedSwitchedIn {
                cpu: 0,
                prio: 0,
                preempt: false,
            },
        };
        let ret = tracker.update(&e);
        assert_eq!(ret, None);

        let e = TraceEvent {
            ts: 20,
            id,
            ev: SchedSwitchedOut {
                cpu: 0,
                prio: 0,
                state: 0,
            },
        };
        let ret = tracker.update(&e);
        assert_eq!(ret, None);

        let e = TraceEvent {
            ts: 30,
            id,
            ev: SchedSwitchedIn {
                cpu: 0,
                prio: 0,
                preempt: false,
            },
        };
        let ret = tracker.update(&e);
        assert_eq!(ret, None);

        let e = TraceEvent {
            ts: 40,
            id,
            ev: SchedSwitchedOut {
                cpu: 0,
                prio: 0,
                state: 1,
            },
        };
        let ret = tracker.update(&e);
        assert_eq!(
            ret,
            Some(Demand {
                arrival: 5,
                work: 20
            })
        );
    }

    #[test]
    pub fn test_rbf_fixed_steps() {
        let step_widths = [10, 10, 30, 30];
        let rbf = FixedRbfExtractor::new(&step_widths);
        let mut rbf = RbfExtractor::from_fixed_steps_rbf_extractor(rbf, true);

        let demands = [
            Demand {
                arrival: 10,
                work: 10,
            },
            Demand {
                arrival: 20,
                work: 20,
            },
            Demand {
                arrival: 30,
                work: 30,
            },
            Demand {
                arrival: 60,
                work: 60,
            },
        ];

        for d in demands.iter() {
            rbf.consume_demand(d.clone());
        }

        let expected = vec![
            Step { delta: 1, work: 60 },
            Step {
                delta: 11,
                work: 60,
            },
            Step {
                delta: 21,
                work: 110,
            },
            Step {
                delta: 51,
                work: 120,
            },
        ];

        assert_eq!(expected, rbf.steps());
    }

    #[test]
    fn test_rbf_mode_switch() {
        let rbf = DynamicRbfExtractor::new(10, Some(100));
        let step_widths = [10, 10, 30, 30];
        let mut rbf = RbfExtractor::from_exact_rbf_extractor(rbf, true, &step_widths);

        let demands = [
            Demand {
                arrival: 10,
                work: 10,
            },
            Demand {
                arrival: 20,
                work: 20,
            },
            Demand {
                arrival: 30,
                work: 30,
            },
            Demand {
                arrival: 60,
                work: 60,
            },
        ];

        for d in demands.iter() {
            rbf.consume_demand(d.clone());
        }

        rbf.switch_to_fixed_steps_extractor();

        let expected = vec![
            Step { delta: 1, work: 60 },
            Step {
                delta: 11,
                work: 60,
            },
            Step {
                delta: 21,
                work: 110,
            },
            Step {
                delta: 51,
                work: 120,
            },
        ];

        assert_eq!(expected, rbf.steps());
    }

    #[test]
    fn test_rbf_internals() {
        let dynamic_extractor = DynamicRbfExtractor::new(3, None);
        let mut extractor = RbfExtractor::from_exact_rbf_extractor(dynamic_extractor, true, &[]);

        let demands = [
            Demand {
                arrival: 0,
                work: 10,
            },
            Demand {
                arrival: 20,
                work: 10,
            },
            Demand {
                arrival: 40,
                work: 10,
            },
            // Demand{arrival: 60, work: 10},
            // Demand{arrival: 75, work: 5}
            //  Demand{arrival: 90, work: 50};
        ];

        for d in demands.iter() {
            extractor.consume_demand(d.clone())
        }

        let expected = &[
            Step { delta: 1, work: 10 },
            Step {
                delta: 21,
                work: 20,
            },
            Step {
                delta: 41,
                work: 30,
            },
        ];

        assert_eq!(extractor.steps(), expected);

        extractor.consume_demand(Demand {
            arrival: 60,
            work: 10,
        });

        let expected = &[
            Step { delta: 1, work: 10 },
            Step {
                delta: 21,
                work: 20,
            },
            Step {
                delta: 41,
                work: 40,
            },
        ];

        extractor.to_rbf();
        assert_eq!(extractor.steps(), expected);

        let expected = &[
            Step { delta: 1, work: 10 },
            Step {
                delta: 16,
                work: 20,
            },
            Step {
                delta: 36,
                work: 45,
            },
        ];

        extractor.consume_demand(Demand {
            arrival: 75,
            work: 5,
        });
        assert_eq!(extractor.steps(), expected);

        let rbf = extractor.to_rbf();

        assert_eq!(rbf.max_delta_covered(), Some(36));

        assert_eq!(rbf.request_bound(0), Some(0));
        assert_eq!(rbf.request_bound(1), Some(10));
        assert_eq!(rbf.request_bound(14), Some(10));
        assert_eq!(rbf.request_bound(26), Some(20));
        assert_eq!(rbf.request_bound(36), Some(45));
        assert_eq!(rbf.request_bound(77), None);

        // assert_eq!(rbf.cum_work, 25);

        // assert_eq!(rbf.steps, &[(70, 40), (50, 30), (30, 20)]);

        let expected = &[
            Step { delta: 1, work: 50 },
            Step {
                delta: 16,
                work: 55,
            },
            Step {
                delta: 31,
                work: 95,
            },
        ];

        extractor.consume_demand(Demand {
            arrival: 90,
            work: 50,
        });
        assert_eq!(extractor.steps(), expected);

        let rbf = extractor.to_rbf();

        assert_eq!(rbf.max_delta_covered(), Some(31));

        assert_eq!(rbf.request_bound(0), Some(0));
        assert_eq!(rbf.request_bound(1), Some(50));
        assert_eq!(rbf.request_bound(25), Some(55));
        assert_eq!(rbf.request_bound(31), Some(95));
        assert_eq!(rbf.request_bound(72), None);

        extractor.to_rbf();
    }

    #[test]
    fn test_rbf_internal_horizon() {
        let rbf = DynamicRbfExtractor::new(3, Some(30));
        let mut rbf = RbfExtractor::from_exact_rbf_extractor(rbf, true, &[]);

        let demands = [
            Demand {
                arrival: 0,
                work: 10,
            },
            Demand {
                arrival: 20,
                work: 10,
            },
            Demand {
                arrival: 40,
                work: 10,
            },
            // Demand{arrival: 60, work: 10},
            // Demand{arrival: 75, work: 5}
            //  Demand{arrival: 90, work: 50};
        ];

        for d in demands.iter() {
            rbf.consume_demand(d.clone())
        }

        let expected = &[
            Step { delta: 1, work: 10 },
            Step {
                delta: 21,
                work: 20,
            },
        ];

        assert_eq!(rbf.steps(), expected);

        rbf.consume_demand(Demand {
            arrival: 60,
            work: 10,
        });

        let expected = &[
            Step { delta: 1, work: 10 },
            Step {
                delta: 21,
                work: 20,
            },
        ];

        assert_eq!(rbf.steps(), expected);
    }

    #[test]
    pub fn test_min_sep_compression() {
        let mut steps: Vec<Step> = Vec::new();
        let var_name = MinSepCompressor::new(2);
        let compressor = var_name;

        compressor.compress(&mut steps);
        assert_eq!(steps, vec![]);

        let mut steps = vec![Step { delta: 1, work: 1 }];
        let expected = steps.clone();

        compressor.compress(&mut steps);
        assert_eq!(steps, expected);

        let mut steps = vec![Step { delta: 1, work: 1 }, Step { delta: 2, work: 2 }];

        let expected = vec![Step { delta: 1, work: 2 }];

        compressor.compress(&mut steps);
        assert_eq!(steps, expected);

        let mut steps = vec![
            Step { delta: 1, work: 1 },
            Step { delta: 2, work: 2 },
            Step { delta: 3, work: 3 },
            Step { delta: 4, work: 4 },
            Step { delta: 5, work: 5 },
            Step { delta: 6, work: 6 },
        ];

        let expected = vec![
            Step { delta: 1, work: 2 },
            Step { delta: 3, work: 4 },
            Step { delta: 5, work: 6 },
        ];

        compressor.compress(&mut steps);
        assert_eq!(steps, expected);

        let mut steps = vec![
            Step { delta: 1, work: 1 },
            Step { delta: 2, work: 2 },
            Step { delta: 3, work: 3 },
            Step { delta: 4, work: 4 },
            Step { delta: 5, work: 5 },
            Step { delta: 6, work: 6 },
            Step { delta: 7, work: 7 },
        ];

        let expected = vec![
            Step { delta: 1, work: 2 },
            Step { delta: 3, work: 4 },
            Step { delta: 5, work: 6 },
            Step { delta: 7, work: 7 },
        ];

        compressor.compress(&mut steps);
        assert_eq!(steps, expected);
    }
}
