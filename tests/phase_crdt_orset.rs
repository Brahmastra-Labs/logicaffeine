//! Phase CRDT ORSet: Tests for SharedSet (OR-Set with configurable bias)
//!
//! Wave 3.1 of CRDT Expansion: Add-wins and remove-wins sets.
//!
//! TDD: These are RED tests - they define the spec before implementation.

// ===== WAVE 3.1: ORSET BASICS =====

#[test]
fn test_orset_new() {
    use logos_core::crdt::ORSet;
    let set: ORSet<String> = ORSet::new(1);
    assert!(set.is_empty());
    assert_eq!(set.len(), 0);
}

#[test]
fn test_orset_add() {
    use logos_core::crdt::ORSet;
    let mut set: ORSet<String> = ORSet::new(1);
    set.add("alice".to_string());
    assert!(set.contains(&"alice".to_string()));
    assert!(!set.contains(&"bob".to_string()));
    assert_eq!(set.len(), 1);
}

#[test]
fn test_orset_add_multiple() {
    use logos_core::crdt::ORSet;
    let mut set: ORSet<String> = ORSet::new(1);
    set.add("alice".to_string());
    set.add("bob".to_string());
    set.add("charlie".to_string());
    assert_eq!(set.len(), 3);
}

#[test]
fn test_orset_add_duplicate() {
    use logos_core::crdt::ORSet;
    let mut set: ORSet<String> = ORSet::new(1);
    set.add("alice".to_string());
    set.add("alice".to_string()); // Duplicate
    assert!(set.contains(&"alice".to_string()));
    assert_eq!(set.len(), 1); // Still just one element
}

#[test]
fn test_orset_remove() {
    use logos_core::crdt::ORSet;
    let mut set: ORSet<String> = ORSet::new(1);
    set.add("alice".to_string());
    set.remove(&"alice".to_string());
    assert!(!set.contains(&"alice".to_string()));
    assert!(set.is_empty());
}

#[test]
fn test_orset_remove_nonexistent() {
    use logos_core::crdt::ORSet;
    let mut set: ORSet<String> = ORSet::new(1);
    set.remove(&"alice".to_string()); // No-op, doesn't panic
    assert!(set.is_empty());
}

#[test]
fn test_orset_iter() {
    use logos_core::crdt::ORSet;
    let mut set: ORSet<String> = ORSet::new(1);
    set.add("alice".to_string());
    set.add("bob".to_string());

    let items: Vec<_> = set.iter().collect();
    assert_eq!(items.len(), 2);
}

// ===== WAVE 3.1: ORSET ADD-WINS (DEFAULT) =====

#[test]
fn test_orset_add_wins_default() {
    use logos_core::crdt::{Merge, ORSet};
    let mut a: ORSet<String> = ORSet::new(1);
    let mut b: ORSet<String> = ORSet::new(2);

    // A adds item
    a.add("item".to_string());
    b.merge(&a);

    // Concurrent: A removes, B adds again
    a.remove(&"item".to_string());
    b.add("item".to_string());

    // Merge
    a.merge(&b);

    // Add wins: item should be present
    assert!(a.contains(&"item".to_string()));
}

#[test]
fn test_orset_add_wins_multiple_replicas() {
    use logos_core::crdt::{Merge, ORSet};
    let mut a: ORSet<String> = ORSet::new(1);
    let mut b: ORSet<String> = ORSet::new(2);
    let mut c: ORSet<String> = ORSet::new(3);

    // A adds, syncs to B
    a.add("x".to_string());
    b.merge(&a);

    // B removes, C adds (concurrent)
    b.remove(&"x".to_string());
    c.merge(&a); // C saw original add
    c.add("x".to_string()); // C adds again

    // Merge all
    a.merge(&b);
    a.merge(&c);

    // Add wins
    assert!(a.contains(&"x".to_string()));
}

// ===== WAVE 3.1: ORSET REMOVE-WINS BIAS =====

