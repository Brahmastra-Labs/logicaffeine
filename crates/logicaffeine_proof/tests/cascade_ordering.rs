//! The ordering-principle specialist. GT(n) asserts a strict total order (totality + antisymmetry +
//! transitivity) in which every element has a strictly greater one ("no maximum") — impossible, since a
//! finite strict total order always has a maximum. It is UNSAT, and a canonical family the general
//! cascade decides only by super-polynomial search (measured: GT(20) ≈ 2.7s / 68k conflicts, growing
//! ~10x every 4 steps). The specialist recognizes the COMPLETE GT(n) core and refutes it in polynomial
//! time from that structure.
//!
//! Soundness is the whole game here: the detector must be FAITHFUL — it may fire only when the clauses
//! genuinely contain a complete ordering-principle core (which is then unsatisfiable as a superset of a
//! theorem-UNSAT core), and must DECLINE on anything else, never a false refutation. These tests pin the
//! win AND every way the structure can be incomplete or absent.

use logicaffeine_proof::families::{self, ExpectedVerdict};
use logicaffeine_proof::hypercube::clauses_to_expr;
use logicaffeine_proof::ordering::{check_ordering_cert, ordering_certificate, refute_ordering};
use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};

#[test]
fn ordering_specialist_fires_on_complete_gt_n_and_scales() {
    // The specialist recognizes the complete GT(n) structure and refutes in polynomial time — instant
    // at sizes where the general cascade walls super-polynomially.
    for n in [4usize, 6, 10, 30, 50] {
        let (cnf, v) = families::ordering_principle(n);
        assert_eq!(v, ExpectedVerdict::Unsat);
        assert!(
            refute_ordering(cnf.num_vars, &cnf.clauses),
            "GT({n}) is a complete ordering principle — the specialist must refute it"
        );
    }
}

#[test]
fn ordering_specialist_declines_when_a_maximum_is_allowed() {
    // Drop ONE element's "no maximum" clause → that element may be the maximum → the formula becomes
    // SATISFIABLE. The specialist must DECLINE (the ordering core is incomplete), and the general cascade
    // must not refute it either — the critical soundness boundary.
    for n in [4usize, 6, 10] {
        let (cnf, _) = families::ordering_principle(n);
        // the no-maximum clauses are the last n clauses (one per element); drop the last.
        let mut clauses = cnf.clauses.clone();
        clauses.pop();
        assert!(
            !refute_ordering(cnf.num_vars, &clauses),
            "GT({n}) minus a no-maximum clause is SATISFIABLE — the specialist must decline"
        );
        let e = clauses_to_expr(&clauses).unwrap();
        if let UnsatOutcome::Refuted = prove_unsat(&e) {
            panic!("GT({n}) minus a no-maximum clause is satisfiable — the cascade must not refute it");
        }
    }
}

#[test]
fn ordering_specialist_declines_on_incomplete_transitivity() {
    // Remove a transitivity clause → not a COMPLETE ordering core → the specialist must DECLINE and bail
    // to the general engine (which stays correct), never fire on a partial structure.
    for n in [5usize, 7] {
        let (cnf, _) = families::ordering_principle(n);
        // totality + antisymmetry are the first 2*C(n,2) = n(n-1) clauses; the next is the first
        // transitivity clause — remove it.
        let mut clauses = cnf.clauses.clone();
        let first_transitivity = n * (n - 1);
        clauses.remove(first_transitivity);
        assert!(
            !refute_ordering(cnf.num_vars, &clauses),
            "GT({n}) with a missing transitivity clause is not a complete core — the specialist must decline"
        );
    }
}

#[test]
fn ordering_certificate_round_trips_and_re_checks() {
    // The recovered element/edge certificate must re-check INDEPENDENTLY against the raw clauses, and a
    // certificate from a different instance must be rejected — so the re-check is not vacuous.
    for n in [4usize, 6, 10] {
        let (cnf, _) = families::ordering_principle(n);
        let cert = ordering_certificate(cnf.num_vars, &cnf.clauses).expect("GT(n) yields a certificate");
        assert_eq!(cert.n, n, "the certificate must recover the element count");
        assert!(
            check_ordering_cert(&cert, &cnf.clauses),
            "the certificate must re-check against GT({n}) from scratch"
        );
        assert_eq!(cert.byte_len(), 8 + n * (n - 1) * 4, "byte length = element count + one var id per ordered pair");
        let (other, _) = families::ordering_principle(n + 1);
        assert!(
            !check_ordering_cert(&cert, &other.clauses),
            "a GT({n}) certificate must NOT re-check against GT({})'s clauses",
            n + 1
        );
    }
}

#[test]
fn prove_unsat_refutes_gt_n_via_the_ordering_cut() {
    // The full cascade must reach the ordering specialist (after the matching detector declines) and
    // refute GT(n) fast — the exact path the benchmark drives. At n=30 the general search would wall
    // (~minutes); if this returns quickly the ordering cut fired.
    for n in [10usize, 20, 30] {
        let (cnf, _) = families::ordering_principle(n);
        let e = clauses_to_expr(&cnf.clauses).expect("flat CNF clausifies");
        assert_eq!(
            prove_unsat(&e),
            UnsatOutcome::Refuted,
            "prove_unsat must refute GT({n}) via the ordering cut"
        );
    }
}

#[test]
fn ordering_specialist_declines_on_non_ordering_formulas() {
    // Pigeonhole and random 3-SAT are not ordering principles. The specialist must never claim them (a
    // false fire would be unsound on any satisfiable non-ordering formula). Pigeonhole is UNSAT (its own
    // route decides it); the random instance is a general control.
    for n in [3usize, 4, 5] {
        let (php, _) = families::php(n);
        assert!(
            !refute_ordering(php.num_vars, &php.clauses),
            "pigeonhole PHP({n}) is not an ordering principle — the specialist must decline"
        );
    }
    let r = families::random_3sat(40, 170, 0xABCDEF);
    assert!(
        !refute_ordering(r.num_vars, &r.clauses),
        "a random 3-SAT instance is not an ordering principle — the specialist must decline"
    );
}
