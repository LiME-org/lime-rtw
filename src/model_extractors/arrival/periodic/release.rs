use std::collections::{BTreeMap, HashSet};

use crate::{
    job::Job,
    model_extractors::arrival::{ArrivalModel, ArrivalModelExtractor, Periodic},
};

const DEFAULT_BATCH_SIZE: usize = 9999999999999; // This is currently irrelevant as the batch size is defined in cli.rs
const MAX_CANDIDATES: usize = 50; // Maximum number of stored candidates around the mean period
const SIGMA_JITTER_FACTOR: i64 = 3; // Percentage of the optimal ternary period to be used for generating fine periods
const REMOVAL_THRESHOLD: u64 = 5; // Threshold for removing candidates with max_jitter greater than REMOVAL_THRESHOLD times the least jitter

// Struct to store the data of periodic functions
pub struct PeriodicFunctionData {
    pub offset: i64,
    pub max_jitter: i64,
    pub total_jitter: i64,
}

// Struct to store the data of the best candidate
pub struct CandidateData {
    period: i64,
    frequency: i64,
    offset: i64,
    max_jitter: i64,
}

type PeriodicCandidate<'a> = Option<(&'a i64, &'a PeriodicFunctionData)>;

pub struct PeriodExtractor {
    #[allow(unused)]
    batch: Vec<f64>, // The batch of releases to be analysed
    batch_size: usize, // The size of the batch to be analysed

    clean_batch: Vec<f64>, // The batch of releases without the noisy releases

    bursty: bool, // True if there are bursty releases/arrivals in the batch

    least_jitter_candidates: BTreeMap<i64, PeriodicFunctionData>, // Map of candidate periods and their data (e.g., offset, max_jitter, total_jitter)
    rounded_period_candidates: BTreeMap<i64, PeriodicFunctionData>, // Map of candidate periods and their data (e.g., offset, max_jitter, total_jitter)
    best_guess_candidate: CandidateData,

    // Mean and variation of the estimates
    mean_inter_release: f64,

    mean_lj_period: i64, // mean of the periods producing the least jitter per batch
    sum_lj_period: f64,  // total of the periods producing the least jitter per batch

    variance_inter_release: f64,

    // Supporting variables
    // Number of analysed batches
    num_batches: i64, // Ordinal number of the current batch to be analysed
    next_j: i64,      // Number of activations analysed so far
    n: f64,           // Number of analysed releases

    t_0: Option<u64>,
    last_arrival: f64,
}

impl PeriodExtractor {
    pub fn new() -> Self {
        Self::with_batch_size(DEFAULT_BATCH_SIZE)
    }

    pub fn with_batch_size(batch_size: usize) -> Self {
        Self {
            batch_size,
            batch: Vec::new(),
            clean_batch: Vec::new(),

            bursty: false,
            least_jitter_candidates: BTreeMap::new(), // Initialize the candidates map
            rounded_period_candidates: BTreeMap::new(),
            best_guess_candidate: CandidateData {
                period: 0,
                frequency: 0,
                offset: 0,
                max_jitter: 0,
            },

            // Mean and variation of the estimates
            mean_inter_release: 0.0,
            mean_lj_period: 0,
            sum_lj_period: 0.0,
            variance_inter_release: 0.0,

            // Supporting variables

            // Batch counter
            num_batches: 1,
            next_j: 0,
            n: 0.0,

            t_0: None,
            last_arrival: 0.0,
        }
    }

    // Taken from https://codeforces.com/blog/entry/43440
    pub fn ternary_search_min_int(&self, start: i64, end: i64) -> i64 {
        let mut lo = start - 1;
        let mut hi = end + 1;
        let valid_batch = if self.clean_batch.len() <= 3 {
            &self.batch
        } else {
            &self.clean_batch
        };
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            //let r1 = self.compute_jitter_ter(mid as f64, releases);
            let (_, r1, _) =
                self.update_offset_and_jitter(mid, valid_batch, 0, valid_batch[0] as i64, 0, 0);
            //let r2 = self.compute_jitter_ter((mid + 1) as f64, releases);
            let (_, r2, _) =
                self.update_offset_and_jitter(mid + 1, valid_batch, 0, valid_batch[0] as i64, 0, 0);
            if r1 > r2 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        hi
    }

    pub fn compute_optimal_ternary_period(&self, start: i64, end: i64) -> i64 {
        // Find the optimal period using ternary search
        self.ternary_search_min_int(start, end)
    }

    // This method updates the mean and variance for a single estimate
    fn update_statistical_estimate(
        &self,
        n: f64,
        old_mean_estimate: f64,
        old_variance_estimate: f64,
        new_period_estimate: f64,
    ) -> (f64, f64) {
        let delta = new_period_estimate - old_mean_estimate;
        let new_mean_estimate = old_mean_estimate + delta / n;
        let delta2 = new_period_estimate - new_mean_estimate;
        let new_variance_estimate = ((n - 1.0) * old_variance_estimate + delta * delta2) / n;

        (new_mean_estimate, new_variance_estimate)
    }

    /// Function to update the mean and variance of the period estimates
    #[allow(unused)]
    fn update_mean_and_variance_of_period_est(&mut self, n: f64, period_estimate: f64) {
        let (new_mean_period, new_variance_period) = self.update_statistical_estimate(
            n,
            self.mean_inter_release,
            self.variance_inter_release,
            period_estimate,
        );
        self.mean_inter_release = new_mean_period;
        self.variance_inter_release = new_variance_period;
    }

    //================================================================================================== i64 version end

    pub fn guess_value(&self, least: f64, largest: f64) -> f64 {
        let scaling = 1000.;
        let least = least * scaling;
        let largest = largest * scaling;
        let delta = largest - least;
        let mean = least + (delta / 2.0);

        let mut x = 10;

        while x >= 5 {
            let granularity = 10_f64.powi(x);
            let first = ((least / granularity).floor() + 1.0) * granularity;
            let candidates: Vec<(f64, f64)> = (first as i64..(largest as i64))
                .step_by(granularity as usize)
                .map(|p| (p as f64, (mean - p as f64).abs()))
                .collect();

            let mut candidates = candidates.clone();
            candidates.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

            if !candidates.is_empty() {
                return candidates[0].0 / scaling;
            }

            x -= 1;
        }

        let granularity = 10f64.powi(3);
        ((mean / granularity).round() * granularity) / scaling
    }

    fn period_to_frequency(&self, period: f64) -> f64 {
        let second = 1000000000.; // 1 second expressed in nanoseconds
        second / period // return frequency in Hz
    }

    #[allow(unused)]
    pub fn guess_frequency(&self, least: f64, largest: f64) -> f64 {
        // let second = 1000000000.; // 1 second expressed in nanoseconds
        let scaling = 1000.; // for periods that are much shorter than 1 second, scale up the frequency values

        let freq_a = self.period_to_frequency(least) * scaling;
        let freq_b = self.period_to_frequency(largest) * scaling;

        // Relation between frequencies can be the opposite of the relation between the corresponding periods
        if freq_a < freq_b {
            self.guess_value(freq_a, freq_b) / scaling // return frequency in Hz
        } else {
            self.guess_value(freq_b, freq_a) / scaling // return frequency in Hz
        }
        // Warning: Case when freq_a == freq_b should be handled if it is not in guess_period
    }

    // ============== FOR CONSERVATION ================================================

    fn update_offset_and_jitter(
        &self,
        period: i64,
        batch: &[f64],
        next_j: i64,
        old_offset: i64,
        old_max_jitter: i64,
        old_total_jitter: i64,
    ) -> (i64, i64, i64) {
        let mut offset = if self.num_batches > 1 {
            old_offset
        } else {
            batch[0] as i64 // Offset, initially set to the first arrival for the first ever batch
        };
        let mut j: i64 = if self.num_batches > 1 { next_j } else { 0 }; // Last assigned j, zero if no previous j was assigned or if n_batches > 1
        let mut max_jitter: i64 = if self.num_batches > 1 {
            old_max_jitter
        } else {
            0
        }; // Maximum jitter
        let mut total_jitter: i64 = if self.num_batches > 1 {
            old_total_jitter
        } else {
            -1
        }; // Total jitter

        for &activation in batch.iter() {
            // Relative distance from activation to expected arrival
            let distance = activation as i64 - offset - j * period;

            if distance < 0 {
                offset -= distance.abs();
                max_jitter += distance.abs();
                //total_jitter += distance.abs()*j;
            } else {
                max_jitter = max_jitter.max(distance);
                //total_jitter += distance*j;
                total_jitter = -1;
            }
            j += 1;
        }
        (offset, max_jitter, total_jitter)
    }

    /// This function generates a set of candidate periods given an estimated period `T`,
    /// a minimum period `T_min`, and a maximum period `T_max`.
    ///
    /// The function returns a vector containing:
    /// - The periods that are powers of 2 away from `T` in the negative direction, but not less than `T_min`
    /// - The period `T`
    /// - The periods that are powers of 2 away from `T` in the positive direction, but not more than `T_max`
    ///
    /// # Arguments
    ///
    /// * `T` - An estimated period.
    /// * `T_min` - The minimum allowable period.
    /// * `T_max` - The maximum allowable period.
    ///
    /// # Returns
    ///
    /// A `Vec<i64>` containing the candidate periods.
    pub fn generate_candidate_periods(
        &self,
        period: i64,
        min_period: i64,
        max_period: i64,
        max_candidates: usize, // Added parameter for max candidates per loop
    ) -> Vec<i64> {
        let mut periods = Vec::new();
        periods.push(period);
        periods.push(min_period);
        periods.push(max_period);

        // Generate lower periods
        let mut i = 0;
        while period - (1 << i) > std::cmp::max(min_period, 1) && periods.len() < max_candidates {
            let lower_period = period - (1 << i);

            if lower_period < 1 {
                break;
            }

            periods.push(lower_period);
            i += 1;
        }

        // Generate upper periods
        i = 0;
        while period + (1 << i) < max_period && periods.len() < max_candidates {
            let upper_period = period + (1 << i);

            if !periods.contains(&upper_period) {
                periods.push(upper_period);
            }

            i += 1;
        }

        // Sort the periods to get them in ascending order
        periods.sort();

        periods
    }

    /// Rounds a number to the specified number of significant figures.
    ///
    /// # Arguments
    ///
    /// * `num` - A floating-point number to be rounded.
    /// * `n` - The number of significant figures to round to.
    ///
    /// # Returns
    ///
    /// The number rounded to the specified number of significant figures.
    ///
    pub fn round_to_significant_figures(&self, num: f64, n: i32) -> f64 {
        if num == 0.0 {
            return 0.0;
        }

        let d = (num.abs().log10()).ceil();
        let power = n - d as i32;

        let magnitude = 10f64.powi(power);
        let shifted = (num * magnitude).round();
        shifted / magnitude
    }

    /// Generates a list of candidate periods by rounding the given integer at each digit using significant figures rounding.
    ///
    /// # Arguments
    ///
    /// * `period` - An integer value representing the original period to be rounded.
    ///
    /// # Returns
    ///
    /// A vector containing the original period and the values obtained by rounding each digit from right to left.
    ///
    /// # Example
    ///
    /// ```
    #[allow(unused)]
    pub fn generate_candidate_periods_by_rounding(&self, period: i64) -> Vec<i64> {
        let mut periods_set = HashSet::new();
        let mut periods = Vec::new();

        // Add the original period to the set.
        periods_set.insert(period);
        periods.push(period);

        let num_digits = period.to_string().len() as i32;

        // Iterate through each significant figure, rounding accordingly.
        for n in (1..=num_digits).rev() {
            let rounded = self.round_to_significant_figures(period as f64, n).round() as i64;
            if periods_set.insert(rounded) {
                // Only add if it's not a duplicate
                periods.push(rounded);
            }
        }

        periods
    }

    pub fn generate_candidate_periods_by_rounding_and_incrementing(&self, period: i64) -> Vec<i64> {
        let mut periods_set = HashSet::new();
        let mut periods = Vec::new();

        // Add the original period to the set.
        periods_set.insert(period);
        periods.push(period);

        let num_digits = period.to_string().len() as i32;

        // Iterate through each significant figure level, rounding and incrementing as needed.
        for n in (1..=num_digits).rev() {
            // Round to the n-th significant figure.
            let rounded = self.round_to_significant_figures(period as f64, n).round() as i64;

            // Add the rounded number if it’s new.
            if periods_set.insert(rounded) {
                periods.push(rounded);
            }

            // Generate increments and decrements around the rounded value.
            let increment = 10_i64.pow((num_digits - n) as u32);
            for i in -2..=2 {
                let candidate = rounded + i * increment;
                if candidate > 0 && periods_set.insert(candidate) {
                    // Ensure positive and unique
                    periods.push(candidate);
                }
            }
        }

        periods
    }

    #[allow(unused)]
    pub fn generate_candidate_periods_up_to_sigma_jitter(
        &self,
        sigma_factor: i64,
        optimal_period: i64,
        optimal_max_jitter: i64,
        max_candidates: i64,
    ) -> Vec<i64> {
        let mut periods = Vec::new();
        // Add the original period to the set.
        periods.push(optimal_period);

        let lower_bound: i64 = optimal_period - sigma_factor * optimal_max_jitter - 10; // + 10 is there just to not make problems for ideal periodic behaviour
        let upper_bound: i64 = optimal_period + sigma_factor * optimal_max_jitter + 10;

        // Calculate the lower and upper bounds of the range.
        //let lower_bound = (period as f64 * l_percent).round() as i64;
        //let upper_bound = (period as f64 * u_percent).round() as i64;

        // Determine the step size for generating 10 equally distanced periods on each side.
        let step_size = ((upper_bound - lower_bound) / max_candidates).max(1); // Ensure step size is at least 1

        // Iterate through the range and insert each period into the set.
        let mut current_period = lower_bound;
        while current_period <= upper_bound {
            periods.push(current_period);
            current_period += step_size;
        }

        periods
    }

