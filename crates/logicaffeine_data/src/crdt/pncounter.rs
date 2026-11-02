//! PN-Counter (Positive-Negative Counter) CRDT.
//!
//! A counter that supports both increment and decrement.
//! Implemented as two G-Counters: one for increments, one for decrements.

use super::causal::VClock;
use super::delta::DeltaCrdt;
use super::replica::{generate_replica_id, ReplicaId};
use super::Merge;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Delta for PNCounter synchronization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PNCounterDelta {
    pub increments: HashMap<ReplicaId, u64>,
    pub decrements: HashMap<ReplicaId, u64>,
    pub version: VClock,
}

/// A counter that can be incremented and decremented.
///
/// The value is the difference between total increments and total decrements.
/// Each replica maintains its own increment and decrement counts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PNCounter {
    /// Map from replica ID to increment count
    increments: HashMap<ReplicaId, u64>,
    /// Map from replica ID to decrement count
    decrements: HashMap<ReplicaId, u64>,
    /// This replica's ID
    replica_id: ReplicaId,
    /// Version clock tracking operations
    #[serde(default)]
    version: VClock,
}

impl PNCounter {
    /// Create a new counter with a random replica ID.
    pub fn new() -> Self {
        Self {
            increments: HashMap::new(),
            decrements: HashMap::new(),
            replica_id: generate_replica_id(),
            version: VClock::new(),
        }
    }

    /// Create a counter with a specific replica ID.
    pub fn with_replica_id(id: ReplicaId) -> Self {
        Self {
            increments: HashMap::new(),
            decrements: HashMap::new(),
            replica_id: id,
            version: VClock::new(),
        }
    }

    /// Increment the counter by the given amount.
    pub fn increment(&mut self, amount: u64) {
        *self.increments.entry(self.replica_id).or_insert(0) += amount;
        self.version.increment(self.replica_id);
    }

    /// Decrement the counter by the given amount.
    pub fn decrement(&mut self, amount: u64) {
        *self.decrements.entry(self.replica_id).or_insert(0) += amount;
        self.version.increment(self.replica_id);
    }

    /// Get the current value (increments - decrements).
    pub fn value(&self) -> i64 {
        let inc: u64 = self.increments.values().sum();
        let dec: u64 = self.decrements.values().sum();
        inc as i64 - dec as i64
    }

    /// Get the replica ID for this counter.
    pub fn replica_id(&self) -> ReplicaId {
        self.replica_id
    }
}

impl Merge for PNCounter {
    /// Merge another counter into this one.
    ///
    /// For each replica ID, takes the maximum increment and decrement counts.
    fn merge(&mut self, other: &Self) {
        for (&replica, &count) in &other.increments {
            let entry = self.increments.entry(replica).or_insert(0);
            *entry = (*entry).max(count);
        }
        for (&replica, &count) in &other.decrements {
            let entry = self.decrements.entry(replica).or_insert(0);
            *entry = (*entry).max(count);
        }
        self.version.merge_vclock(&other.version);
    }
}

impl DeltaCrdt for PNCounter {
    type Delta = PNCounterDelta;

    fn delta_since(&self, since: &VClock) -> Option<Self::Delta> {
        // If we're at or behind the given version, no delta needed
        if since.dominates(&self.version) {
            return None;
        }

        // For simplicity, return full state as delta
        // A more efficient implementation would track only changes
        Some(PNCounterDelta {
            increments: self.increments.clone(),
            decrements: self.decrements.clone(),
            version: self.version.clone(),
        })
    }

    fn apply_delta(&mut self, delta: &Self::Delta) {
        // Merge the delta's state
        for (&replica, &count) in &delta.increments {
            let entry = self.increments.entry(replica).or_insert(0);
            *entry = (*entry).max(count);
        }
        for (&replica, &count) in &delta.decrements {
            let entry = self.decrements.entry(replica).or_insert(0);
            *entry = (*entry).max(count);
        }
        self.version.merge_vclock(&delta.version);
    }

    fn version(&self) -> VClock {
        self.version.clone()
    }
}

// NOTE: Showable impl is in logicaffeine_system (io module)

/// Allow comparing PNCounter to integers for ergonomic conditionals
impl PartialEq<i64> for PNCounter {
    fn eq(&self, other: &i64) -> bool {
        self.value() == *other
    }
}

impl PartialEq<i32> for PNCounter {
    fn eq(&self, other: &i32) -> bool {
        self.value() == (*other as i64)
    }
}

impl Default for PNCounter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_data/crdt/pncounter/tests.rs"]
mod tests;
