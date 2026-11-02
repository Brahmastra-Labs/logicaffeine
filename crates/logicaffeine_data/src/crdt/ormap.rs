//! OR-Map (Observed-Remove Map) CRDT.
//!
//! A key-value map where values are nested CRDTs.
//! Keys use add-wins semantics, values are merged recursively.

use super::causal::{Dot, DotContext, VClock};
use super::delta::DeltaCrdt;
use super::latest::LatestWrite;
use super::replica::{generate_replica_id, ReplicaId};
use super::Merge;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

/// Delta for ORMap synchronization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    bound = "K: Serialize + serde::de::DeserializeOwned + Hash + Eq, V: Serialize + serde::de::DeserializeOwned"
)]
pub struct ORMapDelta<K, V> {
    pub keys: HashMap<K, HashSet<Dot>>,
    pub values: HashMap<K, V>,
    pub context: DotContext,
}

/// An observed-remove map with nested CRDT values.
///
/// Keys are managed with OR-Set add-wins semantics.
/// Values are merged using their CRDT merge function.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    bound = "K: Serialize + serde::de::DeserializeOwned + Hash + Eq, V: Serialize + serde::de::DeserializeOwned + Merge + Default + Clone"
)]
pub struct ORMap<K, V: Merge + Default + Clone> {
    /// Map of active keys to their dots
    keys: HashMap<K, HashSet<Dot>>,
    /// Map of all values (including for removed keys, for resurrection)
    values: HashMap<K, V>,
    /// Tracks seen dots
    context: DotContext,
    /// This replica's ID
    replica_id: ReplicaId,
}

impl<K: Hash + Eq + Clone, V: Merge + Default + Clone> ORMap<K, V> {
    /// Create a new map with a specific replica ID.
    pub fn new(replica_id: ReplicaId) -> Self {
        Self {
            keys: HashMap::new(),
            values: HashMap::new(),
            context: DotContext::new(),
            replica_id,
        }
    }

    /// Create a new map with a random replica ID.
    pub fn new_random() -> Self {
        Self::new(generate_replica_id())
    }

    /// Get a reference to a value if the key exists.
    pub fn get(&self, key: &K) -> Option<&V> {
        if self.contains_key(key) {
            self.values.get(key)
        } else {
            None
        }
    }

    /// Get a mutable reference to a value, creating the key if necessary.
    /// Always creates a new dot to ensure add-wins semantics.
    pub fn get_or_insert(&mut self, key: K) -> &mut V {
        // Always create a new dot for add-wins semantics
        let dot = self.context.next(self.replica_id);
        self.keys.entry(key.clone()).or_default().insert(dot);

        // Ensure value exists
        self.values.entry(key).or_default()
    }

    /// Get a mutable reference to a value without creating a new dot.
    /// Use this for read-heavy access patterns where add-wins isn't needed.
    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        if self.contains_key(key) {
            self.values.get_mut(key)
        } else {
            None
        }
    }

    /// Check if a key exists in the map.
    pub fn contains_key(&self, key: &K) -> bool {
        self.keys.get(key).map_or(false, |dots| !dots.is_empty())
    }

    /// Remove a key from the map.
    pub fn remove(&mut self, key: &K) {
        self.keys.remove(key);
    }

    /// Get the number of keys.
    pub fn len(&self) -> usize {
        self.keys.values().filter(|dots| !dots.is_empty()).count()
    }

    /// Check if the map is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iterate over active keys.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.keys
            .iter()
            .filter(|(_, dots)| !dots.is_empty())
            .map(|(k, _)| k)
    }

    /// Iterate over key-value pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.keys
            .iter()
            .filter(|(_, dots)| !dots.is_empty())
            .filter_map(|(k, _)| self.values.get(k).map(|v| (k, v)))
    }
}

impl<K: Hash + Eq + Clone + Ord, V: Merge + Default + Clone> ORMap<K, V> {
    /// The present entries sorted by key. A merged map has no global insertion order, so this is the
    /// order every replica holding the same state renders and iterates in.
    pub fn sorted_entries(&self) -> Vec<(&K, &V)> {
        let mut entries: Vec<(&K, &V)> = self.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        entries
    }
}

/// A map to plain values: each value is a [`LatestWrite`] stamped with this map's Lamport time (the
/// events its causal context has observed), so a write that has seen another beats it on every replica.
impl<K: Hash + Eq + Clone, T: Ord + Clone + Default> ORMap<K, LatestWrite<T>> {
    /// `Set m[key] to value`.
    pub fn put(&mut self, key: K, value: T) {
        self.get_or_insert(key.clone());
        let stamp = self.context.observed();
        self.values
            .get_mut(&key)
            .expect("get_or_insert created the entry")
            .write(value, stamp);
    }

    /// The value under `key`, if the key is present.
    pub fn value(&self, key: &K) -> Option<&T> {
        self.get(key).map(LatestWrite::get)
    }
}

