use super::*;
use crate::cardinality::at_most;

fn atom(s: &str) -> ProofExpr {
    ProofExpr::Atom(s.to_string())
}
fn or(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Or(Box::new(a), Box::new(b))
}
fn and(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(a), Box::new(b))
}
fn contradiction() -> ProofExpr {
    let f = atom("__f");
    and(f.clone(), ProofExpr::Not(Box::new(f)))
}

/// Minimum number of `true` atoms satisfying a set of clauses, found and CERTIFIED.
#[test]
fn finds_certified_minimum_hitting_count() {
    // (a∨b) ∧ (b∨c) ∧ (a∨c): every assignment with <2 trues leaves a clause unsatisfied,
    // so the minimum is exactly 2.
    let (a, b, c) = (atom("a"), atom("b"), atom("c"));
    let vars = vec![a.clone(), b.clone(), c.clone()];
    let clauses = and(
        and(or(a.clone(), b.clone()), or(b.clone(), c.clone())),
        or(a.clone(), c.clone()),
    );
    let feasible_at = |bound: i64| {
        if bound < 0 {
            contradiction()
        } else {
            and(clauses.clone(), at_most(&vars, bound as usize, "cost"))
        }
    };
    let res = minimize_certified(feasible_at, 0, 3).expect("feasible at 3");
    assert_eq!(res.optimum, 2, "minimum hitting count is 2");
    assert!(
        res.minimal_certified,
        "a 1-true solution must be RUP-refuted"
    );
    // Sanity: the witness really turns on at least 2 of a/b/c.
    let ons = res
        .witness
        .iter()
        .filter(|(n, v)| *v && matches!(n.as_str(), "a" | "b" | "c"))
        .count();
    assert!(
        ons >= 2,
        "witness must satisfy the clauses: {:?}",
        res.witness
    );
}

#[test]
fn zero_cost_optimum_when_unconstrained() {
    // No constraints → 0 trues suffices; optimum is the floor and is trivially minimal.
    let vars = vec![atom("a"), atom("b")];
    let feasible_at = |bound: i64| at_most(&vars, bound.max(0) as usize, "c");
    let res = minimize_certified(feasible_at, 0, 2).unwrap();
    assert_eq!(res.optimum, 0);
    assert!(res.minimal_certified);
}

#[test]
fn infeasible_range_returns_none() {
    // "cost ≤ 1" can never satisfy a clause needing 2 trues → no solution in [0, 1].
    let (a, b, c) = (atom("a"), atom("b"), atom("c"));
    let vars = vec![a.clone(), b.clone(), c.clone()];
    let clauses = and(
        and(or(a.clone(), b.clone()), or(b.clone(), c.clone())),
        or(a.clone(), c.clone()),
    );
    let feasible_at = |bound: i64| {
        and(
            clauses.clone(),
            at_most(&vars, bound.max(0) as usize, "cost"),
        )
    };
    assert!(minimize_certified(feasible_at, 0, 1).is_none());
}

#[test]
fn larger_cover_minimum_is_three() {
    // Five clauses forcing at least 3 of {a,b,c,d}: pairwise-ish coverage that 2 can't hit.
    // K4 edges as clauses (each edge needs an endpoint) only forces a vertex cover; instead
    // force exactly: require a, and (b∨c), and (c∨d), and (b∨d) → need b,c,d cover {bc,cd,bd}
    // = at least 2 of b,c,d, plus a ⇒ min 3.
    let (a, b, c, d) = (atom("a"), atom("b"), atom("c"), atom("d"));
    let vars = vec![a.clone(), b.clone(), c.clone(), d.clone()];
    let clauses = and(
        and(a.clone(), or(b.clone(), c.clone())),
        and(or(c.clone(), d.clone()), or(b.clone(), d.clone())),
    );
    let feasible_at = |bound: i64| {
        and(
            clauses.clone(),
            at_most(&vars, bound.max(0) as usize, "cost"),
        )
    };
    let res = minimize_certified(feasible_at, 0, 4).unwrap();
    assert_eq!(res.optimum, 3);
    assert!(res.minimal_certified);
}

#[test]
fn singleton_range_returns_that_bound() {
    // Force both a and b true (cost 2); search the singleton range [2, 2].
    let vars = vec![atom("a"), atom("b")];
    let clauses = and(atom("a"), atom("b"));
    let feasible_at = |bound: i64| and(clauses.clone(), at_most(&vars, bound.max(0) as usize, "c"));
    let res = minimize_certified(feasible_at, 2, 2).unwrap();
    assert_eq!(res.optimum, 2);
    assert!(res.minimal_certified, "optimum == lo is trivially minimal");
}

#[test]
fn lo_greater_than_hi_is_none() {
    let vars = vec![atom("a")];
    let feasible_at = |bound: i64| at_most(&vars, bound.max(0) as usize, "c");
    assert!(minimize_certified(feasible_at, 5, 2).is_none());
}
