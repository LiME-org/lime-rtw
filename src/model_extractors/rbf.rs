use std::{cmp::Ordering, collections::VecDeque, marker::PhantomData};

use index_list::{IndexList, ListIndex};
use serde::Serialize;

use crate::{
    context::LimeContext,
    events::{EventData, TraceEvent},
};

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

#[derive(Clone, PartialEq, Eq, Debug)]
struct Demand {
    pub arrival: u64,
    pub work: u64,
}

impl Demand {
    pub fn new(arrival: u64, work: u64) -> Self {
        Self { arrival, work }
    }

    pub fn new_rounded(arrival: u64, work: u64, time_granularity_ns: u64) -> Self {
        let rounded_arrival = arrival - (arrival % time_granularity_ns);

        Self::new(rounded_arrival, work)
    }
}

struct DemandWindow {
    demand_tracker: DemandTracker,
    aggregator: DemandAggregator,
    demands: VecDeque<Demand>,
    horizon: Option<u64>,
    max_length: usize,
}

impl DemandWindow {
    pub fn new(horizon: Option<u64>, max_length: usize) -> Self {
        Self {
            demand_tracker: DemandTracker::new(),
            demands: VecDeque::new(),
            aggregator: DemandAggregator::new(None),
            horizon,
            max_length,
        }
    }

    pub fn should_increase_granularity(&mut self) -> bool {
        self.demands.len() > self.max_length && self.aggregator.get_time_granularity_ns().is_none()
    }

    pub fn increase_granularity(&mut self) {
        if let Some(d) = self.aggregator.flush() {
            self.push_aggregated_demand(d);
        }

        // let new_granularity = match self.aggregator.get_time_granularity_ns() {
        //     Some(g) => g + Self::MIN_RES,
        //     None => Self::MIN_RES,
        // };

        let new_granularity = self.horizon.unwrap_or(10_000_000_000) / self.max_length as u64;

        self.aggregator.set_time_granularity_ns(new_granularity);

        let mut new_demands = VecDeque::new();

        for d in self.demands.drain(..) {
            if let Some(agg) = self.aggregator.push(d) {
                new_demands.push_back(agg);
            }
        }

        self.demands = new_demands;
    }

    fn push_aggregated_demand(&mut self, demand: Demand) {
        self.demands.push_back(demand);
    }

    fn do_update(&mut self, event: &TraceEvent) -> bool {
        if let Some(raw_demand) = self.demand_tracker.update(event) {
            if let Some(agg_demand) = self.aggregator.push(raw_demand) {
                self.push_aggregated_demand(agg_demand);
                return true;
            }
        }

        false
    }

    pub fn update(&mut self, event: &TraceEvent) -> bool {
        self.do_update(event)
    }

    pub fn trim(&mut self) {
        if let (Some(h), Some(d)) = (self.horizon, self.demands.back()) {
            const MIN_WINDOW_SIZE: usize = 3;
            let last_arrival = d.arrival;
            let mut to_pop = 0;

            let mut elem_nb = self.demands.len();

            for s in self.demands.make_contiguous().iter() {
                if elem_nb <= MIN_WINDOW_SIZE {
                    break;
                }

                if (last_arrival - s.arrival) < h {
                    break;
                }

                elem_nb -= 1;
                to_pop += 1;
            }

            for _ in 0..to_pop {
                self.demands.pop_front();
            }
        }
    }

    pub fn flush(&mut self) -> bool {
        if let Some(d) = self.aggregator.flush() {
            self.push_aggregated_demand(d);
            return true;
        }

        false
    }

    pub fn is_aggregated(&self) -> bool {
        self.aggregator.is_active()
    }

    pub fn demands(&mut self) -> &[Demand] {
        self.demands.make_contiguous()
    }
}

struct DemandAggregator {
    last_demand: Option<Demand>,
    time_granularity_ns: Option<u64>,
}

impl DemandAggregator {
    pub fn new(time_granularity_ns: Option<u64>) -> Self {
        Self {
            last_demand: None,
            time_granularity_ns,
        }
    }

    fn get_time_granularity_ns(&self) -> Option<u64> {
        self.time_granularity_ns
    }

    fn is_active(&self) -> bool {
        self.time_granularity_ns.is_some()
    }

    fn set_time_granularity_ns(&mut self, val: u64) {
        self.time_granularity_ns = Some(val);
    }

    fn should_aggregate(&self, d1: &Demand, d2: &Demand) -> bool {
        d1.arrival == d2.arrival
    }

