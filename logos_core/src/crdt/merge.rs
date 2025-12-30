//! The Merge trait for CRDTs

/// A type that can be merged with another instance of itself.
///
/// The merge operation must satisfy CRDT properties:
/// - **Commutative**: `a.merge(b) == b.merge(a)` (result is the same regardless of order)
/// - **Associative**: `a.merge(b.merge(c)) == a.merge(b).merge(c)`
/// - **Idempotent**: `a.merge(a) == a` (merging with self has no effect)
///
/// These properties ensure that replicas converge to the same state
/// regardless of message ordering or delivery.
pub trait Merge {
    /// Merge another instance into self.
    ///
    /// After merging, `self` contains the combined state of both instances.
    fn merge(&mut self, other: &Self);
}
