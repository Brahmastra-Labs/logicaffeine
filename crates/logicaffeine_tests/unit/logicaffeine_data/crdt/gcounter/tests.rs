use super::*;

#[test]
fn test_gcounter_new() {
    let c = GCounter::new();
    assert_eq!(c.value(), 0);
}

#[test]
fn test_gcounter_increment() {
    let mut c = GCounter::with_replica_id(1);
    c.increment(5);
    c.increment(3);
    assert_eq!(c.value(), 8);
}

#[test]
fn test_gcounter_merge_disjoint() {
    let mut c1 = GCounter::with_replica_id(1);
    let mut c2 = GCounter::with_replica_id(2);

    c1.increment(5);
    c2.increment(3);

    c1.merge(&c2);
    assert_eq!(c1.value(), 8);
}

#[test]
fn test_gcounter_merge_commutative() {
    let mut c1 = GCounter::with_replica_id(1);
    let mut c2 = GCounter::with_replica_id(2);

    c1.increment(5);
    c2.increment(3);

    let mut c1_copy = c1.clone();
    let mut c2_copy = c2.clone();

    c1_copy.merge(&c2);
    c2_copy.merge(&c1);

    assert_eq!(c1_copy.value(), c2_copy.value());
}

#[test]
fn test_gcounter_merge_idempotent() {
    let mut c1 = GCounter::with_replica_id(1);
    c1.increment(5);

    let before = c1.value();
    c1.merge(&c1.clone());
    assert_eq!(c1.value(), before);
}

#[test]
fn test_gcounter_merge_same_replica() {
    // When two counters have the same replica ID (simulating sync after divergence)
    let mut c1 = GCounter::with_replica_id(1);
    let mut c2 = GCounter::with_replica_id(1);

    c1.increment(5);
    c2.increment(3);

    // After merge, should have max(5, 3) = 5
    c1.merge(&c2);
    assert_eq!(c1.value(), 5);
}
