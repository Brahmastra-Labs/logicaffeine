use super::*;

#[test]
fn test_vclock_new() {
    let clock = VClock::new();
    assert_eq!(clock.get(42), 0);
}

#[test]
fn test_vclock_increment() {
    let mut clock = VClock::new();
    assert_eq!(clock.increment(42), 1);
    assert_eq!(clock.increment(42), 2);
    assert_eq!(clock.get(42), 2);
}

#[test]
fn test_vclock_merge() {
    let mut a = VClock::new();
    let mut b = VClock::new();

    a.increment(1);
    a.increment(1);
    b.increment(2);

    a.merge(&b);

    assert_eq!(a.get(1), 2);
    assert_eq!(a.get(2), 1);
}

#[test]
fn test_vclock_dominates() {
    let mut a = VClock::new();
    let mut b = VClock::new();

    a.increment(1);
    a.increment(1);
    b.increment(1);

    assert!(a.dominates(&b));
    assert!(!b.dominates(&a));
}

#[test]
fn test_vclock_concurrent() {
    let mut a = VClock::new();
    let mut b = VClock::new();

    a.increment(1);
    b.increment(2);

    assert!(a.concurrent(&b));
}
