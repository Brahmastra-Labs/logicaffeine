//! Phase CRDT ORMap: Tests for SharedMap (OR-Map)
//!
//! Wave 3.4 of CRDT Expansion: Key-value stores with nested CRDTs.
//!
//! TDD: These are RED tests - they define the spec before implementation.

// ===== WAVE 3.4: ORMAP BASICS =====

#[test]
fn test_ormap_new() {
    use logos_core::crdt::{ORMap, PNCounter};
    let map: ORMap<String, PNCounter> = ORMap::new(1);
    assert!(map.is_empty());
    assert_eq!(map.len(), 0);
}

#[test]
fn test_ormap_get_or_insert() {
    use logos_core::crdt::{ORMap, PNCounter};
    let mut map: ORMap<String, PNCounter> = ORMap::new(1);

    let counter = map.get_or_insert("score".to_string());
    counter.increment(10);

    assert_eq!(map.get(&"score".to_string()).unwrap().value(), 10);
}

#[test]
fn test_ormap_get_nonexistent() {
    use logos_core::crdt::{ORMap, PNCounter};
    let map: ORMap<String, PNCounter> = ORMap::new(1);
    assert!(map.get(&"missing".to_string()).is_none());
}

#[test]
fn test_ormap_contains_key() {
    use logos_core::crdt::{ORMap, PNCounter};
    let mut map: ORMap<String, PNCounter> = ORMap::new(1);
    map.get_or_insert("key".to_string());

    assert!(map.contains_key(&"key".to_string()));
    assert!(!map.contains_key(&"other".to_string()));
}

#[test]
fn test_ormap_remove() {
    use logos_core::crdt::{ORMap, PNCounter};
    let mut map: ORMap<String, PNCounter> = ORMap::new(1);
    map.get_or_insert("key".to_string()).increment(5);
    map.remove(&"key".to_string());

    assert!(map.get(&"key".to_string()).is_none());
}

#[test]
fn test_ormap_keys() {
    use logos_core::crdt::{ORMap, PNCounter};
    let mut map: ORMap<String, PNCounter> = ORMap::new(1);
    map.get_or_insert("a".to_string());
    map.get_or_insert("b".to_string());

    let keys: Vec<_> = map.keys().collect();
    assert_eq!(keys.len(), 2);
}

// ===== WAVE 3.4: ORMAP CONCURRENT OPERATIONS =====

#[test]
fn test_ormap_concurrent_update() {
    use logos_core::crdt::{Merge, ORMap, PNCounter};
    let mut a: ORMap<String, PNCounter> = ORMap::new(1);
    let mut b: ORMap<String, PNCounter> = ORMap::new(2);

    // Both update same key concurrently
    a.get_or_insert("score".to_string()).increment(10);
    b.get_or_insert("score".to_string()).increment(5);

    a.merge(&b);

    // Values should merge: 10 + 5 = 15
    assert_eq!(a.get(&"score".to_string()).unwrap().value(), 15);
}

#[test]
fn test_ormap_concurrent_add_different_keys() {
    use logos_core::crdt::{Merge, ORMap, PNCounter};
    let mut a: ORMap<String, PNCounter> = ORMap::new(1);
    let mut b: ORMap<String, PNCounter> = ORMap::new(2);

    a.get_or_insert("x".to_string()).increment(1);
    b.get_or_insert("y".to_string()).increment(2);

    a.merge(&b);

    assert_eq!(a.len(), 2);
    assert!(a.contains_key(&"x".to_string()));
    assert!(a.contains_key(&"y".to_string()));
}

#[test]
fn test_ormap_concurrent_add_remove() {
    use logos_core::crdt::{Merge, ORMap, PNCounter};
    let mut a: ORMap<String, PNCounter> = ORMap::new(1);
    let mut b: ORMap<String, PNCounter> = ORMap::new(2);

    // A adds key
    a.get_or_insert("key".to_string()).increment(10);
    b.merge(&a);

    // Concurrent: A removes, B updates
    a.remove(&"key".to_string());
    b.get_or_insert("key".to_string()).increment(5);

    a.merge(&b);

    // Add-wins semantics: key should be present
    assert!(a.contains_key(&"key".to_string()));
}

// ===== WAVE 3.4: ORMAP MERGE PROPERTIES =====

#[test]
fn test_ormap_merge_commutative() {
    use logos_core::crdt::{Merge, ORMap, PNCounter};
    let mut a: ORMap<String, PNCounter> = ORMap::new(1);
    let mut b: ORMap<String, PNCounter> = ORMap::new(2);

    a.get_or_insert("x".to_string()).increment(10);
    b.get_or_insert("y".to_string()).increment(5);

    let mut a1 = a.clone();
    let mut b1 = b.clone();
    a1.merge(&b);
    b1.merge(&a);

    assert_eq!(a1.len(), b1.len());
}

#[test]
fn test_ormap_merge_idempotent() {
    use logos_core::crdt::{Merge, ORMap, PNCounter};
    let mut map: ORMap<String, PNCounter> = ORMap::new(1);
    map.get_or_insert("key".to_string()).increment(10);

    let before_len = map.len();
    let before_val = map.get(&"key".to_string()).unwrap().value();
    map.merge(&map.clone());

    assert_eq!(map.len(), before_len);
    assert_eq!(map.get(&"key".to_string()).unwrap().value(), before_val);
}

// ===== WAVE 3.4: ORMAP SERIALIZATION =====

#[test]
fn test_ormap_serialization() {
    use logos_core::crdt::{ORMap, PNCounter};

    let mut map: ORMap<String, PNCounter> = ORMap::new(42);
    map.get_or_insert("score".to_string()).increment(100);

    let bytes = bincode::serialize(&map).unwrap();
    let decoded: ORMap<String, PNCounter> = bincode::deserialize(&bytes).unwrap();

    assert_eq!(map.len(), decoded.len());
    assert_eq!(
        map.get(&"score".to_string()).unwrap().value(),
        decoded.get(&"score".to_string()).unwrap().value()
    );
}

// ===== WAVE 3.4: ORMAP WITH DIFFERENT VALUE TYPES =====

#[test]
fn test_ormap_with_mvregister() {
    use logos_core::crdt::{MVRegister, Merge, ORMap};

    let mut map: ORMap<String, MVRegister<String>> = ORMap::new(1);
    map.get_or_insert("title".to_string()).set("Hello".to_string());

    assert!(map.contains_key(&"title".to_string()));
}

#[test]
fn test_ormap_nested_conflict_resolution() {
    use logos_core::crdt::{MVRegister, Merge, ORMap};

    let mut a: ORMap<String, MVRegister<String>> = ORMap::new(1);
    let mut b: ORMap<String, MVRegister<String>> = ORMap::new(2);

    // Both set same key concurrently
    a.get_or_insert("doc".to_string()).set("v1".to_string());
    b.get_or_insert("doc".to_string()).set("v2".to_string());

    a.merge(&b);

    // MVRegister inside should have both values
    let values = a.get(&"doc".to_string()).unwrap().values();
    assert_eq!(values.len(), 2);
}