#[test]
fn test_orset_remove_wins() {
    use logos_core::crdt::{Merge, ORSet, RemoveWins};
    let mut a: ORSet<String, RemoveWins> = ORSet::new(1);
    let mut b: ORSet<String, RemoveWins> = ORSet::new(2);

    // A adds item
    a.add("item".to_string());
    b.merge(&a);

    // Concurrent: A removes, B adds again
    a.remove(&"item".to_string());
    b.add("item".to_string());

    // Merge
    a.merge(&b);

    // Remove wins: item should NOT be present
    assert!(!a.contains(&"item".to_string()));
}

#[test]
fn test_orset_add_wins_explicit() {
    use logos_core::crdt::{AddWins, Merge, ORSet};
    let mut a: ORSet<String, AddWins> = ORSet::new(1);
    let mut b: ORSet<String, AddWins> = ORSet::new(2);

    a.add("item".to_string());
    b.merge(&a);
    a.remove(&"item".to_string());
    b.add("item".to_string());
    a.merge(&b);

    assert!(a.contains(&"item".to_string()));
}

// ===== WAVE 3.1: ORSET MERGE PROPERTIES =====

#[test]
fn test_orset_merge_commutative() {
    use logos_core::crdt::{Merge, ORSet};
    let mut a: ORSet<String> = ORSet::new(1);
    let mut b: ORSet<String> = ORSet::new(2);

    a.add("x".to_string());
    b.add("y".to_string());

    let mut a1 = a.clone();
    let mut b1 = b.clone();
    a1.merge(&b);
    b1.merge(&a);

    // Both should have same elements
    assert_eq!(a1.len(), b1.len());
    assert!(a1.contains(&"x".to_string()));
    assert!(a1.contains(&"y".to_string()));
    assert!(b1.contains(&"x".to_string()));
    assert!(b1.contains(&"y".to_string()));
}

#[test]
fn test_orset_merge_associative() {
    use logos_core::crdt::{Merge, ORSet};
    let mut a: ORSet<String> = ORSet::new(1);
    let mut b: ORSet<String> = ORSet::new(2);
    let mut c: ORSet<String> = ORSet::new(3);

    a.add("a".to_string());
    b.add("b".to_string());
    c.add("c".to_string());

    // (a merge b) merge c
    let mut ab = a.clone();
    ab.merge(&b);
    let mut abc1 = ab.clone();
    abc1.merge(&c);

    // a merge (b merge c)
    let mut bc = b.clone();
    bc.merge(&c);
    let mut abc2 = a.clone();
    abc2.merge(&bc);

    assert_eq!(abc1.len(), abc2.len());
}

#[test]
fn test_orset_merge_idempotent() {
    use logos_core::crdt::{Merge, ORSet};
    let mut set: ORSet<String> = ORSet::new(1);
    set.add("x".to_string());

    let before = set.len();
    set.merge(&set.clone());
    assert_eq!(set.len(), before);
}

// ===== WAVE 3.1: ORSET SERIALIZATION =====

#[test]
fn test_orset_serialization() {
    use logos_core::crdt::ORSet;

    let mut set: ORSet<String> = ORSet::new(42);
    set.add("alice".to_string());
    set.add("bob".to_string());

    let bytes = bincode::serialize(&set).unwrap();
    let decoded: ORSet<String> = bincode::deserialize(&bytes).unwrap();

    assert_eq!(set.len(), decoded.len());
    assert!(decoded.contains(&"alice".to_string()));
    assert!(decoded.contains(&"bob".to_string()));
}

// ===== WAVE 3.1: ORSET WITH DIFFERENT TYPES =====

#[test]
fn test_orset_with_integers() {
    use logos_core::crdt::{Merge, ORSet};
    let mut set: ORSet<i64> = ORSet::new(1);
    set.add(42);
    set.add(99);
    assert!(set.contains(&42));
    assert_eq!(set.len(), 2);
}
