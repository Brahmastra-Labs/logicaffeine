//! `BoundedRing<T>` — a fixed-capacity queue that drops the OLDEST item on overflow.
//!
//! This is the backpressure primitive for the real-time relay's per-connection outbound queue.
//! The relay hands each connection an UNBOUNDED channel today (`logicaffeine_system::relay`), so a
//! single slow consumer under a fast producer grows without bound — a latent OOM. A live-media
//! queue must instead be bounded, and when it fills it should shed the STALEST frames (a late video
//! frame is worthless; FEC and the next keyframe cover the gap), never the freshest, and never
//! stall the whole relay. Every shed frame is counted in [`BoundedRing::dropped`] — the exact
//! signal the congestion controller (`adaptRate`) reads to back off. Bounded memory,
//! freshest-data-wins, drop-aware: the correct real-time transport policy.

use std::collections::VecDeque;

/// A FIFO queue capped at `cap` items; a push into a full ring evicts the oldest.
#[derive(Debug, Clone)]
pub struct BoundedRing<T> {
    buf: VecDeque<T>,
    cap: usize,
    dropped: u64,
}

impl<T> BoundedRing<T> {
    /// A ring holding at most `cap` items. `cap` must be positive.
    pub fn new(cap: usize) -> Self {
        assert!(cap > 0, "a BoundedRing needs a positive capacity");
        Self {
            buf: VecDeque::with_capacity(cap),
            cap,
            dropped: 0,
        }
    }

    /// Enqueue `item`. If the ring is already full, the OLDEST item is evicted first (and counted
    /// in [`dropped`](Self::dropped)); the evicted item is returned so the caller can telemeter it.
    pub fn push(&mut self, item: T) -> Option<T> {
        let evicted = if self.buf.len() == self.cap {
            self.dropped += 1;
            self.buf.pop_front()
        } else {
            None
        };
        self.buf.push_back(item);
        evicted
    }

    /// Dequeue the oldest surviving item (FIFO).
    pub fn pop(&mut self) -> Option<T> {
        self.buf.pop_front()
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.buf.len() == self.cap
    }

    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// The number of items shed to overflow since construction — the drop telemetry the congestion
    /// controller reads to lower the send rate.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_data/backpressure/tests.rs"]
mod tests;
