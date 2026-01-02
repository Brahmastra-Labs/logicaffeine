//! Phase CRDT Serialization: Serialization edge case tests
//!
//! Tests for serialization/deserialization correctness including
//! large payloads, malformed data handling, and edge cases.
//!
//! All tests use unique replica IDs to prevent cross-talk between parallel tests.

use logos_core::crdt::generate_replica_id;

/// Generate a unique base ID for this test run to avoid cross-talk
fn test_base_id() -> u64 {
    generate_replica_id()
}

// ===== ORSET SERIALIZATION TESTS =====

#[test]
fn test_large_orset_serialization() {
    use logos_core::crdt::ORSet;

    let mut set: ORSet<i64> = ORSet::new(test_base_id());

    // Add 1000 elements
    for i in 0..1000 {
        set.add(i);
    }

    // Serialize
    let bytes = bincode::serialize(&set).expect("Serialization should succeed");

    // Verify reasonable size (each element + metadata)
    assert!(bytes.len() > 0);

    // Deserialize
    let decoded: ORSet<i64> = bincode::deserialize(&bytes).expect("Deserialization should succeed");

    // Verify contents
    assert_eq!(decoded.len(), 1000);
    for i in 0..1000 {
        assert!(decoded.contains(&i), "Missing element {}", i);
    }
}

#[test]
fn test_orset_string_serialization() {
    use logos_core::crdt::ORSet;

    let mut set: ORSet<String> = ORSet::new(test_base_id());

    // Add various string types
    set.add("".to_string()); // Empty string
    set.add("hello".to_string());
    set.add("hello world with spaces".to_string());
    set.add("unicode: こんにちは 🎉".to_string());
    set.add("a".repeat(1000)); // Long string

    let bytes = bincode::serialize(&set).expect("Serialization should succeed");
    let decoded: ORSet<String> = bincode::deserialize(&bytes).expect("Deserialization should succeed");

    assert_eq!(decoded.len(), 5);
    assert!(decoded.contains(&"".to_string()));
    assert!(decoded.contains(&"unicode: こんにちは 🎉".to_string()));
}

// ===== EMPTY CRDT SERIALIZATION =====

#[test]
fn test_empty_crdt_serialization() {
    use logos_core::crdt::{GCounter, MVRegister, ORSet, PNCounter, VClock};

    let base = test_base_id();

    // Empty GCounter
    let gc = GCounter::with_replica_id(base);
    let gc_bytes = bincode::serialize(&gc).unwrap();
    let gc_decoded: GCounter = bincode::deserialize(&gc_bytes).unwrap();
    assert_eq!(gc_decoded.value(), 0);

    // Empty PNCounter
    let pnc = PNCounter::with_replica_id(base);
    let pnc_bytes = bincode::serialize(&pnc).unwrap();
    let pnc_decoded: PNCounter = bincode::deserialize(&pnc_bytes).unwrap();
    assert_eq!(pnc_decoded.value(), 0);

    // Empty ORSet
    let set: ORSet<String> = ORSet::new(base);
    let set_bytes = bincode::serialize(&set).unwrap();
    let set_decoded: ORSet<String> = bincode::deserialize(&set_bytes).unwrap();
    assert!(set_decoded.is_empty());

    // Empty MVRegister
    let reg: MVRegister<String> = MVRegister::new(base);
    let reg_bytes = bincode::serialize(&reg).unwrap();
    let reg_decoded: MVRegister<String> = bincode::deserialize(&reg_bytes).unwrap();
    assert!(reg_decoded.values().is_empty());

    // Empty VClock
    let vc = VClock::new();
    let vc_bytes = bincode::serialize(&vc).unwrap();
    let vc_decoded: VClock = bincode::deserialize(&vc_bytes).unwrap();
    assert_eq!(vc_decoded.get(base), 0);
}

// ===== MALFORMED DATA HANDLING =====

#[test]
fn test_malformed_data_handling() {
    use logos_core::crdt::GCounter;

    // Empty bytes
    let result: Result<GCounter, _> = bincode::deserialize(&[]);
    assert!(result.is_err());

    // Random garbage bytes
    let garbage = vec![0xFF, 0xFE, 0x00, 0x01, 0x02, 0x03];
    let _result: Result<GCounter, _> = bincode::deserialize(&garbage);
    // May or may not error depending on bincode interpretation
    // The important thing is it doesn't panic

    // Truncated valid data
    let gc = GCounter::with_replica_id(test_base_id());
    let mut bytes = bincode::serialize(&gc).unwrap();
    if bytes.len() > 2 {
        bytes.truncate(bytes.len() / 2);
        let result: Result<GCounter, _> = bincode::deserialize(&bytes);
        assert!(result.is_err());
    }
}

#[test]
fn test_corrupt_orset_handling() {
    use logos_core::crdt::ORSet;

    // Create valid ORSet
    let mut set: ORSet<i64> = ORSet::new(test_base_id());
    set.add(42);

    let mut bytes = bincode::serialize(&set).unwrap();

    // Flip some bits in the middle
    if bytes.len() > 10 {
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xFF;
    }

    // Deserialization may succeed or fail, but shouldn't panic
    let _result: Result<ORSet<i64>, _> = bincode::deserialize(&bytes);
    // We don't assert the result - the important thing is no panic
}

// ===== COMPLEX NESTED STATE =====

