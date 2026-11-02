//! Last-Write-Wins Register CRDT
//!
//! A register that resolves conflicts using timestamps.
//! The value with the highest timestamp wins on merge.
//!
//! ## Time Injection (Lamport Invariant)
//!
//! This type is pure - it does not access system time.
//! Callers must provide timestamps, enabling WASM compatibility.

use super::Merge;
use serde::{Deserialize, Serialize};

/// A register that resolves conflicts using "last write wins" semantics.
///
/// Each write records a timestamp, and on merge the value with
/// the higher timestamp is kept.
///
/// ## Time Injection
///
/// Timestamps must be provided by the caller. This makes the type
/// pure and WASM-compatible. Use a logical clock or system time
/// at the call site.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LWWRegister<T> {
    value: T,
    /// Logical timestamp (e.g., microseconds since epoch, or lamport clock)
    timestamp: u64,
}

impl<T: Default> Default for LWWRegister<T> {
    fn default() -> Self {
        Self::new(T::default(), 0)
    }
}

impl<T> LWWRegister<T> {
    /// Create a new register with the given initial value and timestamp.
    ///
    /// The timestamp should be provided by the caller (e.g., from system time
    /// or a logical clock). This enables WASM compatibility.
    pub fn new(value: T, timestamp: u64) -> Self {
        Self { value, timestamp }
    }

    /// Set a new value with the given timestamp.
    ///
    /// The timestamp should be greater than or equal to the current timestamp
    /// to ensure the new value takes precedence on merge.
    pub fn set(&mut self, value: T, timestamp: u64) {
        self.value = value;
        self.timestamp = timestamp;
    }

    /// Get the current value.
    pub fn get(&self) -> &T {
        &self.value
    }

    /// Get the timestamp of the last write.
    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }
}

impl<T: Clone> Merge for LWWRegister<T> {
    /// Merge another register into this one.
    ///
    /// The value with the higher timestamp wins.
    /// If timestamps are equal, the other value wins (arbitrary but deterministic).
    fn merge(&mut self, other: &Self) {
        if other.timestamp >= self.timestamp {
            self.value = other.value.clone();
            self.timestamp = other.timestamp;
        }
    }
}

// NOTE: Showable impl is in logicaffeine_system (io module)

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_data/crdt/lww/tests.rs"]
mod tests;