    pub fn flush(&mut self) -> Option<Demand> {
        self.last_demand.take()
    }

    pub fn push(&mut self, demand: Demand) -> Option<Demand> {
        let demand = match self.time_granularity_ns {
            Some(g) => Demand::new_rounded(demand.arrival, demand.work, g),
            None => demand,
        };

        match self.last_demand.take() {
            Some(d) if self.should_aggregate(&d, &demand) => {
                self.last_demand = Some(Demand {
                    arrival: d.arrival,
                    work: d.work + demand.work,
                });
                None
            }

            Some(d) => {
                self.last_demand = Some(demand);
                Some(d)
            }

            None => {
                self.last_demand = Some(demand);
                None
            }
        }
    }
}

struct DemandTracker {
    preempted: u64,
    arrival: Option<u64>,
    last_switched_in: Option<u64>,
    last_preempted: Option<u64>,
    work: u64,
    sleeping: bool,
}

impl DemandTracker {
    pub fn new() -> Self {
        Self {
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

struct RBFSteps {
    steps: IndexList<Step>,
}

impl RBFSteps {
    pub fn new() -> Self {
        Self {
            steps: IndexList::new(),
        }
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
            return self.steps.insert_last(Step { delta, work });
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

    fn coarsen(&mut self, time_granularity: u64) -> usize {
        let ret = 0;

        let mut new_steps = IndexList::new();

        let mut current_delta = 1;
        let mut work = 0;
        let mut merging = false;

        for step in self.steps.drain_iter() {
            let mut s_delta = step.delta - 1;
            s_delta -= s_delta % time_granularity;
            s_delta += 1;

            if s_delta == current_delta {
                merging = true;
                work = step.work;
            } else {
                merging = false;
                new_steps.insert_last(Step {
                    delta: current_delta,
                    work,
                });

                current_delta = s_delta;
                work = step.work;
            }
        }

        if merging {
            new_steps.insert_last(Step {
                delta: current_delta,
                work,
            });
        }

        self.steps = new_steps;

        ret
    }

    pub fn update(&mut self, window: &[Demand]) {
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

    fn step_pair_aggregated(&self, ret: &mut Vec<(u64, u64)>, horizon: Option<u64>) {
        if self.steps.len() < 2 {
            return;
        }

        let h = horizon.unwrap_or(u64::MAX);
        let mut last_work = 0;

        for (s1, s2) in self.steps.iter().zip(self.steps.iter().skip(1)) {
            if s1.delta > h {
                ret.push((h, last_work));
                return;
            }

            last_work = s2.work;

            ret.push((s1.delta, s2.work));
        }
    }

    fn step_pair_exact(&self, ret: &mut Vec<(u64, u64)>, horizon: Option<u64>) {
        let h = horizon.unwrap_or(u64::MAX);

        let mut last_work = 0;

        for s in self.steps.iter() {
            if s.delta > h {
                ret.push((h, last_work));
                return;
            }

            last_work = s.work;

            ret.push((s.delta, s.work));
        }

        if ret.len() == 1 {
            ret.push((1 + ret[0].1, ret[0].1));
        }
    }

    pub fn step_pairs(&self, ret: &mut Vec<(u64, u64)>, shifted: bool, horizon: Option<u64>) {
        if shifted {
            return self.step_pair_aggregated(ret, horizon);
        }

        self.step_pair_exact(ret, horizon)
    }

    fn do_trim_steps(&mut self, horizon: Option<u64>) {
        if let Some(horizon) = horizon {
            let mut to_pop: usize = 0;

            for step in self.steps.iter().rev() {
                if step.delta < horizon {
                    break;
                }

                to_pop += 1;
            }

            for _ in 0..to_pop {
                self.steps.remove_last();
            }
        }
    }
}

pub struct RbfExtractor<'a> {
    demand_window: DemandWindow,
    horizon: Option<u64>,
    rbf_steps: RBFSteps,
    phantom: PhantomData<&'a ()>,
    enabled: bool,
}

impl RbfExtractor<'_> {
    pub fn new(horizon: Option<u64>, max_steps: usize, enabled: bool) -> Self {
        Self {
            demand_window: DemandWindow::new(horizon, max_steps),
            horizon,
            rbf_steps: RBFSteps::new(),
            phantom: PhantomData,
            enabled,
        }
    }

    pub fn from_lime_context(ctx: &LimeContext) -> Self {
        let horizon = match ctx.rbf_horizon {
            0 => None,
            h => Some(h),
        };

        let enabled = ctx.enable_rbf;
        let max_steps = ctx.rbf_max_steps;

        Self::new(horizon, max_steps, enabled)
    }

    fn update_steps(&mut self) {
        let demands = self.demand_window.demands();
        self.rbf_steps.update(demands);
    }

    pub fn update(&mut self, event: &TraceEvent) {
        if !self.enabled {
            return;
        }

        let did_change = self.demand_window.update(event);

        if did_change {
            self.update_steps();

            let change_granularity = self.demand_window.should_increase_granularity();
            if change_granularity {
                self.demand_window.increase_granularity();
                self.compress();
                self.update_steps();
            }

            self.demand_window.trim();
            self.rbf_steps.do_trim_steps(self.horizon);
        }
    }

    pub fn flush(&mut self) {
        if !self.enabled {
            return;
        }

        let did_change = self.demand_window.flush();
        if did_change {
            self.update_steps();
        }
    }

    pub fn is_empty(&self) -> bool {
        !self.enabled || self.rbf_steps.steps.is_empty()
    }

    pub fn to_rbf(&self) -> Rbf {
        if !self.enabled {
            Rbf::default()
        } else {
            Rbf::new(self.rbf_step_pairs())
        }
    }

    pub fn rbf_step_pairs(&self) -> Vec<(u64, u64)> {
        let mut ret = Vec::with_capacity(self.rbf_steps.steps.len());

        self.rbf_steps
            .step_pairs(&mut ret, self.demand_window.is_aggregated(), self.horizon);

        ret
    }

    fn compress(&mut self) {
        if let Some(g) = self.demand_window.aggregator.get_time_granularity_ns() {
            self.rbf_steps.coarsen(g);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::events::TraceEvent;

    use super::{Demand, DemandAggregator, DemandTracker};

    const EXAMPLE_1: &str = r#"[
    {"ts":10000000,"pid":98218,"event":"sched_wake_up","cpu":0},
    {"ts":20000000,"pid":98218,"event":"sched_switched_in","cpu":0,"prio":99,"preempt":false},
    {"ts":30000000,"pid":98218,"event":"sched_switched_out","cpu":0,"prio":99,"state":1},
    {"ts":39000000,"pid":98218,"event":"sched_waking","cpu":0},
    {"ts":40000000,"pid":98218,"event":"sched_wake_up","cpu":1},
    {"ts":40500000,"pid":98218,"event":"sched_switched_in","cpu":1,"prio":99,"preempt":true},
    {"ts":50500000,"pid":98218,"event":"sched_switched_out","cpu":1,"prio":99,"state":0},
    {"ts":58000000,"pid":98218,"event":"sched_waking","cpu":0},
    {"ts":58500000,"pid":98218,"event":"sched_wake_up","cpu":1},
    {"ts":60000000,"pid":98218,"event":"sched_switched_in","cpu":1,"prio":99,"preempt":true},
    {"ts":65000000,"pid":98218,"event":"sched_switched_out","cpu":0,"prio":99,"state":1}]
    "#;

    #[test]
    fn test_demand_tracker() {
        let values: Vec<TraceEvent> = serde_json::from_str(EXAMPLE_1).unwrap();
        let mut tracker = DemandTracker::new();

        let ret = values
            .iter()
            .map(|e| tracker.update(e))
            .collect::<Vec<Option<Demand>>>();

        let expected = [
            None,
            None,
            Some(Demand::new(20_000_000, 10_000_000)),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(Demand::new(40_000_000, 15_000_000)),
        ];

        assert_eq!(ret, &expected);
    }

    #[test]
    fn test_demand_aggregator() {
        let demands = &[
            Demand::new(10, 10),
            Demand::new(20, 10),
            Demand::new(40, 10),
            Demand::new(50, 10),
            Demand::new(60, 10),
            Demand::new(80, 10),
        ];

        let mut agg = DemandAggregator::new(None);
        let mut ret = Vec::new();

        for demand in demands.iter().cloned() {
            if let Some(d) = agg.push(demand) {
                ret.push(d);
            }
        }
        if let Some(d) = agg.flush() {
            ret.push(d);
        }

        assert_eq!(demands, ret.as_slice());

        ret.clear();
        agg.set_time_granularity_ns(50);

        for demand in demands.iter().cloned() {
            if let Some(d) = agg.push(demand) {
                ret.push(d);
            }
        }
        if let Some(d) = agg.flush() {
            ret.push(d);
        }

        let expected = &[Demand::new(0, 30), Demand::new(50, 30)];

        assert_eq!(ret, expected);
    }
}
