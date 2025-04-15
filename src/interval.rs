//! Time intervals.
//!
//! Handling time intervals and sequence of intervals is necessary for querying
//! and maintaining the IR. This module contains types for intervals and
//! sequence of intervals that can be intersected efficiently.
//!
//! By default, intervals are half-open. However, both ends can be unbounded.
//! Interval bounds are specified using the `Bound` enum.
//!
//! ```
//! use lime_rtw::interval::{Bound, Interval};
//!     
//! let a = Bound::Value(1);
//! let b = Bound::Value(5);
//!
//! let i1 = Interval::new(a, b); // constructs [a, b).
//! let i2 = Interval::new(a, Bound::Infinity); // constructs [a, inf).
//! let i3 = Interval::new(Bound::MinusInfinity, a); // constructs (-inf, a).
//! let i4 = Interval::<i32>::new(Bound::MinusInfinity, Bound::Infinity); // constructs (-inf, inf).
//!
//! // The use of the `Bound` type can be precluded by using these constructors.
//! let i5 = Interval::range(1, 5); // constructs [a, b).
//! let i6 = Interval::infinite_right(1); // constructs [a, inf).
//! let i7 = Interval::<i32>::infinite(); // constructs (-inf, inf).
//! ```

use std::{
    collections::{BTreeMap, VecDeque},
    fmt::Debug,
    hash::Hash,
    ops::Sub,
};

/// An interval bound
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum Bound<T: Eq + Ord + Copy + Clone> {
    MinusInfinity,
    Infinity,
    Value(T),
}

impl<T: Eq + Ord + Copy + Clone> Bound<T> {
    pub fn value(&self) -> Option<T> {
        match self {
            Value(value) => Some(*value),
            _ => None,
        }
    }
}

use Bound::*;

impl<T: Eq + Ord + Copy> Ord for Bound<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self {
            Infinity => match other {
                Infinity => std::cmp::Ordering::Equal,
                MinusInfinity | Value(_) => std::cmp::Ordering::Greater,
            },
            MinusInfinity => match other {
                MinusInfinity => std::cmp::Ordering::Equal,
                _ => std::cmp::Ordering::Less,
            },
            Value(a) => match other {
                MinusInfinity => std::cmp::Ordering::Greater,
                Infinity => std::cmp::Ordering::Less,
                Value(b) => a.cmp(b),
            },
        }
    }
}

impl<T: Eq + Ord + Copy> PartialOrd for Bound<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// An half-open interval.
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct Interval<T: Eq + Ord + Copy> {
    a: Bound<T>,
    b: Bound<T>,
}

impl<T: Ord + Eq + Copy + Clone> Interval<T> {
    pub fn new(a: Bound<T>, b: Bound<T>) -> Self {
        Self { a, b }
    }

    /// Builds the smallest interval covering `self` and `other`.
    ///
    ///
    /// ```
    /// use lime_rtw::interval::Interval;
    ///
    /// let a = Interval::range(0, 1);
    /// let b = Interval::range(2, 3);
    ///
    /// assert_eq!(a.extent(&b), Interval::range(0 ,3))
    /// ```
    pub fn extent(&self, other: &Interval<T>) -> Interval<T> {
        let a = self.a.min(other.a);
        let b = self.b.max(other.b);

        Interval::new(a, b)
    }

    pub fn lower_bound(&self) -> Bound<T> {
        self.a
    }

    pub fn upper_bound(&self) -> Bound<T> {
        self.b
    }

    /// Returns true if `self` and `other` are disjoint.
    ///
    /// ```
    /// use lime_rtw::interval::Interval;
    ///
    /// let a = Interval::range(0, 1);
    /// let b = Interval::range(2, 3);
    ///
    /// assert_eq!(a.is_disjoint_with(&b), true);
    /// ```
    pub fn is_disjoint_with(&self, other: &Interval<T>) -> bool {
        self.a >= other.b || other.a >= self.b
    }

