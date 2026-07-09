//! The certified RECURSIVE symmetry breaker, wired into the cascade. Each round re-detects the residual
//! automorphism group, certifies ONE lex-leader lead clause as a PR step, and re-detects — looping to a
//! fixpoint that breaks the COMPLETE group, not just the adjacent positive-row swaps the single-pass
//! `symmetry::break_symmetries` sees. It closes the verdict-only gap: the cascade's symmetric-UNSAT
//! refutations now carry a proof that `check_pr_refutation` independently re-validates.

use logicaffeine_proof::cdcl::Lit;
use logicaffeine_proof::hypercube::clauses_to_expr;
use logicaffeine_proof::pr::check_pr_refutation;
use logicaffeine_proof::sat::{prove_unsat, prove_unsat_certified, UnsatOutcome};
use logicaffeine_proof::sym_certify::certified_unsat_auto;
use logicaffeine_proof::families;

/// The breaker must refute, ACTUALLY break symmetry (≥ 1 certified lead), produce a proof that re-checks
/// against the formula, and be wired into both the cascade verdict and the certified API.
fn assert_certified_recursive_break(nv: usize, clauses: &[Vec<Lit>]) {
    let r = certified_unsat_auto(nv, clauses);
    assert!(r.refuted, "the recursive breaker must refute the symmetric UNSAT formula");
    assert!(r.sbp_clauses >= 1, "it must actually break symmetry — at least one certified lex-leader lead");
    assert!(
        check_pr_refutation(nv, clauses, &r.steps),
        "the composed PR(symmetry) + RUP(search) proof must independently re-check against the formula"
    );

    // Wired: the certified cascade entry point refutes it, and the certified API returns the same proof.
    let e = clauses_to_expr(clauses).expect("a CNF is always expressible as a ProofExpr");
    assert_eq!(prove_unsat(&e), UnsatOutcome::Refuted, "the cascade must refute the symmetric formula");
    let steps = prove_unsat_certified(&e).expect("the certified API returns a refutation proof");
    assert!(!steps.is_empty(), "the certified proof must carry steps");
}

#[test]
fn cascade_certifies_clique_color_symmetry_recursively() {
    // clique-coloring's hardness is COLOR-permutation symmetry (a column swap) that the single-pass
    // ROW-swap break is structurally blind to — the recursive group re-detection breaks and certifies it.
    for (n, k) in [(4usize, 3usize), (5, 4), (6, 5)] {
        let (cnf, _) = families::clique_coloring(n, k);
        assert_certified_recursive_break(cnf.num_vars, &cnf.clauses);
    }
}

#[test]
fn cascade_certifies_pigeonhole_recursively() {
    // The row-symmetric counterpart — the recursive breaker certifies it the same way.
    for n in [4usize, 5, 6] {
        let (cnf, _) = families::php(n);
        assert_certified_recursive_break(cnf.num_vars, &cnf.clauses);
    }
}

#[test]
fn recursive_break_is_fail_closed_on_satisfiable() {
    // K_3 with 3 colours is SATISFIABLE: the certified breaker must NOT manufacture a refutation, and the
    // certified API must decline (return None).
    let (cnf, _) = families::clique_coloring(3, 3);
    assert!(!certified_unsat_auto(cnf.num_vars, &cnf.clauses).refuted, "must not refute a satisfiable formula");
    let e = clauses_to_expr(&cnf.clauses).expect("expressible");
    assert!(prove_unsat_certified(&e).is_none(), "the certified API must decline on a satisfiable formula");
    assert!(matches!(prove_unsat(&e), UnsatOutcome::Sat(_)), "the cascade must report SAT, with a model");
}
