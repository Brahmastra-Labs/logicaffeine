//! YATA (Yet Another Transformation Approach) CRDT
//!
//! Wave 3.3: A sequence CRDT optimized for collaborative text editing.
//! Uses origin-left and origin-right to handle concurrent insertions.

use crate::crdt::causal::VClock;
use crate::crdt::delta::DeltaCrdt;
use crate::crdt::replica::{generate_replica_id, ReplicaId};
use crate::crdt::Merge;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// Delta for YATA synchronization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YATADelta<T> {
    pub items: Vec<YataItem<T>>,
    pub clock: u64,
}

/// Unique identifier for a YATA item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct YataId {
    /// Logical clock
    pub clock: u64,
    /// Replica that created this item
    pub replica: ReplicaId,
}

impl YataId {
    fn new(clock: u64, replica: ReplicaId) -> Self {
        Self { clock, replica }
    }
}

impl Ord for YataId {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.clock.cmp(&other.clock) {
            Ordering::Equal => self.replica.cmp(&other.replica),
            ord => ord,
        }
    }
}

impl PartialOrd for YataId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A YATA item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct YataItem<T> {
    pub id: YataId,
    pub value: T,
    pub deleted: bool,
    /// The item this was inserted to the left of (origin)
    pub origin_left: Option<YataId>,
    /// The item this was inserted to the right of (for tie-breaking)
    pub origin_right: Option<YataId>,
}

/// YATA sequence CRDT.
///
/// Better handling of interleaving for collaborative text editing
/// compared to RGA.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct YATA<T> {
    items: Vec<YataItem<T>>,
    clock: u64,
    replica_id: ReplicaId,
}

impl<T: Clone + PartialEq> YATA<T> {
    /// Create a new empty YATA sequence.
    pub fn new(replica_id: ReplicaId) -> Self {
        Self {
            items: Vec::new(),
            clock: 0,
            replica_id,
        }
    }

    /// Create with random replica ID.
    pub fn new_random() -> Self {
        Self::new(generate_replica_id())
    }

    /// Append a value to the end.
    pub fn append(&mut self, value: T) {
        self.clock += 1;
        let id = YataId::new(self.clock, self.replica_id);
        let origin_left = self.last_visible_id();

        self.items.push(YataItem {
            id,
            value,
            deleted: false,
            origin_left,
            origin_right: None,
        });
    }

    /// Insert a value after the element at the given index.
    pub fn insert_after(&mut self, index: usize, value: T) {
        let origin_left = self.visible_id_at(index);
        let origin_right = self.visible_id_at(index + 1);

        self.clock += 1;
        let id = YataId::new(self.clock, self.replica_id);

        self.items.push(YataItem {
            id,
            value,
            deleted: false,
            origin_left,
            origin_right,
        });
    }

    /// Insert a value before the element at the given index.
    pub fn insert_before(&mut self, index: usize, value: T) {
        if index == 0 {
            self.clock += 1;
            let id = YataId::new(self.clock, self.replica_id);
            let origin_right = self.visible_id_at(0);

            self.items.push(YataItem {
                id,
                value,
                deleted: false,
                origin_left: None,
                origin_right,
            });
        } else {
            self.insert_after(index - 1, value);
        }
    }

