//! A last-writer register ordered by LAMPORT time, for the values of a shared map.
//!
//! Unlike [`super::LWWRegister`] (whose caller supplies a timestamp), a `LatestWrite` is stamped by the
//! map that holds it with the number of events that map has observed — so a write that has SEEN another
//! write always carries the larger stamp. Two concurrent writes can share a stamp; the larger value then
//! wins. Ordering by `(stamp, value)` makes the merge a join over a total order: commutative,
//! associative, idempotent, and independent of replica ids and wall clocks.

use super::Merge;
use serde::{Deserialize, Serialize};

/// One value of a shared map, with the Lamport stamp of the write that produced it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatestWrite<T> {
    stamp: u64,
    value: T,
}

impl<T> LatestWrite<T> {
    /// A register holding `value`, written at Lamport time `stamp`.
    pub fn new(value: T, stamp: u64) -> Self {
        Self { stamp, value }
    }

    /// The current value.
    pub fn get(&self) -> &T {
        &self.value
    }

    /// The Lamport stamp of the write that produced the current value.
    pub fn stamp(&self) -> u64 {
        self.stamp
    }

    /// Overwrite with `value` at Lamport time `stamp`.
    pub fn write(&mut self, value: T, stamp: u64) {
        self.stamp = stamp;
        self.value = value;
    }
}

impl<T: Ord + Clone> Merge for LatestWrite<T> {
    fn merge(&mut self, other: &Self) {
        if (other.stamp, &other.value) > (self.stamp, &self.value) {
            self.stamp = other.stamp;
            self.value = other.value.clone();
        }
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_data/crdt/latest/tests.rs"]
mod tests;
