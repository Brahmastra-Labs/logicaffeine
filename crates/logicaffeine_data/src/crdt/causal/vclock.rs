//! Vector Clock for causal ordering.
//!
//! VClock tracks causality between distributed events.

use super::super::replica::ReplicaId;
use super::super::Merge;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A vector clock for tracking causal relationships between events.
///
/// Each replica has a logical clock counter. The vector clock captures
/// which events have been "seen" by tracking the maximum counter for each replica.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct VClock {
    entries: HashMap<ReplicaId, u64>,
}

impl VClock {
    /// Create a new empty vector clock.
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment the clock for a replica and return the new counter value.
    pub fn increment(&mut self, replica: ReplicaId) -> u64 {
        let entry = self.entries.entry(replica).or_insert(0);
        *entry += 1;
        *entry
    }

    /// Get the current counter for a replica (0 if not seen).
    pub fn get(&self, replica: ReplicaId) -> u64 {
        *self.entries.get(&replica).unwrap_or(&0)
    }

    /// Check if self dominates other (self >= other for all replicas).
    ///
    /// Returns true if for every replica in `other`, self has an equal or greater counter.
    pub fn dominates(&self, other: &Self) -> bool {
        for (&replica, &count) in &other.entries {
            if self.get(replica) < count {
                return false;
            }
        }
        true
    }

    /// Check if self and other are concurrent (neither dominates the other).
    pub fn concurrent(&self, other: &Self) -> bool {
        !self.dominates(other) && !other.dominates(self)
    }

    /// Merge another vector clock into this one (pointwise max).
    pub fn merge_vclock(&mut self, other: &Self) {
        for (&replica, &count) in &other.entries {
            let entry = self.entries.entry(replica).or_insert(0);
            *entry = (*entry).max(count);
        }
    }

    /// The sum of every replica's counter — the number of events the clock covers.
    pub fn total(&self) -> u64 {
        self.entries.values().sum()
    }

    /// Get all replicas in this clock.
    pub fn replicas(&self) -> impl Iterator<Item = &ReplicaId> {
        self.entries.keys()
    }
}

impl Merge for VClock {
    fn merge(&mut self, other: &Self) {
        self.merge_vclock(other);
    }
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_data/crdt/causal/vclock/tests.rs"]
mod tests;