    /// Returns true if `self` ends before `value`.
    ///
    /// ```
    /// use lime_rtw::interval::Interval;
    ///
    /// let a = Interval::range(0, 2);
    ///
    /// assert_eq!(a.is_left_of(&2), true);
    /// assert_eq!(a.is_left_of(&3), true);
    /// assert_eq!(a.is_left_of(&1), false);
    /// ```
    pub fn is_left_of(&self, value: &T) -> bool {
        match self.upper_bound() {
            Infinity => false,
            MinusInfinity => unreachable!(),
            Value(v) => &v <= value,
        }
    }

    /// Returns true if `self` ends before `value`.
    ///
    /// ```
    /// use lime_rtw::interval::Interval;
    ///
    /// let a = Interval::range(0, 2);
    ///
    /// assert_eq!(a.is_left_of(&2), true);
    /// assert_eq!(a.is_left_of(&3), true);
    /// assert_eq!(a.is_left_of(&1), false);
    ///
    /// let b = Interval::infinite_right(0);
    /// assert_eq!(b.is_left_of(&10), false);
    /// ```
    pub fn is_right_of(&self, value: &T) -> bool {
        match self.lower_bound() {
            MinusInfinity => false,
            Infinity => unreachable!(),
            Value(v) => &v > value,
        }
    }

    /// Returns true if `value` is contained in `self`.
    ///
    /// ```
    /// use lime_rtw::interval::Interval;
    ///
    /// let a = Interval::range(0, 2);
    ///
    /// assert_eq!(a.contains(&0), true);
    /// assert_eq!(a.contains(&1), true);
    /// assert_eq!(a.contains(&2), false);
    /// ```
    pub fn contains(&self, value: &T) -> bool {
        let v = Bound::Value(*value);

        (self.lower_bound() <= v) && (v < self.upper_bound())
    }

    /// Returns the intersection of `self` and `other` if it exist or `None` otherwise.
    ///
    /// ```
    /// use lime_rtw::interval::Interval;
    ///
    /// let a = Interval::range(0, 2);
    /// let b = Interval::range(1, 3);
    /// let c = Interval::range(4, 5);
    ///
    /// assert_eq!(a.intersection(&b), Some(Interval::range(1, 2)));
    /// assert_eq!(a.intersection(&c), None);
    /// ```
    pub fn intersection(&self, other: &Interval<T>) -> Option<Interval<T>> {
        let a = self.a.max(other.a);
        let b = self.b.min(other.b);

        if a < b {
            return Some(Interval::new(a, b));
        }

        None
    }

    /// Returns the distanc between the interval lower and upper bounds.
    ///
    /// ```
    /// use lime_rtw::interval::{Interval, Bound};
    ///
    /// let a = Interval::range(0, 2);
    /// let b = Interval::infinite_right(0);
    ///
    /// assert_eq!(a.span(), Bound::Value(2));
    /// assert_eq!(b.span(), Bound::Infinity);
    /// ```
    pub fn span(&self) -> Bound<T>
    where
        for<'a> &'a T: Sub<&'a T, Output = T>,
    {
        match (&self.a, &self.b) {
            (Value(a), Value(b)) => Value(b - a),
            _ => Infinity,
        }
    }

    fn intersect_inplace_if_overlapping(&mut self, other: &Interval<T>) {
        if !self.is_disjoint_with(other) {
            let a = self.a.max(other.a);
            let b = self.b.min(other.b);

            self.a = a;
            self.b = b;
        }
    }

    /// Builds the `(-inf, inf)` open interval.
    pub fn infinite() -> Self {
        Self::new(MinusInfinity, Infinity)
    }

    /// Builds the `[lower, inf)` interval.
    pub fn infinite_right(lower: T) -> Self {
        Self::new(Value(lower), Infinity)
    }

