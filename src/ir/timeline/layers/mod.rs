//! This modules contains the layers currently implemented by LiME.
//!
//! Future layer definitions should live here.
//!
//! The `BaseLayer` struct provides basic layer handling methods. Other Layers are
//! expected to wrap a `BaseLayer` and to implement the `Layer` trait. Moreover,
//! if one needs to iterate of the intervals contained in a layer, this layer
//! should implement a `plain_intervals` method. For reference, you can have a
//! look at the suspension and execution layers implementations.
use crate::{
    events::TraceEvent,
    interval::{Interval, TimeInterval},
};

use self::base::BaseLayer;

/// A layer implemented on top of a `BaseLayer`.
pub trait Layer {
    /// Defines how the layer is updated given a trace events.
    /// This is where linux-specific details are abstracted.
    fn update(&mut self, event: &TraceEvent);
    /// Return a reference to the contained base layer.
    fn get_base(&self) -> &BaseLayer;
    /// Return a mutable reference to the contained base layer.
    fn get_base_mut(&mut self) -> &mut BaseLayer;

    fn trim(&mut self, active_range: &TimeInterval) {
        self.get_base_mut().trim(active_range)
    }

    fn has_compressed_intervals(&self, range: &Interval<u64>) -> bool {
        !self.get_base().only_plain_intervals_in(range)
    }

    fn compress(&mut self, until: u64, new_block: bool) {
        self.get_base_mut().compress(until, new_block);
    }

    fn compress_all(&mut self, new_block: bool) {
        self.get_base_mut().compress_all(new_block);
    }

    /// Time spent during `range`` in the state represented by the layer.
    fn time(&mut self, range: &TimeInterval) -> u64 {
        self.get_base_mut().coverage(range)
    }
}

pub mod base;
pub mod execution;
pub mod suspension;
