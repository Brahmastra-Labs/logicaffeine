use super::*;
use crate::recursor::bounded::{GenerationResult, RecursorBudget, RecursorGenerationError};
fn equivalent(a: &Universe, b: &Universe, budget: &mut RecursorBudget) -> GenerationResult<bool> {
    a.equiv_bounded(b, budget)
}
fn budget() -> RecursorBudget {
    RecursorBudget {
        max_depth: crate::type_checker::MAX_TERM_DEPTH,
        max_tree_nodes: 2_000_000,
        max_store_nodes: 2_000_000,
        work: 10_000_000,
        allocation: 10_000_000,
        materialized_nodes: 10_000_000,
    }
}
fn var(n: &str) -> Universe {
    Universe::Var(n.into())
}
fn max(a: Universe, b: Universe) -> Universe {
    Universe::Max(Box::new(a), Box::new(b))
}
fn imax(a: Universe, b: Universe) -> Universe {
    Universe::IMax(Box::new(a), Box::new(b))
}
fn succ(a: Universe) -> Universe {
    Universe::Succ(Box::new(a))
}
#[test]
fn repeated_max_and_successor_distribution_are_equal() {
    let a = max(
        max(succ(var("u")), succ(var("v"))),
        succ(max(var("u"), var("v"))),
    );
    let b = succ(max(var("u"), var("v")));
    assert!(a.equiv(&b));
    assert!(equivalent(&a, &b, &mut budget()).unwrap());
}
#[test]
fn cumulativity_does_not_make_distinct_rigid_levels_equal() {
    for (a, b) in [
        (var("u"), var("v")),
        (var("u"), succ(var("u"))),
        (var("u"), max(var("u"), var("v"))),
        (Universe::Prop, Universe::Type(0)),
        (Universe::SProp, Universe::Prop),
    ] {
        assert!(!a.equiv(&b));
        assert!(!equivalent(&a, &b, &mut budget()).unwrap());
    }
}
#[test]
fn imax_split_witness_is_fresh_in_both_sides() {
    let a = imax(Universe::Prop, var("u"));
    let b = max(a.clone(), succ(var("u✦")));
    assert!(!a.equiv(&b));
    assert!(!equivalent(&a, &b, &mut budget()).unwrap());
    let c = imax(var("u✦"), max(var("u"), var("u✦✦")));
    assert!(equivalent(&c, &c, &mut budget()).unwrap());
}
#[test]
fn metered_algorithm_matches_existing_exact_equivalence_rules() {
    let levels = vec![
        Universe::SProp,
        Universe::Prop,
        Universe::Type(0),
        var("u"),
        var("v"),
        succ(var("u")),
        max(var("u"), var("v")),
        max(var("v"), var("u")),
        imax(var("u"), var("v")),
        imax(var("u"), max(var("v"), var("w"))),
        imax(var("u"), imax(var("v"), var("w"))),
        max(imax(var("u"), var("v")), imax(var("u"), var("w"))),
        imax(Universe::Prop, var("u")),
        var("u✦"),
    ];
    for a in &levels {
        for b in &levels {
            assert_eq!(
                equivalent(a, b, &mut budget()).unwrap(),
                a.equiv(b),
                "{a:?} {b:?}"
            );
        }
    }
}
#[test]
fn work_allocation_and_depth_exhaustion_are_resources_not_inequality() {
    let a = imax(var("u"), var("v"));
    let b = max(var("u"), var("v"));
    for which in 0..3 {
        let mut budget = budget();
        match which {
            0 => budget.work = 0,
            1 => budget.allocation = 0,
            _ => budget.max_depth = 1,
        };
        assert!(matches!(
            equivalent(&a, &b, &mut budget),
            Err(RecursorGenerationError::ResourceLimit)
        ));
    }
}
#[test]
fn repeated_comparisons_consume_one_shared_budget() {
    let mut budget = budget();
    let work = budget.work;
    let allocation = budget.allocation;
    assert!(equivalent(&var("u"), &max(var("u"), var("u")), &mut budget).unwrap());
    assert!(budget.work < work && budget.allocation < allocation);
    budget.work = 0;
    assert!(matches!(
        equivalent(&var("u"), &var("u"), &mut budget),
        Err(RecursorGenerationError::ResourceLimit)
    ));
}