#[test]
fn test_complex_nested_state() {
    use logos_core::crdt::MVRegister;

    let base = test_base_id();

    // MVRegister with complex nested data type
    #[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize, Debug)]
    struct ComplexData {
        id: u64,
        name: String,
        values: Vec<i64>,
    }

    let mut reg: MVRegister<ComplexData> = MVRegister::new(base);
    reg.set(ComplexData {
        id: 12345,
        name: "test data".to_string(),
        values: vec![1, 2, 3, 4, 5],
    });

    let bytes = bincode::serialize(&reg).unwrap();
    let decoded: MVRegister<ComplexData> = bincode::deserialize(&bytes).unwrap();

    let values = decoded.values();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].id, 12345);
    assert_eq!(values[0].name, "test data");
    assert_eq!(values[0].values, vec![1, 2, 3, 4, 5]);
}

// ===== VCLOCK SERIALIZATION =====

#[test]
fn test_vclock_serialization_many_replicas() {
    use logos_core::crdt::VClock;

    let base = test_base_id();
    let mut clock = VClock::new();

    // Add 100 replicas
    for i in 0..100 {
        for _ in 0..=i {
            clock.increment(base + i);
        }
    }

    let bytes = bincode::serialize(&clock).unwrap();
    let decoded: VClock = bincode::deserialize(&bytes).unwrap();

    // Verify all entries
    for i in 0..100 {
        assert_eq!(decoded.get(base + i), (i + 1) as u64);
    }
}

// ===== DOTCONTEXT SERIALIZATION =====

#[test]
fn test_dotcontext_serialization() {
    use logos_core::crdt::{Dot, DotContext};

    let base = test_base_id();
    let mut ctx = DotContext::new();

    // Add some dots
    for i in 1..=50 {
        ctx.add(Dot::new(base, i));
    }

    // Add some out-of-order dots (creates cloud entries)
    ctx.add(Dot::new(base + 1, 100));
    ctx.add(Dot::new(base + 1, 102)); // Gap at 101

    let bytes = bincode::serialize(&ctx).unwrap();
    let decoded: DotContext = bincode::deserialize(&bytes).unwrap();

    // Verify state preserved
    for i in 1..=50 {
        assert!(decoded.has_seen(&Dot::new(base, i)));
    }
    assert!(decoded.has_seen(&Dot::new(base + 1, 100)));
    assert!(decoded.has_seen(&Dot::new(base + 1, 102)));
    assert!(!decoded.has_seen(&Dot::new(base + 1, 101))); // Gap should still exist
}

// ===== DELTA BUFFER SERIALIZATION =====

#[test]
fn test_delta_buffer_serialization() {
    use logos_core::crdt::{DeltaBuffer, VClock};

    let base = test_base_id();
    let mut buffer: DeltaBuffer<String> = DeltaBuffer::new(10);

    // Add some deltas
    for i in 0..5 {
        let mut version = VClock::new();
        version.increment(base);
        for _ in 0..i {
            version.increment(base);
        }
        buffer.push(version, format!("delta-{}", i));
    }

    let bytes = bincode::serialize(&buffer).unwrap();
    let decoded: DeltaBuffer<String> = bincode::deserialize(&bytes).unwrap();

    // Buffer should be usable after deserialization
    let empty_version = VClock::new();
    let can_serve = decoded.can_serve(&empty_version);
    // Result depends on implementation, just verify no panic
    let _ = can_serve;
}

// ===== LWWREGISTER SERIALIZATION =====

#[test]
fn test_lww_serialization_preserves_timestamp() {
    use logos_core::crdt::LWWRegister;

    let reg = LWWRegister::new("test value".to_string());
    let original_timestamp = reg.timestamp();

    let bytes = bincode::serialize(&reg).unwrap();
    let decoded: LWWRegister<String> = bincode::deserialize(&bytes).unwrap();

    assert_eq!(decoded.get(), "test value");
    assert_eq!(decoded.timestamp(), original_timestamp);
}

// ===== MERGE AFTER DESERIALIZATION =====

#[test]
fn test_merge_after_deserialization() {
    use logos_core::crdt::{GCounter, Merge};

    let base = test_base_id();

    let mut a = GCounter::with_replica_id(base);
    a.increment(10);

    let mut b = GCounter::with_replica_id(base + 1);
    b.increment(20);

    // Serialize and deserialize both
    let a_bytes = bincode::serialize(&a).unwrap();
    let b_bytes = bincode::serialize(&b).unwrap();

    let mut a_decoded: GCounter = bincode::deserialize(&a_bytes).unwrap();
    let b_decoded: GCounter = bincode::deserialize(&b_bytes).unwrap();

    // Merge should work correctly
    a_decoded.merge(&b_decoded);
    assert_eq!(a_decoded.value(), 30);
}

#[test]
fn test_orset_merge_after_deserialization() {
    use logos_core::crdt::{Merge, ORSet};

    let base = test_base_id();

    let mut a: ORSet<String> = ORSet::new(base);
    a.add("from-a".to_string());

    let mut b: ORSet<String> = ORSet::new(base + 1);
    b.add("from-b".to_string());

    // Serialize and deserialize
    let a_bytes = bincode::serialize(&a).unwrap();
    let b_bytes = bincode::serialize(&b).unwrap();

    let mut a_decoded: ORSet<String> = bincode::deserialize(&a_bytes).unwrap();
    let b_decoded: ORSet<String> = bincode::deserialize(&b_bytes).unwrap();

    // Merge should work correctly
    a_decoded.merge(&b_decoded);
    assert_eq!(a_decoded.len(), 2);
    assert!(a_decoded.contains(&"from-a".to_string()));
    assert!(a_decoded.contains(&"from-b".to_string()));
}
