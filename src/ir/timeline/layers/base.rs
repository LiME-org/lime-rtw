//! Base layer.

use std::collections::VecDeque;

use crate::interval::{Bound, Interval, IntervalSeq, TimeInterval};

/// Interval iterator
///
/// Built by the `BaseLayer`'s `plain_intervals` method.
pub struct Intervals<I> {
    inner: I,
}

impl<I> Intervals<I>
where
    I: Iterator,
{
    pub fn new(inner: I) -> Self {
        Self { inner }
    }
}

impl<I: Iterator> Iterator for Intervals<I> {
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

/// A compressible layer.
///
/// This is the basic layer structre.
/// Any other layer is expected to wrap this structure and implement
/// the `Layer` trait.
pub struct BaseLayer {
    compressed: VecDeque<(TimeInterval, u64)>,
    plains: IntervalSeq<u64>,
    curr_interval: Option<TimeInterval>,
}

impl BaseLayer {
    pub fn new() -> Self {
        Self {
            compressed: VecDeque::new(),
            plains: IntervalSeq::new(),
            curr_interval: None,
        }
    }

    fn push_compressed_block(&mut self, block: (TimeInterval, u64)) {
        self.compressed.push_back(block)
    }

    fn merge_compressed_block(&mut self, block: (TimeInterval, u64)) {
        match self.compressed.back_mut() {
            Some(last) => {
                assert!(last.0.upper_bound() <= block.0.lower_bound());
                assert!(last.0.is_disjoint_with(&block.0));

                last.0 = last.0.extent(&block.0);
                last.1 += block.1;
            }

            None => self.compressed.push_back(block),
        }
    }

    /// Compress all plain blocks in the range `(-inf, until]`.
    /// If `new_block` is true, the new compressed block is added to the compressed range.
    /// Otherwise, it is merged with the last compressed block.
    pub fn compress(&mut self, until: u64, new_block: bool) {
        let compressed = self.plains.compress_until(until);

        if new_block {
            if let Some(v) = compressed {
                self.push_compressed_block(v)
            }
        } else if let Some(v) = compressed {
            self.merge_compressed_block(v)
        }
    }

    /// Compress all plain blocks.
    /// If `new_block` is true, the new compressed block is added to the compressed range.
    /// Otherwise, it is merged with the last compressed block.
    pub fn compress_all(&mut self, new_block: bool) {
        let until = u64::MAX;

        self.compress(until, new_block);
    }

    /// Set the lower bound of the last interval.
    pub fn push_lower_bound(&mut self, lower: u64) {
        self.curr_interval.replace(Interval::infinite_right(lower));
    }

    /// Set the upper bound of the last interval.
    /// All future call to this method is a nop until `push_lower_bound` is invoked.
    pub fn push_upper_bound(&mut self, upper: u64) {
        if let Some(mut interval) = self.curr_interval.take() {
            interval.set_upper_bound(upper);
            self.plains.push(interval);
        }
    }

    /// Returns true if there is no compressed intervals in `range`.
    pub fn only_plain_intervals_in(&self, range: &TimeInterval) -> bool {
        if let Some(last) = self.compressed.back() {
            return last.0.upper_bound() < range.lower_bound();
        }

        true
    }

    /// Returns all the compressed intervals in the range `[start, inf)`.
    pub fn compressed(&self, start: u64) -> impl Iterator<Item = &(TimeInterval, u64)> + '_ {
        self.compressed
            .iter()
            .skip_while(move |&(i, _)| i.lower_bound() < Bound::Value(start))
    }

