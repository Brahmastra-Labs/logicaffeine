use super::*;
use crate::Universe;
fn budget() -> RecursorBudget {
    RecursorBudget {
        max_depth: crate::type_checker::MAX_TERM_DEPTH,
        max_tree_nodes: 100_000,
        max_store_nodes: 100_000,
        work: 10_000_000,
        allocation: 10_000_000,
        materialized_nodes: 1_000_000,
    }
}
#[test]
fn stored_sort_ids_need_not_match_for_semantic_equality() {
    let u = Universe::Var("u".into());
    let v = Universe::Var("v".into());
    let a = Term::Sort(Universe::Max(Box::new(u.clone()), Box::new(v.clone())));
    let b = Term::Sort(Universe::Max(Box::new(v), Box::new(u)));
    let mut budget = budget();
    let mut d = Construction::new(&mut budget).unwrap();
    let a = d.intern(&a).unwrap();
    let b = d.intern(&b).unwrap();
    assert_ne!(a, b);
    assert!(same_sort(&mut d, a, b).unwrap());
}
#[test]
fn unequal_and_nonsort_ids_are_not_accepted() {
    let mut budget = budget();
    let mut d = Construction::new(&mut budget).unwrap();
    let a = d.intern(&Term::Sort(Universe::Var("u".into()))).unwrap();
    let b = d.intern(&Term::Sort(Universe::Var("v".into()))).unwrap();
    assert!(!same_sort(&mut d, a, b).unwrap());
    let v = d.var("u").unwrap();
    assert!(!same_sort(&mut d, v, v).unwrap());
}
