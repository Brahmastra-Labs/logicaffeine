//! The certified cascade must invoke the universal algebraic cut — bounded-degree Nullstellensatz over
//! GF(2) — for the low-degree-algebraic cores that match no structural recognizer. NS subsumes the
//! narrow cuts (parity is its degree-1 fragment); at degree ≥ 2 it certifies the "rigid" residue the
//! census mapped, which nonetheless has a small algebraic certificate.

use logicaffeine_proof::cdcl::Lit;
use logicaffeine_proof::hypercube::clauses_to_expr;
use logicaffeine_proof::polycalc::nullstellensatz_refutes;
use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
use logicaffeine_proof::{lyapunov, pigeonhole, pseudo_boolean, xorsat};

#[test]
fn cascade_certifies_low_degree_algebraic_core_via_nullstellensatz() {
    // {x∨y, x∨¬y, ¬x∨z, ¬x∨¬z}: resolves to x ∧ ¬x. Not a clean pigeonhole / XOR bundle / counting
    // shape — a degree-2 Nullstellensatz refutation, not a structural one.
    let p = |v: u32| Lit::new(v, true);
    let q = |v: u32| Lit::new(v, false);
    let clauses = vec![vec![p(0), p(1)], vec![p(0), q(1)], vec![q(0), p(2)], vec![q(0), q(2)]];
    let e = clauses_to_expr(&clauses).unwrap();

    // The structural recognizers all miss it...
    assert!(!pigeonhole::decide_pigeonhole_unsat(&e), "pigeonhole does not fire");
    assert!(!pseudo_boolean::refute_clausal(&e), "cutting-planes does not fire");
    assert!(!xorsat::refute_via_parity(&e), "parity does not fire");
    assert!(matches!(lyapunov::auto_collapse(3, &clauses), lyapunov::AutoCollapse::None), "collapse does not fire");
    // ...but a low-degree Nullstellensatz certificate exists...
    assert!(nullstellensatz_refutes(3, &clauses, 2), "a degree-2 NS refutation exists");
    // ...and the cascade now certifies it.
    assert_eq!(prove_unsat(&e), UnsatOutcome::Refuted);
}

#[test]
fn nullstellensatz_cut_stays_sound_on_satisfiable() {
    // A satisfiable formula of the same shape must NOT be refuted.
    let p = |v: u32| Lit::new(v, true);
    let q = |v: u32| Lit::new(v, false);
    // {x∨y, x∨¬y, ¬x∨z}: forces x=true, then z free — SATISFIABLE.
    let clauses = vec![vec![p(0), p(1)], vec![p(0), q(1)], vec![q(0), p(2)]];
    let e = clauses_to_expr(&clauses).unwrap();
    assert!(
        matches!(prove_unsat(&e), UnsatOutcome::Sat(_)),
        "satisfiable — the NS cut must never falsely refute"
    );
}
