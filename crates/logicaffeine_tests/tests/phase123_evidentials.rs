//! Phase 123 — §4.3 Evidentials / perspectival predicates (MISSING_ENGLISH.md).
//!
//! Raising verbs seem/appear/look mark an evidence source; the complement is NOT
//! asserted:
//!   "John seems happy." → Seem(⟨Happy(john)⟩)   — does NOT entail Happy(john).
//! Previously treated as ordinary verbs (the Raising feature was unused).

use logicaffeine_language::compile;

#[test]
fn seem_wraps_complement_without_asserting() {
    let out = compile("John seems happy.").unwrap();
    eprintln!("seem: {out}");
    assert!(out.contains("Seem"), "the evidential predicate: {out}");
    assert!(out.contains("Happy") || out.contains("happy"), "the complement state: {out}");
    // The complement is embedded under Seem (evidential), NOT a bare assertion
    // "Happy(John)" sitting at top level.
    assert!(
        !out.trim().starts_with("Happy(") && !out.contains("∧ Happy(John)"),
        "complement must be embedded under the evidential, not asserted: {out}"
    );
}

#[test]
fn appear_is_evidential() {
    let out = compile("Mary appears tired.").unwrap();
    eprintln!("appear: {out}");
    assert!(out.contains("Appear"), "the evidential predicate: {out}");
    assert!(out.contains("Tired") || out.contains("tired"), "the complement: {out}");
}

#[test]
fn plain_copula_still_asserts() {
    // Regression: an ordinary copular predication DOES assert its predicate.
    let out = compile("John is happy.").unwrap();
    eprintln!("plain: {out}");
    assert!(out.contains("Happy"), "plain copula asserts the predicate: {out}");
    assert!(!out.contains("Seem"), "no spurious evidential: {out}");
}

// ============================================================================
// Entailment spec (§4.3): evidentials are non-factive modals — the complement
// is never entailed, but the report is consistent with either outcome.
// ============================================================================
#[cfg(feature = "verification")]
mod verification_spec {
    use logicaffeine_compile::{check_theorem_premises_consistent, check_theorem_smt};
    use logicaffeine_proof::oracle::{SmtConsistency, SmtVerdict};

    fn theorem(premises: &[&str], goal: &str) -> String {
        let givens: String = premises.iter().map(|p| format!("Given: {p}\n")).collect();
        format!("## Theorem: Phase123V\n{givens}Prove: {goal}\nProof: Auto.\n")
    }

    #[test]
    fn seems_does_not_entail_is() {
        let src = theorem(&["John seems happy."], "John is happy.");
        assert_eq!(
            check_theorem_premises_consistent(&src).expect("must parse"),
            SmtConsistency::Consistent,
            "the evidential premise is consistent"
        );
        assert_eq!(
            check_theorem_smt(&src).expect("must parse"),
            SmtVerdict::NotEntailed,
            "Seem(⟨Happy(j)⟩) must NOT entail Happy(j)"
        );
    }

    #[test]
    fn seems_is_consistent_with_the_opposite() {
        let src = theorem(
            &["John seems happy.", "John is not happy."],
            "John seems happy.",
        );
        assert_eq!(
            check_theorem_premises_consistent(&src).expect("must parse"),
            SmtConsistency::Consistent,
            "appearances may deceive: Seem(P) ∧ ¬P is satisfiable"
        );
    }

    #[test]
    fn appears_does_not_entail_is() {
        let src = theorem(&["John appears tired."], "John is tired.");
        assert_eq!(
            check_theorem_smt(&src).expect("must parse"),
            SmtVerdict::NotEntailed,
            "'appear' is the same non-factive evidential as 'seem'"
        );
    }

    #[test]
    fn evidential_is_self_entailing() {
        let src = theorem(&["John seems happy."], "John seems happy.");
        assert_eq!(
            check_theorem_smt(&src).expect("must parse"),
            SmtVerdict::Entailed,
            "identity through the evidential encoding"
        );
    }
}

// ============================================================================
// IR spec: the evidential is a MODAL in the proof IR (flavor Evidential), so
// the frame machinery — not predicate opacity — carries the non-factivity.
// ============================================================================
#[cfg(feature = "verification")]
mod ir_spec {
    use logicaffeine_compile::compile_for_proof;
    use logicaffeine_proof::ProofExpr;

    #[test]
    fn evidential_lowers_to_modal_with_evidential_flavor() {
        let result = compile_for_proof("John seems happy.");
        let expr = result
            .proof_expr
            .expect("evidential must convert to a proof expression");
        fn find_modal(e: &ProofExpr) -> Option<(&str, &str)> {
            match e {
                ProofExpr::Modal { domain, flavor, .. } => Some((domain, flavor)),
                ProofExpr::And(l, r) | ProofExpr::Or(l, r) | ProofExpr::Implies(l, r) => {
                    find_modal(l).or_else(|| find_modal(r))
                }
                ProofExpr::Not(i) => find_modal(i),
                ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => find_modal(body),
                _ => None,
            }
        }
        let (domain, flavor) =
            find_modal(&expr).unwrap_or_else(|| panic!("no Modal in the IR: {expr}"));
        assert_eq!(domain, "Alethic", "evidential domain");
        assert_eq!(flavor, "Evidential", "evidential flavor");
    }
}
