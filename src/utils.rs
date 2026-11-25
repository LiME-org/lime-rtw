//! Utility types and functions.

use std::collections::HashMap;
use std::hash::Hash;

use serde::{Deserialize, Serialize};

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
    unsafe { std::slice::from_raw_parts_mut((p as *mut T) as *mut u8, std::mem::size_of::<T>()) }
}
