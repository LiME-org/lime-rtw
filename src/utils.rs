//! Utility types and functions.

use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::ops::{Add, DivAssign, Mul};
use std::path::{Path, PathBuf};

use min_max_heap::MinMaxHeap;
use serde::{Deserialize, Serialize};
use time::macros::format_description;
use time::OffsetDateTime;

use std::hash::Hash;
#[derive(Default)]
pub struct Dispatcher<K, V> {
    items: HashMap<K, V>,
}

impl<K, V> Dispatcher<K, V>
where
    K: Hash + Eq + Clone,
{
    pub fn new() -> Self {
        Self {
            items: HashMap::new(),
        }
    }

    pub fn get_or_new<F: FnOnce() -> V>(&mut self, k: &K, f: F) -> &mut V {
        self.items.entry(k.clone()).or_insert_with(f)
    }

    pub fn get(&self, k: &K) -> Option<&V> {
        self.items.get(k)
    }

    pub fn get_mut(&mut self, k: &K) -> Option<&mut V> {
        self.items.get_mut(k)
    }

    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.items.values()
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.items.values_mut()
    }

    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.items.keys()
    }

    pub fn items(&self) -> impl Iterator<Item = (&K, &V)> {
        self.items.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl<K, V> Dispatcher<K, V>
where
    K: Hash + Eq + Clone,
    V: Default,
{
    pub fn get_or_default(&mut self, k: &K) -> &mut V {
        self.items.entry(k.clone()).or_default()
    }
}

#[derive(Default)]
pub struct UpperBoundTracker<T> {
    max_val: Option<T>,
}

impl<T: Ord + Copy> UpperBoundTracker<T> {
    pub fn update(&mut self, new: T) -> T {
        let v = self.max_val.map_or(new, |prev| prev.max(new));
        self.max_val = Some(v);
        v
    }

    pub fn get(&self) -> Option<T> {
        self.max_val
    }
}

pub struct KthMaxTracker<T> {
    k: usize,
    heap: MinMaxHeap<T>,
}

impl<T> KthMaxTracker<T>
where
    T: Ord,
{
    pub fn new(k: usize) -> Self {
        Self {
            k,
            heap: MinMaxHeap::with_capacity(k),
        }
    }

    /// Adds an element in the tracker.
    pub fn update(&mut self, val: T) {
        match self.heap.len().cmp(&self.k) {
            Ordering::Less => self.heap.push(val),
            Ordering::Equal => {
                let _ = self.heap.push_pop_min(val);
            }
            Ordering::Greater => unreachable!(),
        }
    }

    pub fn get(&self) -> Option<&T> {
        if self.heap.len() < self.k {
            None
        } else {
            self.heap.peek_min()
        }
    }
}

#[derive(Default)]
pub struct LowerBoundTracker<T> {
    min_val: Option<T>,
}

impl<T: Ord + Copy> LowerBoundTracker<T> {
    pub fn update(&mut self, new: T) -> T {
        let v = self.min_val.map_or(new, |prev| prev.min(new));
        self.min_val = Some(v);
        v
    }

    pub fn get(&self) -> Option<T> {
        self.min_val
    }
}

#[derive(Debug, Serialize, Deserialize, Copy, Clone, Default, Eq)]
pub struct ThreadId {
    pub pid: u32,
    pub tgid: u32,
}

impl PartialEq for ThreadId {
    fn eq(&self, other: &Self) -> bool {
        self.pid == other.pid
    }
}

impl ThreadId {
    pub fn new(pid: u32, tgid: u32) -> Self {
        Self { pid, tgid }
    }

    pub fn to_pid_tgid(&self) -> u64 {
        let mut ret = self.pid as u64;
        ret |= (self.tgid as u64) << 32;
        ret
    }
}

impl From<u64> for ThreadId {
    fn from(pid_tgid: u64) -> ThreadId {
        let pid = pid_tgid as u32;
        let tgid = pid_tgid >> 32;
        ThreadId::new(pid, tgid as u32)
    }
}

/// Converts any type to a mutable slice of bytes.
/// # Safety
/// Under the hood this function uses `std::slice::from_raw_parts_mut`.
/// The same precautions must be taken; refer to the standard documentation.
pub unsafe fn any_as_u8_slice_mut<T: Sized>(p: &mut T) -> &mut [u8] {
    std::slice::from_raw_parts_mut((p as *mut T) as *mut u8, std::mem::size_of::<T>())
}

pub fn unique_json_filename<P: AsRef<Path>>(path: Option<P>) -> Option<PathBuf> {
    let now = OffsetDateTime::now_local().unwrap();
    let date_format = format_description!("[year]_[month]_[day]_[hour][minute][second]");
    let formatted_date = now.format(&date_format).unwrap();

    let name = format!("lime_{}.json", formatted_date);

    let p = path
        .as_ref()
        .map(|t| t.as_ref().join(&name))
        .unwrap_or_else(|| PathBuf::from(&name));

    if !p.exists() {
        return Some(p);
    }

    for c in 'a'..='z' {
        let name = format!("lime_{}_{}.json", formatted_date, c);
        let p = path
            .as_ref()
            .map(|p| p.as_ref().join(&name))
            .unwrap_or_else(|| PathBuf::from(&name));

        if !p.exists() {
            return Some(p);
        }
    }

    None
}

pub struct WeightedMovingAverage<T> {
    average: T,
    total: T,
}

impl<T> WeightedMovingAverage<T>
where
    T: Add<Output = T> + DivAssign + Mul<Output = T> + Copy + Default,
{
    pub fn new() -> Self {
        Self {
            average: T::default(),
            total: T::default(),
        }
    }

    pub fn update(&mut self, value: T, weight: T) {
        self.average = (self.average * self.total) + (value * weight);
        self.total = self.total + weight;
        self.average /= self.total;
    }

    pub fn get_average(&self) -> T {
        self.average
    }

    pub fn get_total(&self) -> T {
        self.total
    }
}

impl<T> Default for WeightedMovingAverage<T>
where
    T: Add<Output = T> + DivAssign + Mul<Output = T> + Copy + Default,
{
    fn default() -> Self {
        Self::new()
    }
}

pub struct RingBuffer<T> {
    capacity: usize,
    buff: VecDeque<T>,
}

impl<T> RingBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            buff: VecDeque::new(),
        }
    }

    pub fn pop(&mut self) -> Option<T> {
        self.buff.pop_front()
    }

    pub fn push(&mut self, val: T) -> Option<T> {
        self.buff.push_back(val);
        debug_assert!(self.buff.len() <= self.capacity + 1, "RingBuffer overflow");
        if self.buff.len() > self.capacity {
            return self.pop();
        }
        None
    }

    pub fn as_slice(&mut self) -> &mut [T] {
        self.buff.make_contiguous()
    }

    pub fn first(&self) -> Option<&T> {
        self.buff.front()
    }

    pub fn last(&self) -> Option<&T> {
        self.buff.back()
    }
}