    /// Returns true if the interval has the form `[a, +inf)`.
    /// ```
    /// use lime_rtw::interval::Interval;
    ///
    /// let a = Interval::range(0, 2);
    /// let b = Interval::infinite_right(0);
    ///
    /// assert_eq!(a.is_infinite_right(), false);
    /// assert_eq!(b.is_infinite_right(), true);
    /// ```
    pub fn is_infinite_right(&self) -> bool {
        matches!((self.a, self.b), (Value(_), Infinity))
    }

    /// Builds the `[a, b)` half-open interval.
    pub fn range(a: T, b: T) -> Self {
        Self {
            a: Value(a),
            b: Value(b),
        }
    }

    pub fn set_lower_bound(&mut self, lower: T) {
        let value = Value(lower);

        assert!(value < self.b);

        self.a = value;
    }

    pub fn set_upper_bound(&mut self, upper: T) {
        let value = Value(upper);

        assert!(value >= self.a);

        self.b = value;
    }
}

pub type TimeInterval = Interval<u64>;

impl TimeInterval {
    pub fn duration(&self) -> Bound<u64> {
        self.span()
    }

    pub fn duration_unwrap(&self) -> u64 {
        self.duration().value().unwrap()
    }
}

fn interval_binary_search<T: Eq + Ord + Copy>(
    seq: &[Interval<T>],
    value: T,
    hint: &Option<usize>,
) -> usize {
    let mut l = 0;
    let mut r = seq.len();
    let mut p = 0;

    if let Some(h) = hint {
        p = *h;

        if p < seq.len() {
            if seq[p].contains(&value) {
                return p;
            }

            if seq[p].is_left_of(&value) {
                l = p + 1;
            } else {
                r = p;
            }
        }
    }

    while l < r {
        p = (l + r) / 2;

        if seq[p].contains(&value) {
            return p;
        }

        if seq[p].is_left_of(&value) {
            l = p + 1;
        } else {
            r = p;
        }
    }

    p
}

fn do_search_lower_bound<T: Eq + Ord + Copy>(
    seq: &[Interval<T>],
    value: T,
    hint: &Option<usize>,
) -> Option<usize> {
    let x = Bound::Value(value);
    if seq.is_empty() || (seq[seq.len() - 1].upper_bound() < x) {
        return None;
    }

    let mut p = interval_binary_search(seq, value, hint);

    if seq[p].is_left_of(&value) {
        p += 1
    }

    Some(p)
}

fn do_search_upper_bound<T: Eq + Ord + Copy>(seq: &[Interval<T>], value: T) -> Option<usize> {
    let x = Bound::Value(value);

    if seq.is_empty() || (seq[0].lower_bound() > x) {
        return None;
    }

    let mut p = interval_binary_search(seq, value, &None);

    if seq[p].is_right_of(&value) {
        p -= 1;
    }

    Some(p)
}

/// A ordered sequence of intervals.
pub struct IntervalSeq<T: Eq + Ord + Copy + Clone + Hash> {
    lower_bound_hint: Option<usize>,
    lower_bound_cache: BTreeMap<T, usize>,
    upper_bound_cache: BTreeMap<T, usize>,
    buff: VecDeque<Interval<T>>,
}

