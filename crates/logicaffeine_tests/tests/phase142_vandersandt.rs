#![cfg(feature = "verification")]
//! ============================================================================
//! PHASE 142: P8 VAN DER SANDT — PRESUPPOSITION AS ANAPHORA
//! ============================================================================
//!
//! A presupposition BINDS to an accessible antecedent if one exists, else
//! ACCOMMODATES at the highest accessible box; a definite with no consistent
//! accommodation site is a PRESUPPOSITION FAILURE, not a silent success.
//! After the projection pass, surviving presuppositions are real premises
//! (proof-side: `Presupposition{a, p} → a ∧ p`), so plain kernel entailment
//! must see them:
//!
//!   - projection through ¬     : "doesn't regret lying" ⊢ "lied"
//!   - projection through modal : "might regret lying"   ⊢ "lied"
//!   - filtering under if       : "If John has children, his children…"
//!                                does NOT presuppose John has children
//!   - failure                  : "France has no king. The king of France…"
//!                                must flag, not verify

use logicaffeine_compile::verify_theorem;

fn theorem(premises: &[&str], goal: &str) -> String {
    let givens: String = premises
        .iter()
        .map(|p| format!("Given: {p}\n"))
        .collect();
    format!("## Theorem: Phase142\n{givens}Prove: {goal}\nProof: Auto.\n")
}

// ============================================================================
// A. Projection through negation (the classic)
// ============================================================================

#[test]
fn negated_factive_presupposition_projects_and_proves() {
    let src = theorem(&["Mary does not regret lying."], "Mary lied.");
    assert!(
        verify_theorem(&src).is_ok(),
        "the presupposition of 'regret' must project through ¬ and be \
         derivable as a premise: {:?}",
        verify_theorem(&src).err()
    );
}

#[test]
fn negated_assertion_does_not_prove_the_assertion() {
    // The ¬ stays on the assertion: she does NOT regret it.
    let src = theorem(&["Mary does not regret lying."], "Mary regrets lying.");
    assert!(
        verify_theorem(&src).is_err(),
        "¬Regret must not prove Regret"
    );
}

#[test]
fn positive_factive_presupposition_also_proves() {
    let src = theorem(&["Mary regrets lying."], "Mary lied.");
    assert!(
        verify_theorem(&src).is_ok(),
        "the unembedded factive also yields its presupposition: {:?}",
        verify_theorem(&src).err()
    );
}

#[test]
fn aspectual_stop_presupposes_prior_state() {
    let src = theorem(&["John stopped smoking."], "John smoked.");
    assert!(
        verify_theorem(&src).is_ok(),
        "'stop V-ing' presupposes the prior habit: {:?}",
        verify_theorem(&src).err()
    );
}

#[test]
fn negated_stop_still_presupposes_prior_state() {
    let src = theorem(&["John did not stop smoking."], "John smoked.");
    assert!(
        verify_theorem(&src).is_ok(),
        "the 'stop' presupposition projects through negation: {:?}",
        verify_theorem(&src).err()
    );
}

// ============================================================================
// B. Projection out of a modal (global accommodation)
// ============================================================================

#[test]
fn presupposition_projects_out_of_might() {
    let src = theorem(&["Mary might regret lying."], "Mary lied.");
    assert!(
        verify_theorem(&src).is_ok(),
        "an unbound presupposition under a modal accommodates globally: {:?}",
        verify_theorem(&src).err()
    );
}

#[test]
fn modal_assertion_itself_stays_modal() {
    let src = theorem(&["Mary might regret lying."], "Mary regrets lying.");
    assert!(
        verify_theorem(&src).is_err(),
        "◇Regret must not prove Regret"
    );
}

// ============================================================================
// C. Filtering: binding in the antecedent kills projection
// ============================================================================

#[test]
fn conditional_filters_bound_presupposition() {
    // Van der Sandt binding: "his children" binds to "John has children" in
    // the antecedent ⇒ nothing projects.
    let src = theorem(
        &["If John has children, his children are happy."],
        "John has children.",
    );
    assert!(
        verify_theorem(&src).is_err(),
        "a presupposition bound inside the if-clause must NOT project"
    );
}

#[test]
fn unconditional_possessive_does_presuppose() {
    // Control for the filtering test: outside the conditional, the
    // possessive's existence presupposition projects normally.
    let src = theorem(&["His children are happy."], "He has children.");
    assert!(
        verify_theorem(&src).is_ok(),
        "an unbound possessive presupposes existence: {:?}",
        verify_theorem(&src).err()
    );
}

// ============================================================================
// D. Presupposition failure is flagged, never silently verified
// ============================================================================

#[test]
fn inconsistent_accommodation_is_a_failure_not_a_proof() {
    // "France has no king" makes the definite's accommodation inconsistent.
    // Whatever the failure surface (parse-time flag or verification error),
    // the goal must NOT verify.
    let src = theorem(
        &["France has no king.", "The king of France is bald."],
        "France has a king.",
    );
    assert!(
        verify_theorem(&src).is_err(),
        "accommodating 'the king of France' against 'France has no king' \
         must fail, not prove a king into existence"
    );
}
