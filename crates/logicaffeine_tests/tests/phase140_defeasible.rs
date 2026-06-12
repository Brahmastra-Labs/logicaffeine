#![cfg(feature = "verification")]
//! ============================================================================
//! PHASE 140: P4 DEFEASIBLE REASONING — THE NON-MONOTONIC SPEC
//! ============================================================================
//!
//! Generics (§6.1), habituals (§4.4), and implicatures (§8.7) license
//! CANCELLABLE inferences. The strict/monotonic export keeps `Generic → ∀`,
//! but the defeasible layer (circumscription with per-rule abnormality
//! predicates) must:
//!
//!   1. derive the default      — Birds fly + Tweety is a bird ⊨~ Tweety flies
//!   2. tolerate the exception  — the penguin premise set stays CONSISTENT
//!   3. defeat, not explode     — Opus the penguin does NOT fly, and that is
//!                                a defeated default, not a contradiction
//!   4. keep strict facts       — Opus is still a bird (subsumption is hard)
//!   5. cancel implicatures     — "some" +> ¬all is derivable AND cancellable
//!
//! Doors: `check_theorem_defeasible` / `check_theorem_defeasible_consistent`
//! (non-monotonic) vs `check_theorem_smt` (classical control). Verdicts are
//! never kernel-certified.

use logicaffeine_compile::{
    check_theorem_defeasible, check_theorem_defeasible_consistent, check_theorem_smt,
};
use logicaffeine_proof::oracle::{SmtConsistency, SmtVerdict};

fn theorem(premises: &[&str], goal: &str) -> String {
    let givens: String = premises
        .iter()
        .map(|p| format!("Given: {p}\n"))
        .collect();
    format!("## Theorem: Phase140\n{givens}Prove: {goal}\nProof: Auto.\n")
}

const TWEETY: &[&str] = &["Birds fly.", "Tweety is a bird."];

const PENGUIN: &[&str] = &[
    "Birds fly.",
    "Penguins are birds.",
    "Penguins do not fly.",
    "Opus is a penguin.",
];

// ============================================================================
// A. The default fires for the unexceptional instance
// ============================================================================

#[test]
fn tweety_flies_by_default() {
    let src = theorem(TWEETY, "Tweety flies.");
    assert_eq!(
        check_theorem_defeasible(&src).expect("generic premises must parse"),
        SmtVerdict::Entailed,
        "Birds fly + Tweety is a bird ⊨~ Tweety flies"
    );
}

#[test]
fn tweety_premises_consistent() {
    let src = theorem(TWEETY, "Tweety flies.");
    assert_eq!(
        check_theorem_defeasible_consistent(&src).expect("must parse"),
        SmtConsistency::Consistent,
        "the unexceptional generic theory is consistent"
    );
}

#[test]
fn classical_penguin_theory_is_inconsistent() {
    // Control: the STRICT export deliberately keeps `Generic → ∀` (the
    // monotonic approximation the kernel can use), which is exactly why the
    // penguin theory explodes classically — and why the defeasible layer
    // exists. The classical door must report the contradiction the
    // defeasible door avoids.
    use logicaffeine_compile::check_theorem_premises_consistent;
    let src = theorem(PENGUIN, "Opus flies.");
    assert_eq!(
        check_theorem_premises_consistent(&src).expect("must parse"),
        SmtConsistency::Inconsistent,
        "classically (GEN as ∀), birds-fly + flightless penguins contradict"
    );
}

// ============================================================================
// B. The exception defeats the default WITHOUT contradiction
// ============================================================================

#[test]
fn penguin_theory_is_consistent() {
    // THE load-bearing test: under Generic→∀ this theory is classically
    // inconsistent; under circumscription it must be satisfiable.
    let src = theorem(PENGUIN, "Opus flies.");
    assert_eq!(
        check_theorem_defeasible_consistent(&src).expect("must parse"),
        SmtConsistency::Consistent,
        "Birds fly + Penguins are flightless birds + Opus must be CONSISTENT"
    );
}

#[test]
fn opus_does_not_fly() {
    let src = theorem(PENGUIN, "Opus flies.");
    assert_eq!(
        check_theorem_defeasible(&src).expect("must parse"),
        SmtVerdict::NotEntailed,
        "the more specific rule (penguins don't fly) defeats the default"
    );
}

#[test]
fn opus_does_not_fly_is_derivable() {
    let src = theorem(PENGUIN, "Opus does not fly.");
    assert_eq!(
        check_theorem_defeasible(&src).expect("must parse"),
        SmtVerdict::Entailed,
        "the specific rule fires: ⊨~ ¬Fly(Opus)"
    );
}

#[test]
fn opus_is_still_a_bird() {
    // Strict subsumption must survive the defeasible machinery.
    let src = theorem(PENGUIN, "Opus is a bird.");
    assert_eq!(
        check_theorem_defeasible(&src).expect("must parse"),
        SmtVerdict::Entailed,
        "hard taxonomy is untouched: Opus is a bird"
    );
}

#[test]
fn tweety_unaffected_by_penguin_rule() {
    // Adding the penguin rules must not defeat the default for a
    // non-penguin: per-rule abnormality predicates, no cross-talk.
    let mut premises = PENGUIN.to_vec();
    premises.push("Tweety is a bird.");
    let src = theorem(&premises, "Tweety flies.");
    assert_eq!(
        check_theorem_defeasible(&src).expect("must parse"),
        SmtVerdict::Entailed,
        "Tweety still flies: the penguin exception is scoped to penguins"
    );
}

// ============================================================================
// C. Scalar implicature: derivable, base-preserving, cancellable (§8.7)
// ============================================================================

#[test]
fn some_implicates_not_all() {
    let src = theorem(&["Some students passed."], "Not every student passed.");
    assert_eq!(
        check_theorem_defeasible(&src).expect("must parse"),
        SmtVerdict::Entailed,
        "defeasibly: some ⊨~ ¬all"
    );
}

#[test]
fn some_does_not_classically_entail_not_all() {
    // Control: the implicature lives ONLY in the defeasible layer; the
    // literal base meaning is compatible with "all".
    let src = theorem(&["Some students passed."], "Not every student passed.");
    assert_eq!(
        check_theorem_smt(&src).expect("must parse"),
        SmtVerdict::NotEntailed,
        "classically: some ⊬ ¬all (the base meaning is preserved)"
    );
}

#[test]
fn implicature_is_cancellable() {
    // "Some students passed — in fact, all of them did." Cancellation must
    // be CONSISTENT (an Ab flip), not a contradiction.
    let premises = &["Some students passed.", "Every student passed."];
    let src = theorem(premises, "Not every student passed.");
    assert_eq!(
        check_theorem_defeasible_consistent(&src).expect("must parse"),
        SmtConsistency::Consistent,
        "cancelling the implicature must not be a contradiction"
    );
    assert_eq!(
        check_theorem_defeasible(&src).expect("must parse"),
        SmtVerdict::NotEntailed,
        "after cancellation the implicature no longer follows"
    );
}

#[test]
fn base_assertion_survives_cancellation() {
    let premises = &["Some students passed.", "Every student passed."];
    let src = theorem(premises, "Some students passed.");
    assert_eq!(
        check_theorem_defeasible(&src).expect("must parse"),
        SmtVerdict::Entailed,
        "the literal assertion is hard content and survives"
    );
}
