//! Regression pin for Bug Report #1, BUG-023.
//!
//! The Kripke lowering of "cannot" (alethic impossibility, force 0) must negate
//! the complement in accessible worlds — ∀w'(Accessible → ¬P) — not assert a
//! possibility ∃w'(Accessible ∧ P), which is the logical opposite. This path
//! feeds Z3 verification, so the inversion is a soundness bug.

use logicaffeine_language::compile_kripke;

#[test]
fn kripke_cannot_lowers_to_impossibility_not_possibility() {
    let out = compile_kripke("Some birds cannot fly.").unwrap();

    // The Kripke formatter renders operators as words (ForAll / Implies / Not),
    // not Unicode symbols.
    assert!(
        out.contains("Accessible_Alethic"),
        "Cannot should lower over alethic accessible worlds. Got: {}",
        out
    );
    // Impossibility = ForAll w'(Accessible -> Not operand). The buggy possibility
    // lowering is Exists w'(Accessible And operand) — no ForAll, no Not.
    assert!(
        out.contains("ForAll"),
        "Cannot should lower to a universal-over-accessible-worlds impossibility \
         (ForAll w'(Accessible Implies Not operand)), not an existential possibility. Got: {}",
        out
    );
    assert!(
        out.contains("Not"),
        "Cannot (force 0 = impossibility) must negate the complement in accessible \
         worlds; lowering instead asserted the complement is possible. Got: {}",
        out
    );
}
