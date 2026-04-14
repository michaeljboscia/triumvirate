//! Event replay ring buffer for WebSocket reconnect recovery.
//!
//! When a Pantheon client disconnects and reconnects, it sends its last
//! seen sequence number. The daemon replays all events with seq > lastSeq
//! from this buffer, then switches the client to the live broadcast channel.
//!
//! If the client's lastSeq is older than the buffer's oldest event, the
//! client must perform a full /api/state refresh instead.
//!
//! Design:
//! - VecDeque<AgentStreamEvent> capped at 1000 events
//! - Wrapped in std::sync::RwLock for thread safety
//! - Oldest event dropped when at capacity (FIFO)
//! - Memory cost: ~1000 events × ~200 bytes = ~200KB
//!
//! CRITICAL: The subscribe-before-read pattern must be followed on the
//! WebSocket handler side. Acquire the broadcast subscriber BEFORE reading
//! the replay buffer, or events that arrive between read and subscribe will
//! be lost.
//!
//! FEAT-013 (REQ-020)

use shared_types::AgentStreamEvent;
use std::collections::VecDeque;
use std::sync::{Arc, RwLock};
use tracing::{debug, warn};

/// Default capacity for the replay buffer.
pub const DEFAULT_CAPACITY: usize = 1000;

/// Result of a replay request.
#[derive(Debug, Clone, PartialEq)]
pub enum ReplayResult {
    /// Client's lastSeq is within buffer range. Events to replay.
    /// If the vec is empty, client is fully caught up (lastSeq >= newest).
    Events(Vec<AgentStreamEvent>),
    /// Client's lastSeq is older than the buffer's oldest event.
    /// Client must do full /api/state refresh.
    OutOfRange {
        /// The oldest seq currently in the buffer. Client's lastSeq < this.
        oldest_seq: u64,
    },
}

/// Thread-safe event replay buffer.
#[derive(Debug, Clone)]
pub struct EventReplayBuffer {
    inner: Arc<RwLock<EventReplayBufferInner>>,
}

#[derive(Debug)]
struct EventReplayBufferInner {
    buffer: VecDeque<AgentStreamEvent>,
    capacity: usize,
}

impl EventReplayBuffer {
    /// Create a new buffer with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(EventReplayBufferInner {
                buffer: VecDeque::with_capacity(capacity),
                capacity,
            })),
        }
    }

    /// Create a buffer with the default capacity (1000).
    pub fn default_capacity() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }

    /// Push an event into the buffer.
    ///
    /// If the buffer is at capacity, the oldest event is dropped (FIFO).
    /// This method is called by the event emission path alongside the
    /// broadcast channel send — both the live channel and the ring buffer
    /// see every event.
    pub fn push(&self, event: AgentStreamEvent) {
        let mut inner = match self.inner.write() {
            Ok(guard) => guard,
            Err(poisoned) => {
                warn!("event replay buffer lock poisoned, recovering");
                poisoned.into_inner()
            }
        };
        if inner.buffer.len() >= inner.capacity {
            inner.buffer.pop_front();
        }
        inner.buffer.push_back(event);
    }

    /// Replay events with seq > last_seq.
    ///
    /// Returns `Events(vec)` if the buffer can cover the requested range.
    /// Returns `OutOfRange { oldest_seq }` if last_seq is older than the
    /// buffer's oldest event — the client must do a full state refresh.
    ///
    /// An empty Events vec means the client is caught up (last_seq >= newest).
    pub fn replay_since(&self, last_seq: u64) -> ReplayResult {
        let inner = match self.inner.read() {
            Ok(guard) => guard,
            Err(poisoned) => {
                warn!("event replay buffer lock poisoned, recovering");
                poisoned.into_inner()
            }
        };

        // Empty buffer: nothing to replay, but also not out-of-range.
        // Client is considered caught up.
        let Some(oldest) = inner.buffer.front() else {
            debug!("replay requested against empty buffer");
            return ReplayResult::Events(Vec::new());
        };

        let oldest_seq = oldest.seq();

        // Check if client's lastSeq is older than the buffer's oldest event.
        // Example: buffer has [500..1500], client has lastSeq=200.
        // last_seq < oldest_seq means the client missed events 201..499 which
        // are not in the buffer. Full refresh required.
        if last_seq + 1 < oldest_seq {
            return ReplayResult::OutOfRange { oldest_seq };
        }

        // Collect events with seq > last_seq.
        let events: Vec<AgentStreamEvent> = inner
            .buffer
            .iter()
            .filter(|e| e.seq() > last_seq)
            .cloned()
            .collect();

        ReplayResult::Events(events)
    }

    /// Get the current number of events in the buffer.
    pub fn len(&self) -> usize {
        self.inner.read().map(|g| g.buffer.len()).unwrap_or(0)
    }

    /// Check if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get the oldest and newest sequence numbers currently in the buffer.
    pub fn seq_range(&self) -> Option<(u64, u64)> {
        let inner = self.inner.read().ok()?;
        let oldest = inner.buffer.front()?.seq();
        let newest = inner.buffer.back()?.seq();
        Some((oldest, newest))
    }
}

