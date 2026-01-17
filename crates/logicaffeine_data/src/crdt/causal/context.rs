//! Dot Context - Tracks seen events for OR-Set semantics.
//!
//! DotContext combines a VClock with a "cloud" of non-contiguous dots.

use super::dot::Dot;
use super::vclock::VClock;
use super::super::replica::ReplicaId;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Tracks which events have been seen in the system.
///
/// The DotContext maintains a VClock for contiguous sequences and a "cloud"
/// set for dots that arrive out of order. When gaps are filled, dots are
/// compacted from the cloud into the clock.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DotContext {
    /// Contiguous event counters per replica
    clock: VClock,
    /// Non-contiguous dots (arrived out of order)
    cloud: HashSet<Dot>,
}

impl DotContext {
    /// Create a new empty context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Generate the next dot for a replica and mark it as seen.
    pub fn next(&mut self, replica: ReplicaId) -> Dot {
        let counter = self.clock.increment(replica);
        Dot::new(replica, counter)
    }

    /// Check if a dot has been seen (either in clock or cloud).
    pub fn has_seen(&self, dot: &Dot) -> bool {
        self.clock.get(dot.replica) >= dot.counter || self.cloud.contains(dot)
    }

    /// Add a dot to the context.
    ///
    /// If the dot is contiguous with the clock, increment the clock.
    /// Otherwise, add it to the cloud. Then attempt to compact.
    pub fn add(&mut self, dot: Dot) {
        if dot.counter == self.clock.get(dot.replica) + 1 {
            // Contiguous - add to clock
            self.clock.increment(dot.replica);
            // Compact: pull any waiting dots from cloud
            self.compact_replica(dot.replica);
        } else if dot.counter > self.clock.get(dot.replica) {
            // Out of order - add to cloud
            self.cloud.insert(dot);
        }
        // If dot.counter <= clock.get(dot.replica), we've already seen it
    }

    /// Compact the cloud for a specific replica.
    fn compact_replica(&mut self, replica: ReplicaId) {
        loop {
            let next_counter = self.clock.get(replica) + 1;
            let next_dot = Dot::new(replica, next_counter);
            if self.cloud.remove(&next_dot) {
                self.clock.increment(replica);
            } else {
                break;
            }
        }
    }

    /// Compact all replicas in the cloud.
    fn compact(&mut self) {
        let replicas: Vec<ReplicaId> = self.cloud.iter().map(|d| d.replica).collect();
        for replica in replicas {
            self.compact_replica(replica);
        }
    }

    /// Merge another context into this one.
    pub fn merge(&mut self, other: &Self) {
        self.clock.merge_vclock(&other.clock);
        for &dot in &other.cloud {
            if !self.has_seen(&dot) {
                self.cloud.insert(dot);
            }
        }
        self.compact();
    }

    /// Get the underlying vector clock.
    pub fn clock(&self) -> &VClock {
        &self.clock
    }

    /// Get the version as a VClock clone (for DeltaCrdt).
    pub fn version(&self) -> VClock {
        self.clock.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_new() {
        let ctx = DotContext::new();
        assert!(!ctx.has_seen(&Dot::new(1, 1)));
    }

    #[test]
    fn test_context_next() {
        let mut ctx = DotContext::new();
        let d1 = ctx.next(42);
        let d2 = ctx.next(42);
        assert_eq!(d1.counter, 1);
        assert_eq!(d2.counter, 2);
        assert!(ctx.has_seen(&d1));
        assert!(ctx.has_seen(&d2));
    }

    #[test]
    fn test_context_add_contiguous() {
        let mut ctx = DotContext::new();
        ctx.add(Dot::new(1, 1));
        ctx.add(Dot::new(1, 2));
        ctx.add(Dot::new(1, 3));

        assert!(ctx.has_seen(&Dot::new(1, 1)));
        assert!(ctx.has_seen(&Dot::new(1, 2)));
        assert!(ctx.has_seen(&Dot::new(1, 3)));
        assert!(!ctx.has_seen(&Dot::new(1, 4)));
    }

    #[test]
    fn test_context_add_out_of_order() {
        let mut ctx = DotContext::new();
        ctx.add(Dot::new(1, 3)); // Into cloud
        ctx.add(Dot::new(1, 1)); // Into clock
        ctx.add(Dot::new(1, 2)); // Should trigger compaction

        assert!(ctx.has_seen(&Dot::new(1, 1)));
        assert!(ctx.has_seen(&Dot::new(1, 2)));
        assert!(ctx.has_seen(&Dot::new(1, 3)));
        // Cloud should be empty after compaction
        assert!(ctx.cloud.is_empty());
    }

    #[test]
    fn test_context_merge() {
        let mut a = DotContext::new();
        let mut b = DotContext::new();

        a.next(1);
        a.next(1);
        b.next(2);

        a.merge(&b);

        assert!(a.has_seen(&Dot::new(1, 1)));
        assert!(a.has_seen(&Dot::new(1, 2)));
        assert!(a.has_seen(&Dot::new(2, 1)));
    }
}
