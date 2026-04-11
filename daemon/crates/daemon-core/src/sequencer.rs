//! Global monotonic event sequencer for AgentStreamEvent ordering.
//!
//! Provides gap detection for WebSocket consumers (watch CLI).
//! REQ-E01, REQ-W06

use std::sync::atomic::{AtomicU64, Ordering};

/// Global atomic counter for monotonic event ordering.
///
/// Thread-safe, lock-free. One instance shared via Arc across the daemon.
/// Each call to `next()` returns a strictly increasing sequence number.
#[derive(Debug)]
pub struct EventSequencer {
    counter: AtomicU64,
}

impl EventSequencer {
    pub fn new() -> Self {
        Self {
            counter: AtomicU64::new(1),
        }
    }

    /// Return the next sequence number. Guaranteed monotonically increasing.
    pub fn next(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::Relaxed)
    }

    /// Current value (for diagnostics only — not guaranteed to be unused).
    pub fn current(&self) -> u64 {
        self.counter.load(Ordering::Relaxed)
    }
}

impl Default for EventSequencer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequencer_is_monotonic() {
        let seq = EventSequencer::new();
        let a = seq.next();
        let b = seq.next();
        let c = seq.next();
        assert_eq!(a, 1);
        assert_eq!(b, 2);
        assert_eq!(c, 3);
    }

    #[test]
    fn sequencer_starts_at_one() {
        let seq = EventSequencer::new();
        assert_eq!(seq.next(), 1);
    }

    #[test]
    fn current_reflects_state() {
        let seq = EventSequencer::new();
        assert_eq!(seq.current(), 1);
        seq.next();
        assert_eq!(seq.current(), 2);
    }
}