impl<T> IntervalSeq<T>
where
    T: Ord + Copy + Clone + Hash,
{
    pub fn new() -> Self {
        Self {
            lower_bound_hint: None,
            lower_bound_cache: BTreeMap::new(),
            upper_bound_cache: BTreeMap::new(),
            buff: VecDeque::new(),
        }
    }

    fn clear_cache(&mut self) {
        self.lower_bound_hint = Some(0);
        self.lower_bound_cache.clear();
        self.upper_bound_cache.clear();
    }

    /// Returns a reference on the last interval in the sequence.
    ///
    /// ```
    /// use lime_rtw::interval::Interval;
    /// use lime_rtw::interval::IntervalSeq;
    ///
    /// let mut s = IntervalSeq::new();
    /// s.push(Interval::range(1, 2));
    /// s.push(Interval::range(3, 4));
    /// assert_ne!(s.last(), Some(&Interval::range(1,2)));
    /// assert_eq!(s.last(), Some(&Interval::range(3,4)));
    /// ```
    pub fn last(&self) -> Option<&Interval<T>> {
        self.buff.back()
    }

    /// Returns a mutable reference on the last interval in the sequence.
    ///
    /// ```
    /// use lime_rtw::interval::Interval;
    /// use lime_rtw::interval::IntervalSeq;
    ///
    /// let mut s = IntervalSeq::new();
    /// s.push(Interval::range(1, 2));
    /// s.push(Interval::range(3, 4));
    /// assert_ne!(s.last_mut(), Some(&mut Interval::range(1,2)));
    /// assert_eq!(s.last_mut(), Some(&mut Interval::range(3,4)));
    /// ```
    pub fn last_mut(&mut self) -> Option<&mut Interval<T>> {
        self.buff.back_mut()
    }

    /// Returns a reference on the first interval in the sequence.
    ///
    /// ```
    /// use lime_rtw::interval::Interval;
    /// use lime_rtw::interval::IntervalSeq;
    ///
    /// let mut s = IntervalSeq::new();
    /// s.push(Interval::range(1, 2));
    /// s.push(Interval::range(3, 4));
    /// assert_eq!(s.first(), Some(&Interval::range(1,2)));
    /// assert_ne!(s.first(), Some(&Interval::range(3,4)));
    /// ```
    pub fn first(&self) -> Option<&Interval<T>> {
        self.buff.front()
    }

    /// Adds an interval at the end of the sequence.
    /// ```
    /// use lime_rtw::interval::Interval;
    /// use lime_rtw::interval::IntervalSeq;
    ///
    /// let mut s = IntervalSeq::new();
    /// s.push(Interval::range(1, 2));
    /// s.push(Interval::range(3, 4));
    /// s.push(Interval::range(5, 6));
    ///
    /// let mut expected: Vec<Interval<u64>> = Vec::new();
    /// expected.push(Interval::range(1, 2));
    /// expected.push(Interval::range(3, 4));
    /// expected.push(Interval::range(5, 6));
    ///
    /// let intervals = s.iter().cloned().collect::<Vec<Interval<u64>>>();
    /// assert_eq!(intervals, expected);
    ///
    /// ```
    ///
    /// # Panics
    ///
    /// This method panics if `interval` overlaps with or does not start after `self.last()`.
    ///
    /// ```should_panic
    /// use lime_rtw::interval::Interval;
    /// use lime_rtw::interval::IntervalSeq;
    ///
    /// let mut s = IntervalSeq::new();
    /// s.push(Interval::range(1, 2));
    /// s.push(Interval::range(3, 4));
    /// // should panic
    /// s.push(Interval::range(2, 3));
    /// ```
    pub fn push(&mut self, interval: Interval<T>) {
        if let Some(last) = self.last() {
            assert!(interval.is_disjoint_with(last));

            match last.upper_bound() {
                MinusInfinity => unreachable!(),
                Infinity => panic!("Cannot insert an interval after infinity!"),
                Value(v) => assert!(interval.is_right_of(&v)),
            }
        }

        self.buff.push_back(interval);
    }

    /// Removes the last interval in the sequence
    ///
    /// ```
    /// use lime_rtw::interval::Interval;
    /// use lime_rtw::interval::IntervalSeq;
    ///
    /// let mut s = IntervalSeq::new();
    /// s.push(Interval::range(1, 2));
    /// s.push(Interval::range(3, 4));
    /// assert_eq!(s.last(), Some(&Interval::range(3,4)));
    /// s.pop();
    /// assert_eq!(s.last(), Some(&Interval::range(1,2)));
    /// s.pop();
    /// assert_eq!(s.last(), None);
    /// ```
    pub fn pop(&mut self) -> Option<Interval<T>> {
        self.buff.pop_back()
    }

    /// Returns an iterator of the intersection of `self` with `other`.
    ///
    /// ```
    /// use lime_rtw::interval::Interval;
    /// use lime_rtw::interval::IntervalSeq;
    ///
    /// let mut s = IntervalSeq::new();
    /// s.push(Interval::range(1, 2));
    /// s.push(Interval::range(4, 6));
    /// s.push(Interval::range(7, 9));
    ///
    /// let xs = s.intersection(Interval::range(3, 8)).collect::<Vec<Interval<u64>>>();
    /// assert_eq!(xs, vec![Interval::range(4,6), Interval::range(7, 8)]);
    /// ```
    pub fn intersection(&mut self, other: Interval<T>) -> impl Iterator<Item = Interval<T>> + '_ {
        let mut ret = match other.lower_bound() {
            MinusInfinity => self.buff.make_contiguous(),
            Infinity => unreachable!(),
            Value(v) => {
                if let Some(i) = self.search_lower_bound(v) {
                    &mut self.buff.make_contiguous()[i..]
                } else {
                    &mut self.buff.make_contiguous()[0..0]
                }
            }
        };

        ret = match other.upper_bound() {
            MinusInfinity => unreachable!(),
            Infinity => ret,
            Value(v) => {
                if let Some(i) = do_search_upper_bound(ret, v) {
                    &mut ret[..=i]
                } else {
                    ret
                }
            }
        };

        ret.iter().filter_map(move |i| i.intersection(&other))
    }

    /*
    pub fn intersection_already_contiguous(&self, other: Interval<T>) -> impl Iterator<Item = Interval<T>> + '_ {
        let slice = self.buff.as_slices().0;

        let mut ret = match other.lower_bound() {
            MinusInfinity => slice,
            Infinity => unreachable!(),
            Value(v) => {
                if let Some(i) = self.search_lower_bound(v) {
                    &slice[i..]
                } else {
                    &slice[0..0]
                }
            }
        };

        ret = match other.upper_bound() {
            MinusInfinity => unreachable!(),
            Infinity => ret,
            Value(v) => {
                if let Some(i) = do_search_upper_bound(ret, v) {
                    &ret[..=i]
                } else {
                    ret
                }
            }
        };

        ret.iter().filter_map(move |i| i.intersection(&other))
    }
    */

    /// Update `self` with the intersection of `self` and `other`.
    ///
    /// ```
    /// use lime_rtw::interval::Interval;
    /// use lime_rtw::interval::IntervalSeq;
    ///
    /// let mut s = IntervalSeq::new();
    /// s.push(Interval::range(1, 2));
    /// s.push(Interval::range(4, 6));
    /// s.push(Interval::range(7, 9));
    ///
    /// s.intersection_inplace(&Interval::range(3, 8));
    ///
    /// let xs = s.iter().collect::<Vec<&Interval<u64>>>();
    /// assert_eq!(xs, vec![&Interval::range(4,6), &Interval::range(7, 8)]);
    /// ```
    pub fn intersection_inplace(&mut self, other: &Interval<T>) {
        self.remove_disjoints(other);
        self.clear_cache();

        if let Some(v) = self.buff.back_mut() {
            v.intersect_inplace_if_overlapping(other)
        }

        if let Some(v) = self.buff.front_mut() {
            v.intersect_inplace_if_overlapping(other)
        }
    }

    fn search_lower_bound(&mut self, value: T) -> Option<usize> {
        if let Some(lower_bound) = self.lower_bound_cache.get(&value) {
            return Some(*lower_bound);
        }

        if let Some(lower_bound) =
            do_search_lower_bound(self.buff.make_contiguous(), value, &self.lower_bound_hint)
        {
            self.lower_bound_cache.insert(value, lower_bound);
            self.lower_bound_hint.replace(lower_bound);
            return Some(lower_bound);
        }

        None
    }

    fn search_upper_bound(&mut self, value: T) -> Option<usize> {
        if let Some(upper_bound) = self.upper_bound_cache.get(&value) {
            return Some(*upper_bound);
        }

        if let Some(upper_bound) = do_search_upper_bound(self.buff.make_contiguous(), value) {
            self.upper_bound_cache.insert(value, upper_bound);
            return Some(upper_bound);
        }

        None
    }

    fn trim_left(&mut self, value: T) {
        if let Some(l) = self.search_lower_bound(value) {
            self.buff.drain(..l);
        } else {
            self.buff.drain(..);
        }
    }

    fn trim_right(&mut self, value: T) {
        if let Some(r) = self.search_upper_bound(value) {
            self.buff.drain((r + 1)..);
        }
    }

    /// Removes intervals not overlapping with `other`
    fn remove_disjoints(&mut self, other: &Interval<T>) {
        match other.lower_bound() {
            MinusInfinity => {}
            Infinity => unreachable!(),
            Value(l) => self.trim_left(l),
        }

        match other.upper_bound() {
            Infinity => {}
            MinusInfinity => unreachable!(),
            Value(r) => self.trim_right(r),
        }
    }

    /// Returns an iterator of the intervals contained in the sequence.
    pub fn iter(&mut self) -> impl Iterator<Item = &Interval<T>> {
        self.buff.iter()
    }

    pub(crate) fn len(&self) -> usize {
        self.buff.len()
    }
}

