use super::*;

#[test]
fn test_orset_add_contains() {
    let mut set: ORSet<String> = ORSet::new(1);
    set.add("alice".to_string());
    assert!(set.contains(&"alice".to_string()));
    assert!(!set.contains(&"bob".to_string()));
}

#[test]
fn test_orset_remove() {
    let mut set: ORSet<String> = ORSet::new(1);
    set.add("alice".to_string());
    set.remove(&"alice".to_string());
    assert!(!set.contains(&"alice".to_string()));
}

#[test]
fn test_orset_add_wins() {
    let mut a: ORSet<String> = ORSet::new(1);
    let mut b: ORSet<String> = ORSet::new(2);

    a.add("item".to_string());
    b.merge(&a);

    a.remove(&"item".to_string());
    b.add("item".to_string());

    a.merge(&b);
    assert!(a.contains(&"item".to_string()));
}

#[test]
fn test_orset_merge_commutative() {
    let mut a: ORSet<String> = ORSet::new(1);
    let mut b: ORSet<String> = ORSet::new(2);

    a.add("x".to_string());
    b.add("y".to_string());

    let mut a1 = a.clone();
    let mut b1 = b.clone();
    a1.merge(&b);
    b1.merge(&a);

    assert_eq!(a1.len(), b1.len());
}
