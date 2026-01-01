//! OR-Set (Observed-Remove Set) CRDT with configurable bias
//!
//! Wave 3.1: A set that supports add and remove with configurable conflict resolution.
//! Default is AddWins (concurrent add beats remove), can be configured to RemoveWins.

use super::causal::{Dot, DotContext, VClock};
use super::delta::DeltaCrdt;
use super::replica::{generate_replica_id, ReplicaId};
use super::Merge;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::marker::PhantomData;

/// Delta for ORSet synchronization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound = "T: Serialize + serde::de::DeserializeOwned + Hash + Eq")]
pub struct ORSetDelta<T> {
    pub entries: HashMap<T, HashSet<Dot>>,
    pub context: DotContext,
}

/// Bias for concurrent add/remove resolution.
pub trait SetBias: Default + Clone + Send + 'static {
    /// Resolve whether to keep an element based on concurrent operations.
    /// - `local_has_dots`: this replica has active dots for the element
    /// - `remote_has_dots`: other replica has active dots for the element
    /// - `local_removed`: this replica saw and removed the element
    /// - `remote_removed`: other replica saw and removed the element
    fn resolve(
        local_has_dots: bool,
        remote_has_dots: bool,
        local_removed: bool,
        remote_removed: bool,
    ) -> bool;
}

/// Add-wins bias: concurrent add beats remove.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct AddWins;

impl SetBias for AddWins {
    fn resolve(
        local_has_dots: bool,
        remote_has_dots: bool,
        _local_removed: bool,
        _remote_removed: bool,
    ) -> bool {
        // Add wins: keep if either side has dots
        local_has_dots || remote_has_dots
    }
}

/// Remove-wins bias: concurrent remove beats add.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct RemoveWins;

impl SetBias for RemoveWins {
    fn resolve(
        local_has_dots: bool,
        remote_has_dots: bool,
        local_removed: bool,
        remote_removed: bool,
    ) -> bool {
        // Remove wins: if either side explicitly removed, remove
        if local_removed || remote_removed {
            false
        } else {
            // No explicit removals, keep if dots exist
            local_has_dots || remote_has_dots
        }
    }
}

/// An observed-remove set with configurable bias.
///
/// Each element is tagged with dots (event identifiers). When removing,
/// we record which dots we've seen. On merge, we compare dots to determine
/// if adds/removes are concurrent or causal.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound = "T: Serialize + serde::de::DeserializeOwned + Hash + Eq")]
pub struct ORSet<T, B: SetBias = AddWins> {
    /// Map from element to its active dots
    entries: HashMap<T, HashSet<Dot>>,
    /// Tracks all dots we've seen
    context: DotContext,
    /// This replica's ID
    replica_id: ReplicaId,
    /// Bias marker
    #[serde(skip)]
    _bias: PhantomData<B>,
}

impl<T: Hash + Eq + Clone, B: SetBias> ORSet<T, B> {
    /// Create a new set with a specific replica ID.
    pub fn new(replica_id: ReplicaId) -> Self {
        Self {
            entries: HashMap::new(),
            context: DotContext::new(),
            replica_id,
            _bias: PhantomData,
        }
    }

    /// Create a new set with a random replica ID.
    pub fn new_random() -> Self {
        Self::new(generate_replica_id())
    }

    /// Add an element to the set.
    pub fn add(&mut self, value: T) {
        let dot = self.context.next(self.replica_id);
        self.entries.entry(value).or_default().insert(dot);
    }

    /// Alias for `add` - for compatibility with HashSet API.
    pub fn insert(&mut self, value: T) {
        self.add(value);
    }

    /// Remove an element from the set.
    ///
    /// This removes all dots associated with the element.
    pub fn remove(&mut self, value: &T) {
        self.entries.remove(value);
    }

    /// Check if the set contains an element.
    pub fn contains(&self, value: &T) -> bool {
        self.entries
            .get(value)
            .map_or(false, |dots| !dots.is_empty())
    }

    /// Get the number of elements in the set.
    pub fn len(&self) -> usize {
        self.entries
            .values()
            .filter(|dots| !dots.is_empty())
            .count()
    }

    /// Check if the set is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iterate over elements in the set.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.entries
            .iter()
            .filter(|(_, dots)| !dots.is_empty())
            .map(|(v, _)| v)
    }

    /// Get the replica ID for this set.
    pub fn replica_id(&self) -> ReplicaId {
        self.replica_id
    }
}

// Phase 49b: Manual PartialEq impl with proper bounds
impl<T: Hash + Eq, B: SetBias> PartialEq for ORSet<T, B> {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries && self.context == other.context
    }
}

