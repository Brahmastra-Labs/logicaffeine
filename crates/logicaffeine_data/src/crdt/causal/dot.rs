//! Dot - Unique event identifier.
//!
//! A Dot uniquely identifies an event in a CRDT's history.

use super::super::replica::ReplicaId;
use serde::{Deserialize, Serialize};

/// A unique event identifier in a CRDT's history.
///
/// A Dot represents a single event that occurred at a specific replica
/// with a specific sequence number. Two dots are equal if and only if
/// they have the same replica and counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Dot {
    /// The replica that generated this event.
    pub replica: ReplicaId,
    /// The sequence number of this event at the replica.
    pub counter: u64,
}

impl Dot {
    /// Create a new Dot with the given replica and counter.
    pub fn new(replica: ReplicaId, counter: u64) -> Self {
        Self { replica, counter }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_dot_creation() {
        let dot = Dot::new(42, 1);
        assert_eq!(dot.replica, 42);
        assert_eq!(dot.counter, 1);
    }

    #[test]
    fn test_dot_equality() {
        let a = Dot::new(1, 5);
        let b = Dot::new(1, 5);
        let c = Dot::new(1, 6);
        let d = Dot::new(2, 5);

        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
    }

    #[test]
    fn test_dot_hash() {
        let mut set = HashSet::new();
        set.insert(Dot::new(1, 1));
        set.insert(Dot::new(1, 2));
        set.insert(Dot::new(1, 1)); // Duplicate

        assert_eq!(set.len(), 2);
    }
}