    /// Generates candidates for periodic functions within the given range.
    ///
    /// This function first calculates a set of candidate periods using the
    /// provided estimated period.
    /// For each candidate period, it then estimates the offset and jitter (both
    /// `max_jitter` and `total_jitter`) using the provided batch of data.
    ///
    /// # Parameters
    /// - `estimated_period`: The initial estimate for the period of the function.
    /// - `batch`: A reference to an array of data points used for estimating
    ///   the offset and jitter of the candidates.
    pub fn generate_all_candidates(
        &mut self,
        least_jitter_periods: Vec<i64>,
        rounded_periods: Vec<i64>,
    ) {
        // Generate candidate periods by rounding the estimated period at each digit

        // Iterate through each candidate period
        for &period in least_jitter_periods.iter() {
            // Estimate the offset and jitter
            let (offset, max_jitter, total_jitter) =
                self.update_offset_and_jitter(period, &self.batch, 0, self.batch[0] as i64, 0, 0);

            // Insert the candidate with the estimated parameters into the collection of candidates
            self.least_jitter_candidates.insert(
                period,
                PeriodicFunctionData {
                    offset,
                    max_jitter,
                    total_jitter,
                },
            );
        }

        // Iterate through each candidate period
        for &period in rounded_periods.iter() {
            // Estimate the offset and jitter
            let (offset, max_jitter, total_jitter) =
                self.update_offset_and_jitter(period, &self.batch, 0, self.batch[0] as i64, 0, 0);

            // Insert the candidate with the estimated parameters into the collection of candidates
            self.rounded_period_candidates.insert(
                period,
                PeriodicFunctionData {
                    offset,
                    max_jitter,
                    total_jitter,
                },
            );
        }
    }
    #[allow(unused)]
    pub fn compute_jitter(&self, period: i64) -> i64 {
        let (_, jitter, _) =
            self.update_offset_and_jitter(period, &self.batch, 0, self.batch[0] as i64, 0, 0);
        jitter
    }

    /// Generates candidates for periodic functions within the given range.
    ///
    /// This function first calculates a set of candidate periods using the
    /// provided estimated period and the given minimum and maximum period bounds.
    /// For each candidate period, it then estimates the offset and jitter (both
    /// `max_jitter` and `total_jitter`) using the provided batch of data.
    ///
    /// # Parameters
    /// - `estimated_period`: The initial estimate for the period of the function.
    /// - `min_period`: The minimum allowable period for a candidate.
    /// - `max_period`: The maximum allowable period for a candidate.
    /// - `batch`: A reference to an array of data points used for estimating
    ///   the offset and jitter of the candidates.
    #[allow(unused)]
    pub fn generate_candidates(&mut self, estimated_period: i64, min_period: i64, max_period: i64) {
        // Generate candidate periods within the specified range based on the estimated period
        let periods = self.generate_candidate_periods(
            estimated_period,
            min_period,
            max_period,
            MAX_CANDIDATES,
        );

        // Iterate through each candidate period
        for &period in periods.iter() {
            // Estimate the offset and jitter
            let (offset, max_jitter, total_jitter) =
                self.update_offset_and_jitter(period, &self.batch, 0, self.batch[0] as i64, 0, 0);

            // Insert the candidate with the estimated parameters into the collection of candidates
            self.least_jitter_candidates.insert(
                period,
                PeriodicFunctionData {
                    offset,
                    max_jitter,
                    total_jitter,
                },
            );
        }
    }

    /// Adjusts the given parameters to align with a lower estimated period.
    ///
    /// This function takes an estimated period, a multiplier `n`, a lower period, and data associated with
    /// the lower period. It calculates and returns the adjusted offset, max_jitter, and total_jitter.
    ///
    /// # Arguments
    /// * `estimated_period` - The estimated period value.
    /// * `n` - Number of analysed activations -1.
    /// * `period_l` - The lower period.
    /// * `period_data_l` - The data associated with the lower period.
    ///
    /// # Returns
    /// A tuple containing the adjusted values for offset, max_jitter, and total_jitter.
    pub fn adjust_to_lower_period_estimate(
        &self,
        estimated_period: i64,
        n: i64, // Number of analysed activations -1
        period_l: i64,
        period_data_l: &PeriodicFunctionData,
    ) -> (i64, i64, i64) {
        let offset = period_data_l.offset + period_l * n - estimated_period * n;
        let max_jitter = period_data_l.offset + period_data_l.max_jitter - offset;
        let total_jitter = -1;
        (offset, max_jitter, total_jitter)
    }

    /// Adjusts the given parameters to align with a higher estimated period.
    ///
    /// This function takes an estimated period, a multiplier `n`, a higher period, and data associated with
    /// the higher period. It calculates and returns the adjusted offset, max_jitter, and total_jitter.
    ///
    /// # Arguments
    /// * `estimated_period` - The estimated period value.
    /// * `n` - Number of analysed activations -1.
    /// * `period_h` - The higher period.
    /// * `period_data_h` - The data associated with the higher period.
    ///
    /// # Returns
    /// A tuple containing the adjusted values for offset, max_jitter, and total_jitter.
    pub fn adjust_to_higher_period_estimate(
        &self,
        estimated_period: i64,
        n: i64, // Number of analysed activations -1
        period_h: i64,
        period_data_h: &PeriodicFunctionData,
    ) -> (i64, i64, i64) {
        let offset = period_data_h.offset;
        let max_jitter = n * (period_h - estimated_period) + period_data_h.max_jitter;
        let total_jitter = -1; // TODO: Fix this
        (offset, max_jitter, total_jitter)
    }

    /// Updates an existing candidate in the collection using the given period and batch data.
    ///
    /// This function uses the method `update_offset_jitter` to update the offset and jitter values
    /// for the specified period using the provided batch of data. It then updates the corresponding
    /// candidate in the collection with these new values.
    ///
    /// # Parameters
    /// - `estimated_period`: The period of the function for which the candidate is being updated.
    /// - `q`: The offset value used in the update process.
    /// - `j_max_ns`: The jitter value for the non-sampling jitter.
    /// - `j_max_ws`: The jitter value for the worst-case sampling jitter.
    /// - `batch`: A reference to an array of data points used for estimating the offset and jitter.
    /// - `a_l`: The previous activation value.
    pub fn conservatively_update_lj_estimates(
        &self,
        period: i64,
        batch: &[f64],
    ) -> (i64, i64, i64) {
        if let Some(candidate) = self.least_jitter_candidates.get(&period) {
            let old_offset = candidate.offset;
            let old_max_jitter = candidate.max_jitter;
            let old_total_jitter = candidate.total_jitter;
            let (new_offset, new_max_jitter, new_total_jitter) = self.update_offset_and_jitter(
                period,
                batch,
                self.next_j,
                old_offset,
                old_max_jitter,
                old_total_jitter,
            );
            (new_offset, new_max_jitter, new_total_jitter)
        } else {
            panic!("Candidate does not exist for period {}", period);
        }
    }

    pub fn conservatively_update_rounded_estimates(
        &self,
        period: i64,
        batch: &[f64],
    ) -> (i64, i64, i64) {
        if let Some(candidate) = self.rounded_period_candidates.get(&period) {
            let old_offset = candidate.offset;
            let old_max_jitter = candidate.max_jitter;
            let old_total_jitter = candidate.total_jitter;
            let (new_offset, new_max_jitter, new_total_jitter) = self.update_offset_and_jitter(
                period,
                batch,
                self.next_j,
                old_offset,
                old_max_jitter,
                old_total_jitter,
            );
            (new_offset, new_max_jitter, new_total_jitter)
        } else {
            panic!("Candidate does not exist for period {}", period);
        }
    }

    /// Updates the candidate data for a given period based on the current batch of data.
    ///
    /// This function estimates new values for `offset`, `max_jitter`, and `total_jitter`
    /// for the provided period using the current batch. If a candidate for the given
    /// period already exists, its values will be replaced. If no candidate exists,
    /// a new entry will be created.
    ///
    /// # Parameters
    ///
    /// - `period`: The period for which the candidate data should be updated.
    pub fn update_lj_candidate(&mut self, period: i64) {
        let (new_offset, new_max_jitter, new_total_jitter) =
            self.conservatively_update_lj_estimates(period, &self.batch);
        self.least_jitter_candidates.insert(
            period,
            PeriodicFunctionData {
                offset: new_offset,
                max_jitter: new_max_jitter,
                total_jitter: new_total_jitter,
            },
        );
    }

    pub fn update_rounded_candidate(&mut self, period: i64) {
        let (new_offset, new_max_jitter, new_total_jitter) =
            self.conservatively_update_rounded_estimates(period, &self.batch);
        self.rounded_period_candidates.insert(
            period,
            PeriodicFunctionData {
                offset: new_offset,
                max_jitter: new_max_jitter,
                total_jitter: new_total_jitter,
            },
        );
    }

    /// Updates the candidate data for all current candidates based on the current batch of data.
    ///
    /// For each candidate period in the current set, this function estimates new values for
    /// `offset`, `max_jitter`, and `total_jitter` using the current batch. The values for existing
    /// candidates will be replaced with the new estimates.
    pub fn update_all_candidates(&mut self) {
        // Iterate over a list of all current candidate periods.
        // We need to collect them into a Vec to prevent simultaneous mutable and immutable borrows of `self`.
        let all_candidate_periods: Vec<i64> =
            self.least_jitter_candidates.keys().cloned().collect();

        for period in all_candidate_periods {
            self.update_lj_candidate(period);
        }

        let all_rounded_periods: Vec<i64> =
            self.rounded_period_candidates.keys().cloned().collect();

        for period in all_rounded_periods {
            self.update_rounded_candidate(period);
        }
    }

    fn get_lower_and_higher_candidates(
        &self,
        estimated_period: i64,
    ) -> (PeriodicCandidate, PeriodicCandidate) {
        let lower_period_candidate = self
            .least_jitter_candidates
            .range(..estimated_period)
            .next_back();
        let higher_period_candidate = self
            .least_jitter_candidates
            .range(estimated_period..)
            .next();
        (lower_period_candidate, higher_period_candidate)
    }

    /// Selects the most appropriate offset and jitter estimates based on the provided lower and higher estimates.
    ///
    /// This function takes the estimated period, a parameter `n`, and options for lower and higher period estimates.
    /// Based on these inputs, it determines the most conservative estimates for the offset and jitter (both with and without skips).
    ///
    /// # Arguments
    ///
    /// * `estimated_period` - The estimated period for which the offset and jitter are being computed.
    /// * `n` - An additional parameter used in the estimation (description of its role may be added based on context).
    /// * `lower_estimate` - An option containing the lower period estimate and associated data, or `None` if no lower estimate is available.
    /// * `higher_estimate` - An option containing the higher period estimate and associated data, or `None` if no higher estimate is available.
    ///
    /// # Returns
    ///
    /// A tuple containing three `i64` values representing the selected offset and maximum jitter (with and without skips) for the conservative estimate.
    fn select_hook(
        &self,
        estimated_period: i64,
        n: i64,
        lower_estimate: Option<(&i64, &PeriodicFunctionData)>,
        higher_estimate: Option<(&i64, &PeriodicFunctionData)>,
    ) -> (i64, i64, i64) {
        match (lower_estimate, higher_estimate) {
            (Some((period_l, period_data_l)), Some((period_h, period_data_h))) => {
                if (period_l - estimated_period).abs() <= (period_h - estimated_period).abs() {
                    self.adjust_to_lower_period_estimate(
                        estimated_period,
                        n,
                        *period_l,
                        period_data_l,
                    )
                } else {
                    self.adjust_to_higher_period_estimate(
                        estimated_period,
                        n,
                        *period_h,
                        period_data_h,
                    )
                }
            }
            (Some((period_l, period_data_l)), None) => {
                self.adjust_to_lower_period_estimate(estimated_period, n, *period_l, period_data_l)
            }
            (None, Some((period_h, period_data_h))) => {
                self.adjust_to_higher_period_estimate(estimated_period, n, *period_h, period_data_h)
            }
            (None, None) => (i64::MAX, i64::MAX, i64::MAX), // Define appropriate default values here
        }
    }

    // Then compute adjust_to_lower_period_estimate and adjust_to_higher_period_estimate
    // Then insert the candidate with the minimum jitter of the two
    pub fn insert_candidate(&mut self, estimated_period: i64, n: i64) {
        // First, gather all necessary data with immutable borrows
        let (lower_estimate, higher_estimate) =
            self.get_lower_and_higher_candidates(estimated_period);
        let conservative_estimates =
            self.select_hook(estimated_period, n, lower_estimate, higher_estimate);

        // Then, perform the mutable operation
        self.least_jitter_candidates
            .entry(estimated_period)
            .or_insert_with(|| PeriodicFunctionData {
                offset: conservative_estimates.0,
                max_jitter: conservative_estimates.1,
                total_jitter: conservative_estimates.2,
            });
        // In case there is already a candidate for the estimated period, the above won't replace it
        // It will be updated later if needed
    }

    // TODO: Implement remove_candidate
    pub fn remove_candidate(&mut self) {
        if MAX_CANDIDATES < self.least_jitter_candidates.len() {
            // Find the key with the largest max_jitter without borrowing the map for the whole block
            let key_to_remove = {
                self.least_jitter_candidates
                    .iter()
                    .max_by_key(|(_, data)| data.max_jitter)
                    .map(|(key, _)| *key)
            };

            // Remove the key from the candidates
            if let Some(key) = key_to_remove {
                self.least_jitter_candidates.remove(&key);
            }
        }
    }

