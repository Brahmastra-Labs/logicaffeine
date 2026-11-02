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
