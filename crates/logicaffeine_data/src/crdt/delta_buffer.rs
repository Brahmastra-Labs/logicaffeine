//! Delta Buffer - Ring buffer for recent deltas.
//!
//! Stores recent deltas for efficient sync with late joiners.

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
#[path = "../../../logicaffeine_tests/unit/logicaffeine_data/crdt/delta_buffer/tests.rs"]
mod tests;