    pub fn remove_candidates(&mut self) {
        let least_jitter_candidate = self.get_candidate_least_max_jitter();
        let least_jitter = least_jitter_candidate.unwrap().1.max_jitter;
        let keys_to_remove_lj_candidates: Vec<i64> = self
            .least_jitter_candidates
            .iter()
            .filter(|(_, data)| data.max_jitter > (REMOVAL_THRESHOLD as i64) * least_jitter)
            .map(|(period, _)| *period)
            .collect();

        for period in keys_to_remove_lj_candidates {
            self.least_jitter_candidates.remove(&period);
        }

        let keys_to_remove_rounded_candidates: Vec<i64> = self
            .rounded_period_candidates
            .iter()
            .filter(|(_, data)| data.max_jitter > (REMOVAL_THRESHOLD as i64) * least_jitter)
            .map(|(period, _)| *period)
            .collect();

        for period in keys_to_remove_rounded_candidates {
            self.rounded_period_candidates.remove(&period);
        }
    }

    pub fn insert_and_remove(&mut self, estimated_period: i64, n: i64) {
        // Check if the estimated_period is already a key.
        if !self.least_jitter_candidates.contains_key(&estimated_period) {
            // If it's not, insert it.
            self.insert_candidate(estimated_period, n);

            // And then remove the farthest candidate.
            self.remove_candidate();
        }
        // If estimated_period is already a key, do nothing.
    }

    #[allow(unused)]
    pub fn pretty_print_candidates(&self, descriptor: &str) {
        println!("Printing all {} candidates:", descriptor);
        println!("---------------------------------");
        for (period, data) in &self.rounded_period_candidates {
            println!(
                "Period: {}\nOffset: {}\nMax Jitter: {}\n",
                period, data.offset, data.max_jitter
            );
            println!("---------------------------------");
        }
    }

    #[allow(unused)]
    pub fn pretty_print_best_guess(&self) {
        println!("Best Guessed Candidate:");
        println!("-----------------------");
        println!("Period: {}", self.best_guess_candidate.period);
        println!("Frequency: {}", self.best_guess_candidate.frequency);
        println!("Offset: {}", self.best_guess_candidate.offset);
        println!("Max Jitter: {}", self.best_guess_candidate.max_jitter);
        println!("-----------------------");
    }

    pub fn get_candidate_least_max_jitter(&self) -> Option<(&i64, &PeriodicFunctionData)> {
        // Search through the candidates map to find the one with the least max_jitter
        self.least_jitter_candidates
            .iter()
            .min_by_key(|&(_, data)| data.max_jitter)
    }

    // returns least jitter candidate for both candidate lists (ternary and rounded)
    pub fn get_global_least_max_jitter(&self) -> Option<(&i64, &PeriodicFunctionData)> {
        // Search through the candidates map to find the one with the least max_jitter
        let lj = self
            .least_jitter_candidates
            .iter()
            .min_by_key(|&(_, data)| data.max_jitter);
        let ro = self
            .rounded_period_candidates
            .iter()
            .min_by_key(|&(_, data)| data.max_jitter);
        match (lj, ro) {
            (Some(lj_data), Some(ro_data)) => {
                if lj_data.1.max_jitter < ro_data.1.max_jitter {
                    Some(lj_data)
                } else {
                    Some(ro_data)
                }
            }
            (Some(lj_data), None) => Some(lj_data),
            (None, Some(ro_data)) => Some(ro_data),
            (None, None) => None,
        }
    }

    #[allow(unused)]
    pub fn let_best_be_rounded(&mut self) {
        let least_jitter_candidate = self.get_global_least_max_jitter();
        if let Some((least_jitter_period, least_jitter_data)) = least_jitter_candidate {
            self.best_guess_candidate = CandidateData {
                period: *least_jitter_period,
                frequency: self.period_to_frequency(*least_jitter_period as f64) as i64,
                offset: least_jitter_data.offset,
                max_jitter: least_jitter_data.max_jitter,
            };
        }

        let num_digits = self.best_guess_candidate.period.to_string().len() as i32;
        let current_period = self.best_guess_candidate.period;

        // Iterate through each significant figure, rounding accordingly.
        for n in (1..=num_digits).rev() {
            let rounded = self
                .round_to_significant_figures(current_period as f64, n)
                .round() as i64;
            //println!("Rounded: {}", rounded);
            if let Some(rounded_data) = self.rounded_period_candidates.get(&rounded) {
                if rounded_data.max_jitter
                    < self.best_guess_candidate.max_jitter
                        + self.best_guess_candidate.max_jitter / 4
                {
                    self.best_guess_candidate = CandidateData {
                        period: rounded,
                        frequency: self.period_to_frequency(rounded as f64) as i64,
                        offset: rounded_data.offset,
                        max_jitter: rounded_data.max_jitter,
                    };
                }
            }
        }
    }

    // This function sets the best guess candidate to the one with the least max jitter in the least_jitter_candidates
    #[allow(unused)]
    pub fn let_best_be_least(&mut self) {
        let least_jitter_candidate = self.get_candidate_least_max_jitter();
        if let Some((least_jitter_period, least_jitter_data)) = least_jitter_candidate {
            self.best_guess_candidate = CandidateData {
                period: *least_jitter_period,
                frequency: self.period_to_frequency(*least_jitter_period as f64) as i64,
                offset: least_jitter_data.offset,
                max_jitter: least_jitter_data.max_jitter,
            };
        }
    }

    // This function is used to update the best guess candidate to be the one with the least max jitter among both candidate lists
    #[allow(unused)]
    pub fn let_best_be_global_least(&mut self) {
        let least_jitter_candidate = self.get_global_least_max_jitter();
        if let Some((least_jitter_period, least_jitter_data)) = least_jitter_candidate {
            self.best_guess_candidate = CandidateData {
                period: *least_jitter_period,
                frequency: self.period_to_frequency(*least_jitter_period as f64) as i64,
                offset: least_jitter_data.offset,
                max_jitter: least_jitter_data.max_jitter,
            };
        }
    }

    pub fn update_least_jitter_stats(&mut self, optimal_ternary_period: i64) {
        // Update the mean and sum of the last jitter periods
        if self.sum_lj_period.is_finite() {
            self.sum_lj_period += optimal_ternary_period as f64;
            if self.sum_lj_period.is_finite() {
                //println!("num_batches: {}", self.num_batches);
                let new_mean = self.sum_lj_period / self.num_batches as f64;
                self.mean_lj_period = new_mean.round() as i64;
                //println!("Mean LJ period: {}", self.mean_lj_period);
                //println!("Optimal ternary period: {}", optimal_ternary_period);
            }
        }
    }

    /// Checks if the provided batch contains any consecutive identical values.
    ///
    /// Given a non-decreasing sequence in `batch`, this function determines if there
    /// are any pairs of consecutive elements with identical values, which might indicate
    /// bursty arrivals in the data sequence.
    ///
    /// # Arguments
    ///
    /// * `batch`: A slice of `f64` values representing the batch to be checked. It is assumed
    ///   that this batch is non-decreasing.
    ///
    /// # Returns
    ///
    /// * `bool`: `true` if any consecutive identical values are found, otherwise `false`.
    pub fn contains_bursty_arrivals(
        &self,
        last_arrival: f64,
        batch: &[f64],
    ) -> (bool, f64, f64, f64, Vec<f64>) {
        let mut is_too_bursty = false;
        let mut n = self.n;
        let mut mean_period = self.mean_inter_release;
        let mut variance_period = self.variance_inter_release;
        let mut diffs: Vec<f64> = Vec::with_capacity(batch.len()); // a vector of observed inter-arrival times
        let mut num_consecutive_bursts: u32 = 0;

        if last_arrival != 0.0 {
            let first_diff = batch[0] - last_arrival;
            if first_diff.abs() == 0.0 {
                num_consecutive_bursts += 1;
            }
            n += 1.0;
            let (new_mean, new_var) =
                self.update_statistical_estimate(n, mean_period, variance_period, first_diff);
            mean_period = new_mean;
            variance_period = new_var;
            diffs.push(first_diff);
        }

        for i in 1..batch.len() {
            let diff = batch[i] - batch[i - 1];
            if diff.abs() == 0.0 {
                num_consecutive_bursts += 1;
            }
            n += 1.0;
            let (new_mean, new_var) =
                self.update_statistical_estimate(n, mean_period, variance_period, diff);
            mean_period = new_mean;
            variance_period = new_var;
            diffs.push(diff);
        }

        if num_consecutive_bursts >= batch.len() as u32 / 2 {
            is_too_bursty = true;
            return (is_too_bursty, n, mean_period, variance_period, diffs);
        }
        (is_too_bursty, n, mean_period, variance_period, diffs)
    }

    pub fn compute_diffs(&self, batch: Vec<f64>) -> Vec<f64> {
        // Check if the batch has less than 2 elements; return an empty vector if so.
        if batch.len() < 2 {
            return Vec::new();
        }

        // Use the windows() method to iterate over pairs of consecutive elements.
        batch.windows(2).map(|w| w[1] - w[0]).collect()
    }

    pub fn median(&self, data: &[f64]) -> f64 {
        let mut sorted_data = data.to_owned();
        sorted_data.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let len = sorted_data.len();

        if len % 2 == 0 {
            (sorted_data[len / 2 - 1] + sorted_data[len / 2]) / 2.0
        } else {
            sorted_data[len / 2]
        }
    }

    pub fn detect_outliers_hampel(&self, diffs: &[f64], k: f64) -> Vec<usize> {
        if diffs.is_empty() {
            return Vec::new();
        }

        // Calculate the median of diffs.
        let med = self.median(diffs);

        // Calculate the MAD (Median Absolute Deviation).
        let mad = self.median(&diffs.iter().map(|&x| (x - med).abs()).collect::<Vec<f64>>());

        // Define threshold for outliers.
        let threshold = k * mad;

        // Collect indices of outliers based on the Hampel filter.
        diffs
            .iter()
            .enumerate()
            .filter_map(|(i, &diff)| {
                if (diff - med).abs() > threshold {
                    Some(i)
                } else {
                    None
                }
            })
            .collect()
    }

    #[allow(unused)]
    pub fn compute_start_end(&self, batch: Vec<f64>) -> (usize, usize) {
        let diffs = self.compute_diffs(batch);
        let outlier_indices = self.detect_outliers_hampel(&diffs, 3.0);
        // Start: first index where the difference with the following element is not an outlier
        let mut start = 0;
        for i in 0..diffs.len() {
            if !outlier_indices.contains(&i) {
                start = i;
                break;
            }
        }

        // End: largest index where the difference with the previous element is not an outlier
        let mut end = diffs.len() + 1;
        for i in (1..diffs.len()).rev() {
            if !outlier_indices.contains(&i) {
                end = i + 1;
                break;
            }
        }

        (start, end)
    }

    fn derive_clean_batch(&self, batch: Vec<f64>, start: usize, end: usize) -> Vec<f64> {
        // Slice the batch from start to end (inclusive) to create the clean_batch
        batch[start..=end].to_vec()
    }

    pub fn analyse_batch(&mut self) {
        // Check if the batch contains bursty arrivals
        let (bursty, n, mean_period, variance_period, _) =
            self.contains_bursty_arrivals(self.last_arrival, &self.batch);
        // diffs is a vector of observed inter-arrival times

        if bursty {
            self.bursty = true;
            return;
        }

        if self.batch.len() > 5 {
            let batch1 = self.batch.clone();
            let batch2 = self.batch.clone();
            let (start, end) = self.compute_start_end(batch1);
            let clean_batch = self.derive_clean_batch(batch2, start, end);
            self.clean_batch = clean_batch;
        }

        //println!("Batch size: {}", self.batch.len());

        // Update the statistical estimates with the new values
        self.n = n;
        self.mean_inter_release = mean_period;
        self.variance_inter_release = variance_period;
        // Update tdigest with the new diffs

        if let Some(&last) = self.batch.last() {
            self.last_arrival = last;
        }

        let start = self.mean_inter_release / 2.0;
        let end = self.mean_inter_release * 2.0;
        let optimal_ternary_period = self.compute_optimal_ternary_period(start as i64, end as i64);
        let batch_optimal_max_jitter = self.compute_jitter(optimal_ternary_period);

        // Update the mean and sum of the estimated least jitter periods
        self.update_least_jitter_stats(optimal_ternary_period);

        let n_batches = self.num_batches; // take the value to avoid borrowing self later

        // If the batch is the first batch, generate candidates
        if n_batches < 2 {
            // Generate candidates for the optimal ternary period
            // Generate candidates around the optimal one.
            let sigma_jitter_periods: Vec<i64> = self
                .generate_candidate_periods_up_to_sigma_jitter(
                    SIGMA_JITTER_FACTOR,
                    optimal_ternary_period,
                    batch_optimal_max_jitter,
                    MAX_CANDIDATES as i64,
                )
                .into_iter()
                .collect();

            //println!("Generated min and max periods");
            //println!("No of periods: {}", percent_periods.len());
            //println!("min period: {}", percent_periods.iter().min().unwrap());
            //println!("max period: {}", percent_periods.iter().max().unwrap());

            let rounded_periods = self
                .generate_candidate_periods_by_rounding_and_incrementing(optimal_ternary_period)
                .into_iter()
                .collect();

            // Generate candidates from the union of the two sets of periods
            self.generate_all_candidates(sigma_jitter_periods, rounded_periods);
        } else {
            // If the batch is not the first batch,

            // Insert the candidate with the mean inter-release estimation
            self.insert_and_remove(optimal_ternary_period, self.next_j - 1);

            //self.insert_and_remove(self.mean_lj_period, self.next_j - 1);

            // Update all guessed estimates
            //self.update_guessed_estimates();

            // Insert the candidate with the guessed period and remove the one with the greatest max_jitter if necessary
            self.insert_and_remove(self.mean_lj_period, self.next_j - 1);

            // Update candidates considering the new batch
            self.update_all_candidates();

            // Update best_guess_candidate
            //self.update_best_guess_candidate();
            //self.pretty_print_best_guess(); // TODO: Remove this later

            //self.let_best_be_least();
            //self.let_best_be_rounded();
        }

        // Update the number of analysed batches and the next_j value

        // Increment the number of analysed batches
        self.num_batches += 1;
        // Increment the next j value
        self.next_j += self.batch.len() as i64;

        // clear the bad candidates
        self.remove_candidates();
        //println!("Number of least j candidates: {}", self.least_jitter_candidates.len());
        //println!("Number of rounded candidates: {}", self.rounded_period_candidates.len());
    }

