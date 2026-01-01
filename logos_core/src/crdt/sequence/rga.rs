//! RGA (Replicated Growable Array) CRDT
//!
//! Wave 3.2: A sequence CRDT suitable for collaborative lists.
//! Uses timestamps and replica IDs to order concurrent insertions.

use crate::crdt::causal::VClock;
use crate::crdt::delta::DeltaCrdt;
use crate::crdt::replica::{generate_replica_id, ReplicaId};
use crate::crdt::Merge;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;

/// Delta for RGA synchronization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RGADelta<T> {
    pub nodes: Vec<RgaNode<T>>,
    pub timestamp: u64,
}

/// Unique identifier for a node in the RGA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RgaId {
    /// Logical timestamp
    pub timestamp: u64,
    /// Replica that created this node
    pub replica: ReplicaId,
}

impl RgaId {
    fn new(timestamp: u64, replica: ReplicaId) -> Self {
        Self { timestamp, replica }
    }
}

impl Ord for RgaId {
    fn cmp(&self, other: &Self) -> Ordering {
        // Higher timestamp wins, then higher replica ID as tiebreaker
        match self.timestamp.cmp(&other.timestamp) {
            Ordering::Equal => self.replica.cmp(&other.replica),
            ord => ord,
        }
    }
}

impl PartialOrd for RgaId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A node in the RGA linked structure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RgaNode<T> {
    pub id: RgaId,
    pub value: T,
    pub deleted: bool,
    /// ID of the node this was inserted after (None for head)
    pub parent: Option<RgaId>,
}

/// Replicated Growable Array - a sequence CRDT.
///
/// Supports append, insert, and remove operations with deterministic
/// conflict resolution for concurrent operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RGA<T> {
    /// All nodes (including deleted ones for tombstones)
    nodes: Vec<RgaNode<T>>,
    /// Current logical timestamp
    timestamp: u64,
    /// This replica's ID
    replica_id: ReplicaId,
}

impl<T: Clone + PartialEq> RGA<T> {
    /// Create a new empty RGA.
    pub fn new(replica_id: ReplicaId) -> Self {
        Self {
            nodes: Vec::new(),
            timestamp: 0,
            replica_id,
        }
    }

    /// Create a new RGA with a random replica ID.
    pub fn new_random() -> Self {
        Self::new(generate_replica_id())
    }

    /// Append a value to the end of the sequence.
    pub fn append(&mut self, value: T) {
        self.timestamp += 1;
        let id = RgaId::new(self.timestamp, self.replica_id);
        let parent = self.last_visible_id();

        self.nodes.push(RgaNode {
            id,
            value,
            deleted: false,
            parent,
        });
    }

    /// Insert a value before the element at the given index.
    pub fn insert_before(&mut self, index: usize, value: T) {
        if index == 0 {
            // Insert at head
            self.timestamp += 1;
            let id = RgaId::new(self.timestamp, self.replica_id);
            self.nodes.push(RgaNode {
                id,
                value,
                deleted: false,
                parent: None,
            });
        } else {
            // Insert after the element before index
            self.insert_after(index - 1, value);
        }
    }

    /// Insert a value after the element at the given index.
    pub fn insert_after(&mut self, index: usize, value: T) {
        let parent_id = self.visible_id_at(index);
        self.timestamp += 1;
        let id = RgaId::new(self.timestamp, self.replica_id);

        self.nodes.push(RgaNode {
            id,
            value,
            deleted: false,
            parent: parent_id,
        });
    }

    /// Remove the element at the given index (tombstone deletion).
    pub fn remove(&mut self, index: usize) {
        if let Some(id) = self.visible_id_at(index) {
            if let Some(node) = self.nodes.iter_mut().find(|n| n.id == id) {
                node.deleted = true;
            }
        }
    }

    /// Get the element at the given index.
    pub fn get(&self, index: usize) -> Option<&T> {
        self.visible_nodes().nth(index).map(|n| &n.value)
    }

    /// Get the number of visible elements.
    pub fn len(&self) -> usize {
        self.visible_nodes().count()
    }

    /// Check if the sequence is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Convert to a vector of values.
    pub fn to_vec(&self) -> Vec<T> {
        self.visible_nodes().map(|n| n.value.clone()).collect()
    }