impl<T> Default for IntervalSeq<T>
where
    T: Ord + Copy + Clone + Hash,
{
    fn default() -> Self {
        Self::new()
    }
}

impl IntervalSeq<u64> {
    fn search_until_index(&self, until: u64) -> usize {
        self.buff.partition_point(|i| i.is_left_of(&until))
    }

    fn until_index(&self, until: u64) -> Option<usize> {
        let last = self.buff.back()?;

        if last.is_left_of(&until) {
            return Some(self.buff.len());
        }

        let first = self.buff.front()?;

        if first.is_right_of(&until) {
            return None;
        }

        Some(self.search_until_index(until))
    }

    /// Removes all intervals left of `until`.
    /// The removed intervals are compressed in a tuple `(A, c)`, in which
    /// `A` is the extent of all the removed intervals, and `c` their coverage.
    /// ```
    /// use lime_rtw::interval::Interval;
    /// use lime_rtw::interval::IntervalSeq;
    ///
    /// let mut s = IntervalSeq::new();
    /// s.push(Interval::range(1, 2));
    /// s.push(Interval::range(4, 6));
    /// s.push(Interval::range(7, 9));
    ///
    /// let compressed = s.compress_until(5);
    ///
    /// let expected = (Interval::range(1,5), 2);
    /// assert_eq!(compressed, Some(expected));
    ///
    /// let intervals = s.iter().collect::<Vec<&Interval<u64>>>();
    /// assert_eq!(intervals, vec![&Interval::range(5,6), &Interval::range(7, 9)]);
    ///
    /// let compressed = s.compress_until(0);
    /// assert_eq!(compressed, None);
    ///
    /// let intervals = s.iter().collect::<Vec<&Interval<u64>>>();
    /// assert_eq!(intervals, vec![&Interval::range(5,6), &Interval::range(7, 9)]);
    /// ```
    pub fn compress_until(&mut self, until: u64) -> Option<(TimeInterval, u64)> {
        let idx = self.until_index(until)?;

        self.clear_cache();

        if idx == 0 {
            return None;
        }

        assert!(idx > 0);

        let lower = self.buff.front()?.lower_bound();
        let mut upper = self.buff.get(idx - 1)?.upper_bound();

        let mut cov = self
            .buff
            .drain(..idx)
            .fold(0, |acc, i| acc + i.duration_unwrap());

        if let Some(first) = self.buff.front_mut() {
            if first.contains(&until) {
                upper = Value(until);
                cov += until.saturating_sub(first.lower_bound().value().unwrap());

                first.set_lower_bound(until);
            }
        }

        let compressed_interval = Interval::new(lower, upper);

        Some((compressed_interval, cov))
    }
}
#[cfg(test)]
mod tests {
    use crate::interval::do_search_upper_bound;