impl<T: Hash + Eq + Clone, B: SetBias> Merge for ORSet<T, B> {
    fn merge(&mut self, other: &Self) {
        // Collect all keys from both sets
        let all_keys: HashSet<T> = self
            .entries
            .keys()
            .chain(other.entries.keys())
            .cloned()
            .collect();

        for value in all_keys {
            let my_dots_before: HashSet<Dot> = self
                .entries
                .get(&value)
                .cloned()
                .unwrap_or_default();
            let other_dots: HashSet<Dot> = other
                .entries
                .get(&value)
                .cloned()
                .unwrap_or_default();

            // Detect if either side "removed" the element:
            // A side has removed if they've seen some dots for this element
            // but currently have no dots (or fewer dots than they've seen).
            // We approximate by: has context seen any dots that are no longer present?
            let my_removed = my_dots_before.is_empty()
                && other_dots.iter().any(|dot| self.context.has_seen(dot));
            let other_removed = other_dots.is_empty()
                && my_dots_before.iter().any(|dot| other.context.has_seen(dot));

            // Compute surviving dots
            let mut combined_dots: HashSet<Dot> = HashSet::new();

            // Add my dots that other hasn't seen or still has
            for dot in &my_dots_before {
                if !other.context.has_seen(dot) || other_dots.contains(dot) {
                    combined_dots.insert(*dot);
                }
            }

            // Add other's dots that I haven't seen or still have
            for dot in &other_dots {
                if !self.context.has_seen(dot) || my_dots_before.contains(dot) {
                    combined_dots.insert(*dot);
                }
            }

            let my_has_dots = !my_dots_before.is_empty();
            let other_has_dots = !other_dots.is_empty();

            // Apply bias
            let keep = B::resolve(my_has_dots, other_has_dots, my_removed, other_removed);

            let my_dots = self.entries.entry(value).or_default();
            if keep {
                *my_dots = combined_dots;
            } else {
                my_dots.clear();
            }
        }

        // Merge contexts
        self.context.merge(&other.context);

        // Clean up empty entries
        self.entries.retain(|_, dots| !dots.is_empty());
    }
}

impl<T: Hash + Eq + Clone + Serialize + DeserializeOwned + Send + 'static, B: SetBias> DeltaCrdt
    for ORSet<T, B>
{
    type Delta = ORSetDelta<T>;

    fn delta_since(&self, since: &VClock) -> Option<Self::Delta> {
        let current = self.version();
        if since.dominates(&current) {
            return None;
        }

        // Return full state as delta
        Some(ORSetDelta {
            entries: self.entries.clone(),
            context: self.context.clone(),
        })
    }

    fn apply_delta(&mut self, delta: &Self::Delta) {
        // Create a temporary ORSet from the delta and merge
        let temp: ORSet<T, B> = ORSet {
            entries: delta.entries.clone(),
            context: delta.context.clone(),
            replica_id: 0, // Doesn't matter for merge
            _bias: PhantomData,
        };
        self.merge(&temp);
    }

    fn version(&self) -> VClock {
        self.context.version()
    }
}

impl<T: Hash + Eq + Clone, B: SetBias> Default for ORSet<T, B> {
    fn default() -> Self {
        Self::new_random()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_orset_add_contains() {
        let mut set: ORSet<String> = ORSet::new(1);
        set.add("alice".to_string());
        assert!(set.contains(&"alice".to_string()));
        assert!(!set.contains(&"bob".to_string()));
    }

    #[test]
    fn test_orset_remove() {
        let mut set: ORSet<String> = ORSet::new(1);
        set.add("alice".to_string());
        set.remove(&"alice".to_string());
        assert!(!set.contains(&"alice".to_string()));
    }

    #[test]
    fn test_orset_add_wins() {
        let mut a: ORSet<String> = ORSet::new(1);
        let mut b: ORSet<String> = ORSet::new(2);

        a.add("item".to_string());
        b.merge(&a);

        a.remove(&"item".to_string());
        b.add("item".to_string());

        a.merge(&b);
        assert!(a.contains(&"item".to_string()));
    }

    #[test]
    fn test_orset_merge_commutative() {
        let mut a: ORSet<String> = ORSet::new(1);
        let mut b: ORSet<String> = ORSet::new(2);

        a.add("x".to_string());
        b.add("y".to_string());

        let mut a1 = a.clone();
        let mut b1 = b.clone();
        a1.merge(&b);
        b1.merge(&a);

        assert_eq!(a1.len(), b1.len());
    }
}
