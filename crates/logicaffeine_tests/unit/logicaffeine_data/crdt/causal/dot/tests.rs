use super::*;
use std::collections::HashSet;

#[test]
fn test_dot_creation() {
    let dot = Dot::new(42, 1);
    assert_eq!(dot.replica, 42);
    assert_eq!(dot.counter, 1);
}

#[test]
fn test_dot_equality() {
    let a = Dot::new(1, 5);
    let b = Dot::new(1, 5);
    let c = Dot::new(1, 6);
    let d = Dot::new(2, 5);

    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_ne!(a, d);
}

#[test]
fn test_dot_hash() {
    let mut set = HashSet::new();
    set.insert(Dot::new(1, 1));
    set.insert(Dot::new(1, 2));
    set.insert(Dot::new(1, 1)); // Duplicate

    assert_eq!(set.len(), 2);
}