#[test]
fn positive_branch_witness_cannot_capture_an_existing_parameter_in_either_order() {
    // The zero branch agrees; only the positive branch distinguishes these levels.
    // Reusing u✦ for u's witness ties two independent variables together and can
    // falsely turn the implication into equality.
    let a = imax(var("u✦"), var("u"));
    let b = var("u");
    assert!(!a.equiv(&b));
    assert!(!b.equiv(&a));
    assert!(!equivalent(&a, &b, &mut budget()).unwrap());
    assert!(!equivalent(&b, &a, &mut budget()).unwrap());
}

#[path = "level_order_oracle.rs"]
mod oracle;

#[test]
fn shared_order_core_matches_frozen_baseline_on_imax_and_freshness_cases() {
    let base = [
        Universe::Prop,
        Universe::Type(0),
        var("u"),
        var("v"),
        var("u✦"),
    ];
    let mut levels = vec![Universe::SProp];
    levels.extend(base.iter().cloned());
    for a in &base {
        levels.push(succ(a.clone()));
        for b in &base {
            levels.push(max(a.clone(), b.clone()));
            levels.push(imax(a.clone(), b.clone()));
            levels.push(imax(a.clone(), imax(b.clone(), var("w"))));
            levels.push(imax(a.clone(), max(b.clone(), var("u✦✦"))));
        }
    }
    for a in &levels {
        for b in &levels {
            let forward = oracle::leq(a, b);
            let backward = oracle::leq(b, a);
            assert_eq!(a.is_subtype_of(b), forward, "order {a:?} {b:?}");
            assert_eq!(a.equiv(b), forward && backward, "equality {a:?} {b:?}");
            assert_eq!(
                equivalent(a, b, &mut budget()).unwrap(),
                forward && backward,
                "bounded equality {a:?} {b:?}"
            );
        }
    }
}

#[test]
fn second_order_direction_cannot_replenish_the_callers_budget() {
    let a = max(var("u"), var("v"));
    let b = max(var("v"), var("u"));
    let mut measured = budget();
    let work = measured.work;
    assert!(equivalent(&a, &b, &mut measured).unwrap());
    let spent = work - measured.work;
    let mut short = budget();
    short.work = spent - 1;
    assert!(matches!(
        equivalent(&a, &b, &mut short),
        Err(RecursorGenerationError::ResourceLimit)
    ));
}

#[test]
fn an_oversized_depth_budget_is_refused_before_equality_work() {
    let mut b = budget();
    b.max_depth = crate::type_checker::MAX_TERM_DEPTH + 1;
    let work = b.work;
    let allocation = b.allocation;
    assert!(matches!(
        equivalent(&Universe::Prop, &Universe::Prop, &mut b),
        Err(RecursorGenerationError::ResourceLimit)
    ));
    assert_eq!(b.work, work);
    assert_eq!(b.allocation, allocation);
}

#[test]
fn branch_frames_and_traversal_share_one_active_depth_budget() {
    let branch = |n: &str| imax(Universe::Prop, var(n));
    let a = max(max(branch("a"), branch("b")), max(branch("c"), branch("d")));
    let b = max(max(var("a"), var("b")), max(var("c"), var("d")));
    assert!(oracle::leq(&a, &b) && oracle::leq(&b, &a));
    let mut shallow = budget();
    shallow.max_depth = 8;
    assert!(matches!(
        equivalent(&a, &b, &mut shallow),
        Err(RecursorGenerationError::ResourceLimit)
    ));
    let mut sufficient = budget();
    sufficient.max_depth = 128;
    assert!(equivalent(&a, &b, &mut sufficient).unwrap());
}
