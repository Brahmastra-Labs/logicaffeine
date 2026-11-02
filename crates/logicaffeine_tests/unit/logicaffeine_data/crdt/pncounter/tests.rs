use super::*;

#[test]
fn test_pncounter_new() {
    let c = PNCounter::new();
    assert_eq!(c.value(), 0);
}

#[test]
fn test_pncounter_increment_decrement() {
    let mut c = PNCounter::with_replica_id(1);
    c.increment(10);
    c.decrement(3);
    assert_eq!(c.value(), 7);
}

#[test]
fn test_pncounter_negative() {
    let mut c = PNCounter::with_replica_id(1);
    c.decrement(5);
    assert_eq!(c.value(), -5);
}

#[test]
fn test_pncounter_merge() {
    let mut a = PNCounter::with_replica_id(1);
    let mut b = PNCounter::with_replica_id(2);

    a.increment(10);
    b.decrement(3);

    a.merge(&b);
    assert_eq!(a.value(), 7);
}

#[test]
fn test_pncounter_merge_commutative() {
    let mut a = PNCounter::with_replica_id(1);
    let mut b = PNCounter::with_replica_id(2);

    a.increment(10);
    b.decrement(5);

    let mut a1 = a.clone();
    let mut b1 = b.clone();
    a1.merge(&b);
    b1.merge(&a);

    assert_eq!(a1.value(), b1.value());
}
