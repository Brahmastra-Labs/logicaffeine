//! Phase 131 — §2.3 Correlative coordination (work/MISSING_ENGLISH.md).
//!
//! Paired coordinators scope a shared predicate over two subjects:
//!   "Neither John nor Mary came." → ¬Came(john) ∧ ¬Came(mary)
//!   "Either John or Mary came."   → (Came(john) ∨ Came(mary)) ∧ ¬(Came(john) ∧ Came(mary))
//! (exclusive "either…or").

use logicaffeine_language::{compile, compile_pragmatic};

#[test]
fn neither_nor_negates_both() {
    let out = compile("Neither John nor Mary came.").unwrap();
    eprintln!("neither: {out}");
    assert!(out.contains("Come") || out.contains("Came"), "the shared predicate: {out}");
    assert!(out.contains("John") || out.contains('J'), "first subject: {out}");
    assert!(out.contains("Mary") || out.contains('M'), "second subject: {out}");
    // Both negated.
    assert!(out.matches('¬').count() >= 2 || out.contains("¬") && out.contains('∧'),
        "neither negates both conjuncts: {out}");
}

#[test]
fn either_or_is_inclusive_by_default() {
    // Default compile() keeps the plain inclusive disjunction (so the proof engine
    // sees a usable ∨ for disjunction elimination).
    let out = compile("Either John or Mary came.").unwrap();
    eprintln!("either-default: {out}");
    assert!(out.contains('∨'), "inclusive core disjunction: {out}");
    assert!(!out.contains('¬'), "no exclusivity in the literal reading: {out}");
}

#[test]
fn either_or_exclusive_is_pragmatic() {
    // The exclusivity implicature of "either…or" is a pragmatic enrichment.
    let out = compile_pragmatic("Either John or Mary came.").unwrap();
    eprintln!("either-pragmatic: {out}");
    assert!(out.contains('∨'), "inclusive core disjunction: {out}");
    assert!(out.contains('¬') && out.contains('∧'), "exclusive 'either…or' rules out both: {out}");
}

// ============================================================================
// Entailment spec (§2.3): neither…nor IS the De Morgan conjunction — each
// negated conjunct must be kernel-derivable.
// ============================================================================
#[cfg(feature = "verification")]
mod verification_spec {
    use logicaffeine_compile::verify_theorem;

    fn theorem(premises: &[&str], goal: &str) -> String {
        let givens: String = premises.iter().map(|p| format!("Given: {p}\n")).collect();
        format!("## Theorem: Phase131V\n{givens}Prove: {goal}\nProof: Auto.\n")
    }

    const NEITHER: &str = "Neither John nor Mary came.";

    #[test]
    fn neither_nor_derives_first_negation() {
        let src = theorem(&[NEITHER], "John did not come.");
        assert!(
            verify_theorem(&src).is_ok(),
            "neither…nor ⊢ ¬Came(John): {:?}",
            verify_theorem(&src).err()
        );
    }

    #[test]
    fn neither_nor_derives_second_negation() {
        let src = theorem(&[NEITHER], "Mary did not come.");
        assert!(
            verify_theorem(&src).is_ok(),
            "neither…nor ⊢ ¬Came(Mary): {:?}",
            verify_theorem(&src).err()
        );
    }

    #[test]
    fn neither_nor_does_not_prove_the_positive() {
        let src = theorem(&[NEITHER], "John came.");
        assert!(
            verify_theorem(&src).is_err(),
            "neither…nor must not prove that John came"
        );
    }
}
