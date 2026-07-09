//! Phase 120 — §8.3 Clefts & pseudo-clefts (work/MISSING_ENGLISH.md).
//!
//! An it-cleft focuses a constituent and adds EXHAUSTIVITY + an existence
//! presupposition:
//!   "It was John who broke the vase."
//!     → Break(john, vase) ∧ ∃!x Break(x, vase) ∧ exhaustive(john)
//!   i.e. John broke the vase, and he is the ONLY one who did.

use logicaffeine_language::compile;

#[test]
fn it_cleft_adds_exhaustivity() {
    let out = compile("It was John who broke the vase.").unwrap();
    eprintln!("cleft: {out}");
    // The core predication survives.
    assert!(out.contains("Break") || out.contains("Broke"), "the cleft predicate: {out}");
    assert!(out.contains("John") || out.contains('J'), "the focused constituent: {out}");
    // Exhaustivity: no one other than John broke the vase (uniqueness / ∀x→x=john).
    assert!(
        out.contains("∃!") || out.contains("exhaustive") || out.contains("Exhaustive")
            || (out.contains('∀') && out.contains('=')),
        "cleft contributes exhaustivity (only John): {out}"
    );
}

#[test]
fn it_cleft_distinct_from_plain_predication() {
    let cleft = compile("It was John who broke the vase.").unwrap();
    let plain = compile("John broke the vase.").unwrap();
    eprintln!("cleft: {cleft}\nplain: {plain}");
    // The cleft is richer than the plain sentence (adds exhaustivity), so they differ.
    assert!(cleft != plain, "cleft ≠ plain predication: {cleft} vs {plain}");
}

// ============================================================================
// Entailment spec (§8.3): the cleft's exhaustivity is Z3-checkable — anyone
// else who broke the vase IS John; and the prejacent is asserted.
// ============================================================================
#[cfg(feature = "verification")]
mod verification_spec {
    use logicaffeine_compile::{check_theorem_premises_consistent, check_theorem_smt};
    use logicaffeine_proof::oracle::{SmtConsistency, SmtVerdict};

    fn theorem(premises: &[&str], goal: &str) -> String {
        let givens: String = premises.iter().map(|p| format!("Given: {p}\n")).collect();
        format!("## Theorem: Phase120V\n{givens}Prove: {goal}\nProof: Auto.\n")
    }

    const CLEFT: &str = "It was John who broke the vase.";

    #[test]
    fn cleft_asserts_the_prejacent() {
        let src = theorem(&[CLEFT], "John broke the vase.");
        assert_eq!(
            check_theorem_smt(&src).expect("must parse"),
            SmtVerdict::Entailed,
            "the cleft asserts Break(john, vase)"
        );
    }

    #[test]
    fn cleft_exhaustivity_forces_identity() {
        let src = theorem(&[CLEFT, "Bill broke the vase."], "Bill is John.");
        assert_eq!(
            check_theorem_premises_consistent(&src).expect("must parse"),
            SmtConsistency::Consistent,
            "exhaustivity + a second breaker is consistent only via identity"
        );
        assert_eq!(
            check_theorem_smt(&src).expect("must parse"),
            SmtVerdict::Entailed,
            "exhaustivity: ∀z(Break(z, vase) → z = John) forces Bill = John"
        );
    }

    #[test]
    fn plain_sentence_has_no_exhaustivity() {
        // Control: without the cleft, no exhaustivity inference.
        let src = theorem(
            &["John broke the vase.", "Bill broke the vase."],
            "Bill is John.",
        );
        assert_eq!(
            check_theorem_smt(&src).expect("must parse"),
            SmtVerdict::NotEntailed,
            "non-cleft predication must not be exhaustive"
        );
    }
}
