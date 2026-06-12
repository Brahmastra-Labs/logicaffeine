//! Phase 113 — §8.2 Presupposition projection (Van der Sandt) (MISSING_ENGLISH.md).
//!
//! A presupposition trigger under negation has its presupposition PROJECT (survive
//! outside the ¬), while the assertion is negated:
//!   "Mary doesn't regret lying." → ¬Regret(Mary) [Presup: P(Lie(Mary))]  — she still lied.
//!   "John did not stop smoking." → ¬¬Smoke(John) [Presup: P(Smoke(John))] — he still smokes.
//! Previously the trigger under do-support/aux negation was parsed as a plain verb,
//! dropping the presupposition entirely.

use logicaffeine_language::compile;

#[test]
fn positive_presupposition_present() {
    let out = compile("Mary regrets lying.").unwrap();
    eprintln!("pos: {out}");
    assert!(out.contains("Regret"), "the assertion: {out}");
    assert!(out.contains("Presup"), "the presupposition is attached: {out}");
    assert!(out.contains("Lie") || out.contains("Lying"), "presupposes the lying: {out}");
}

#[test]
fn regret_presupposition_projects_through_does_not() {
    let out = compile("Mary does not regret lying.").unwrap();
    eprintln!("does-not: {out}");
    assert!(out.contains("Presup"), "presupposition must survive negation (project): {out}");
    assert!(out.contains("Lie") || out.contains("Lying"), "the lying still presupposed: {out}");
    assert!(out.contains('¬'), "the assertion is negated: {out}");
}

#[test]
fn regret_presupposition_projects_through_did_not() {
    let out = compile("Mary did not regret lying.").unwrap();
    eprintln!("did-not: {out}");
    assert!(out.contains("Presup"), "presupposition projects through past negation: {out}");
    assert!(out.contains("Lie") || out.contains("Lying"), "the lying still presupposed: {out}");
}

#[test]
fn stop_presupposition_projects_through_negation() {
    let out = compile("John did not stop smoking.").unwrap();
    eprintln!("stop-neg: {out}");
    assert!(out.contains("Presup"), "the prior-smoking presupposition projects: {out}");
    assert!(out.contains("Smoke") || out.contains("Smoking"), "presupposes prior smoking: {out}");
}

#[test]
fn doesnt_contraction_projects() {
    let out = compile("Mary doesn't regret lying.").unwrap();
    eprintln!("doesnt: {out}");
    assert!(out.contains("Presup"), "contraction also projects: {out}");
    assert!(out.contains("Lie") || out.contains("Lying"), "the lying still presupposed: {out}");
}

// ============================================================================
// Entailment spec (§8.2): a projected presupposition is a real premise — the
// kernel must derive it; the negated assertion must not flip.
// ============================================================================
#[cfg(feature = "verification")]
mod verification_spec {
    use logicaffeine_compile::verify_theorem;

    fn theorem(premises: &[&str], goal: &str) -> String {
        let givens: String = premises.iter().map(|p| format!("Given: {p}\n")).collect();
        format!("## Theorem: Phase113V\n{givens}Prove: {goal}\nProof: Auto.\n")
    }

    #[test]
    fn projected_presupposition_is_derivable() {
        let src = theorem(&["Mary does not regret lying."], "Mary lied.");
        assert!(
            verify_theorem(&src).is_ok(),
            "the presupposition projects through ¬ and proves: {:?}",
            verify_theorem(&src).err()
        );
    }

    #[test]
    fn factive_presupposition_is_derivable_unembedded() {
        let src = theorem(&["Mary regrets lying."], "Mary lied.");
        assert!(
            verify_theorem(&src).is_ok(),
            "the unembedded factive presupposition proves: {:?}",
            verify_theorem(&src).err()
        );
    }

    #[test]
    fn negation_still_negates_the_assertion() {
        let src = theorem(&["Mary does not regret lying."], "Mary regrets lying.");
        assert!(
            verify_theorem(&src).is_err(),
            "projection must not erase the negation on the assertion"
        );
    }
}
