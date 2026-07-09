//! Phase 110 — §4.5 Counterfactual conditionals (work/MISSING_ENGLISH.md).
//!
//! Subjunctive conditionals over the closest worlds:
//!   "If John had studied, he would have passed." → Study(John) □→ Pass(John)
//!
//! The `Counterfactual` node + the `□→` glyph already exist; this phase fixes a
//! real bug — a spurious `∀John` universally quantifying the rigid proper-name
//! constant — and locks the counterfactual operator as DISTINCT from material
//! implication (`□→`, not `→`) in every export, per the Kratzer analysis where
//! `would` is a necessity modal whose ordering source is similarity (closest
//! worlds), so it is not the material conditional's truth table.

use logicaffeine_language::{compile, compile_kripke, compile_simple};

#[test]
fn counterfactual_uses_box_arrow_not_material() {
    let out = compile("If John had studied, he would have passed.").unwrap();
    eprintln!("cf: {out}");
    assert!(out.contains("□→"), "counterfactual uses the □→ operator: {out}");
    assert!(out.contains("Study"), "antecedent predicate: {out}");
    assert!(out.contains("Pass"), "consequent predicate: {out}");
}

#[test]
fn counterfactual_does_not_universally_bind_proper_name() {
    // Regression for the spurious-∀ bug: "John" is a rigid constant, not a
    // universally quantified variable.
    let out = compile("If John had studied, he would have passed.").unwrap();
    eprintln!("cf-no-forall: {out}");
    assert!(
        !out.contains("∀John") && !out.contains("∀ John"),
        "must not universally quantify the proper-name constant John: {out}"
    );
}

#[test]
fn counterfactual_distinct_from_indicative_material() {
    // An indicative material conditional uses → ; the counterfactual must not be
    // collapsed to it.
    let cf = compile("If John had studied, he would have passed.").unwrap();
    eprintln!("cf-distinct: {cf}");
    // The counterfactual connective is □→, which is strictly richer than → ;
    // assert the modal arrow is present (the marker of non-material semantics).
    assert!(cf.contains("□→"), "counterfactual is modal (□→), not bare material →: {cf}");
}

#[test]
fn counterfactual_renders_in_simple_and_kripke() {
    let simple = compile_simple("If John had studied, he would have passed.").unwrap();
    eprintln!("cf(simple): {simple}");
    assert!(simple.contains("Study") && simple.contains("Pass"), "SimpleFOL keeps both clauses: {simple}");

    let kripke = compile_kripke("If John had studied, he would have passed.").unwrap();
    eprintln!("cf(kripke): {kripke}");
    // Kripke export marks the counterfactual distinctly (not a plain Implies).
    assert!(
        kripke.contains("Counterfactual") || kripke.contains("□→") || kripke.contains("Closest"),
        "Kripke marks the counterfactual: {kripke}"
    );
    assert!(!kripke.contains("∀John") && !kripke.contains("ForAll John"), "no spurious ∀John: {kripke}");
}

#[test]
fn ordinary_definite_counterfactual_still_works() {
    // Regression: the existing definite-subject counterfactual is unaffected.
    let out = compile("If the glass had fallen, it would have broken.").unwrap();
    eprintln!("cf-glass: {out}");
    assert!(
        out.contains("□→") || out.contains("Fall") || out.contains("Break"),
        "definite counterfactual still parses: {out}"
    );
}

// ============================================================================
// Entailment spec (§4.5): □→ has closest-world semantics — strictly distinct
// from material implication in BOTH directions, and (v1, no weak centering)
// no counterfactual modus ponens from actual-world facts.
// ============================================================================
#[cfg(feature = "verification")]
mod verification_spec {
    use logicaffeine_compile::{check_theorem_premises_consistent, check_theorem_smt};
    use logicaffeine_proof::oracle::{SmtConsistency, SmtVerdict};

    fn theorem(premises: &[&str], goal: &str) -> String {
        let givens: String = premises.iter().map(|p| format!("Given: {p}\n")).collect();
        format!("## Theorem: Phase110V\n{givens}Prove: {goal}\nProof: Auto.\n")
    }

    const CF: &str = "If John had studied, he would have passed.";

    #[test]
    fn counterfactual_with_actual_antecedent_does_not_detach() {
        // No weak centering in v1: the actual world need not be among the
        // closest antecedent-worlds, so modus ponens fails (documented).
        let src = theorem(&[CF, "John studied."], "John passed.");
        assert_eq!(
            check_theorem_premises_consistent(&src).expect("must parse"),
            SmtConsistency::Consistent,
            "CF + actual antecedent is consistent"
        );
        assert_eq!(
            check_theorem_smt(&src).expect("must parse"),
            SmtVerdict::NotEntailed,
            "(P □→ Q) + P must NOT detach Q without weak centering"
        );
    }

    #[test]
    fn counterfactual_is_self_entailing() {
        let src = theorem(&[CF], CF);
        assert_eq!(
            check_theorem_smt(&src).expect("must parse"),
            SmtVerdict::Entailed,
            "identity: the counterfactual entails itself"
        );
    }

    #[test]
    fn negated_consequent_fact_is_consistent_with_counterfactual() {
        // "If John had studied he would have passed — and he didn't pass" is
        // the canonical USE of a counterfactual; it must be satisfiable.
        let src = theorem(&[CF, "John did not pass."], CF);
        assert_eq!(
            check_theorem_premises_consistent(&src).expect("must parse"),
            SmtConsistency::Consistent,
            "CF ∧ ¬Q must be consistent (unlike material implication + ¬Q + P)"
        );
    }
}

#[test]
fn negated_antecedent_and_consequent_parse() {
    use logicaffeine_language::compile;
    let out = compile("If John had not studied, he would have failed.").unwrap();
    eprintln!("neg-CF: {out}");
    assert!(out.contains("□→"), "still a counterfactual: {out}");
    assert!(out.contains('¬'), "the antecedent negation survives: {out}");
    let out2 = compile("If John had studied, he would not have failed.").unwrap();
    assert!(out2.contains('¬'), "the consequent negation survives: {out2}");
}
