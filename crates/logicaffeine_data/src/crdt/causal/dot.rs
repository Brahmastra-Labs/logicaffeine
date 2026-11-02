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
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_data/crdt/causal/dot/tests.rs"]
mod tests;
