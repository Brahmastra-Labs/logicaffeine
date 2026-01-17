//! Phase CRDT PNCounter: Tests for Tally (PN-Counter)
//!
//! Wave 2.1 of CRDT Expansion: Bidirectional counter.
//!
//! TDD: These are RED tests - they define the spec before implementation.

// ===== WAVE 2.1: PNCOUNTER BASICS =====

#[test]
fn test_pncounter_new() {
    use logicaffeine_data::crdt::PNCounter;
    let counter = PNCounter::new();
    assert_eq!(counter.value(), 0);
}

#[test]
fn test_pncounter_with_replica_id() {
    use logicaffeine_data::crdt::PNCounter;
    let counter = PNCounter::with_replica_id(42);
    assert_eq!(counter.replica_id(), 42);
    assert_eq!(counter.value(), 0);
}

#[test]
fn test_pncounter_increment() {
    use logicaffeine_data::crdt::PNCounter;
    let mut counter = PNCounter::with_replica_id(1);
    counter.increment(10);
    assert_eq!(counter.value(), 10);
    counter.increment(5);
    assert_eq!(counter.value(), 15);
}

#[test]
fn test_pncounter_decrement() {
    use logicaffeine_data::crdt::PNCounter;
    let mut counter = PNCounter::with_replica_id(1);
    counter.increment(10);
    counter.decrement(3);
    assert_eq!(counter.value(), 7);
}

#[test]
fn test_pncounter_negative() {
    use logicaffeine_data::crdt::PNCounter;
    let mut counter = PNCounter::with_replica_id(1);
    counter.decrement(5);
    assert_eq!(counter.value(), -5);
}

#[test]
fn test_pncounter_increment_decrement_interleaved() {
    use logicaffeine_data::crdt::PNCounter;
    let mut counter = PNCounter::with_replica_id(1);
    counter.increment(100);
    counter.decrement(30);
    counter.increment(10);
    counter.decrement(5);
    assert_eq!(counter.value(), 75);
}

// ===== WAVE 2.1: PNCOUNTER MERGE =====

#[test]
fn test_pncounter_merge_disjoint() {
    use logicaffeine_data::crdt::{Merge, PNCounter};
    let mut a = PNCounter::with_replica_id(1);
    let mut b = PNCounter::with_replica_id(2);

    a.increment(10);
    b.decrement(3);

    a.merge(&b);
    assert_eq!(a.value(), 7); // 10 - 3
}

#[test]
fn test_pncounter_merge_commutative() {
    use logicaffeine_data::crdt::{Merge, PNCounter};
    let mut a = PNCounter::with_replica_id(1);
    let mut b = PNCounter::with_replica_id(2);

    a.increment(10);
    a.decrement(2);
    b.increment(5);
    b.decrement(8);

    let mut a1 = a.clone();
    let mut b1 = b.clone();
    a1.merge(&b);
    b1.merge(&a);

    assert_eq!(a1.value(), b1.value()); // Commutative
}

#[test]
fn test_pncounter_merge_associative() {
    use logicaffeine_data::crdt::{Merge, PNCounter};
    let mut a = PNCounter::with_replica_id(1);
    let mut b = PNCounter::with_replica_id(2);
    let mut c = PNCounter::with_replica_id(3);

    a.increment(10);
    b.decrement(5);
    c.increment(3);

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

    assert_eq!(abc1.value(), abc2.value()); // Associative
}

#[test]
fn test_pncounter_merge_idempotent() {
    use logicaffeine_data::crdt::{Merge, PNCounter};
    let mut counter = PNCounter::with_replica_id(1);
    counter.increment(10);
    counter.decrement(3);

    let before = counter.value();
    counter.merge(&counter.clone());
    assert_eq!(counter.value(), before); // Idempotent
}

#[test]
fn test_pncounter_merge_same_replica() {
    use logicaffeine_data::crdt::{Merge, PNCounter};
    let mut a = PNCounter::with_replica_id(1);
    let mut b = PNCounter::with_replica_id(1);

    a.increment(10);
    b.increment(5);

    // After merge, should have max(10, 5) = 10
    a.merge(&b);
    assert_eq!(a.value(), 10);
}

#[test]
fn test_pncounter_merge_complex() {
    use logicaffeine_data::crdt::{Merge, PNCounter};
    let mut a = PNCounter::with_replica_id(1);
    let mut b = PNCounter::with_replica_id(2);

    // Node A increments and decrements
    a.increment(100);
    a.decrement(20);

    // Node B does different operations
    b.increment(50);
    b.decrement(30);

    // Merge: (100 - 20) from A + (50 - 30) from B = 80 + 20 = 100
    a.merge(&b);
    assert_eq!(a.value(), 100);
}

// ===== WAVE 2.1: PNCOUNTER SERIALIZATION =====

#[test]
fn test_pncounter_serialization() {
    use logicaffeine_data::crdt::PNCounter;

    let mut counter = PNCounter::with_replica_id(42);
    counter.increment(100);
    counter.decrement(30);

    let bytes = bincode::serialize(&counter).unwrap();
    let decoded: PNCounter = bincode::deserialize(&bytes).unwrap();

    assert_eq!(counter.value(), decoded.value());
}
