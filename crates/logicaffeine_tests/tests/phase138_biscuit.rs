//! Phase 138 — §4.2 Biscuit / relevance conditionals (MISSING_ENGLISH.md).
//!
//! In a biscuit conditional the consequent is asserted UNCONDITIONALLY; the
//! if-clause restricts the relevance/speech act, not the truth of the consequent:
//!   "If you want tea, the kettle is hot."
//!     → Hot(kettle) ∧ Relevance(⟨Want(addressee, tea)⟩)
//! The kettle's heat does not depend on the addressee wanting tea. (Contrast a
//! hypothetical conditional "If it rains, the ground gets wet.")

use logicaffeine_language::compile;

#[test]
fn biscuit_asserts_consequent_with_relevance() {
    let out = compile("If you want tea, the kettle is hot.").unwrap();
    eprintln!("biscuit: {out}");
    assert!(out.contains("Hot") && out.contains("Kettle"), "the consequent is present: {out}");
    assert!(out.contains("Relevance"), "the if-clause is a relevance condition: {out}");
    // The consequent is asserted, not placed under a material/∀ conditional arrow.
    assert!(!out.contains('→'), "consequent is asserted unconditionally, not implied: {out}");
}

#[test]
fn hypothetical_conditional_unchanged() {
    // Regression: an ordinary hypothetical conditional keeps its implication form.
    let out = compile("If it rains, the ground gets wet.").unwrap();
    eprintln!("hypothetical: {out}");
    assert!(out.contains('→'), "a hypothetical conditional is an implication: {out}");
    assert!(!out.contains("Relevance"), "no relevance marker on a hypothetical: {out}");
}

// ============================================================================
// Entailment spec (§4.2): the biscuit consequent is asserted OUTRIGHT — it
// must be derivable with no antecedent premise; the antecedent never is.
// ============================================================================
#[cfg(feature = "verification")]
mod verification_spec {
    use logicaffeine_compile::verify_theorem;

    fn theorem(premises: &[&str], goal: &str) -> String {
        let givens: String = premises.iter().map(|p| format!("Given: {p}\n")).collect();
        format!("## Theorem: Phase138V\n{givens}Prove: {goal}\nProof: Auto.\n")
    }

    const BISCUIT: &str = "If you want tea, the kettle is hot.";

    #[test]
    fn biscuit_consequent_is_unconditionally_derivable() {
        let src = theorem(&[BISCUIT], "The kettle is hot.");
        assert!(
            verify_theorem(&src).is_ok(),
            "the biscuit consequent is asserted regardless of the antecedent: {:?}",
            verify_theorem(&src).err()
        );
    }

    #[test]
    fn biscuit_antecedent_is_not_derivable() {
        let src = theorem(&[BISCUIT], "You want tea.");
        assert!(
            verify_theorem(&src).is_err(),
            "the relevance antecedent is never asserted"
        );
    }

    #[test]
    fn hypothetical_conditional_consequent_is_not_unconditional() {
        // Control: a genuinely hypothetical conditional must NOT assert its
        // consequent outright.
        let src = theorem(&["If John studied, John passed."], "John passed.");
        assert!(
            verify_theorem(&src).is_err(),
            "a hypothetical conditional's consequent needs its antecedent"
        );
    }
}