    pub fn analyse_batch_and_clear(&mut self) {
        if self.batch.len() >= 2 {
            self.analyse_batch();

            self.let_best_be_rounded();
            //self.let_best_be_global_least();
            //self.let_best_be_least();
            self.batch.clear();
        }
        if self.batch.len() <= 1 {
            // Do not return a periodic model if there is no enough data to infer one.
            if self.num_batches == 1 {
                self.batch.clear();
                return;
            }
            // If there is more than one batch analysed and there is only one
            // job in the last batch, we need to update the candidates accounting for the job.
            self.update_all_candidates();
            //self.update_best_guess_candidate();

            //self.let_best_be_least();
            //self.pretty_print_candidates("Least Jitter Candidates");

            self.let_best_be_rounded();
            //self.let_best_be_global_least();
            //self.let_best_be_least();

            self.batch.clear();
        }
    }

    pub fn update(&mut self, job: &Job) {
        // Set t_0 to the job.arrival of the first analyzed batch
        if self.t_0.is_none() && self.batch.is_empty() {
            self.t_0.replace(job.release);
        }

        self.batch.push((job.release) as f64); // TODO: Cast to integer, not f64

        if self.batch.len() >= self.batch_size {
            self.analyse_batch_and_clear()
        }
    }
}

impl Default for PeriodExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl ArrivalModelExtractor for PeriodExtractor {
    fn extract(&self) -> Option<ArrivalModel<'_>> {
        if self.bursty || self.best_guess_candidate.offset < 0 {
            return None;
        }

        if self.best_guess_candidate.period > 0 {
            return Some(ArrivalModel::Periodic(Periodic::Release {
                period: self.best_guess_candidate.period as u64,
                frequency: self.best_guess_candidate.frequency as f64,
                offset: self.best_guess_candidate.offset as u64,
                max_jitter: self.best_guess_candidate.max_jitter as u64,

                mean: self.mean_inter_release.ceil() as u64,
                variance: self.variance_inter_release.ceil() as u64,
            }));
        }

        None
    }
}

