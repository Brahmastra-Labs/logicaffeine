//! Phase CRDT Sequence: Tests for SharedSequence (RGA/YATA)
//!
//! Wave 3.2-3.3 of CRDT Expansion: Collaborative sequences.
//!
//! TDD: These are RED tests - they define the spec before implementation.

// ===== WAVE 3.2: RGA BASICS =====

#[test]
fn test_rga_new() {
    use logos_core::crdt::RGA;
    let seq: RGA<String> = RGA::new(1);
    assert!(seq.is_empty());
    assert_eq!(seq.len(), 0);
}

#[test]
fn test_rga_append() {
    use logos_core::crdt::RGA;
    let mut seq: RGA<String> = RGA::new(1);
    seq.append("a".to_string());
    seq.append("b".to_string());
    assert_eq!(seq.to_vec(), vec!["a", "b"]);
}

#[test]
fn test_rga_insert_before() {
    use logos_core::crdt::RGA;
    let mut seq: RGA<String> = RGA::new(1);
    seq.append("b".to_string());
    seq.insert_before(0, "a".to_string());
    assert_eq!(seq.to_vec(), vec!["a", "b"]);
}

#[test]
fn test_rga_insert_after() {
    use logos_core::crdt::RGA;
    let mut seq: RGA<String> = RGA::new(1);
    seq.append("a".to_string());
    seq.append("c".to_string());
    seq.insert_after(0, "b".to_string());
    assert_eq!(seq.to_vec(), vec!["a", "b", "c"]);
}

#[test]
fn test_rga_remove() {
    use logos_core::crdt::RGA;
    let mut seq: RGA<String> = RGA::new(1);
    seq.append("a".to_string());
    seq.append("b".to_string());
    seq.append("c".to_string());
    seq.remove(1);
    assert_eq!(seq.to_vec(), vec!["a", "c"]);
}

#[test]
fn test_rga_get() {
    use logos_core::crdt::RGA;
    let mut seq: RGA<String> = RGA::new(1);
    seq.append("a".to_string());
    seq.append("b".to_string());
    assert_eq!(seq.get(0), Some(&"a".to_string()));
    assert_eq!(seq.get(1), Some(&"b".to_string()));
    assert_eq!(seq.get(2), None);
}

// ===== WAVE 3.2: RGA CONCURRENT OPERATIONS =====

#[test]
fn test_rga_concurrent_append() {
    use logos_core::crdt::{Merge, RGA};
    let mut a: RGA<String> = RGA::new(1);
    let mut b: RGA<String> = RGA::new(2);

    // Both append concurrently
    a.append("from-a".to_string());
    b.append("from-b".to_string());

    a.merge(&b);
    b.merge(&a);

    // Both should converge to same order
    assert_eq!(a.to_vec(), b.to_vec());
    assert_eq!(a.len(), 2);
}

#[test]
fn test_rga_concurrent_insert() {
    use logos_core::crdt::{Merge, RGA};
    let mut a: RGA<String> = RGA::new(1);
    let mut b: RGA<String> = RGA::new(2);

    // Base state
    a.append("x".to_string());
    b.merge(&a);

    // Both insert after "x" concurrently
    a.insert_after(0, "a-insert".to_string());
    b.insert_after(0, "b-insert".to_string());

    a.merge(&b);
    b.merge(&a);

    // Must converge to same order
    assert_eq!(a.to_vec(), b.to_vec());
    assert_eq!(a.len(), 3);
}

#[test]
fn test_rga_concurrent_remove() {
    use logos_core::crdt::{Merge, RGA};
    let mut a: RGA<String> = RGA::new(1);
    let mut b: RGA<String> = RGA::new(2);

    // Base state
    a.append("keep".to_string());
    a.append("delete".to_string());
    b.merge(&a);

    // Both remove same element
    a.remove(1);
    b.remove(1);

    a.merge(&b);
    b.merge(&a);

    assert_eq!(a.to_vec(), vec!["keep"]);
    assert_eq!(a.to_vec(), b.to_vec());
}

// ===== WAVE 3.2: RGA MERGE PROPERTIES =====

#[test]
fn test_rga_merge_commutative() {
    use logos_core::crdt::{Merge, RGA};
    let mut a: RGA<String> = RGA::new(1);
    let mut b: RGA<String> = RGA::new(2);

    a.append("a".to_string());
    b.append("b".to_string());

    let mut a1 = a.clone();
    let mut b1 = b.clone();
    a1.merge(&b);
    b1.merge(&a);

    assert_eq!(a1.to_vec(), b1.to_vec());
}

#[test]
fn test_rga_merge_idempotent() {
    use logos_core::crdt::{Merge, RGA};
    let mut seq: RGA<String> = RGA::new(1);
    seq.append("x".to_string());

    let before = seq.to_vec();
    seq.merge(&seq.clone());
    assert_eq!(seq.to_vec(), before);
}

// ===== WAVE 3.2: RGA SERIALIZATION =====

#[test]
fn test_rga_serialization() {
    use logos_core::crdt::RGA;

    let mut seq: RGA<String> = RGA::new(42);
    seq.append("a".to_string());
    seq.append("b".to_string());

    let bytes = bincode::serialize(&seq).unwrap();
    let decoded: RGA<String> = bincode::deserialize(&bytes).unwrap();

    assert_eq!(seq.to_vec(), decoded.to_vec());
}

// ===== WAVE 3.3: YATA BASICS =====

#[test]
fn test_yata_new() {
    use logos_core::crdt::YATA;
    let seq: YATA<char> = YATA::new(1);
    assert!(seq.is_empty());
}

#[test]
fn test_yata_append() {
    use logos_core::crdt::YATA;
    let mut seq: YATA<char> = YATA::new(1);
    seq.append('a');
    seq.append('b');
    assert_eq!(seq.to_vec(), vec!['a', 'b']);
}

#[test]
fn test_yata_insert() {
    use logos_core::crdt::YATA;
    let mut seq: YATA<char> = YATA::new(1);
    seq.append('a');
    seq.append('c');
    seq.insert_after(0, 'b');
    assert_eq!(seq.to_vec(), vec!['a', 'b', 'c']);
}

// ===== WAVE 3.3: YATA CONCURRENT (INTERLEAVING) =====

#[test]
fn test_yata_interleaving() {
    use logos_core::crdt::{Merge, YATA};
    let mut a: YATA<char> = YATA::new(1);
    let mut b: YATA<char> = YATA::new(2);

    // Both type at same position
    a.append('A');
    b.append('B');

    a.merge(&b);
    b.merge(&a);

    // YATA interleaves: deterministic order
    assert_eq!(a.to_vec(), b.to_vec());
    assert_eq!(a.len(), 2);
}

#[test]
fn test_yata_concurrent_insert_same_position() {
    use logos_core::crdt::{Merge, YATA};
    let mut a: YATA<char> = YATA::new(1);
    let mut b: YATA<char> = YATA::new(2);

    // Shared base
    a.append('X');
    b.merge(&a);

    // Both insert after X
    a.insert_after(0, 'A');
    b.insert_after(0, 'B');

    a.merge(&b);
    b.merge(&a);

    // Must converge
    assert_eq!(a.to_vec(), b.to_vec());
}

#[test]
fn test_yata_serialization() {
    use logos_core::crdt::YATA;

    let mut seq: YATA<char> = YATA::new(42);
    seq.append('H');
    seq.append('i');

    let bytes = bincode::serialize(&seq).unwrap();
    let decoded: YATA<char> = bincode::deserialize(&bytes).unwrap();

    assert_eq!(seq.to_vec(), decoded.to_vec());
}
