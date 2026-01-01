//! Phase CRDT MVRegister: Tests for Divergent (MV-Register)
//!
//! Wave 2.2 of CRDT Expansion: Multi-value register preserving concurrent edits.
//!
//! TDD: These are RED tests - they define the spec before implementation.

// ===== WAVE 2.2: MVREGISTER BASICS =====

#[test]
fn test_mvregister_new() {
    use logos_core::crdt::MVRegister;
    let reg: MVRegister<String> = MVRegister::new(1);
    // New register should have no values
    assert!(reg.values().is_empty());
}

#[test]
fn test_mvregister_set_get() {
    use logos_core::crdt::MVRegister;
    let mut reg: MVRegister<String> = MVRegister::new(1);
    reg.set("hello".to_string());

    let values = reg.values();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0], &"hello".to_string());
}

#[test]
fn test_mvregister_set_overwrites() {
    use logos_core::crdt::MVRegister;
    let mut reg: MVRegister<String> = MVRegister::new(1);
    reg.set("first".to_string());
    reg.set("second".to_string());

    let values = reg.values();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0], &"second".to_string());
}

// ===== WAVE 2.2: MVREGISTER CONCURRENT VALUES =====

#[test]
fn test_mvregister_concurrent_values() {
    use logos_core::crdt::{Merge, MVRegister};
    let mut a: MVRegister<String> = MVRegister::new(1);
    let mut b: MVRegister<String> = MVRegister::new(2);

    // Concurrent writes
    a.set("draft-a".to_string());
    b.set("draft-b".to_string());

    a.merge(&b);

    let values = a.values();
    assert_eq!(values.len(), 2); // Both values kept!

    // Check both values are present (order not guaranteed)
    let has_a = values.iter().any(|v| *v == "draft-a");
    let has_b = values.iter().any(|v| *v == "draft-b");
    assert!(has_a, "Should contain draft-a");
    assert!(has_b, "Should contain draft-b");
}

#[test]
fn test_mvregister_sequential_overwrites() {
    use logos_core::crdt::{Merge, MVRegister};
    let mut a: MVRegister<String> = MVRegister::new(1);
    let mut b: MVRegister<String> = MVRegister::new(2);

    // A sets, B merges (now B has seen A's value)
    a.set("v1".to_string());
    b.merge(&a);

    // B overwrites (this dominates A's value)
    b.set("v2".to_string());
    a.merge(&b);

    let values = a.values();
    assert_eq!(values.len(), 1); // Only latest survives
    assert_eq!(values[0], &"v2".to_string());
}

#[test]
fn test_mvregister_three_way_concurrent() {
    use logos_core::crdt::{Merge, MVRegister};
    let mut a: MVRegister<String> = MVRegister::new(1);
    let mut b: MVRegister<String> = MVRegister::new(2);
    let mut c: MVRegister<String> = MVRegister::new(3);

    // All three write concurrently (none have seen others)
    a.set("from-a".to_string());
    b.set("from-b".to_string());
    c.set("from-c".to_string());

    // Merge all together
    a.merge(&b);
    a.merge(&c);

    let values = a.values();
    assert_eq!(values.len(), 3); // All three values kept
}

// ===== WAVE 2.2: MVREGISTER RESOLVE =====

#[test]
fn test_mvregister_resolve() {
    use logos_core::crdt::{Merge, MVRegister};
    let mut a: MVRegister<String> = MVRegister::new(1);
    let mut b: MVRegister<String> = MVRegister::new(2);

    // Create conflict
    a.set("draft-a".to_string());
    b.set("draft-b".to_string());
    a.merge(&b);

    assert_eq!(a.values().len(), 2); // Conflict exists

    // Resolve the conflict
    a.resolve("final".to_string());

    let values = a.values();
    assert_eq!(values.len(), 1); // Conflict resolved
    assert_eq!(values[0], &"final".to_string());
}

#[test]
fn test_mvregister_resolve_propagates() {
    use logos_core::crdt::{Merge, MVRegister};
    let mut a: MVRegister<String> = MVRegister::new(1);
    let mut b: MVRegister<String> = MVRegister::new(2);

    // Create and resolve conflict on A
    a.set("draft-a".to_string());
    b.set("draft-b".to_string());
    a.merge(&b);
    a.resolve("resolved".to_string());

    // B merges A's resolution
    b.merge(&a);

    let values = b.values();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0], &"resolved".to_string());
}

// ===== WAVE 2.2: MVREGISTER MERGE PROPERTIES =====

#[test]
fn test_mvregister_merge_commutative() {
    use logos_core::crdt::{Merge, MVRegister};
    let mut a: MVRegister<String> = MVRegister::new(1);
    let mut b: MVRegister<String> = MVRegister::new(2);

    a.set("from-a".to_string());
    b.set("from-b".to_string());

    let mut a1 = a.clone();
    let mut b1 = b.clone();
    a1.merge(&b);
    b1.merge(&a);

    // Both should have same values (order may differ)
    let mut va: Vec<_> = a1.values().into_iter().cloned().collect();
    let mut vb: Vec<_> = b1.values().into_iter().cloned().collect();
    va.sort();
    vb.sort();
    assert_eq!(va, vb);
}

#[test]
fn test_mvregister_merge_idempotent() {
    use logos_core::crdt::{Merge, MVRegister};
    let mut reg: MVRegister<String> = MVRegister::new(1);
    reg.set("value".to_string());

    let before: Vec<_> = reg.values().into_iter().cloned().collect();
    reg.merge(&reg.clone());
    let after: Vec<_> = reg.values().into_iter().cloned().collect();

    assert_eq!(before, after);
}

// ===== WAVE 2.2: MVREGISTER SERIALIZATION =====

#[test]
fn test_mvregister_serialization() {
    use logos_core::crdt::MVRegister;

    let mut reg: MVRegister<String> = MVRegister::new(42);
    reg.set("test-value".to_string());

    let bytes = bincode::serialize(&reg).unwrap();
    let decoded: MVRegister<String> = bincode::deserialize(&bytes).unwrap();

    assert_eq!(reg.values(), decoded.values());
}

#[test]
fn test_mvregister_serialization_with_conflict() {
    use logos_core::crdt::{Merge, MVRegister};

    let mut a: MVRegister<String> = MVRegister::new(1);
    let mut b: MVRegister<String> = MVRegister::new(2);
    a.set("a".to_string());
    b.set("b".to_string());
    a.merge(&b);

    let bytes = bincode::serialize(&a).unwrap();
    let decoded: MVRegister<String> = bincode::deserialize(&bytes).unwrap();

    assert_eq!(a.values().len(), decoded.values().len());
}

// ===== WAVE 2.2: MVREGISTER WITH DIFFERENT TYPES =====

#[test]
fn test_mvregister_with_integers() {
    use logos_core::crdt::{Merge, MVRegister};
    let mut a: MVRegister<i64> = MVRegister::new(1);
    let mut b: MVRegister<i64> = MVRegister::new(2);

    a.set(42);
    b.set(99);
    a.merge(&b);

    assert_eq!(a.values().len(), 2);
}