pub struct InterleaveBy<F, I, J, T> {
    cmp: F,
    i: I,
    j: J,
    v_i: Option<T>,
    v_j: Option<T>,
    phantom: std::marker::PhantomData<T>,
}

impl<F, I, J, T> InterleaveBy<F, I, J, T>
where
    I: Iterator<Item = T>,
    J: Iterator<Item = T>,
    F: Fn(&T, &T) -> Ordering,
{
    pub fn new(i: I, j: J, cmp: F) -> Self {
        Self {
            cmp,
            i,
            j,
            v_i: None,
            v_j: None,
            phantom: std::marker::PhantomData,
        }
    }
}

impl<F, I, J, T> Iterator for InterleaveBy<F, I, J, T>
where
    I: Iterator<Item = T>,
    J: Iterator<Item = T>,
    F: Fn(&T, &T) -> Ordering,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        self.v_i = self.v_i.take().or_else(|| self.i.next());
        self.v_j = self.v_j.take().or_else(|| self.j.next());

        match (&self.v_i, &self.v_j) {
            (None, None) => None,
            (None, Some(_)) => self.v_j.take(),
            (Some(_), None) => self.v_i.take(),
            (Some(i), Some(j)) => {
                if (self.cmp)(i, j) == Ordering::Greater {
                    self.v_j.take()
                } else {
                    self.v_i.take()
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::utils::InterleaveBy;

    use super::WeightedMovingAverage;

    #[test]
    fn test_moving_average() {
        let mut avg = WeightedMovingAverage::new();

        avg.update(1u64, 10);

        assert_eq!(avg.get_average(), 1);
        assert_eq!(avg.get_total(), 10);

        avg.update(2u64, 20);
        avg.update(1u64, 10);

        assert_eq!(avg.get_average(), 1);
        assert_eq!(avg.get_total(), 40);
    }

    #[test]
    fn test_interleave() {
        let x1 = &[1, 3, 5];
        let x2 = &[2, 4, 6];

        let it = InterleaveBy::new(x1.iter(), x2.iter(), |a, b| a.cmp(b));

        let res: Vec<u32> = it.copied().collect();

        assert_eq!(res, &[1, 2, 3, 4, 5, 6]);

        let it = InterleaveBy::new(x1.iter(), x2.iter(), |a, b| b.cmp(a));

        let res: Vec<u32> = it.copied().collect();

        assert_eq!(res, &[2, 4, 6, 1, 3, 5]);

        let x1 = &[5, 3, 1];
        let x2 = &[6, 4, 2];

        let it = InterleaveBy::new(x1.iter(), x2.iter(), |a, b| b.cmp(a));

        let res: Vec<u32> = it.copied().collect();

        assert_eq!(res, &[6, 5, 4, 3, 2, 1]);
    }
}