impl<K: Hash + Eq + Clone, V: Merge + Default + Clone> Merge for ORMap<K, V> {
    fn merge(&mut self, other: &Self) {
        // First, merge all values (regardless of key presence)
        // This ensures nested CRDTs are properly merged
        for (key, other_value) in &other.values {
            let my_value = self.values.entry(key.clone()).or_default();
            my_value.merge(other_value);
        }

        // Merge keys with OR-Set add-wins semantics
        for (key, other_dots) in &other.keys {
            let my_dots = self.keys.entry(key.clone()).or_default();

            // Keep dots from other that we haven't seen
            for &dot in other_dots {
                if !self.context.has_seen(&dot) {
                    my_dots.insert(dot);
                }
            }
        }

        // Handle removals: if we have a key that other doesn't, check if they saw our dots
        let my_keys: Vec<_> = self.keys.keys().cloned().collect();
        for key in my_keys {
            if !other.keys.contains_key(&key) {
                // Other doesn't have this key
                if let Some(my_dots) = self.keys.get_mut(&key) {
                    // Only keep dots that other hasn't seen
                    my_dots.retain(|dot| !other.context.has_seen(dot));
                }
            } else {
                // Both have the key
                let other_dots = other.keys.get(&key).unwrap();
                if let Some(my_dots) = self.keys.get_mut(&key) {
                    // Keep dots that other hasn't seen OR that other still has
                    my_dots.retain(|dot| !other.context.has_seen(dot) || other_dots.contains(dot));
                }
            }
        }

        // Merge contexts
        self.context.merge(&other.context);

        // Clean up empty key entries
        self.keys.retain(|_, dots| !dots.is_empty());
    }
}

impl<
        K: Hash + Eq + Clone + Serialize + DeserializeOwned + Send + 'static,
        V: Merge + Default + Clone + Serialize + DeserializeOwned + Send + 'static,
    > DeltaCrdt for ORMap<K, V>
{
    type Delta = ORMapDelta<K, V>;

    fn delta_since(&self, since: &VClock) -> Option<Self::Delta> {
        let current = self.version();
        if since.dominates(&current) {
            return None;
        }

        Some(ORMapDelta {
            keys: self.keys.clone(),
            values: self.values.clone(),
            context: self.context.clone(),
        })
    }

    fn apply_delta(&mut self, delta: &Self::Delta) {
        // Merge values
        for (key, other_value) in &delta.values {
            let my_value = self.values.entry(key.clone()).or_default();
            my_value.merge(other_value);
        }

        // Merge keys
        for (key, other_dots) in &delta.keys {
            let my_dots = self.keys.entry(key.clone()).or_default();
            for &dot in other_dots {
                if !self.context.has_seen(&dot) {
                    my_dots.insert(dot);
                }
            }
        }

        // Merge context
        self.context.merge(&delta.context);

        // Clean up empty entries
        self.keys.retain(|_, dots| !dots.is_empty());
    }

    fn version(&self) -> VClock {
        self.context.version()
    }
}

impl<K: Hash + Eq + Clone, V: Merge + Default + Clone> Default for ORMap<K, V> {
    fn default() -> Self {
        Self::new_random()
    }
}

/// Owned iteration over a map to plain values visits `(key, value)` pairs by key.
impl<K: Hash + Eq + Clone + Ord, T: Ord + Clone + Default> IntoIterator
    for ORMap<K, LatestWrite<T>>
{
    type Item = (K, T);
    type IntoIter = std::vec::IntoIter<(K, T)>;

    fn into_iter(self) -> Self::IntoIter {
        self.sorted_entries()
            .into_iter()
            .map(|(k, v)| (k.clone(), v.get().clone()))
            .collect::<Vec<_>>()
            .into_iter()
    }
}

/// Two maps are equal when they hold the same convergent state (keys with their dots, values, and
/// causal context) — the same rule as `ORSet`; the replica id is identity, not state.
impl<K: Hash + Eq, V: Merge + Default + Clone + PartialEq> PartialEq for ORMap<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.keys == other.keys && self.values == other.values && self.context == other.context
    }
}

/// `m[key]` on a map to plain values: the value under a PRESENT key; a missing key traps with the
/// same message the interpreter raises.
impl<K: Hash + Eq + Clone + std::fmt::Display, T: Ord + Clone + Default> crate::LogosIndex<K>
    for ORMap<K, LatestWrite<T>>
{
    type Output = T;

    fn logos_get(&self, key: K) -> T {
        match self.value(&key) {
            Some(v) => v.clone(),
            None => panic!("Key '{key}' not found in map"),
        }
    }
}

/// `Set m[key] to value` on a map to plain values.
impl<K: Hash + Eq + Clone + std::fmt::Display, T: Ord + Clone + Default> crate::LogosIndexMut<K>
    for ORMap<K, LatestWrite<T>>
{
    fn logos_set(&mut self, key: K, value: T) {
        self.put(key, value);
    }
}

/// `m contains key` — observed-remove key membership.
impl<K: Hash + Eq + Clone, V: Merge + Default + Clone> crate::LogosContains<K> for ORMap<K, V> {
    fn logos_contains(&self, key: &K) -> bool {
        self.contains_key(key)
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_data/crdt/ormap/tests.rs"]
mod tests;
