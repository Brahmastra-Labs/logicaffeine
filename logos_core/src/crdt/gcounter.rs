//! G-Counter (Grow-only Counter) CRDT
//!
//! A counter that can only be incremented, never decremented.
//! Each replica maintains its own local count, and the total value
//! is the sum of all replica counts.

use super::Merge;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A grow-only counter that supports distributed increment operations.
///
/// Each replica has a unique ID and maintains its own count.
/// The total value is the sum across all replicas.
/// Merging takes the maximum count for each replica ID.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GCounter {
    /// Map from replica ID to local count
    counts: HashMap<String, u64>,
    /// This replica's ID (set on first increment)
    replica_id: String,
}

impl GCounter {
    /// Create a new empty counter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a counter with a specific replica ID.
    pub fn with_replica_id(id: impl Into<String>) -> Self {
        Self {
            counts: HashMap::new(),
            replica_id: id.into(),
        }
    }

    /// Increment the counter by the given amount.
    ///
    /// If this is the first increment, a unique replica ID is generated.
    pub fn increment(&mut self, amount: u64) {
        if self.replica_id.is_empty() {
            self.replica_id = uuid::Uuid::new_v4().to_string();
        }
        *self.counts.entry(self.replica_id.clone()).or_insert(0) += amount;
    }

    /// Get the current value (sum of all replica counts).
    pub fn value(&self) -> u64 {
        self.counts.values().sum()
    }

    /// Get the replica ID for this counter.
    pub fn replica_id(&self) -> &str {
        &self.replica_id
    }
}

impl Merge for GCounter {
    /// Merge another counter into this one.
    ///
    /// For each replica ID, takes the maximum count between the two counters.
    /// This ensures convergence: merging A into B or B into A yields the same result.
    fn merge(&mut self, other: &Self) {
        for (replica, &count) in &other.counts {
            let entry = self.counts.entry(replica.clone()).or_insert(0);
            *entry = (*entry).max(count);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gcounter_new() {
        let c = GCounter::new();
        assert_eq!(c.value(), 0);
    }

    #[test]
    fn test_gcounter_increment() {
        let mut c = GCounter::with_replica_id("r1");
        c.increment(5);
        c.increment(3);
        assert_eq!(c.value(), 8);
    }

    #[test]
    fn test_gcounter_merge_disjoint() {
        let mut c1 = GCounter::with_replica_id("r1");
        let mut c2 = GCounter::with_replica_id("r2");

        c1.increment(5);
        c2.increment(3);

        c1.merge(&c2);
        assert_eq!(c1.value(), 8);
    }

    #[test]
    fn test_gcounter_merge_commutative() {
        let mut c1 = GCounter::with_replica_id("r1");
        let mut c2 = GCounter::with_replica_id("r2");

        c1.increment(5);
        c2.increment(3);

        let mut c1_copy = c1.clone();
        let mut c2_copy = c2.clone();

        c1_copy.merge(&c2);
        c2_copy.merge(&c1);

        assert_eq!(c1_copy.value(), c2_copy.value());
    }

    #[test]
    fn test_gcounter_merge_idempotent() {
        let mut c1 = GCounter::with_replica_id("r1");
        c1.increment(5);

        let before = c1.value();
        c1.merge(&c1.clone());
        assert_eq!(c1.value(), before);
    }

    #[test]
    fn test_gcounter_merge_same_replica() {
        // When two counters have the same replica ID (simulating sync after divergence)
        let mut c1 = GCounter::with_replica_id("r1");
        let mut c2 = GCounter::with_replica_id("r1");

        c1.increment(5);
        c2.increment(3);

        // After merge, should have max(5, 3) = 5
        c1.merge(&c2);
        assert_eq!(c1.value(), 5);
    }
}
