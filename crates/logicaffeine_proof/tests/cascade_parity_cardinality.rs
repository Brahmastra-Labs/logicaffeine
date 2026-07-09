//! The fast GF(2) refuter for the coupled exactly-one + parity family must be BOTH sound and fast: it
//! fires on the genuine coupled contradiction, agrees with the heavyweight fused decider, and declines
//! on satisfiable or non-matching structure — all in microseconds, where the fused route spins up a full
//! CDCL solver with three theories.

use logicaffeine_proof::families;
use logicaffeine_proof::lyapunov::fused_parity_cardinality_decide;
use logicaffeine_proof::parity_cardinality::refute;

#[test]
fn refutes_the_coupled_family_and_agrees_with_the_fused_decider() {
    for n in [2usize, 4, 8, 20, 40, 60, 100] {
        let (cnf, _) = families::parity_exactly_one(n);
        assert!(refute(cnf.num_vars, &cnf.clauses), "n={n}: the coupled exactly-one + parity must be refuted");
        // agree with the ground-truth fused decider (Some(false) = UNSAT).
        assert_eq!(
            fused_parity_cardinality_decide(cnf.num_vars, &cnf.clauses),
            Some(false),
            "n={n}: the fused decider agrees it is UNSAT"
        );
    }
}

#[test]
fn declines_when_the_parity_closure_is_dropped_sat() {
    // Drop the `s_{n-1}=0` unit → SATISFIABLE (the coupling vanishes). The fast refuter must NOT fire,
    // and the fused decider must not report UNSAT.
    for n in [4usize, 8, 20] {
        let (cnf, _) = families::parity_exactly_one(n);
        let mut clauses = cnf.clauses.clone();
        clauses.pop(); // the parity-closure unit
        assert!(!refute(cnf.num_vars, &clauses), "n={n}: without the parity closure it is SAT — must not fire");
        assert_ne!(fused_parity_cardinality_decide(cnf.num_vars, &clauses), Some(false), "n={n}: fused must agree it is not UNSAT");
    }
}

#[test]
fn declines_on_non_parity_cardinality_formulas() {
    // Pigeonhole has cardinality but no parity coupling; a random instance has neither. The refuter must
    // decline (a false fire would be unsound on some satisfiable formula).
    for n in [3usize, 5, 7] {
        let (php, _) = families::php(n);
        assert!(!refute(php.num_vars, &php.clauses), "PHP({n}) is not a parity+cardinality coupling — must decline");
    }
    let r = families::random_3sat(40, 170, 0x1234);
    assert!(!refute(r.num_vars, &r.clauses), "a random 3-SAT instance must not be refuted by the parity+cardinality cut");
}