    /// Remove the element at the given index.
    pub fn remove(&mut self, index: usize) {
        if let Some(id) = self.visible_id_at(index) {
            if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
                item.deleted = true;
            }
        }
    }

    /// Get the element at the given index.
    pub fn get(&self, index: usize) -> Option<&T> {
        self.visible_items().nth(index).map(|i| &i.value)
    }

    /// Get the number of visible elements.
    pub fn len(&self) -> usize {
        self.visible_items().count()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Convert to vector.
    pub fn to_vec(&self) -> Vec<T> {
        self.visible_items().map(|i| i.value.clone()).collect()
    }

    /// Iterate over visible elements.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.visible_items().map(|i| &i.value)
    }

    /// Get sorted visible items.
    fn visible_items(&self) -> impl Iterator<Item = &YataItem<T>> {
        self.sorted_items().into_iter().filter(|i| !i.deleted)
    }

    /// Sort items according to YATA rules.
    fn sorted_items(&self) -> Vec<&YataItem<T>> {
        let mut result: Vec<&YataItem<T>> = Vec::new();

        // Start with items that have no left origin
        let mut to_process: Vec<&YataItem<T>> = self
            .items
            .iter()
            .filter(|i| i.origin_left.is_none())
            .collect();

        // Sort ascending so pop() gives higher IDs first (later inserts appear first)
        to_process.sort_by(|a, b| a.id.cmp(&b.id));

        while let Some(item) = to_process.pop() {
            result.push(item);

            // Find items whose origin_left is this item
            let mut followers: Vec<&YataItem<T>> = self
                .items
                .iter()
                .filter(|i| i.origin_left == Some(item.id))
                .collect();

            // Sort ascending so pop() gives higher IDs first
            followers.sort_by(|a, b| a.id.cmp(&b.id));

            to_process.extend(followers);
        }

        result
    }

    fn last_visible_id(&self) -> Option<YataId> {
        self.sorted_items()
            .into_iter()
            .filter(|i| !i.deleted)
            .last()
            .map(|i| i.id)
    }

    fn visible_id_at(&self, index: usize) -> Option<YataId> {
        self.visible_items().nth(index).map(|i| i.id)
    }
}

impl<T: Clone + PartialEq> Merge for YATA<T> {
    fn merge(&mut self, other: &Self) {
        self.clock = self.clock.max(other.clock);

        for other_item in &other.items {
            let exists = self.items.iter().any(|i| i.id == other_item.id);
            if !exists {
                self.items.push(other_item.clone());
            } else if let Some(my_item) = self.items.iter_mut().find(|i| i.id == other_item.id) {
                if other_item.deleted {
                    my_item.deleted = true;
                }
            }
        }
    }
}

impl<T: Clone + PartialEq + Serialize + DeserializeOwned + Send + 'static> DeltaCrdt for YATA<T> {
    type Delta = YATADelta<T>;

    fn delta_since(&self, since: &VClock) -> Option<Self::Delta> {
        let current = self.version();
        if since.dominates(&current) {
            return None;
        }

        Some(YATADelta {
            items: self.items.clone(),
            clock: self.clock,
        })
    }

    fn apply_delta(&mut self, delta: &Self::Delta) {
        self.clock = self.clock.max(delta.clock);

        for delta_item in &delta.items {
            let exists = self.items.iter().any(|i| i.id == delta_item.id);
            if !exists {
                self.items.push(delta_item.clone());
            } else if let Some(my_item) = self.items.iter_mut().find(|i| i.id == delta_item.id) {
                if delta_item.deleted {
                    my_item.deleted = true;
                }
            }
        }
    }

    fn version(&self) -> VClock {
        let mut clock = VClock::new();
        for item in &self.items {
            let current = clock.get(item.id.replica);
            if item.id.clock > current {
                for _ in current..item.id.clock {
                    clock.increment(item.id.replica);
                }
            }
        }
        clock
    }
}

impl<T: Clone + PartialEq> Default for YATA<T> {
    fn default() -> Self {
        Self::new_random()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_yata_append() {
        let mut seq: YATA<char> = YATA::new(1);
        seq.append('a');
        seq.append('b');
        assert_eq!(seq.to_vec(), vec!['a', 'b']);
    }

    #[test]
    fn test_yata_concurrent() {
        let mut a: YATA<char> = YATA::new(1);
        let mut b: YATA<char> = YATA::new(2);

        a.append('A');
        b.append('B');

        a.merge(&b);
        b.merge(&a);

        assert_eq!(a.to_vec(), b.to_vec());
    }
}
