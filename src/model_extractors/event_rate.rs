//! Event rate recording.

use crate::events::TraceEvent;

/// Tracks event throughput.
pub struct EventRate {
    count: usize,
    first_arrival: u64,
    last_arrival: u64,
}

impl EventRate {
    pub fn new() -> Self {
        Self {
            count: 0,
            first_arrival: 0,
            last_arrival: 0,
        }
    }

    pub fn update(&mut self, event: &TraceEvent) {
        if self.count == 0 {
            self.first_arrival = event.ts;
        }

        if event.ts != 0 {
            self.last_arrival = event.ts;
        }

        self.count += 1;
    }

    pub fn request_per_second(&self) -> f64 {
        if self.count < 2 {
            return 0.0;
        }

        let nsecs = self.last_arrival - self.first_arrival;
        let secs = nsecs as f64 / 1_000_000_000.0;

        self.count as f64 / secs
    }
}