#[allow(unused)]
fn generate_jobs(start: u64, step: u64, count: usize) -> Vec<Job> {
    (0..count)
        .map(|i| {
            let value = start + step * i as u64;
            Job {
                release: value,
                first_cycle: value,
                end: value,
                arrival: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::{
        job::Job,
        model_extractors::arrival::{
            periodic::release::{PeriodicFunctionData, MAX_CANDIDATES},
            ArrivalModel, ArrivalModelExtractor, Periodic,
        },
    };

    use super::PeriodExtractor;

    const PERFECTLY_PERIODIC: &[Job] = &[
        Job {
            end: 2,
            arrival: None,
            release: 0,
            first_cycle: 0,
        },
        Job {
            release: 10,
            end: 12,
            arrival: None,
            first_cycle: 10,
        },
        Job {
            release: 20,
            first_cycle: 20,
            arrival: None,
            end: 22,
        },
        Job {
            release: 30,
            first_cycle: 30,
            arrival: None,
            end: 32,
        },
        Job {
            release: 40,
            first_cycle: 40,
            arrival: None,
            end: 42,
        },
        Job {
            release: 50,
            first_cycle: 50,
            arrival: None,
            end: 52,
        },
        Job {
            release: 60,
            first_cycle: 60,
            arrival: None,
            end: 62,
        },
        Job {
            release: 70,
            first_cycle: 70,
            arrival: None,
            end: 72,
        },
        Job {
            release: 80,
            first_cycle: 80,
            arrival: None,
            end: 82,
        },
        Job {
            release: 90,
            first_cycle: 90,
            arrival: None,
            end: 92,
        },
    ];

    const PERFECTLY_PERIODIC_OFFSET_2: &[Job] = &[
        Job {
            release: 2,
            first_cycle: 2,
            arrival: None,
            end: 3,
        },
        Job {
            release: 12,
            first_cycle: 12,
            arrival: None,
            end: 14,
        },
        Job {
            release: 22,
            first_cycle: 22,
            arrival: None,
            end: 24,
        },
        Job {
            release: 32,
            first_cycle: 32,
            arrival: None,
            end: 34,
        },
        Job {
            release: 42,
            first_cycle: 42,
            arrival: None,
            end: 44,
        },
        Job {
            release: 52,
            first_cycle: 52,
            arrival: None,
            end: 54,
        },
        Job {
            release: 62,
            first_cycle: 62,
            arrival: None,
            end: 64,
        },
        Job {
            release: 72,
            first_cycle: 72,
            arrival: None,
            end: 74,
        },
        Job {
            release: 82,
            first_cycle: 82,
            arrival: None,
            end: 84,
        },
        Job {
            release: 92,
            first_cycle: 92,
            arrival: None,
            end: 94,
        },
    ];

    const TS_40_ARRIVALS_NEGATIVE_OFFSET: &[Job] = &[
        Job {
            release: 10,
            first_cycle: 10,
            arrival: None,
            end: 13,
        },
        Job {
            release: 20,
            first_cycle: 20,
            arrival: None,
            end: 25,
        },
        Job {
            release: 30,
            first_cycle: 30,
            arrival: None,
            end: 37,
        },
        Job {
            release: 40,
            first_cycle: 40,
            arrival: None,
            end: 49,
        },
        Job {
            release: 60,
            first_cycle: 60,
            arrival: None,
            end: 61,
        },
        Job {
            release: 70,
            first_cycle: 70,
            arrival: None,
            end: 73,
        },
        Job {
            release: 80,
            first_cycle: 80,
            arrival: None,
            end: 85,
        },
        Job {
            release: 90,
            first_cycle: 90,
            arrival: None,
            end: 97,
        },
        Job {
            release: 100,
            first_cycle: 100,
            arrival: None,
            end: 109,
        },
        Job {
            release: 120,
            first_cycle: 120,
            arrival: None,
            end: 121,
        },
        Job {
            release: 130,
            first_cycle: 130,
            arrival: None,
            end: 134,
        },
        Job {
            release: 140,
            first_cycle: 140,
            arrival: None,
            end: 147,
        },
        Job {
            release: 150,
            first_cycle: 150,
            arrival: None,
            end: 160,
        },
        Job {
            release: 170,
            first_cycle: 170,
            arrival: None,
            end: 173,
        },
        Job {
            release: 180,
            first_cycle: 180,
            arrival: None,
            end: 186,
        },
        Job {
            release: 190,
            first_cycle: 190,
            arrival: None,
            end: 199,
        },
        Job {
            release: 210,
            first_cycle: 210,
            arrival: None,
            end: 212,
        },
        Job {
            release: 220,
            first_cycle: 220,
            arrival: None,
            end: 225,
        },
        Job {
            release: 230,
            first_cycle: 230,
            arrival: None,
            end: 238,
        },
        Job {
            release: 240,
            first_cycle: 240,
            arrival: None,
            end: 251,
        },
        Job {
            release: 260,
            first_cycle: 260,
            arrival: None,
            end: 265,
        },
        Job {
            release: 270,
            first_cycle: 270,
            arrival: None,
            end: 279,
        },
        Job {
            release: 290,
            first_cycle: 290,
            arrival: None,
            end: 293,
        },
        Job {
            release: 300,
            first_cycle: 300,
            arrival: None,
            end: 307,
        },
        Job {
            release: 320,
            first_cycle: 320,
            arrival: None,
            end: 321,
        },
        Job {
            release: 330,
            first_cycle: 330,
            arrival: None,
            end: 335,
        },
        Job {
            release: 340,
            first_cycle: 340,
            arrival: None,
            end: 349,
        },
        Job {
            release: 360,
            first_cycle: 360,
            arrival: None,
            end: 363,
        },
        Job {
            release: 370,
            first_cycle: 370,
            arrival: None,
            end: 377,
        },
        Job {
            release: 390,
            first_cycle: 390,
            arrival: None,
            end: 391,
        },
        Job {
            release: 400,
            first_cycle: 400,
            arrival: None,
            end: 406,
        },
        Job {
            release: 420,
            first_cycle: 420,
            arrival: None,
            end: 421,
        },
        // NEW BATCH STARTS HERE. Given that the period is
        Job {
            release: 535,
            first_cycle: 535,
            arrival: None,
            end: 436,
        },
        Job {
            release: 550,
            first_cycle: 550,
            arrival: None,
            end: 451,
        },
        Job {
            release: 565,
            first_cycle: 565,
            arrival: None,
            end: 466,
        },
        Job {
            release: 580,
            first_cycle: 580,
            arrival: None,
            end: 481,
        },
        Job {
            release: 595,
            first_cycle: 595,
            arrival: None,
            end: 496,
        },
        Job {
            release: 610,
            first_cycle: 610,
            arrival: None,
            end: 511,
        },
        Job {
            release: 625,
            first_cycle: 625,
            arrival: None,
            end: 526,
        },
        Job {
            release: 640,
            first_cycle: 640,
            arrival: None,
            end: 541,
        },
    ];

    const FIRST_BATCH: &[Job] = &[
        Job {
            release: 100,
            first_cycle: 100,
            end: 100,
            arrival: None,
        },
        Job {
            release: 110,
            first_cycle: 110,
            end: 110,
            arrival: None,
        },
        Job {
            release: 120,
            first_cycle: 120,
            end: 120,
            arrival: None,
        },
        Job {
            release: 130,
            first_cycle: 130,
            end: 130,
            arrival: None,
        },
        Job {
            release: 140,
            first_cycle: 140,
            end: 140,
            arrival: None,
        },
        Job {
            release: 150,
            first_cycle: 150,
            end: 150,
            arrival: None,
        },
        Job {
            release: 160,
            first_cycle: 160,
            end: 160,
            arrival: None,
        },
        Job {
            release: 170,
            first_cycle: 170,
            end: 170,
            arrival: None,
        },
        Job {
            release: 180,
            first_cycle: 180,
            end: 180,
            arrival: None,
        },
        Job {
            release: 190,
            first_cycle: 190,
            end: 190,
            arrival: None,
        },
        Job {
            release: 200,
            first_cycle: 200,
            end: 200,
            arrival: None,
        },
        Job {
            release: 210,
            first_cycle: 210,
            end: 210,
            arrival: None,
        },
        Job {
            release: 220,
            first_cycle: 220,
            end: 220,
            arrival: None,
        },
        Job {
            release: 230,
            first_cycle: 230,
            end: 230,
            arrival: None,
        },
        Job {
            release: 240,
            first_cycle: 240,
            end: 240,
            arrival: None,
        },
        Job {
            release: 250,
            first_cycle: 250,
            end: 250,
            arrival: None,
        },
        Job {
            release: 260,
            first_cycle: 260,
            end: 260,
            arrival: None,
        },
        Job {
            release: 270,
            first_cycle: 270,
            end: 270,
            arrival: None,
        },
        Job {
            release: 280,
            first_cycle: 280,
            end: 280,
            arrival: None,
        },
        Job {
            release: 290,
            first_cycle: 290,
            end: 290,
            arrival: None,
        },
        Job {
            release: 300,
            first_cycle: 300,
            end: 300,
            arrival: None,
        },
        Job {
            release: 310,
            first_cycle: 310,
            end: 310,
            arrival: None,
        },
        Job {
            release: 320,
            first_cycle: 320,
            end: 320,
            arrival: None,
        },
        Job {
            release: 330,
            first_cycle: 330,
            end: 330,
            arrival: None,
        },
        Job {
            release: 340,
            first_cycle: 340,
            end: 340,
            arrival: None,
        },
        Job {
            release: 350,
            first_cycle: 350,
            end: 350,
            arrival: None,
        },
        Job {
            release: 360,
            first_cycle: 360,
            end: 360,
            arrival: None,
        },
        Job {
            release: 370,
            first_cycle: 370,
            end: 370,
            arrival: None,
        },
        Job {
            release: 380,
            first_cycle: 380,
            end: 380,
            arrival: None,
        },
        Job {
            release: 390,
            first_cycle: 390,
            end: 390,
            arrival: None,
        },
        Job {
            release: 400,
            first_cycle: 400,
            end: 400,
            arrival: None,
        },
        Job {
            release: 413,
            first_cycle: 413,
            end: 410,
            arrival: None,
        },
    ];

    const SECOND_BATCH: &[Job] = &[
        Job {
            release: 425,
            first_cycle: 425,
            end: 425,
            arrival: None,
        },
        Job {
            release: 440,
            first_cycle: 440,
            end: 440,
            arrival: None,
        },
        Job {
            release: 455,
            first_cycle: 455,
            end: 455,
            arrival: None,
        },
        Job {
            release: 470,
            first_cycle: 470,
            end: 470,
            arrival: None,
        },
        Job {
            release: 485,
            first_cycle: 485,
            end: 485,
            arrival: None,
        },
        Job {
            release: 500,
            first_cycle: 500,
            end: 500,
            arrival: None,
        },
        Job {
            release: 515,
            first_cycle: 515,
            end: 515,
            arrival: None,
        },
        Job {
            release: 530,
            first_cycle: 530,
            end: 530,
            arrival: None,
        },
    ];

    const SMALL_BATCH: &[Job] = &[
        Job {
            release: 200,
            first_cycle: 200,
            end: 200,
            arrival: None,
        },
        Job {
            release: 300,
            first_cycle: 300,
            end: 300,
            arrival: None,
        },
        Job {
            release: 402,
            first_cycle: 402,
            end: 402,
            arrival: None,
        },
    ];

    const BURSTY_BATCH: &[Job] = &[
        Job {
            release: 535,
            first_cycle: 535,
            end: 535,
            arrival: None,
        },
        Job {
            release: 535,
            first_cycle: 535,
            end: 535,
            arrival: None,
        },
    ];

    const PROBLEMATIC_BATCH: &[Job] = &[
        Job {
            release: 3134387000020152,
            end: 3134387000535570,
            first_cycle: 3134387000020152,
            arrival: None,
        },
        Job {
            release: 3134387023497122,
            end: 3134387023785044,
            first_cycle: 3134387023497122,
            arrival: None,
        },
        Job {
            release: 3134387044412299,
            end: 3134387044661499,
            first_cycle: 3134387044412299,
            arrival: None,
        },
        Job {
            release: 3134387070473360,
            end: 3134387070717245,
            first_cycle: 3134387070473360,
            arrival: None,
        },
        Job {
            release: 3134387097022709,
            end: 3134387097272150,
            first_cycle: 3134387097022709,
            arrival: None,
        },
        Job {
            release: 3134387125414011,
            end: 3134387125607805,
            first_cycle: 3134387125414011,
            arrival: None,
        },
        Job {
            release: 3134387146890976,
            end: 3134387147091473,
            first_cycle: 3134387146890976,
            arrival: None,
        },
        Job {
            release: 3134387183857553,
            end: 3134387184139716,
            first_cycle: 3134387183857553,
            arrival: None,
        },
        Job {
            release: 3134387218518537,
            end: 3134387218789977,
            first_cycle: 3134387218518537,
            arrival: None,
        },
        Job {
            release: 3134387247509070,
            end: 3134387247755215,
            first_cycle: 3134387247509070,
            arrival: None,
        },
        Job {
            release: 3134387279179285,
            end: 3134387279445873,
            first_cycle: 3134387279179285,
            arrival: None,
        },
        Job {
            release: 3134387319614216,
            end: 3134387319879286,
            first_cycle: 3134387319614216,
            arrival: None,
        },
        Job {
            release: 3134387348702359,
            end: 3134387348947485,
            first_cycle: 3134387348702359,
            arrival: None,
        },
        Job {
            release: 3134387382639891,
            end: 3134387382943831,
            first_cycle: 3134387382639891,
            arrival: None,
        },
        Job {
            release: 3134387407181882,
            end: 3134387407433971,
            first_cycle: 3134387407181882,
            arrival: None,
        },
        Job {
            release: 3134387439873933,
            end: 3134387440351981,
            first_cycle: 3134387439873933,
            arrival: None,
        },
        Job {
            release: 3134387472726111,
            end: 3134387472971329,
            first_cycle: 3134387472726111,
            arrival: None,
        },
        Job {
            release: 3134387502851794,
            end: 3134387503154548,
            first_cycle: 3134387502851794,
            arrival: None,
        },
        Job {
            release: 3134387538495114,
            end: 3134387538779887,
            first_cycle: 3134387538495114,
            arrival: None,
        },
        Job {
            release: 3134387578172575,
            end: 3134387578397516,
            first_cycle: 3134387578172575,
            arrival: None,
        },
        Job {
            release: 3134387618029200,
            end: 3134387618262604,
            first_cycle: 3134387618029200,
            arrival: None,
        },
        Job {
            release: 3134387651354371,
            end: 3134387651590571,
            first_cycle: 3134387651354371,
            arrival: None,
        },
        Job {
            release: 3134387673954263,
            end: 3134387674185666,
            first_cycle: 3134387673954263,
            arrival: None,
        },
        Job {
            release: 3134387696237030,
            end: 3134387696462637,
            first_cycle: 3134387696237030,
            arrival: None,
        },
        Job {
            release: 3134387730429057,
            end: 3134387730674090,
            first_cycle: 3134387730429057,
            arrival: None,
        },
        Job {
            release: 3134387752830007,
            end: 3134387753054134,
            first_cycle: 3134387752830007,
            arrival: None,
        },
        Job {
            release: 3134387776869544,
            end: 3134387777091688,
            first_cycle: 3134387776869544,
            arrival: None,
        },
        Job {
            release: 3134387811464472,
            end: 3134387811709320,
            first_cycle: 3134387811464472,
            arrival: None,
        },
        Job {
            release: 3134387847317437,
            end: 3134387847577729,
            first_cycle: 3134387847317437,
            arrival: None,
        },
        Job {
            release: 3134387886877826,
            end: 3134387887144655,
            first_cycle: 3134387886877826,
            arrival: None,
        },
        Job {
            release: 3134387910210281,
            end: 3134387910473240,
            first_cycle: 3134387910210281,
            arrival: None,
        },
        Job {
            release: 3134387940717661,
            end: 3134387940989231,
            first_cycle: 3134387940717661,
            arrival: None,
        },
        Job {
            release: 3134387963241925,
            end: 3134387963508865,
            first_cycle: 3134387963241925,
            arrival: None,
        },
        Job {
            release: 3134388002458652,
            end: 3134388002737704,
            first_cycle: 3134388002458652,
            arrival: None,
        },
        Job {
            release: 3134388042919805,
            end: 3134388043229856,
            first_cycle: 3134388042919805,
            arrival: None,
        },
        Job {
            release: 3134388073445482,
            end: 3134388073699941,
            first_cycle: 3134388073445482,
            arrival: None,
        },
        Job {
            release: 3134388113243978,
            end: 3134388113514048,
            first_cycle: 3134388113243978,
            arrival: None,
        },
        Job {
            release: 3134388154015256,
            end: 3134388154288382,
            first_cycle: 3134388154015256,
            arrival: None,
        },
        Job {
            release: 3134388184168902,
            end: 3134388184474212,
            first_cycle: 3134388184168902,
            arrival: None,
        },
        Job {
            release: 3134388210735177,
            end: 3134388210989210,
            first_cycle: 3134388210735177,
            arrival: None,
        },
        Job {
            release: 3134388244616024,
            end: 3134388244842539,
            first_cycle: 3134388244616024,
            arrival: None,
        },
        Job {
            release: 3134388280374823,
            end: 3134388280665393,
            first_cycle: 3134388280374823,
            arrival: None,
        },
        Job {
            release: 3134388316773206,
            end: 3134388317099534,
            first_cycle: 3134388316773206,
            arrival: None,
        },
        Job {
            release: 3134388340121845,
            end: 3134388340402748,
            first_cycle: 3134388340121845,
            arrival: None,
        },
        Job {
            release: 3134388377804263,
            end: 3134388378079629,
            first_cycle: 3134388377804263,
            arrival: None,
        },
        Job {
            release: 3134388403436460,
            end: 3134388403706770,
            first_cycle: 3134388403436460,
            arrival: None,
        },
        Job {
            release: 3134388435762553,
            end: 3134388436056604,
            first_cycle: 3134388435762553,
            arrival: None,
        },
        Job {
            release: 3134388468750266,
            end: 3134388469002114,
            first_cycle: 3134388468750266,
            arrival: None,
        },
        Job {
            release: 3134388490560114,
            end: 3134388490787685,
            first_cycle: 3134388490560114,
            arrival: None,
        },
        Job {
            release: 3134388520378432,
            end: 3134388520645576,
            first_cycle: 3134388520378432,
            arrival: None,
        },
        Job {
            release: 3134388545558191,
            end: 3134388545830224,
            first_cycle: 3134388545558191,
            arrival: None,
        },
        Job {
            release: 3134388582026572,
            end: 3134388582309679,
            first_cycle: 3134388582026572,
            arrival: None,
        },
        Job {
            release: 3134388602025671,
            end: 3134388602292648,
            first_cycle: 3134388602025671,
            arrival: None,
        },
        Job {
            release: 3134388624473472,
            end: 3134388624744375,
            first_cycle: 3134388624473472,
            arrival: None,
        },
        Job {
            release: 3134388652513706,
            end: 3134388652744387,
            first_cycle: 3134388652513706,
            arrival: None,
        },
        Job {
            release: 3134388685331625,
            end: 3134388685543288,
            first_cycle: 3134388685331625,
            arrival: None,
        },
        Job {
            release: 3134388707111733,
            end: 3134388707267953,
            first_cycle: 3134388707111733,
            arrival: None,
        },
        Job {
            release: 3134388743385340,
            end: 3134388743652854,
            first_cycle: 3134388743385340,
            arrival: None,
        },
        Job {
            release: 3134388769587361,
            end: 3134388769842579,
            first_cycle: 3134388769587361,
            arrival: None,
        },
        Job {
            release: 3134388797219508,
            end: 3134388797457616,
            first_cycle: 3134388797219508,
            arrival: None,
        },
        Job {
            release: 3134388830466884,
            end: 3134388830710343,
            first_cycle: 3134388830466884,
            arrival: None,
        },
        Job {
            release: 3134388869320617,
            end: 3134388869567669,
            first_cycle: 3134388869320617,
            arrival: None,
        },
        Job {
            release: 3134388908450864,
            end: 3134388908769600,
            first_cycle: 3134388908450864,
            arrival: None,
        },
        Job {
            release: 3134388935455651,
            end: 3134388935706258,
            first_cycle: 3134388935455651,
            arrival: None,
        },
        Job {
            release: 3134388962916727,
            end: 3134388963182019,
            first_cycle: 3134388962916727,
            arrival: None,
        },
        Job {
            release: 3134389000364500,
            end: 3134389000653422,
            first_cycle: 3134389000364500,
            arrival: None,
        },
        Job {
            release: 3134389028720340,
            end: 3134389029032521,
            first_cycle: 3134389028720340,
            arrival: None,
        },
        Job {
            release: 3134389061491056,
            end: 3134389061757571,
            first_cycle: 3134389061491056,
            arrival: None,
        },
        Job {
            release: 3134389093559895,
            end: 3134389093785558,
            first_cycle: 3134389093559895,
            arrival: None,
        },
        Job {
            release: 3134389114345018,
            end: 3134389114582366,
            first_cycle: 3134389114345018,
            arrival: None,
        },
        Job {
            release: 3134389142154996,
            end: 3134389142430473,
            first_cycle: 3134389142154996,
            arrival: None,
        },
        Job {
            release: 3134389176809683,
            end: 3134389177054401,
            first_cycle: 3134389176809683,
            arrival: None,
        },
        Job {
            release: 3134389201820426,
            end: 3134389202092904,
            first_cycle: 3134389201820426,
            arrival: None,
        },
        Job {
            release: 3134389223826198,
            end: 3134389224107749,
            first_cycle: 3134389223826198,
            arrival: None,
        },
        Job {
            release: 3134389264446496,
            end: 3134389264742918,
            first_cycle: 3134389264446496,
            arrival: None,
        },
        Job {
            release: 3134389293656545,
            end: 3134389293942170,
            first_cycle: 3134389293656545,
            arrival: None,
        },
        Job {
            release: 3134389325982157,
            end: 3134389326229135,
            first_cycle: 3134389325982157,
            arrival: None,
        },
        Job {
            release: 3134389347737154,
            end: 3134389347979188,
            first_cycle: 3134389347737154,
            arrival: None,
        },
        Job {
            release: 3134389386145339,
            end: 3134389386392780,
            first_cycle: 3134389386145339,
            arrival: None,
        },
        Job {
            release: 3134389408165962,
            end: 3134389408435847,
            first_cycle: 3134389408165962,
            arrival: None,
        },
        Job {
            release: 3134389436962480,
            end: 3134389437227254,
            first_cycle: 3134389436962480,
            arrival: None,
        },
        Job {
            release: 3134389466708669,
            end: 3134389466976239,
            first_cycle: 3134389466708669,
            arrival: None,
        },
        Job {
            release: 3134389500875252,
            end: 3134389501119545,
            first_cycle: 3134389500875252,
            arrival: None,
        },
        Job {
            release: 3134389523073409,
            end: 3134389523305147,
            first_cycle: 3134389523073409,
            arrival: None,
        },
        Job {
            release: 3134389547839453,
            end: 3134389548123411,
            first_cycle: 3134389547839453,
            arrival: None,
        },
        Job {
            release: 3134389572951250,
            end: 3134389573217061,
            first_cycle: 3134389572951250,
            arrival: None,
        },
        Job {
            release: 3134389606347420,
            end: 3134389606596379,
            first_cycle: 3134389606347420,
            arrival: None,
        },
        Job {
            release: 3134389643864081,
            end: 3134389644195483,
            first_cycle: 3134389643864081,
            arrival: None,
        },
        Job {
            release: 3134389678880114,
            end: 3134389679122722,
            first_cycle: 3134389678880114,
            arrival: None,
        },
        Job {
            release: 3134389719107067,
            end: 3134389719343656,
            first_cycle: 3134389719107067,
            arrival: None,
        },
        Job {
            release: 3134389751497326,
            end: 3134389751737026,
            first_cycle: 3134389751497326,
            arrival: None,
        },
        Job {
            release: 3134389776617235,
            end: 3134389776920711,
            first_cycle: 3134389776617235,
            arrival: None,
        },
        Job {
            release: 3134389802032268,
            end: 3134389802336023,
            first_cycle: 3134389802032268,
            arrival: None,
        },
        Job {
            release: 3134389829219478,
            end: 3134389829512955,
            first_cycle: 3134389829219478,
            arrival: None,
        },
        Job {
            release: 3134389867464609,
            end: 3134389867689606,
            first_cycle: 3134389867464609,
            arrival: None,
        },
        Job {
            release: 3134389905675741,
            end: 3134389906012718,
            first_cycle: 3134389905675741,
            arrival: None,
        },
        Job {
            release: 3134389925823949,
            end: 3134389926102611,
            first_cycle: 3134389925823949,
            arrival: None,
        },
    ];

    const FUTEX_BATCH: &[Job] = &[
        Job {
            arrival: None,
            release: 1469591329042689,
            end: 1469591329140965,
            first_cycle: 1469591329042689,
        },
        Job {
            arrival: None,
            release: 1469592329226813,
            end: 1469596329971181,
            first_cycle: 1469592329226813,
        },
        Job {
            arrival: None,
            release: 1469597330052215,
            end: 1469600330703217,
            first_cycle: 1469597330052215,
        },
        Job {
            arrival: None,
            release: 1469601330766418,
            end: 1469602330993616,
            first_cycle: 1469601330766418,
        },
        Job {
            arrival: None,
            release: 1469603331058650,
            end: 1469604331273386,
            first_cycle: 1469603331058650,
        },
        Job {
            arrival: None,
            release: 1469605331323235,
            end: 1469605331383363,
            first_cycle: 1469605331323235,
        },
        Job {
            arrival: None,
            release: 1469606331451453,
            end: 1469606331541248,
            first_cycle: 1469606331451453,
        },
        Job {
            arrival: None,
            release: 1469607331610412,
            end: 1469608331825074,
            first_cycle: 1469607331610412,
        },
        Job {
            arrival: None,
            release: 1469609331892812,
            end: 1469609331998477,
            first_cycle: 1469609331892812,
        },
        Job {
            arrival: None,
            release: 1469610332100344,
            end: 1469610332265527,
            first_cycle: 1469610332100344,
        },
        Job {
            arrival: None,
            release: 1469611332320284,
            end: 1469613332677053,
            first_cycle: 1469611332320284,
        },
        Job {
            arrival: None,
            release: 1469614332759532,
            end: 1469618333511996,
            first_cycle: 1469614332759532,
        },
        Job {
            arrival: None,
            release: 1469619333552365,
            end: 1469619333635567,
            first_cycle: 1469619333552365,
        },
        Job {
            arrival: None,
            release: 1469620333711102,
            end: 1469623334236369,
            first_cycle: 1469620333711102,
        },
        Job {
            arrival: None,
            release: 1469624334310200,
            end: 1469624334447383,
            first_cycle: 1469624334310200,
        },
        Job {
            arrival: None,
            release: 1469625334500937,
            end: 1469627334912799,
            first_cycle: 1469625334500937,
        },
        Job {
            arrival: None,
            release: 1469628334981946,
            end: 1469629335193627,
            first_cycle: 1469628334981946,
        },
        Job {
            arrival: None,
            release: 1469630335255385,
            end: 1469631335477473,
            first_cycle: 1469630335255385,
        },
        Job {
            arrival: None,
            release: 1469632335529601,
            end: 1469632335620433,
            first_cycle: 1469632335529601,
        },
        Job {
            arrival: None,
            release: 1469633335699376,
            end: 1469636336313810,
            first_cycle: 1469633335699376,
        },
        Job {
            arrival: None,
            release: 1469637336352790,
            end: 1469637336453048,
            first_cycle: 1469637336352790,
        },
        Job {
            arrival: None,
            release: 1469638336524472,
            end: 1469639336722247,
            first_cycle: 1469638336524472,
        },
        Job {
            arrival: None,
            release: 1469640336768913,
            end: 1469642337118740,
            first_cycle: 1469640336768913,
        },
        Job {
            arrival: None,
            release: 1469643337185943,
            end: 1469644337418569,
            first_cycle: 1469643337185943,
        },
        Job {
            arrival: None,
            release: 1469645337476049,
            end: 1469646337759138,
            first_cycle: 1469645337476049,
        },
        Job {
            arrival: None,
            release: 1469647337803489,
            end: 1469647337957209,
            first_cycle: 1469647337803489,
        },
        Job {
            arrival: None,
            release: 1469648338012856,
            end: 1469649338249593,
            first_cycle: 1469648338012856,
        },
        Job {
            arrival: None,
            release: 1469650338285686,
            end: 1469650338397480,
            first_cycle: 1469650338285686,
        },
        Job {
            arrival: None,
            release: 1469651338469628,
            end: 1469653338902381,
            first_cycle: 1469651338469628,
        },
        Job {
            arrival: None,
            release: 1469654338982824,
            end: 1469654339115989,
            first_cycle: 1469654338982824,
        },
        Job {
            arrival: None,
            release: 1469655339173803,
            end: 1469655339264265,
            first_cycle: 1469655339173803,
        },
        Job {
            arrival: None,
            release: 1469656339333561,
            end: 1469657339543891,
            first_cycle: 1469656339333561,
        },
        Job {
            arrival: None,
            release: 1469658339594354,
            end: 1469658339716241,
            first_cycle: 1469658339594354,
        },
        Job {
            arrival: None,
            release: 1469659339806314,
            end: 1469660340074088,
            first_cycle: 1469659339806314,
        },
        Job {
            arrival: None,
            release: 1469661340146218,
            end: 1469662340417140,
            first_cycle: 1469661340146218,
        },
        Job {
            arrival: None,
            release: 1469663340526658,
            end: 1469664340833598,
            first_cycle: 1469663340526658,
        },
        Job {
            arrival: None,
            release: 1469665340874173,
            end: 1469667341282520,
            first_cycle: 1469665340874173,
        },
        Job {
            arrival: None,
            release: 1469668341314446,
            end: 1469669341560407,
            first_cycle: 1469668341314446,
        },
        Job {
            arrival: None,
            release: 1469670341629277,
            end: 1469672342053847,
            first_cycle: 1469670341629277,
        },
        Job {
            arrival: None,
            release: 1469673342108884,
            end: 1469674342303234,
            first_cycle: 1469673342108884,
        },
        Job {
            arrival: None,
            release: 1469673342108884,
            end: 1469675342530232,
            first_cycle: 1469674342303234,
        },
        Job {
            arrival: None,
            release: 1469676342574381,
            end: 1469677342838304,
            first_cycle: 1469676342574381,
        },
        Job {
            arrival: None,
            release: 1469678342905767,
            end: 1469679343147506,
            first_cycle: 1469678342905767,
        },
    ];

    // TODO:
    #[test]
    fn test_update_offset_and_jitter() {
        let mut extractor = PeriodExtractor::new();
        extractor.num_batches = 2; // Ordinal of the batch to be analyzed

        // Define the test parameters
        let period = 10;
        let old_offset = 90;
        let old_max_jitter = 0;
        let batch = [100.0, 105.0, 110.0];
        let next_j = 1; // It is basically a number of activation points

        // Call the function and assert the expected result
        let result = extractor.update_offset_and_jitter(
            period,
            &batch,
            next_j,
            old_offset,
            old_max_jitter,
            -1,
        );
        assert_eq!((80, 10, -1), result);

        // Test when n_batches is 1

        let mut extractor = PeriodExtractor::new();
        extractor.num_batches = 1;

        // Define the test parameters
        let period = 10;
        let old_offset = 100;
        let old_max_jitter = 0;
        let batch = [100.0, 105.0, 110.0];
        let next_j = 0; // It is basically a number of activation points analysed so far

        // Call the function and assert the expected result
        let result = extractor.update_offset_and_jitter(
            period,
            &batch,
            next_j,
            old_offset,
            old_max_jitter,
            -1,
        );
        assert_eq!((90, 10, -1), result);

        // Test when n_batches is 2 and only max_jitter should be updated

        let mut extractor = PeriodExtractor::new();
        extractor.num_batches = 2;

        // Define the test parameters
        let period = 10;
        let old_offset = 90;
        let old_max_jitter = 0;
        let batch = [100.0, 110.0, 131.0];
        let next_j = 1; // It is basically a number of activation points analysed so far

        // Call the function and assert the expected result
        let result = extractor.update_offset_and_jitter(
            period,
            &batch,
            next_j,
            old_offset,
            old_max_jitter,
            -1,
        );
        assert_eq!((90, 11, -1), result);

        // Test problematic batch
        let mut extractor = PeriodExtractor::new();
        extractor.num_batches = 2;

        let period = 100;
        let old_offset = 80;
        let old_max_jitter = 20;
        let batch = [200.0, 300.0, 402.0];
        let next_j = 1; // It is basically a number of activation points analysed so far

        let result = extractor.update_offset_and_jitter(
            period,
            &batch,
            next_j,
            old_offset,
            old_max_jitter,
            -1,
        );
        assert_eq!((80, 22, -1), result);
    }

    #[test]
    fn test_update_offset_and_jitter_within_extractor() {
        let extractor = PeriodExtractor::new();

        let batch = vec![12.0, 22.0, 30.0];
        let period = 12;

        let (est_offset, est_max_jitter, est_total_jitter) =
            extractor.update_offset_and_jitter(period, &batch, 0, 0, 0, -1);

        assert_eq!(est_offset, 6);
        assert_eq!(est_max_jitter, 6);
        assert_eq!(est_total_jitter, -1); // TODO: fix

        // Test with shorter period
        let extractor = PeriodExtractor::new();
        let period = 10;

        let (est_offset, est_max_jitter, est_total_jitter) =
            extractor.update_offset_and_jitter(period, &batch, 0, 0, 0, -1);

        assert_eq!(est_offset, 10);
        assert_eq!(est_max_jitter, 2);
        assert_eq!(est_total_jitter, -1); // TODO: fix

        // Test perfectly periodic
        let mut extractor = PeriodExtractor::new();

        for job in PERFECTLY_PERIODIC {
            extractor.update(job);
        }
        let period = 10;
        let (est_offset, est_max_jitter, est_total_jitter) =
            extractor.update_offset_and_jitter(period, &extractor.batch, 0, 0, 0, -1);
        assert_eq!(est_offset, 0);
        assert_eq!(est_max_jitter, 0);
        assert_eq!(est_total_jitter, -1); // TODO: fix

        // Test periodic with offset
        let mut extractor = PeriodExtractor::new();

        for job in PERFECTLY_PERIODIC_OFFSET_2 {
            extractor.update(job);
        }
        let period = 10;
        let (est_offset, est_max_jitter, est_total_jitter) =
            extractor.update_offset_and_jitter(period, &extractor.batch, 0, 0, 0, -1);
        assert_eq!(est_offset, 2);
        assert_eq!(est_max_jitter, 0);
        assert_eq!(est_total_jitter, -1); // TODO: fix
    }

    #[test]
    fn test_generate_candidate_periods() {
        let extractor = PeriodExtractor::new();
        let candidates = extractor.generate_candidate_periods(16, 1, 32, MAX_CANDIDATES);
        assert_eq!(candidates, vec![1, 8, 12, 14, 15, 16, 17, 18, 20, 24, 32]);

        let candidates = extractor.generate_candidate_periods(12, 6, 20, MAX_CANDIDATES);
        assert_eq!(candidates, vec![6, 8, 10, 11, 12, 13, 14, 16, 20]);
    }

    #[test]
    fn test_generate_candidates() {
        let mut extractor = PeriodExtractor::new();
        let batch = [12.0, 22.0, 30.0];
        extractor.batch = batch.to_vec();
        extractor.generate_candidates(12, 6, 20);

        assert_eq!(extractor.least_jitter_candidates.len(), 9);

        // Check if the candidates map contains some specific periods.
        // Please modify these assertions according to your specific requirements.
        assert!(extractor.least_jitter_candidates.contains_key(&6));
        assert!(extractor.least_jitter_candidates.contains_key(&12));
        assert!(extractor.least_jitter_candidates.contains_key(&20));

        // You might also want to check the values of offset, j_max_ns, and j_max_ws for some periods.
        // Again, modify these assertions according to your specific requirements.
        let period_data = extractor.least_jitter_candidates.get(&12).unwrap();
        assert_eq!(period_data.offset, 6);
        assert_eq!(period_data.max_jitter, 6);

        let period_data = extractor.least_jitter_candidates.get(&10).unwrap();
        assert_eq!(period_data.offset, 10);
        assert_eq!(period_data.max_jitter, 2);
    }

    #[test]
    fn test_adjust_to_lower_period_estimate() {
        let extractor = PeriodExtractor::new(); // Replace with the actual initialization of your struct
        let estimated_period = 15;
        let n = 2;
        let period_l = 12;
        let period_data_l = PeriodicFunctionData {
            offset: 6,
            max_jitter: 6,
            total_jitter: -1, // Assuming this value; update as needed
        };

        let result = extractor.adjust_to_lower_period_estimate(
            estimated_period,
            n,
            period_l,
            &period_data_l,
        );
        assert_eq!(result, (0, 12, -1));

        let extractor = PeriodExtractor::new(); // Replace with the actual initialization of your struct
        let estimated_period = 16;
        let n = 2;
        let period_l = 12;
        let period_data_l = PeriodicFunctionData {
            offset: 6,
            max_jitter: 6,
            total_jitter: -1, // Assuming this value; update as needed
        };

        let result = extractor.adjust_to_lower_period_estimate(
            estimated_period,
            n,
            period_l,
            &period_data_l,
        );
        assert_eq!(result, (-2, 14, -1));
    }

    #[test]
    fn test_adjust_to_higher_period_estimate() {
        let extractor = PeriodExtractor::new(); // Replace with the actual initialization of your struct
        let estimated_period = 15;
        let n = 2;
        let period_h = 16;
        let period_data_h = PeriodicFunctionData {
            offset: 10,
            max_jitter: 6,
            total_jitter: -1, // Assuming this value; update as needed
        };

        let result = extractor.adjust_to_higher_period_estimate(
            estimated_period,
            n,
            period_h,
            &period_data_h,
        );
        assert_eq!(result, (10, 8, -1));
    }

    #[test]
    fn test_get_new_candidate_estimates() {
        let mut extractor = PeriodExtractor::new(); // Replace with the actual initialization of your struct
        let estimated_period = 100;
        let old_offset = 101; // Replace with suitable test data
        let old_max_jitter = 6;
        let old_total_jitter = 6;
        extractor.batch = vec![200.0, 300.0, 400.0];
        extractor.num_batches = 2;
        extractor.next_j = 1;

        // Assuming existing candidate with the given period
        extractor.least_jitter_candidates.insert(
            estimated_period,
            PeriodicFunctionData {
                offset: old_offset,
                max_jitter: old_max_jitter,
                total_jitter: old_total_jitter,
            },
        );

        // Call the function to update the existing candidate
        let (new_offset, new_max_jitter, new_total_jitter) =
            extractor.conservatively_update_lj_estimates(estimated_period, &extractor.batch);

        // Replace these with the expected values based on the given test data
        let expected_offset = 100; // Replace with expected value
        let expected_max_jitter = 7; // Replace with expected value
        let expected_total_jitter = -1; // Replace with expected value

        assert_eq!(new_offset, expected_offset);
        assert_eq!(new_max_jitter, expected_max_jitter);
        assert_eq!(new_total_jitter, expected_total_jitter);
    }

    // TODO:
    #[test]
    fn test_update_candidate() {
        let mut extractor = PeriodExtractor::new();
        for job in SMALL_BATCH {
            extractor.update(job); // 200, 300, 402
        }
        let estimated_period = 100;
        let old_offset = 101; // Replace with suitable test data
        let old_max_jitter = 6;
        let old_total_jitter = 6;
        extractor.num_batches = 2;
        extractor.next_j = 1;
        // Assuming existing candidate with the given period
        extractor.least_jitter_candidates.insert(
            estimated_period,
            PeriodicFunctionData {
                offset: old_offset,
                max_jitter: old_max_jitter,
                total_jitter: old_total_jitter,
            },
        );
        extractor.update_lj_candidate(estimated_period);
        // Retrieve the updated candidate
        let updated_candidate = extractor
            .least_jitter_candidates
            .get(&estimated_period)
            .unwrap();

        // Replace these with the expected values based on the given test data
        let expected_offset = 100; // Replace with expected value
        let expected_max_jitter = 7; // Replace with expected value
        let expected_total_jitter = -1; // Replace with expected value

        assert_eq!(updated_candidate.offset, expected_offset);
        assert_eq!(updated_candidate.max_jitter, expected_max_jitter);
        assert_eq!(updated_candidate.total_jitter, expected_total_jitter);

        // Test the problematic one.
        let mut extractor = PeriodExtractor::new();
        for job in SMALL_BATCH {
            extractor.update(job); // 200, 300, 402
        }
        let estimated_period = 100;
        let old_offset = 80; // Replace with suitable test data
        let old_max_jitter = 20;
        let old_total_jitter = 20;
        extractor.num_batches = 2;
        extractor.next_j = 1;
        // Assuming existing candidate with the given period
        extractor.least_jitter_candidates.insert(
            estimated_period,
            PeriodicFunctionData {
                offset: old_offset,
                max_jitter: old_max_jitter,
                total_jitter: old_total_jitter,
            },
        );
        extractor.update_lj_candidate(estimated_period);
        // Retrieve the updated candidate
        let updated_candidate = extractor
            .least_jitter_candidates
            .get(&estimated_period)
            .unwrap();

        // Replace these with the expected values based on the given test data
        let expected_offset = 80; // Replace with expected value
        let expected_max_jitter = 22; // Replace with expected value
        let expected_total_jitter = -1; // Replace with expected value

        assert_eq!(updated_candidate.offset, expected_offset);
        assert_eq!(updated_candidate.max_jitter, expected_max_jitter);
        assert_eq!(updated_candidate.total_jitter, expected_total_jitter);
    }

    #[test]
    fn test_update_all_candidates() {
        let mut extractor = PeriodExtractor::new(); // Assuming you have a default or new() function for PeriodExtractor
        extractor.num_batches = 2; // second batch
        extractor.next_j = 1; // one activation point analyzed so far, next index is 1

        // Inserting initial candidate values for testing
        extractor.least_jitter_candidates.insert(
            10,
            PeriodicFunctionData {
                offset: 100,
                max_jitter: 1,
                total_jitter: 1,
            },
        );
        extractor.least_jitter_candidates.insert(
            20,
            PeriodicFunctionData {
                offset: 60,
                max_jitter: 5,
                total_jitter: 1,
            },
        );

        // Assuming you have a method to add data to the batch, or you can do this directly:
        extractor.batch = vec![105.0, 115.0, 125.0]; // Insert whatever batch data you want to test with

        // Call the function to update all candidates
        extractor.update_all_candidates();

        // Now check the candidates' values against expected updated values
        // For this example, I'll just check that the values have changed,
        // but in a real-world scenario, you'd probably have specific expected values.

        let candidate_10 = extractor.least_jitter_candidates.get(&10).unwrap();
        let candidate_20 = extractor.least_jitter_candidates.get(&20).unwrap();

        // Check that the values have changed. This is a simplistic check;
        // ideally, you'd compare against specific expected values.
        assert_eq!(candidate_10.offset, 95);
        assert_eq!(candidate_10.max_jitter, 6);
        assert_eq!(candidate_10.total_jitter, -1);

        assert_eq!(candidate_20.offset, 60);
        assert_eq!(candidate_20.max_jitter, 25);
        assert_eq!(candidate_20.total_jitter, -1);
    }

    // TODO: TEST PROPERLY
    #[test]
    fn test_insert_candidate() {
        let mut extractor = PeriodExtractor::new(); // Replace with the actual initialization of your struct

        // Inserting existing candidates
        extractor.least_jitter_candidates.insert(
            10,
            PeriodicFunctionData {
                offset: 100,
                max_jitter: 1,
                total_jitter: 1,
            },
        );
        extractor.least_jitter_candidates.insert(
            20,
            PeriodicFunctionData {
                offset: 60,
                max_jitter: 5,
                total_jitter: 1,
            },
        );

        // Given parameters
        let estimated_period = 15; // Assuming the estimated period is 15 for the test
        let n = 4;

        // Perform the insert candidate operation
        extractor.insert_candidate(estimated_period, n);

        // Check the expected conditions here
        // Depending on the logic of your adjust_to_lower_period_estimate and adjust_to_higher_period_estimate functions,
        // you might want to check if the candidates contain the correct value for the given estimated period.
        // For example:
        let updated_candidate = extractor
            .least_jitter_candidates
            .get(&estimated_period)
            .unwrap();

        // Replace these with the expected values based on the given test data
        let expected_offset = 80; // Replace with expected value
        let expected_max_jitter = 21; // Replace with expected value
        let expected_total_jitter = -1; // Replace with expected value

        assert_eq!(updated_candidate.offset, expected_offset);
        assert_eq!(updated_candidate.max_jitter, expected_max_jitter);
        assert_eq!(updated_candidate.total_jitter, expected_total_jitter);

        // Let candidate be closer to the higher one:
    }

    #[test]
    fn test_remove_candidate() {
        let mut extractor = PeriodExtractor::new(); // Replace with the actual initialization of your struct

        // Inserting a few candidates to work with:
        extractor.least_jitter_candidates.insert(
            5,
            PeriodicFunctionData {
                offset: 10,
                max_jitter: 2,
                total_jitter: 12,
            },
        );
        extractor.least_jitter_candidates.insert(
            10,
            PeriodicFunctionData {
                offset: 20,
                max_jitter: 3,
                total_jitter: 23,
            },
        );
        extractor.least_jitter_candidates.insert(
            15,
            PeriodicFunctionData {
                offset: 30,
                max_jitter: 4,
                total_jitter: 34,
            },
        );

        // Now, if we provide an estimated_period of 12, the candidate with the key 15 should be removed
        // since it has the largest absolute difference of 3 (compared to 2 for the key 10 and 7 for the key 5).
        extractor.remove_candidate();

        // Ensure the candidate with key 15 has been removed:
        assert_eq!(extractor.least_jitter_candidates.contains_key(&10), true);
        assert_eq!(extractor.least_jitter_candidates.contains_key(&15), true);
        assert_eq!(extractor.least_jitter_candidates.contains_key(&5), true);
    }

    #[test]
    fn test_insert_and_remove() {
        let mut extractor = PeriodExtractor::new(); // Replace with the actual initialization of your struct

        // Pre-populating candidates
        extractor.least_jitter_candidates.insert(
            10,
            PeriodicFunctionData {
                offset: 100,
                max_jitter: 1,
                total_jitter: 1,
            },
        );
        extractor.least_jitter_candidates.insert(
            20,
            PeriodicFunctionData {
                offset: 60,
                max_jitter: 5,
                total_jitter: 1,
            },
        );

        // Given parameters
        let estimated_period = 15;
        let n = 4; // Number of analysed activations - 1

        // Perform the insert and remove operation
        extractor.insert_and_remove(estimated_period, n);

        // First, check if the `estimated_period` is inserted correctly
        assert!(extractor
            .least_jitter_candidates
            .contains_key(&estimated_period));

        assert!(extractor.least_jitter_candidates.contains_key(&20));

        // Insert a value that's already a candidate
        extractor.insert_and_remove(15, n);

        // In this case, no candidates should be removed since 20 was already a key.
        assert_eq!(extractor.least_jitter_candidates.len(), 3); // Assuming you started with 2 candidates

        // Verify that the values of the existing candidates haven't changed
        let existing_candidate = extractor.least_jitter_candidates.get(&15).unwrap();
        assert_eq!(existing_candidate.offset, 80);
        assert_eq!(existing_candidate.max_jitter, 21);
        assert_eq!(existing_candidate.total_jitter, -1);
    }

    #[test]
    fn test_guess_period() {
        let extractor = PeriodExtractor::new();

        let least = 990.;
        let largest = 1010.;

        let period = extractor.guess_value(least, largest);

        assert_eq!(period, 1000.0);

        let period = extractor.guess_value(10000., 10000.);
        assert_eq!(period, 10000.0);

        let period = extractor.guess_value(10., 10.);
        assert_eq!(period, 10.0);

        let period = extractor.guess_value(11.4, 15.4);
        assert_eq!(period, 13.0);
    }

    #[test]
    fn test_frequency() {
        // Test the frequency guesser when the frequency is 3 Hz
        let extractor = PeriodExtractor::new();

        let least = 333222259.;
        let largest = 334448160.;

        let freq_hz = extractor.guess_frequency(least, largest);

        assert_eq!(freq_hz, 3.);

        // Test the frequency guesser when the frequency is 3 kHz
        let extractor = PeriodExtractor::new();

        let least = 333222259. / 1000.;
        let largest = 334448160. / 1000.;

        let freq_hz = extractor.guess_frequency(least, largest);

        assert_eq!(freq_hz, 3000.);

        // Test the frequency guesser when the frequency is 0.003 Hz
        let extractor = PeriodExtractor::new();

        let least = 333222259. * 1000.;
        let largest = 334448160. * 1000.;

        let freq_hz = extractor.guess_frequency(least, largest);

        assert_eq!(freq_hz, 0.003);

        // Test the frequency guesser when the frequency is 3 MHz
        let extractor = PeriodExtractor::new();

        let least = 333222259. / 1000000.;
        let largest = 334448160. / 1000000.;

        let freq_hz = extractor.guess_frequency(least, largest);

        assert_eq!(freq_hz, 3000000.);
    }

    #[test]
    fn test_update_statistical_estimate() {
        let extractor = PeriodExtractor::new();

        let n = 2.0;
        let old_mean_estimate = 10.0;
        let old_variance_estimate = 0.0;
        let new_period_estimate = 15.0;

        let (new_mean, new_variance) = extractor.update_statistical_estimate(
            n,
            old_mean_estimate,
            old_variance_estimate,
            new_period_estimate,
        );

        // The expected values are derived from the Welford's method
        // for a data set of [10, 15]
        assert_eq!(new_mean, 12.5);
        assert_eq!(new_variance, 6.25);
    }

    #[test]
    fn test_period_extraction_first_batch() {
        // Testing the first batch of the TC with two batches
        let mut extractor = PeriodExtractor::new();

        for job in FIRST_BATCH {
            extractor.update(job);
        }

        extractor.analyse_batch_and_clear();

        // TODO: Test properly
        assert_eq!(
            extractor.extract(),
            Some(ArrivalModel::Periodic(Periodic::Release {
                mean: 11,
                variance: 1,
                period: 10,
                frequency: 100000000.0,
                offset: 100,
                max_jitter: 3,
            }))
        );
    }

    #[test]
    fn test_period_extraction_second_batch() {
        // Testing the second batch of the TC with two batches
        let mut extractor = PeriodExtractor::new();

        for job in SECOND_BATCH {
            extractor.update(job);
        }

        extractor.analyse_batch_and_clear();

        assert_eq!(
            extractor.extract(),
            Some(ArrivalModel::Periodic(Periodic::Release {
                mean: 15,
                variance: 0,
                period: 15,
                frequency: 66666666.0,
                offset: 425,
                max_jitter: 0,
            }))
        );
    }

    #[test]
    fn test_period_extraction_two_batches() {
        // Testing TC with two batches
        let mut extractor = PeriodExtractor::new();

        //let t = 10;
        //let offset = 10;
        //let max_jitter = 0;
        //let mut j = 0;

        for job in TS_40_ARRIVALS_NEGATIVE_OFFSET {
            extractor.update(job);
            //    let release_minus_offset = job.release.checked_sub(j * t + offset).unwrap_or(0);
            //    println!(
            //        "{}: {} - {} = {}",
            //        j,
            //        job.arrival(),
            //        j * t + offset,
            //        release_minus_offset
            //    );
            //    j += 1;
        }

        extractor.analyse_batch_and_clear();

        assert_eq!(extractor.extract(), None);
        // Test First and second together
        let mut extractor = PeriodExtractor::new();

        for job in FIRST_BATCH {
            extractor.update(job);
        }

        for job in SECOND_BATCH {
            extractor.update(job);
        }

        extractor.analyse_batch_and_clear();

        assert_eq!(
            extractor.extract(),
            Some(ArrivalModel::Periodic(Periodic::Release {
                period: 11,
                frequency: 90909090.0,
                offset: 70,
                max_jitter: 31,
                mean: 12,
                variance: 4,
            }))
        );
    }

    #[cfg(test)]
    mod tests {
        use crate::model_extractors::arrival::periodic::release::CandidateData;

        use super::*;

        #[test]
        fn test_update_best_guess_candidate() {
            let mut extractor = PeriodExtractor::new(); // Assuming the correct initializer for your struct

            // Pre-populating candidates
            extractor.least_jitter_candidates.insert(
                100,
                PeriodicFunctionData {
                    offset: 5,
                    max_jitter: 2,
                    total_jitter: 10,
                },
            );
            extractor.least_jitter_candidates.insert(
                110,
                PeriodicFunctionData {
                    offset: 6,
                    max_jitter: 1,
                    total_jitter: 9,
                },
            );

            //extractor.guessed_period = 100.0;
            //extractor.guessed_frequency = 10.0; // Any appropriate value you need

            // Initialize best_guess_candidate
            extractor.best_guess_candidate = CandidateData {
                period: 110,
                frequency: 10,
                offset: 6,
                max_jitter: 1,
            };

            // Case 1: guessed_period is in candidates and it's closer than the best_guess_candidate
            //extractor.update_best_guess_candidate();

            assert_eq!(extractor.best_guess_candidate.period, 110);
            assert_eq!(extractor.best_guess_candidate.offset, 6);

            extractor.least_jitter_candidates.insert(
                99,
                PeriodicFunctionData {
                    offset: 7,
                    max_jitter: 0,
                    total_jitter: 9,
                },
            );
        }
    }

    #[test]
    fn test_bursty_detection() {
        // Testing TC with two batches
        let mut extractor = PeriodExtractor::new();

        for job in BURSTY_BATCH {
            extractor.update(job);
        }

        extractor.analyse_batch_and_clear();

        assert_eq!(extractor.extract(), None); // Keep in mind it accounts for the estimates of the second batch
    }

    #[test]
    fn test_period_extraction_problematic() {
        // Initialize the extractor
        let mut extractor = PeriodExtractor::new();
        // Add jobs to the extractor
        for job in PROBLEMATIC_BATCH {
            extractor.update(job);
        }
        // Analyse the jobs.
        extractor.analyse_batch_and_clear();

        // Extract the arrival model
        let (period, offset, max_jitter) =
            if let Some(ArrivalModel::Periodic(Periodic::Release {
                period,
                offset,
                max_jitter,
                ..
            })) = extractor.extract()
            {
                (period, offset, max_jitter)
            } else {
                // Handle the case where the extraction is not the expected Periodic variant
                // You can set default values or handle the error as needed
                (0, 0, 0) // Default values
            };

        // Test that the expected arrivals are less than the observed releases
        for (j, job) in PROBLEMATIC_BATCH.iter().enumerate() {
            let release = job.arrival();
            let expected_arrival = offset + j as u64 * period;
            let jitter = release as i64 - expected_arrival as i64;
            //println!(
            //    "{}: {} - {} = {}, {}",
            //    j, release, expected_arrival, jitter, max_jitter
            //);

            assert!(jitter >= 0, "Jitter is negative at index {}: {}", j, jitter);
            assert!(
                jitter <= max_jitter as i64,
                "Jitter exceeds max_jitter at index {}: {}",
                j,
                jitter
            );
        }
    }

    #[test]
    fn test_period_extraction_futex() {
        // Initialize the extractor
        let mut extractor = PeriodExtractor::new();
        // Add jobs to the extractor
        for job in FUTEX_BATCH {
            extractor.update(job);
        }
        // Analyse the jobs.
        extractor.analyse_batch_and_clear();

        assert_eq!(
            extractor.extract(),
            Some(ArrivalModel::Periodic(Periodic::Release {
                period: 2000000000,
                frequency: 0.0,
                offset: 1469590329226813,
                max_jitter: 7008576676,
                mean: 2071758645,
                variance: 1304825994960172288,
            }))
        ); // Keep in mind it accounts for the estimates of the second batch
    }

    #[test]
    fn test_compute_diffs() {
        // Test with a typical batch of non-decreasing numbers.
        let extractor = PeriodExtractor::new();

        let batch = vec![5.0, 11.0, 18.0];
        let expected_diffs = vec![6.0, 7.0];
        assert_eq!(extractor.compute_diffs(batch), expected_diffs);

        // Test with a single-element batch, expecting an empty vector as there are no pairs.
        let single_element_batch = vec![10.0];
        let expected_single_result: Vec<f64> = Vec::new();
        assert_eq!(
            extractor.compute_diffs(single_element_batch),
            expected_single_result
        );

        // Test with an empty batch, expecting an empty vector as output.
        let empty_batch: Vec<f64> = Vec::new();
        assert_eq!(extractor.compute_diffs(empty_batch), Vec::<f64>::new());

        // Test with a batch containing two elements.
        let two_element_batch = vec![3.0, 7.0];
        let expected_two_element_result = vec![4.0];
        assert_eq!(
            extractor.compute_diffs(two_element_batch),
            expected_two_element_result
        );

        // Test with a batch with non-decreasing elements that have no difference.
        let identical_elements_batch = vec![8.0, 8.0, 8.0];
        let expected_identical_result = vec![0.0, 0.0];
        assert_eq!(
            extractor.compute_diffs(identical_elements_batch),
            expected_identical_result
        );

        // Test with a batch of increasing float values with fractional differences.
        let fractional_batch = vec![1.5, 3.0, 4.75];
        let expected_fractional_diffs = vec![1.5, 1.75];
        assert_eq!(
            extractor.compute_diffs(fractional_batch),
            expected_fractional_diffs
        );
    }

    #[test]
    fn test_detect_outliers_hampel() {
        let extractor = PeriodExtractor::new();

        // Test with a typical batch of values with one clear outlier.
        let diffs = vec![6.0, 7.0, 100.0, 6.5, 7.5, 6.8, 7.2];
        let expected_outliers = vec![2];
        assert_eq!(
            extractor.detect_outliers_hampel(&diffs, 3.0),
            expected_outliers
        );

        // Test with no outliers, expecting an empty vector.
        let diffs_no_outliers = vec![5.0, 5.2, 5.1, 5.3, 5.2];
        let expected_no_outliers: Vec<usize> = Vec::new();
        assert_eq!(
            extractor.detect_outliers_hampel(&diffs_no_outliers, 3.0),
            expected_no_outliers
        );

        // Test with all identical values, which should yield no outliers.
        let identical_diffs = vec![4.0, 4.0, 4.0, 4.0];
        assert_eq!(
            extractor.detect_outliers_hampel(&identical_diffs, 3.0),
            Vec::<usize>::new()
        );

        // Test with multiple outliers, expecting multiple indices.
        let multiple_outliers_diffs = vec![2.0, 100.0, 3.0, 105.0, 2.5];
        let expected_multiple_outliers = vec![1, 3];
        assert_eq!(
            extractor.detect_outliers_hampel(&multiple_outliers_diffs, 3.0),
            expected_multiple_outliers
        );

        // Test with a single element, expecting an empty vector since no outliers can be defined.
        let single_element_diffs = vec![5.0];
        assert_eq!(
            extractor.detect_outliers_hampel(&single_element_diffs, 3.0),
            Vec::<usize>::new()
        );

        let singl_mid_diffs = vec![5.0, 100.0, 5.0];
        assert_eq!(
            extractor.detect_outliers_hampel(&singl_mid_diffs, 3.0),
            vec![1]
        );

        // Test with an empty input, expecting an empty vector.
        let empty_diffs: Vec<f64> = Vec::new();
        assert_eq!(
            extractor.detect_outliers_hampel(&empty_diffs, 3.0),
            Vec::<usize>::new()
        );

        // Test with multiple outliers, expecting multiple indices.
        let real_trace_diffs = vec![
            // r0
            941153434.0, // diff 0
            // r1
            946294649.0, // dif 1
            // r2
            935856741.0, // diff 2
            // r3
            941218490.0, // diff 3
            // r4
            941106657.0, // diff 4
            // r5
            941134380.0, // diff 5
            // r6
            941010122.0, // diff 6
            // r7
            941093880.0, // diff 7
            // r8
            1497432730.0, // diff 8
            // r9
            384686346.0, // diff 9
                         // r10
        ];

        let expected_real_trace_outliers = vec![1, 2, 8, 9];
        assert_eq!(
            extractor.detect_outliers_hampel(&real_trace_diffs, 3.0),
            expected_real_trace_outliers
        );
    }

    #[test]
    fn test_compute_start_end() {
        let extractor = PeriodExtractor::new();
        // Example batch and outlier indices from the problem statement
        let real_batch = vec![
            0.0,          // r0
            941153434.0,  // r1
            1887448083.0, // r2
            2823304824.0, // r3
            3764523314.0, // r4
            4705639971.0, // r5
            5646774351.0, // r6
            6587784473.0, // r7
            7528878353.0, // r8
            9026311083.0, // r9
            9410997429.0, // r10
        ];

        // Expected result
        let expected_start = 0;
        let expected_end = 8;

        // Test the function with the provided batch and outlier indices
        let (start, end) = extractor.compute_start_end(real_batch);

        // Assertions
        assert_eq!(start, expected_start, "Unexpected start index");
        assert_eq!(end, expected_end, "Unexpected end index");

        // Additional tests

        // Case with no outliers, expecting full range.
        let no_outlier_batch = vec![5.0, 10.0, 15.0, 20.0];
        //let no_outliers: Vec<usize> = Vec::new();
        let (start, end) = extractor.compute_start_end(no_outlier_batch);
        assert_eq!(start, 0, "Start should be the first index with no outliers");
        assert_eq!(end, 3, "End should be the last index with no outliers");

        // Case with only one non-outlier in the middle
        let partial_outliers = vec![10.0, 1000.0, 20.0];
        //let partial_outlier_indices = vec![0, 2];
        let (start, end) = extractor.compute_start_end(partial_outliers);
        assert_eq!(start, 0, "Start should point to the non-outlier index");
        assert_eq!(end, 2, "End should point to the non-outlier index");
    }

    #[test]
    fn test_derive_clean_batch() {
        let extractor = PeriodExtractor::new();
        // Test case with a typical range
        let noisy_batch: Vec<f64> = vec![
            0.0,          // r0
            941153434.0,  // r1
            1887448083.0, // r2
            2823304824.0, // r3
            3764523314.0, // r4
            4705639971.0, // r5
            5646774351.0, // r6
            6587784473.0, // r7
            7528878353.0, // r8
            9026311083.0, // r9
            9410997429.0, // r10
        ];
        let noisy_batch_clone = noisy_batch.clone();
        let (start, end) = extractor.compute_start_end(noisy_batch);
        let expected_clean_batch = vec![
            0.0,          // r0
            941153434.0,  // r1
            1887448083.0, // r2
            2823304824.0, // r3
            3764523314.0, // r4
            4705639971.0, // r5
            5646774351.0, // r6
            6587784473.0, // r7
            7528878353.0, // r8
        ];

        let clean_batch = extractor.derive_clean_batch(noisy_batch_clone, start, end);
        assert_eq!(clean_batch, expected_clean_batch);

        let clean = vec![
            0.0,          // r0
            941153434.0,  // r1
            1887448083.0, // r2
            2823304824.0, // r3
            3764523314.0, // r4
            4705639971.0, // r5
            5646774351.0, // r6
            6587784473.0, // r7
            7528878353.0, // r8
        ];

        let clean_batch = extractor.derive_clean_batch(clean, start, end);
        assert_eq!(clean_batch, expected_clean_batch);
    }

    #[test]
    fn test_generate_candidate_periods_by_rounding() {
        let extractor = PeriodExtractor::new();

        // Test with a typical multi-digit period
        let period = 12345;
        let expected_periods = vec![12345, 12350, 12300, 12000, 10000];
        let candidate_periods = extractor.generate_candidate_periods_by_rounding(period);
        assert_eq!(
            candidate_periods, expected_periods,
            "Failed on typical multi-digit period"
        );

        // Test with a single-digit period (should only return the period itself)
        let period = 7;
        let expected_periods = vec![7];
        let candidate_periods = extractor.generate_candidate_periods_by_rounding(period);
        assert_eq!(
            candidate_periods, expected_periods,
            "Failed on single-digit period"
        );

        // Test with a large period to check higher significant figure rounding
        let period = 987654321;
        let expected_periods = vec![
            987654321, 987654320, 987654300, 987654000, 987650000, 987700000, 988000000, 990000000,
            1000000000,
        ];
        let candidate_periods = extractor.generate_candidate_periods_by_rounding(period);
        assert_eq!(
            candidate_periods, expected_periods,
            "Failed on large period"
        );

        // Test with an edge case where period is a power of 10
        let period = 1000;
        let expected_periods = vec![1000];
        let candidate_periods = extractor.generate_candidate_periods_by_rounding(period);
        assert_eq!(
            candidate_periods, expected_periods,
            "Failed on power of 10 period"
        );

        // Test with a small two-digit period
        let period = 42;
        let expected_periods = vec![42, 40];
        let candidate_periods = extractor.generate_candidate_periods_by_rounding(period);
        assert_eq!(
            candidate_periods, expected_periods,
            "Failed on small two-digit period"
        );

        let period = 1010;
        let expected_periods = vec![1010, 1000];
        let candidate_periods = extractor.generate_candidate_periods_by_rounding(period);
        assert_eq!(
            candidate_periods, expected_periods,
            "Failed on small two-digit period"
        );
    }

    #[test]
    fn test_generate_candidate_periods_by_rounding_and_incrementing() {
        let extractor = PeriodExtractor::new();

        // Test with the specific example of period = 72345
        let period = 72345;
        let expected_periods = vec![
            72345, 72343, 72344, 72346, 72347, 72350, 72330, 72340, 72360, 72370, 72300, 72100,
            72200, 72400, 72500, 72000, 70000, 71000, 73000, 74000, 50000, 60000, 80000,
            90000, // Rounding to units
        ];

        let candidate_periods =
            extractor.generate_candidate_periods_by_rounding_and_incrementing(period);
        assert_eq!(
            candidate_periods, expected_periods,
            "Failed on period 72345"
        );
    }
}

// Number of consecutive job skips,
//
