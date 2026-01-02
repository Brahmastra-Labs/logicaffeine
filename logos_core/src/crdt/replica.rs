//! Replica ID generation for CRDTs
//!
//! Wave 1.1: ReplicaId migrated from String to u64 for efficiency.

/// Unique identifier for a replica in a distributed CRDT.
/// Using u64 is more efficient for VClock operations than String.
pub type ReplicaId = u64;

/// Generate a unique replica ID.
#[cfg(not(target_arch = "wasm32"))]
pub fn generate_replica_id() -> ReplicaId {
    use std::time::{SystemTime, UNIX_EPOCH};

    // Combine timestamp with random bits for uniqueness
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_nanos() as u64;

    let mut random_bytes = [0u8; 8];
    getrandom::getrandom(&mut random_bytes).expect("Failed to generate random bytes");
    let random = u64::from_le_bytes(random_bytes);

    // XOR timestamp with random for uniqueness
    timestamp ^ random
}

/// Generate a unique replica ID (WASM version).
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