    use super::{do_search_lower_bound, Interval, IntervalSeq};

    use super::Bound;

    #[test]
    fn test_bound() {
        assert!(Bound::Value(1) < Bound::Infinity);
        assert!(Bound::Infinity > Bound::Value(1));
        assert!(Bound::Infinity != Bound::Value(1));
        assert_eq!(
            Bound::Value(1).cmp(&Bound::Infinity),
            std::cmp::Ordering::Less
        );
        assert_eq!(Bound::Value(1).min(Bound::Infinity), Bound::Value(1));
    }

    #[test]
    fn test_interval() {
        let i1 = Interval::infinite_right(1);
        let i2 = Interval::range(1, 2);

        let i3 = i1.intersection(&i2);

        assert_eq!(i3, Some(Interval::range(1, 2)));
    }

    #[test]
    fn test_interval_seq() {
        let mut buff = IntervalSeq::<u64>::new();

        assert!(buff.iter().next().is_none());

        let input = vec![
            Interval::range(2, 4),
            Interval::range(5, 6),
            Interval::infinite_right(7),
        ];

        for i in input.iter().cloned() {
            buff.push(i)
        }

        let v: Vec<Interval<u64>> = buff.iter().cloned().collect();

        assert_eq!(v, input);

        let extent = Interval::range(3, 8);

        let expected = vec![
            Interval::range(3, 4),
            Interval::range(5, 6),
            Interval::range(7, 8),
        ];

        let v: Vec<Interval<u64>> = buff.intersection(extent).collect();

        assert_eq!(v, expected);

        let extent = Interval::range(3, 8);
        buff.intersection_inplace(&extent);
        let v: Vec<Interval<u64>> = buff.iter().cloned().collect();
        assert_eq!(v, expected);
    }

