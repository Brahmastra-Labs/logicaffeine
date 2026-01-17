//! Delta CRDT trait and types.
//!
//! Trait for CRDTs that support delta-state synchronization.

use super::causal::VClock;
use super::Merge;
use serde::{de::DeserializeOwned, Serialize};

/// A CRDT that supports delta-state synchronization.
///
/// Delta-state CRDTs can extract small deltas representing recent changes,
/// rather than broadcasting the entire state. This is more efficient for
/// large data structures.
pub trait DeltaCrdt: Merge + Sized {
    /// The type of delta this CRDT produces.
    type Delta: Serialize + DeserializeOwned + Clone + Send + 'static;

    /// Extract a delta containing changes since the given version.
    ///
    /// Returns `None` if the delta history doesn't go back far enough.
    fn delta_since(&self, version: &VClock) -> Option<Self::Delta>;

    /// Apply an incoming delta to this CRDT.
    fn apply_delta(&mut self, delta: &Self::Delta);

    /// Get the current version (vector clock) of this CRDT.
    fn version(&self) -> VClock;
}
