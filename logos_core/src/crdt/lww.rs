//! Last-Write-Wins Register CRDT
//!
//! A register that resolves conflicts using timestamps.
//! The value with the highest timestamp wins on merge.

use super::Merge;
use crate::io::Showable;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

/// A register that resolves conflicts using "last write wins" semantics.
///
/// Each write records a timestamp, and on merge the value with
/// the higher timestamp is kept.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LWWRegister<T> {
    value: T,
    /// Microseconds since UNIX epoch
    timestamp: u64,
}

impl<T: Default> Default for LWWRegister<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T> LWWRegister<T> {
    /// Create a new register with the given initial value.
    pub fn new(value: T) -> Self {
        Self {
            value,
            timestamp: Self::now(),
        }
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64
    }

    /// Set a new value (updates timestamp to now).
    pub fn set(&mut self, value: T) {
        self.value = value;
        self.timestamp = Self::now();
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

impl<T: Showable> Showable for LWWRegister<T> {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.value.format_show(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lww_new() {
        let reg = LWWRegister::new("hello".to_string());
        assert_eq!(reg.get(), "hello");
    }

    #[test]
    fn test_lww_set() {
        let mut reg = LWWRegister::new("hello".to_string());
        reg.set("world".to_string());
        assert_eq!(reg.get(), "world");
    }

    #[test]
    fn test_lww_merge_newer_wins() {
        let r1 = LWWRegister::new("old".to_string());
        std::thread::sleep(std::time::Duration::from_millis(1));
        let r2 = LWWRegister::new("new".to_string());

        let mut r1_copy = r1.clone();
        r1_copy.merge(&r2);
        assert_eq!(r1_copy.get(), "new");
    }

    #[test]
    fn test_lww_merge_older_loses() {
        let r1 = LWWRegister::new("old".to_string());
        std::thread::sleep(std::time::Duration::from_millis(1));
        let r2 = LWWRegister::new("new".to_string());

        let mut r2_copy = r2.clone();
        r2_copy.merge(&r1);
        // r2 had higher timestamp, so it keeps its value
        assert_eq!(r2_copy.get(), "new");
    }

    #[test]
    fn test_lww_merge_idempotent() {
        let reg = LWWRegister::new("test".to_string());
        let mut reg_copy = reg.clone();
        reg_copy.merge(&reg);
        assert_eq!(reg_copy.get(), "test");
    }

    #[test]
    fn test_lww_with_int() {
        let mut reg = LWWRegister::new(42i64);
        assert_eq!(*reg.get(), 42);
        reg.set(100);
        assert_eq!(*reg.get(), 100);
    }

    #[test]
    fn test_lww_with_bool() {
        let mut reg = LWWRegister::new(false);
        assert_eq!(*reg.get(), false);
        reg.set(true);
        assert_eq!(*reg.get(), true);
    }
}