    /// Returns all the plain intervals in `range`.
    pub fn plain_intervals<'a>(
        &'a mut self,
        range: &'a TimeInterval,
    ) -> Intervals<impl Iterator<Item = TimeInterval> + 'a> {
        Intervals::new(
            self.plains.intersection(range.clone()).chain(
                self.curr_interval
                    .iter()
                    .map_while(move |i| i.intersection(range)),
            ),
        )
    }

    /// Discards all intervals not in tracking range.
    pub fn trim(&mut self, tracking_range: &TimeInterval) {
        while let Some(d) = self.compressed.front() {
            if d.0.lower_bound() < tracking_range.lower_bound() {
                self.compressed.pop_front();
            } else {
                break;
            }
        }

        self.plains.intersection_inplace(tracking_range);
    }

    /// Returns the sum of the span of the contained interval intersected with
    /// `range`. Panics if there is any infinite span.
    pub fn coverage(&mut self, range: &TimeInterval) -> u64 {
        let mut ret = 0;

        if let Bound::Value(start) = range.lower_bound() {
            ret += self.compressed(start).map(|x| x.1).sum::<u64>();
        }

        ret += self
            .plain_intervals(range)
            .map(|x| x.duration_unwrap())
            .sum::<u64>();

        ret
    }

    /// Retuns the number of plain intervals.
    pub(crate) fn plain_interval_nb(&self) -> usize {
        self.plains.len()
    }
}

impl Default for BaseLayer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::interval::TimeInterval;

    use super::BaseLayer;

    #[test]
    pub fn test_compression() {
        let mut layer = BaseLayer::new();

        let mut expected_plains = Vec::new();

        for i in 0..10 {
            layer.push_lower_bound(i * 3);
            layer.push_upper_bound(i * 3 + 2);
            expected_plains.push(TimeInterval::range(i * 3, i * 3 + 2));
        }

        layer.compress(6, true);

        let compressed = layer
            .compressed(0)
            .cloned()
            .collect::<Vec<(TimeInterval, u64)>>();
        assert_eq!(compressed, &[(TimeInterval::range(0, 6), 4)]);

        layer.compress(15, false);
        let compressed = layer
            .compressed(0)
            .cloned()
            .collect::<Vec<(TimeInterval, u64)>>();
        let plains = layer.plains.iter().cloned().collect::<Vec<TimeInterval>>();
        assert_eq!(compressed, &[(TimeInterval::range(0, 15), 10)]);
        assert_eq!(plains, expected_plains[5..]);

        let mut layer = BaseLayer::new();

        for i in 0..10 {
            layer.push_lower_bound(i * 3);
            layer.push_upper_bound(i * 3 + 2);
        }

        layer.compress(6, true);

        let compressed = layer
            .compressed(0)
            .cloned()
            .collect::<Vec<(TimeInterval, u64)>>();
        assert_eq!(compressed, &[(TimeInterval::range(0, 6), 4)]);
        let plains = layer.plains.iter().cloned().collect::<Vec<TimeInterval>>();
        assert_eq!(plains, expected_plains[2..]);

        layer.compress(15, true);
        let compressed = layer
            .compressed(0)
            .cloned()
            .collect::<Vec<(TimeInterval, u64)>>();
        let plains = layer.plains.iter().cloned().collect::<Vec<TimeInterval>>();
        assert_eq!(
            compressed,
            &[
                (TimeInterval::range(0, 6), 4),
                (TimeInterval::range(6, 15), 6)
            ]
        );
        assert_eq!(plains, expected_plains[5..]);

        layer.trim(&TimeInterval::infinite_right(6));
        let compressed = layer
            .compressed(0)
            .cloned()
            .collect::<Vec<(TimeInterval, u64)>>();
        assert_eq!(compressed, &[(TimeInterval::range(6, 15), 6)]);
        let plains = layer.plains.iter().cloned().collect::<Vec<TimeInterval>>();
        assert_eq!(plains, expected_plains[5..]);

        layer.trim(&TimeInterval::infinite_right(21));
        let compressed = layer
            .compressed(0)
            .cloned()
            .collect::<Vec<(TimeInterval, u64)>>();
        assert_eq!(compressed, &[]);
        let plains = layer.plains.iter().cloned().collect::<Vec<TimeInterval>>();
        assert_eq!(plains, expected_plains[7..]);
    }
}
