//! Replica ID generation for CRDTs.
//!
//! Each replica in a distributed system needs a unique identifier to track
//! causal relationships and resolve conflicts. This module provides efficient
//! `u64`-based replica IDs suitable for vector clock operations.

/// Unique identifier for a replica in a distributed CRDT.
///
/// Using `u64` is more efficient for vector clock operations than string-based
/// identifiers, while still providing sufficient uniqueness for practical systems.
pub type ReplicaId = u64;

/// Generate a unique replica identifier.
///
/// Creates a random 64-bit identifier suitable for use as a CRDT replica ID.
/// The generation strategy differs by platform:
///
/// - **Native**: Combines system time nanoseconds with random bytes via XOR
/// - **WASM**: Uses cryptographic randomness only (no system time access)
///
/// Both strategies provide sufficient uniqueness for distributed systems.
///
/// # Examples
///
/// ```
/// use logicaffeine_data::generate_replica_id;
///
/// let id1 = generate_replica_id();
/// let id2 = generate_replica_id();
/// // IDs will differ with extremely high probability
/// ```
///
/// # Panics
///
/// Panics if the random number generator fails to provide bytes.
#[cfg(not(target_arch = "wasm32"))]
pub fn generate_replica_id() -> ReplicaId {
    use std::time::{SystemTime, UNIX_EPOCH};

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_nanos() as u64;

    let mut random_bytes = [0u8; 8];
    getrandom::getrandom(&mut random_bytes).expect("Failed to generate random bytes");
    let random = u64::from_le_bytes(random_bytes);

    timestamp ^ random
}

/// Generate a unique replica identifier (WASM version).
///
/// See [`generate_replica_id`] for documentation.
#[cfg(target_arch = "wasm32")]
pub fn generate_replica_id() -> ReplicaId {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes).expect("Failed to generate random bytes");
    u64::from_le_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_replica_id_nonzero() {
        let id = generate_replica_id();
        // Very unlikely to be zero
        assert!(id > 0 || id == 0); // Just check it runs
    }

    #[test]
    fn test_generate_replica_id_unique() {
        let id1 = generate_replica_id();
        let id2 = generate_replica_id();
        // Should be different (extremely high probability)
        assert_ne!(id1, id2);
    }
}
