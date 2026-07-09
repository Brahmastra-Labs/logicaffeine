//! Phase 125 — §1.2 Optatives (work/MISSING_ENGLISH.md).
//!
//! Wish clauses with no asserted truth:
//!   "May you prosper!"        → Wish(speaker, ⟨Prosper(you)⟩)
//!   "Long live the king!"
//! The complement is NOT entailed (a wish, not an assertion).

use logicaffeine_language::compile;

#[test]
fn may_fronting_is_optative_wish() {
    let out = compile("May you prosper!").unwrap();
    eprintln!("may: {out}");
    assert!(out.contains("Wish") || out.contains("Optative"), "optative wish operator: {out}");
    assert!(out.contains("Prosper") || out.contains("prosper"), "the wish content: {out}");
    // The wish is by the speaker (a wish, not a deontic 'may'-permission).
    assert!(out.contains("Speaker") || out.contains("speaker"), "the wisher is the speaker: {out}");
}

#[test]
fn long_live_is_optative() {
    let out = compile("Long live the king!").unwrap();
    eprintln!("long-live: {out}");
    assert!(out.contains("Wish") || out.contains("Optative"), "optative operator: {out}");
    assert!(out.contains("King") || out.contains("king") || out.contains("Live"), "the wish content: {out}");
}

#[test]
fn may_optative_distinct_from_plain_modal() {
    // The optative "May you prosper!" must not be the ordinary deontic/epistemic
    // modal reading (□/◇) of "may".
    let out = compile("May you prosper!").unwrap();
    eprintln!("distinct: {out}");
    assert!(out.contains("Wish") || out.contains("Optative"), "optative, not a plain modal: {out}");
}

// ============================================================================
// Entailment spec (§1.2): a wish quantifies over bouletically ideal worlds —
// the wished content is never entailed as fact.
// ============================================================================
#[cfg(feature = "verification")]
mod verification_spec {
    use logicaffeine_compile::{check_theorem_premises_consistent, check_theorem_smt};
    use logicaffeine_proof::oracle::{SmtConsistency, SmtVerdict};

    fn theorem(premises: &[&str], goal: &str) -> String {
        let givens: String = premises.iter().map(|p| format!("Given: {p}\n")).collect();
        format!("## Theorem: Phase125V\n{givens}Prove: {goal}\nProof: Auto.\n")
    }

    #[test]
    fn wish_complement_is_not_entailed() {
        let src = theorem(&["May you prosper!"], "You prosper.");
        assert_eq!(
            check_theorem_premises_consistent(&src).expect("must parse"),
            SmtConsistency::Consistent,
            "a wish is a consistent premise"
        );
        assert_eq!(
            check_theorem_smt(&src).expect("must parse"),
            SmtVerdict::NotEntailed,
            "Wish(speaker, ⟨Prosper(you)⟩) must NOT entail Prosper(you)"
        );
    }

    #[test]
    fn wish_is_self_entailing() {
        let src = theorem(&["May you prosper!"], "May you prosper!");
        assert_eq!(
            check_theorem_smt(&src).expect("must parse"),
            SmtVerdict::Entailed,
            "identity through the optative encoding"
        );
    }
}
