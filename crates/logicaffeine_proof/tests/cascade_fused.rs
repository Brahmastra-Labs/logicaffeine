//! The certified cascade (`sat::prove_unsat`) on the COUPLED exactly-one + parity family — a MIXED
//! obstruction unsatisfiable only through both structures at once: exactly-one forces an ODD selector
//! count, the parity closure forces EVEN. Neither the pure GF(2) parity cut nor the pure
//! matching/cardinality cut refutes it alone (each substructure is satisfiable on its own) — only the
//! fused parity+cardinality reasoning does. These tests pin that the general prover reaches that route
//! (so the benchmark's "fused engine" claim is honest) and that the cut never over-fires.

use logicaffeine_proof::families::{parity_exactly_one, ExpectedVerdict};
use logicaffeine_proof::hypercube::clauses_to_expr;
use logicaffeine_proof::lyapunov::fused_parity_cardinality_decide;
use logicaffeine_proof::pigeonhole::decide_pigeonhole_unsat;
use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
use logicaffeine_proof::xorsat::refute_via_parity;

#[test]
fn prove_unsat_crushes_coupled_parity_exactly_one() {
    // 2n vars; the general cascade must refute the coupled contradiction with no problem-specific hint.
    // The general `prove_unsat` cascade decides this via the cutting-plane / fused-collapse cuts (the
    // early matching detector declines). Those cuts walk the clausified expression, which
    // `clauses_to_expr` builds as a BALANCED tree (logarithmic depth), so the cascade scales to the
    // benchmark's wall sizes with no stack overflow.
    for n in [8usize, 40, 80, 120] {
        let (cnf, verdict) = parity_exactly_one(n);
        assert_eq!(verdict, ExpectedVerdict::Unsat);
        let e = clauses_to_expr(&cnf.clauses).expect("flat CNF clausifies");
        assert_eq!(
            prove_unsat(&e),
            UnsatOutcome::Refuted,
            "n={n}: coupled exactly-one + parity must be refuted by the general cascade"
        );
    }
}

#[test]
fn only_the_fused_route_closes_it_neither_specialist_alone() {
    // The NECESSITY argument that justifies the "fused parity+cardinality" mechanism claim: each
    // substructure ALONE is satisfiable, so the single-theory specialists must both DECLINE — yet the
    // fused decider refutes and so does the general cascade.
    for n in [6usize, 12, 24] {
        let (cnf, _) = parity_exactly_one(n);
        let e = clauses_to_expr(&cnf.clauses).unwrap();
        assert!(
            !refute_via_parity(&e),
            "n={n}: pure GF(2) parity must NOT refute — the parity chain alone is satisfiable"
        );
        assert!(
            !decide_pigeonhole_unsat(&e),
            "n={n}: pure matching must NOT refute — the exactly-one cardinality alone is satisfiable"
        );
        assert_eq!(
            fused_parity_cardinality_decide(cnf.num_vars, &cnf.clauses),
            Some(false),
            "n={n}: the fused parity+cardinality route refutes it"
        );
        assert_eq!(
            prove_unsat(&e),
            UnsatOutcome::Refuted,
            "n={n}: the general cascade refutes it (through the fused collapse cut)"
        );
    }
}

#[test]
fn the_fused_cut_never_over_fires_dropping_the_parity_closure_is_sat() {
    // Drop the `s_{n-1} = 0` unit (the last clause of `parity_exactly_one`) → the coupling vanishes and
    // the formula becomes SATISFIABLE (exactly-one holds, the free prefix-XOR chain constrains nothing;
    // e.g. one selector true, `s_{n-1} = 1`). Neither the general cascade nor the fused decider may
    // report UNSAT — a guard that the cut is not over-firing on the cardinality substructure alone.
    for n in [4usize, 6, 10] {
        let (cnf, _) = parity_exactly_one(n);
        let mut clauses = cnf.clauses.clone();
        let dropped = clauses.pop().expect("the parity-closure unit is the last clause");
        assert_eq!(dropped.len(), 1, "sanity: the dropped clause is the s_(n-1)=0 unit");
        let e = clauses_to_expr(&clauses).unwrap();
        if let UnsatOutcome::Refuted = prove_unsat(&e) {
            panic!("n={n}: without the parity closure the formula is SAT — the cascade must not over-fire");
        }
        assert_ne!(
            fused_parity_cardinality_decide(cnf.num_vars, &clauses),
            Some(false),
            "n={n}: the fused decider must not claim UNSAT on the satisfiable residue"
        );
    }
}

#[test]
fn fused_decider_scales_to_large_n() {
    // The clause-based fused decider operates directly on the flat clause list. It stays exact (and
    // fast) to large n — the dedicated engine the benchmark drives at the wall sizes, exactly as PHP
    // drives its SR prover and Tseitin its GF(2) solver.
    for n in [80usize, 120, 200] {
        let (cnf, verdict) = parity_exactly_one(n);
        assert_eq!(verdict, ExpectedVerdict::Unsat, "n={n}: UNSAT by construction");
        assert_eq!(
            fused_parity_cardinality_decide(cnf.num_vars, &cnf.clauses),
            Some(false),
            "n={n}: the fused parity+cardinality decider refutes at scale, no expr recursion"
        );
    }
}