    #[test]
    fn test_find_bounds() {
        let intervals = [
            Interval::range(5, 10),
            Interval::range(20, 30),
            Interval::range(40, 50),
            Interval::range(70, 80),
            Interval::range(90, 100),
        ];

        assert_eq!(do_search_lower_bound(&intervals, 110, &None), None);
        assert_eq!(do_search_lower_bound(&intervals, 0, &None), Some(0));
        assert_eq!(do_search_lower_bound(&intervals, 15, &None), Some(1));
        assert_eq!(do_search_lower_bound(&intervals, 35, &None), Some(2));
        assert_eq!(do_search_lower_bound(&intervals, 45, &None), Some(2));
        assert_eq!(do_search_lower_bound(&intervals, 60, &None), Some(3));

        assert_eq!(do_search_upper_bound(&intervals, 0), None);
        assert_eq!(do_search_upper_bound(&intervals, 15), Some(0));
        assert_eq!(do_search_upper_bound(&intervals, 35), Some(1));
        assert_eq!(do_search_upper_bound(&intervals, 45), Some(2));
        assert_eq!(do_search_upper_bound(&intervals, 60), Some(2));
        assert_eq!(do_search_upper_bound(&intervals, 110), Some(4));
    }

    #[test]
    fn test_compression() {
        let mut seq = IntervalSeq::<u64>::new();

        assert_eq!(seq.compress_until(1), None);

        seq.push(Interval::range(1, 2));
        seq.push(Interval::range(3, 4));
        seq.push(Interval::range(5, 7));

        let compressed = seq.compress_until(6);
        assert_eq!(compressed, Some((Interval::range(1, 6), 3)));

        let mut seq = IntervalSeq::<u64>::new();

        seq.push(Interval::range(1, 2));
        seq.push(Interval::range(3, 4));
        seq.push(Interval::range(5, 7));

        let compressed = seq.compress_until(8);
        assert_eq!(compressed, Some((Interval::range(1, 7), 4)));

        let mut seq = IntervalSeq::<u64>::new();

        seq.push(Interval::range(3, 4));
        seq.push(Interval::range(5, 7));

        let compressed = seq.compress_until(2);
        assert_eq!(compressed, None);
    }
}
