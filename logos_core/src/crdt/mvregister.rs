//! MV-Register (Multi-Value Register) CRDT
//!
//! Wave 2.2: A register that preserves all concurrent writes.
//! When conflicts occur, all conflicting values are retained until resolved.

use super::causal::VClock;
use super::delta::DeltaCrdt;
use super::replica::{generate_replica_id, ReplicaId};
use super::Merge;
use crate::io::Showable;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Delta for MVRegister synchronization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MVRegisterDelta<T> {
    pub values: Vec<(T, VClock)>,
}

/// A register that keeps all concurrent values.
///
/// Unlike LWW-Register which silently picks a winner, MVRegister
/// preserves all concurrent writes so conflicts can be detected and resolved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MVRegister<T> {
    /// Each value paired with its vector clock
    values: Vec<(T, VClock)>,
    /// This replica's ID
    replica_id: ReplicaId,
}

impl<T> MVRegister<T> {
    /// Create a new empty register with a specific replica ID.
    pub fn new(replica_id: ReplicaId) -> Self {
        Self {
            values: Vec::new(),
            replica_id,
        }
    }

    /// Create a new register with a random replica ID.
    pub fn new_random() -> Self {
        Self::new(generate_replica_id())
    }

    /// Get the replica ID for this register.
    pub fn replica_id(&self) -> ReplicaId {
        self.replica_id
    }
}

impl<T: Clone + PartialEq> MVRegister<T> {
    /// Set a new value, creating a new version that dominates all current values.
    pub fn set(&mut self, value: T) {
        // Create a clock that dominates all current clocks
        let mut new_clock = VClock::new();
        for (_, clock) in &self.values {
            new_clock.merge_vclock(clock);
        }
        new_clock.increment(self.replica_id);

        // Replace all values with the new one
        self.values = vec![(value, new_clock)];
    }

    /// Get all current values.
    ///
    /// If there's only one value, there's no conflict.
    /// Multiple values indicate concurrent writes that need resolution.
    pub fn values(&self) -> Vec<&T> {
        self.values.iter().map(|(v, _)| v).collect()
    }

    /// Resolve a conflict by setting a new value.
    ///
    /// This is the same as `set`, but semantically indicates conflict resolution.
    pub fn resolve(&mut self, value: T) {
        self.set(value);
    }

    /// Check if there's a conflict (more than one value).
    pub fn has_conflict(&self) -> bool {
        self.values.len() > 1
    }
}

impl<T: Clone + PartialEq> Merge for MVRegister<T> {
    /// Merge another register into this one.
    ///
    /// Keeps values that are not dominated by any other value.
    fn merge(&mut self, other: &Self) {
        // Collect all values from both registers
        let mut all_values: Vec<(T, VClock)> = self.values.clone();
        all_values.extend(other.values.clone());

        // Keep only values that are not dominated by any other
        let mut result: Vec<(T, VClock)> = Vec::new();

        for (value, clock) in &all_values {
            let is_dominated = all_values.iter().any(|(_, other_clock)| {
                other_clock.dominates(clock) && other_clock != clock
            });

            if !is_dominated {
                // Check if we already have this exact value+clock
                let already_exists = result
                    .iter()
                    .any(|(v, c)| v == value && c == clock);

                if !already_exists {
                    result.push((value.clone(), clock.clone()));
                }
            }
        }

        self.values = result;
    }
}

impl<T: Clone + PartialEq + Serialize + DeserializeOwned + Send + 'static> DeltaCrdt
    for MVRegister<T>
{
    type Delta = MVRegisterDelta<T>;

    fn delta_since(&self, since: &VClock) -> Option<Self::Delta> {
        let current = self.version();
        if since.dominates(&current) {
            return None;
        }

        // Return all values as delta
        Some(MVRegisterDelta {
            values: self.values.clone(),
        })
    }

    fn apply_delta(&mut self, delta: &Self::Delta) {
        // Merge the delta values using MVRegister merge semantics
        let mut all_values: Vec<(T, VClock)> = self.values.clone();
        all_values.extend(delta.values.clone());

        let mut result: Vec<(T, VClock)> = Vec::new();

        for (value, clock) in &all_values {
            let is_dominated = all_values.iter().any(|(_, other_clock)| {
                other_clock.dominates(clock) && other_clock != clock
            });

            if !is_dominated {
                let already_exists = result.iter().any(|(v, c)| v == value && c == clock);

                if !already_exists {
                    result.push((value.clone(), clock.clone()));
                }
            }
        }

        self.values = result;
    }

    fn version(&self) -> VClock {
        // Version is the merge of all value clocks
        let mut combined = VClock::new();
        for (_, clock) in &self.values {
            combined.merge_vclock(clock);
        }
        combined
    }
}

impl<T: Clone + PartialEq + Default> Default for MVRegister<T> {
    fn default() -> Self {
        Self::new_random()
    }
}


impl<T: Clone + PartialEq + fmt::Display> Showable for MVRegister<T> {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.values.len() {
            0 => write!(f, "<empty>"),
            1 => write!(f, "{}", self.values[0].0),
            n => {
                write!(f, "[CONFLICT: {} values] ", n)?;
                for (i, (v, _)) in self.values.iter().enumerate() {
                    if i > 0 {
                        write!(f, " | ")?;
                    }
                    write!(f, "{}", v)?;
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mvregister_new() {
        let reg: MVRegister<String> = MVRegister::new(1);
        assert!(reg.values().is_empty());
    }

    #[test]
    fn test_mvregister_set_get() {
        let mut reg: MVRegister<String> = MVRegister::new(1);
        reg.set("hello".to_string());
        assert_eq!(reg.values().len(), 1);
        assert_eq!(reg.values()[0], &"hello".to_string());
    }

    #[test]
    fn test_mvregister_concurrent() {
        let mut a: MVRegister<String> = MVRegister::new(1);
        let mut b: MVRegister<String> = MVRegister::new(2);

        a.set("from-a".to_string());
        b.set("from-b".to_string());
        a.merge(&b);

        assert_eq!(a.values().len(), 2);
    }

    #[test]
    fn test_mvregister_resolve() {
        let mut a: MVRegister<String> = MVRegister::new(1);
        let mut b: MVRegister<String> = MVRegister::new(2);

        a.set("from-a".to_string());
        b.set("from-b".to_string());
        a.merge(&b);
        a.resolve("resolved".to_string());

        assert_eq!(a.values().len(), 1);
        assert_eq!(a.values()[0], &"resolved".to_string());
    }
}
