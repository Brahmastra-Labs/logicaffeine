//! Delta Buffer - Ring buffer for recent deltas
//!
//! Wave 1.6: Stores recent deltas for efficient sync with late joiners.

use super::causal::VClock;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// A ring buffer that stores recent deltas along with their versions.
///
/// When a peer requests sync, we can provide deltas since their last known
/// version. If the gap is too large (oldest delta evicted), the peer needs
/// a full state transfer instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaBuffer<D> {
    /// Ring buffer of (version, delta) pairs
    deltas: VecDeque<(VClock, D)>,
    /// Maximum number of deltas to retain
    max_size: usize,
    /// The oldest version we can provide deltas from
    oldest_version: VClock,
}

impl<D: Clone> DeltaBuffer<D> {
    /// Create a new buffer with the given capacity.
    pub fn new(max_size: usize) -> Self {
        Self {
            deltas: VecDeque::with_capacity(max_size),
            max_size,
            oldest_version: VClock::new(),
        }
    }

    /// Push a new delta with its associated version.
    pub fn push(&mut self, version: VClock, delta: D) {
        if self.deltas.len() >= self.max_size {
            if let Some((old_version, _)) = self.deltas.pop_front() {
                self.oldest_version = old_version;
            }
        }
        self.deltas.push_back((version, delta));
    }

    /// Get all deltas since the given version.
    ///
    /// Returns `None` if the version is older than or equal to our oldest evicted delta.
    /// Returns `Some(vec![])` if the peer is up-to-date.
    pub fn deltas_since(&self, version: &VClock) -> Option<Vec<D>> {
        // If we've evicted deltas and the peer is at or behind what we evicted,
        // we can't help them catch up (they need the evicted delta).
        if self.oldest_version != VClock::new() && self.oldest_version.dominates(version) {
            return None;
        }

        // Return deltas that the peer hasn't seen
        let result: Vec<D> = self
            .deltas
            .iter()
            .filter(|(v, _)| !version.dominates(v))
            .map(|(_, d)| d.clone())
            .collect();

        Some(result)
    }

    /// Check if we can provide deltas since the given version.
    pub fn can_serve(&self, version: &VClock) -> bool {
        self.deltas_since(version).is_some()
    }

    /// Get the number of deltas currently stored.
    pub fn len(&self) -> usize {
        self.deltas.len()
    }

    /// Check if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.deltas.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffer_new() {
        let buf: DeltaBuffer<i32> = DeltaBuffer::new(10);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_buffer_push_and_retrieve() {
        let mut buf: DeltaBuffer<i32> = DeltaBuffer::new(10);
        let mut clock = VClock::new();
        clock.increment(1);
        buf.push(clock, 42);

        let empty = VClock::new();
        let deltas = buf.deltas_since(&empty).unwrap();
        assert_eq!(deltas, vec![42]);
    }

    #[test]
    fn test_buffer_multiple_deltas() {
        let mut buf: DeltaBuffer<i32> = DeltaBuffer::new(10);
        let mut clock = VClock::new();

        clock.increment(1);
        buf.push(clock.clone(), 1);
        clock.increment(1);
        buf.push(clock.clone(), 2);
        clock.increment(1);
        buf.push(clock.clone(), 3);

        let empty = VClock::new();
        let deltas = buf.deltas_since(&empty).unwrap();
        assert_eq!(deltas, vec![1, 2, 3]);
    }

    #[test]
    fn test_buffer_since_version() {
        let mut buf: DeltaBuffer<i32> = DeltaBuffer::new(10);
        let mut clock = VClock::new();

        clock.increment(1);
        let v1 = clock.clone();
        buf.push(clock.clone(), 1);

        clock.increment(1);
        buf.push(clock.clone(), 2);

        clock.increment(1);
        buf.push(clock.clone(), 3);

        // Get deltas since v1 - should only get 2 and 3
        let deltas = buf.deltas_since(&v1).unwrap();
        assert_eq!(deltas, vec![2, 3]);
    }

    #[test]
    fn test_buffer_overflow() {
        let mut buf: DeltaBuffer<i32> = DeltaBuffer::new(2);
        let mut clock = VClock::new();

        clock.increment(1);
        buf.push(clock.clone(), 1);
        clock.increment(1);
        buf.push(clock.clone(), 2);
        clock.increment(1);
        buf.push(clock.clone(), 3); // Evicts 1

        let empty = VClock::new();
        // Gap too large - oldest delta we have is after empty
        assert!(buf.deltas_since(&empty).is_none());
    }

    #[test]
    fn test_buffer_overflow_partial() {
        let mut buf: DeltaBuffer<i32> = DeltaBuffer::new(2);
        let mut clock = VClock::new();

        clock.increment(1);
        let v1 = clock.clone();
        buf.push(clock.clone(), 1);

        clock.increment(1);
        let v2 = clock.clone();
        buf.push(clock.clone(), 2);

        clock.increment(1);
        buf.push(clock.clone(), 3); // Evicts 1

        // v1 is too old - can't provide deltas
        assert!(buf.deltas_since(&v1).is_none());

        // v2 is still in buffer - can provide 3
        let deltas = buf.deltas_since(&v2).unwrap();
        assert_eq!(deltas, vec![3]);
    }
}