    /// Iterate over visible elements.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.visible_nodes().map(|n| &n.value)
    }

    /// Get sorted visible nodes in document order.
    fn visible_nodes(&self) -> impl Iterator<Item = &RgaNode<T>> {
        self.sorted_nodes()
            .into_iter()
            .filter(|n| !n.deleted)
    }

    /// Get all nodes sorted in document order.
    fn sorted_nodes(&self) -> Vec<&RgaNode<T>> {
        // Build a map from parent ID to children
        let mut children_map: HashMap<Option<RgaId>, Vec<&RgaNode<T>>> = HashMap::new();
        for node in &self.nodes {
            children_map.entry(node.parent).or_default().push(node);
        }

        // Sort children at each level - higher ID (later insert) comes first
        for children in children_map.values_mut() {
            children.sort_by(|a, b| b.id.cmp(&a.id));
        }

        // DFS traversal starting from root (parent = None)
        let mut result: Vec<&RgaNode<T>> = Vec::new();
        let mut stack: Vec<&RgaNode<T>> = Vec::new();

        // Start with head nodes in reverse order
        if let Some(heads) = children_map.get(&None) {
            for node in heads.iter().rev() {
                stack.push(node);
            }
        }

        while let Some(node) = stack.pop() {
            result.push(node);

            // Add children in reverse order
            if let Some(children) = children_map.get(&Some(node.id)) {
                for child in children.iter().rev() {
                    stack.push(child);
                }
            }
        }

        result
    }

    /// Get the ID of the last visible node.
    fn last_visible_id(&self) -> Option<RgaId> {
        self.sorted_nodes()
            .into_iter()
            .filter(|n| !n.deleted)
            .last()
            .map(|n| n.id)
    }

    /// Get the ID of the visible node at the given index.
    fn visible_id_at(&self, index: usize) -> Option<RgaId> {
        self.visible_nodes().nth(index).map(|n| n.id)
    }
}

impl<T: Clone + PartialEq> Merge for RGA<T> {
    fn merge(&mut self, other: &Self) {
        // Update our timestamp to be at least as high as other's
        self.timestamp = self.timestamp.max(other.timestamp);

        // Add nodes from other that we don't have
        for other_node in &other.nodes {
            let exists = self.nodes.iter().any(|n| n.id == other_node.id);
            if !exists {
                self.nodes.push(other_node.clone());
            } else {
                // If we have the node, merge deleted status
                if let Some(my_node) = self.nodes.iter_mut().find(|n| n.id == other_node.id) {
                    if other_node.deleted {
                        my_node.deleted = true;
                    }
                }
            }
        }
    }
}

impl<T: Clone + PartialEq + Serialize + DeserializeOwned + Send + 'static> DeltaCrdt for RGA<T> {
    type Delta = RGADelta<T>;

    fn delta_since(&self, since: &VClock) -> Option<Self::Delta> {
        let current = self.version();
        if since.dominates(&current) {
            return None;
        }

        // Return full state as delta
        Some(RGADelta {
            nodes: self.nodes.clone(),
            timestamp: self.timestamp,
        })
    }

    fn apply_delta(&mut self, delta: &Self::Delta) {
        self.timestamp = self.timestamp.max(delta.timestamp);

        for delta_node in &delta.nodes {
            let exists = self.nodes.iter().any(|n| n.id == delta_node.id);
            if !exists {
                self.nodes.push(delta_node.clone());
            } else if let Some(my_node) = self.nodes.iter_mut().find(|n| n.id == delta_node.id) {
                if delta_node.deleted {
                    my_node.deleted = true;
                }
            }
        }
    }

    fn version(&self) -> VClock {
        // Build VClock from the max timestamp per replica
        let mut clock = VClock::new();
        for node in &self.nodes {
            let current = clock.get(node.id.replica);
            if node.id.timestamp > current {
                // Set to the max timestamp seen for this replica
                for _ in current..node.id.timestamp {
                    clock.increment(node.id.replica);
                }
            }
        }
        clock
    }
}

impl<T: Clone + PartialEq> Default for RGA<T> {
    fn default() -> Self {
        Self::new_random()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rga_append() {
        let mut seq: RGA<String> = RGA::new(1);
        seq.append("a".to_string());
        seq.append("b".to_string());
        assert_eq!(seq.to_vec(), vec!["a", "b"]);
    }

    #[test]
    fn test_rga_insert_before() {
        let mut seq: RGA<String> = RGA::new(1);
        seq.append("b".to_string());
        seq.insert_before(0, "a".to_string());
        assert_eq!(seq.to_vec(), vec!["a", "b"]);
    }

    #[test]
    fn test_rga_remove() {
        let mut seq: RGA<String> = RGA::new(1);
        seq.append("a".to_string());
        seq.append("b".to_string());
        seq.remove(0);
        assert_eq!(seq.to_vec(), vec!["b"]);
    }

    #[test]
    fn test_rga_concurrent_append() {
        let mut a: RGA<String> = RGA::new(1);
        let mut b: RGA<String> = RGA::new(2);

        a.append("from-a".to_string());
        b.append("from-b".to_string());

        a.merge(&b);
        b.merge(&a);

        assert_eq!(a.to_vec(), b.to_vec());
    }
}
