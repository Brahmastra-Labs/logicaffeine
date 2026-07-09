//! The certified cascade on the modular counting principle `Count_q(n)`: can the n-element set be
//! exactly partitioned into blocks of size q? It is UNSAT exactly when `q ∤ n` — a mod-q counting
//! obstruction that resolution cannot make at low width, but a GF(q) linear argument decides. These
//! tests pin the refutation on the UNSAT side and — critically — guard the SAT side (`q | n`, an exact
//! partition exists) against a false refutation.
//!
//! Sizes are kept small on purpose: the encoding has `C(n, q)` variables and `O(C(n,q)^2)` overlap
//! clauses, and the general `prove_unsat` clausifies into a nested expr whose recursion depth grows
//! with the clause count (see the fused family's boundary test).

use logicaffeine_proof::families::{mod_counting, ExpectedVerdict};
use logicaffeine_proof::hypercube::clauses_to_expr;
use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};

#[test]
fn prove_unsat_refutes_count_q_when_q_does_not_divide_n() {
    // q ∤ n → no exact q-partition exists → UNSAT, decided by the modular counting cut (q=2 is the
    // matching case, q=3/5 the genuine mod-p obstruction the GF(2) cut is blind to).
    for (n, q) in [(4usize, 3usize), (7, 3), (8, 3), (3, 2), (7, 2), (9, 2), (7, 5)] {
        assert_ne!(n % q, 0, "test setup: q must not divide n for the UNSAT case");
        let (cnf, verdict) = mod_counting(n, q);
        assert_eq!(verdict, ExpectedVerdict::Unsat, "Count_{q}({n}) is UNSAT when q∤n");
        let e = clauses_to_expr(&cnf.clauses).expect("flat CNF clausifies");
        assert_eq!(
            prove_unsat(&e),
            UnsatOutcome::Refuted,
            "Count_{q}({n}): the mod-{q} counting obstruction must be refuted"
        );
    }
}

#[test]
fn count_q_stays_sound_when_q_divides_n() {
    // q | n → an exact q-partition DOES exist → SATISFIABLE. prove_unsat must NEVER report Refuted
    // (Sat or Unsupported are both fine — neither claims the false UNSAT).
    for (n, q) in [(3usize, 3usize), (6, 3), (2, 2), (6, 2), (8, 2), (5, 5)] {
        assert_eq!(n % q, 0, "test setup: q must divide n for the SAT case");
        let (cnf, verdict) = mod_counting(n, q);
        assert_eq!(verdict, ExpectedVerdict::Sat, "Count_{q}({n}) is SAT when q|n");
        let e = clauses_to_expr(&cnf.clauses).unwrap();
        if let UnsatOutcome::Refuted = prove_unsat(&e) {
            panic!("Count_{q}({n}) is satisfiable (q|n) — the cut must not falsely refute it");
        }
    }
}
