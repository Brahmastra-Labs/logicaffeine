//! The certified cascade on graph k-colouring of the complete graph K_n — the canonical
//! colour-permutation family. K_n needs n colours, so it is UNSAT exactly when k < n. Its encoding
//! reduces to pigeonhole: n vertices are the items, k colours the slots, and the all-pairs
//! "adjacent vertices differ" clauses make each colour a full at-most-one clique. So the matching
//! route decides it — the family a prior benchmark DROPPED when a solver beat us, re-locked here so
//! the pigeonhole detector must recognize its encoding and win.
//!
//! The `scales` test pushes the family to a ~12.6k-clause instance to prove the matching route decides
//! it end-to-end with no stack overflow — the O(1) `items > slots` Hall bound is independent of the
//! clause count, so the family scales to a genuine PHP-class resolution wall.

use logicaffeine_proof::families::{clique_coloring, ExpectedVerdict};
use logicaffeine_proof::hypercube::clauses_to_expr;
use logicaffeine_proof::pigeonhole::decide_pigeonhole_unsat;
use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};

#[test]
fn clique_coloring_is_decided_by_the_matching_route() {
    // k < n → UNSAT; the pigeonhole/matching detector MUST fire on the K_n-colouring encoding. This is
    // the regression guard: the dropped clique family fell to slow CDCL because the detector missed it.
    for (n, k) in [(6usize, 5usize), (8, 7), (10, 9), (12, 8), (12, 11)] {
        assert!(k < n, "test setup: k<n for the UNSAT case");
        let (cnf, verdict) = clique_coloring(n, k);
        assert_eq!(verdict, ExpectedVerdict::Unsat, "K_{n} needs {n} colours, only {k} given → UNSAT");
        let e = clauses_to_expr(&cnf.clauses).expect("flat CNF clausifies");
        assert!(
            decide_pigeonhole_unsat(&e),
            "clique_coloring({n},{k}): the matching detector must recognize the K_n-colouring pigeonhole"
        );
        assert_eq!(
            prove_unsat(&e),
            UnsatOutcome::Refuted,
            "clique_coloring({n},{k}) must be refuted by the matching route"
        );
    }
}

#[test]
fn clique_coloring_is_sat_when_enough_colours() {
    // k >= n → a proper colouring exists (χ(K_n) = n) → SATISFIABLE. Neither the detector nor the
    // cascade may falsely refute it.
    for (n, k) in [(5usize, 5usize), (5, 6), (8, 8), (3, 4)] {
        assert!(k >= n, "test setup: k>=n for the SAT case");
        let (cnf, verdict) = clique_coloring(n, k);
        assert_eq!(verdict, ExpectedVerdict::Sat, "K_{n} is {k}-colourable when k >= n");
        let e = clauses_to_expr(&cnf.clauses).unwrap();
        assert!(
            !decide_pigeonhole_unsat(&e),
            "clique_coloring({n},{k}) is SAT — the matching detector must not claim UNSAT"
        );
        if let UnsatOutcome::Refuted = prove_unsat(&e) {
            panic!("clique_coloring({n},{k}) is satisfiable (k >= n) — the cascade must not falsely refute");
        }
    }
}

#[test]
fn clique_coloring_scales_past_the_expr_recursion_limit() {
    // clique_coloring(30, 29) is ~12.6k clauses — a PHP(30→29)-class instance every resolution solver
    // walls on. The matching route returns at the O(1) Hall bound (30 items > 29 slots), so it decides
    // even this large instance in microseconds; a scaling guard for the family and its expr machinery.
    let (cnf, verdict) = clique_coloring(30, 29);
    assert_eq!(verdict, ExpectedVerdict::Unsat);
    let e = clauses_to_expr(&cnf.clauses).expect("flat CNF clausifies");
    assert!(
        decide_pigeonhole_unsat(&e),
        "clique_coloring(30,29): matching decides a ~12.6k-clause instance with no stack overflow"
    );
    assert_eq!(prove_unsat(&e), UnsatOutcome::Refuted);
}
