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
mod tests {
    use super::*;

    #[test]
    fn test_lww_new() {
        let reg = LWWRegister::new("hello".to_string(), 100);
        assert_eq!(reg.get(), "hello");
        assert_eq!(reg.timestamp(), 100);
    }

    #[test]
    fn test_lww_set() {
        let mut reg = LWWRegister::new("hello".to_string(), 100);
        reg.set("world".to_string(), 200);
        assert_eq!(reg.get(), "world");
        assert_eq!(reg.timestamp(), 200);
    }

    #[test]
    fn test_lww_merge_newer_wins() {
        let r1 = LWWRegister::new("old".to_string(), 100);
        let r2 = LWWRegister::new("new".to_string(), 200);

        let mut r1_copy = r1.clone();
        r1_copy.merge(&r2);
        assert_eq!(r1_copy.get(), "new");
    }

    #[test]
    fn test_lww_merge_older_loses() {
        let r1 = LWWRegister::new("old".to_string(), 100);
        let r2 = LWWRegister::new("new".to_string(), 200);

        let mut r2_copy = r2.clone();
        r2_copy.merge(&r1);
        // r2 had higher timestamp, so it keeps its value
        assert_eq!(r2_copy.get(), "new");
    }

    #[test]
    fn test_lww_merge_idempotent() {
        let reg = LWWRegister::new("test".to_string(), 100);
        let mut reg_copy = reg.clone();
        reg_copy.merge(&reg);
        assert_eq!(reg_copy.get(), "test");
    }

    #[test]
    fn test_lww_with_int() {
        let mut reg = LWWRegister::new(42i64, 100);
        assert_eq!(*reg.get(), 42);
        reg.set(100, 200);
        assert_eq!(*reg.get(), 100);
    }

    #[test]
    fn test_lww_with_bool() {
        let mut reg = LWWRegister::new(false, 100);
        assert_eq!(*reg.get(), false);
        reg.set(true, 200);
        assert_eq!(*reg.get(), true);
    }
}