impl Default for EventReplayBuffer {
    fn default() -> Self {
        Self::default_capacity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(seq: u64) -> AgentStreamEvent {
        AgentStreamEvent::ToolCall {
            agent: "test".into(),
            tool_name: "test_tool".into(),
            args_summary: format!("event-{seq}"),
            seq,
        }
    }

    #[test]
    fn push_stores_events_in_order() {
        let buf = EventReplayBuffer::new(10);
        for i in 1..=5 {
            buf.push(make_event(i));
        }
        assert_eq!(buf.len(), 5);
        assert_eq!(buf.seq_range(), Some((1, 5)));
    }

    #[test]
    fn push_drops_oldest_when_at_capacity() {
        let buf = EventReplayBuffer::new(3);
        for i in 1..=5 {
            buf.push(make_event(i));
        }
        // Capacity 3, pushed 5, oldest 2 dropped (oldest 1, 2 dropped)
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.seq_range(), Some((3, 5)));
    }

    #[test]
    fn replay_since_returns_events_within_range() {
        let buf = EventReplayBuffer::new(1000);
        for i in 1..=100 {
            buf.push(make_event(i));
        }
        match buf.replay_since(50) {
            ReplayResult::Events(events) => {
                assert_eq!(events.len(), 50);
                assert_eq!(events[0].seq(), 51);
                assert_eq!(events[49].seq(), 100);
            }
            other => panic!("expected Events, got {other:?}"),
        }
    }

    #[test]
    fn replay_since_zero_returns_all_events() {
        let buf = EventReplayBuffer::new(1000);
        for i in 1..=100 {
            buf.push(make_event(i));
        }
        match buf.replay_since(0) {
            ReplayResult::Events(events) => {
                assert_eq!(events.len(), 100);
                assert_eq!(events[0].seq(), 1);
                assert_eq!(events[99].seq(), 100);
            }
            other => panic!("expected Events, got {other:?}"),
        }
    }

    #[test]
    fn replay_since_out_of_range_when_client_too_old() {
        // Buffer capacity 100, push 1500 events → buffer holds seq 1401..1500.
        // Client with lastSeq=200 is way behind.
        let buf = EventReplayBuffer::new(100);
        for i in 1..=1500 {
            buf.push(make_event(i));
        }
        assert_eq!(buf.seq_range(), Some((1401, 1500)));

        match buf.replay_since(200) {
            ReplayResult::OutOfRange { oldest_seq } => {
                assert_eq!(oldest_seq, 1401);
            }
            other => panic!("expected OutOfRange, got {other:?}"),
        }
    }

    #[test]
    fn replay_since_at_boundary_of_oldest() {
        // Buffer holds seq 10..20. Client with lastSeq=9 should get all 10..20.
        // Client with lastSeq=8 should get OutOfRange (missed 9).
        let buf = EventReplayBuffer::new(100);
        for i in 10..=20 {
            buf.push(make_event(i));
        }

        match buf.replay_since(9) {
            ReplayResult::Events(events) => {
                assert_eq!(events.len(), 11);
                assert_eq!(events[0].seq(), 10);
            }
            other => panic!("expected Events for lastSeq=9, got {other:?}"),
        }

        match buf.replay_since(8) {
            ReplayResult::OutOfRange { oldest_seq } => {
                assert_eq!(oldest_seq, 10);
            }
            other => panic!("expected OutOfRange for lastSeq=8, got {other:?}"),
        }
    }

    #[test]
    fn replay_since_returns_empty_when_client_is_caught_up() {
        let buf = EventReplayBuffer::new(100);
        for i in 1..=50 {
            buf.push(make_event(i));
        }

        match buf.replay_since(50) {
            ReplayResult::Events(events) => assert!(events.is_empty()),
            other => panic!("expected empty Events, got {other:?}"),
        }

        match buf.replay_since(100) {
            ReplayResult::Events(events) => assert!(events.is_empty()),
            other => panic!("expected empty Events, got {other:?}"),
        }
    }

    #[test]
    fn replay_empty_buffer_returns_empty_events() {
        let buf = EventReplayBuffer::new(100);
        match buf.replay_since(0) {
            ReplayResult::Events(events) => assert!(events.is_empty()),
            other => panic!("expected empty Events on empty buffer, got {other:?}"),
        }
    }

    #[test]
    fn cloneable_shares_same_backing_buffer() {
        // EventReplayBuffer uses Arc<RwLock<_>>, so clones share state.
        // This is important — the writer and readers hold the same underlying buffer.
        let buf_a = EventReplayBuffer::new(100);
        let buf_b = buf_a.clone();

        buf_a.push(make_event(1));
        buf_a.push(make_event(2));

        assert_eq!(buf_b.len(), 2);
        assert_eq!(buf_b.seq_range(), Some((1, 2)));

        buf_b.push(make_event(3));
        assert_eq!(buf_a.len(), 3);
    }
}
