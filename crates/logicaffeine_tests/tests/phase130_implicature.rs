//! Phase 130 — §8.7 Conversational (scalar) implicature (MISSING_ENGLISH.md).
//!
//! A weak scalar item ("some") implicates the negation of its stronger Horn
//! alternative ("all") — defeasibly, via an `exh` operator:
//!   "Some students passed." → ∃x(Student(x)∧Pass(x))  +>  ¬∀x(Student(x)→Pass(x))
//! The implicature is a SEPARATE, cancellable line: the literal truth-conditional
//! `compile` output is unchanged (so `compile` stays ∃), and `compile_pragmatic`
//! adds the implicature. The strong term "all" carries no such implicature.

use logicaffeine_language::{compile, compile_pragmatic};

#[test]
fn literal_meaning_is_unchanged() {
    // The truth-conditional output keeps the bare existential (implicature is a
    // separate, non-truth-conditional dimension).
    let out = compile("Some students passed.").unwrap();
    eprintln!("literal: {out}");
    assert!(out.contains('∃'), "literal existential: {out}");
    assert!(!out.contains("+>") && !out.contains("Implicature"), "no implicature in literal output: {out}");
}

#[test]
fn pragmatic_some_implicates_not_all() {
    let out = compile_pragmatic("Some students passed.").unwrap();
    eprintln!("pragmatic: {out}");
    assert!(out.contains('∃'), "literal existential preserved: {out}");
    assert!(out.contains("Pass"), "the predicate: {out}");
    assert!(out.contains("+>") || out.contains("Implicature"), "carries a scalar implicature: {out}");
    assert!(out.contains('∀'), "the negated 'all' alternative: {out}");
    assert!(out.contains('¬'), "the alternative is negated: {out}");
}

#[test]
fn pragmatic_most_implicates_not_all() {
    // The scalar mechanism is the Horn scale ⟨some, most, all⟩, not the single word
    // "some": "most" implicates "not all" just as "some" does.
    let out = compile_pragmatic("Most students passed.").unwrap();
    eprintln!("pragmatic most: {out}");
    assert!(out.contains("MOST"), "literal proportional preserved: {out}");
    assert!(out.contains("+>") || out.contains("Implicature"), "carries a scalar implicature: {out}");
    assert!(out.contains('∀') && out.contains('¬'), "the negated 'all' alternative: {out}");
}

#[test]
fn pragmatic_all_has_no_scalar_implicature() {
    // The strong term is not strengthened further, even pragmatically.
    let out = compile_pragmatic("All students passed.").unwrap();
    eprintln!("all: {out}");
    assert!(out.contains('∀'), "universal: {out}");
    assert!(!out.contains("+>") && !out.contains("Implicature"), "no implicature on the strong term: {out}");
}

// ============================================================================
// Entailment spec (§8.7): the implicature is defeasible — derivable in the
// non-monotonic layer, invisible to the classical one, cancellable.
// ============================================================================
#[cfg(feature = "verification")]
mod verification_spec {
    use logicaffeine_compile::{
        check_theorem_defeasible, check_theorem_defeasible_consistent, check_theorem_smt,
    };
    use logicaffeine_proof::oracle::{SmtConsistency, SmtVerdict};

    fn theorem(premises: &[&str], goal: &str) -> String {
        let givens: String = premises.iter().map(|p| format!("Given: {p}\n")).collect();
        format!("## Theorem: Phase130V\n{givens}Prove: {goal}\nProof: Auto.\n")
    }

    #[test]
    fn implicature_invisible_to_classical_entailment() {
        let src = theorem(&["Some students passed."], "Not every student passed.");
        assert_eq!(
            check_theorem_smt(&src).expect("must parse"),
            SmtVerdict::NotEntailed,
            "the literal meaning of 'some' is compatible with 'all'"
        );
    }

    #[test]
    fn implicature_derivable_defeasibly() {
        let src = theorem(&["Some students passed."], "Not every student passed.");
        assert_eq!(
            check_theorem_defeasible(&src).expect("must parse"),
            SmtVerdict::Entailed,
            "defeasibly: some ⊨~ not-all"
        );
    }

    #[test]
    fn implicature_cancellation_is_consistent() {
        let src = theorem(
            &["Some students passed.", "Every student passed."],
            "Some students passed.",
        );
        assert_eq!(
            check_theorem_defeasible_consistent(&src).expect("must parse"),
            SmtConsistency::Consistent,
            "'some — in fact all' must cancel, not contradict"
        );
    }
}
