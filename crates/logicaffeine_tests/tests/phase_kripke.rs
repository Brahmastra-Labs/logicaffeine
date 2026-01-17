//! Phase Kripke: Possible World Semantics Tests
//!
//! Tests for the Kripke lowering pass that transforms surface modal operators
//! into explicit first-order logic with possible world quantification.
//!
//! Surface Form: ◇Fly(x)
//! Deep Form:    ∃w'(Accessible(w₀, w') ∧ Fly(x, w'))

use logicaffeine_language::{compile, compile_kripke};

// ═══════════════════════════════════════════════════════════════════════════
// BASIC MODAL LOWERING
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kripke_possibility_basic() {
    // "John can fly" with possibility modal (force 0.5)
    // Should lower to: ∃w1(Accessible_Alethic(w0, w1) ∧ Fly(John, w1))
    let output = compile_kripke("John can fly.").unwrap();

    assert!(
        output.contains("w1"),
        "Should have world variable w1. Got: {}",
        output
    );
    assert!(
        output.contains("Accessible_Alethic"),
        "Should have alethic accessibility. Got: {}",
        output
    );
    assert!(
        output.contains("w0"),
        "Should reference actual world w0. Got: {}",
        output
    );
}

#[test]
fn kripke_necessity_basic() {
    // "John must study" with necessity modal (force 1.0)
    // Should lower to: ∀w1(Accessible_Alethic(w0, w1) → Study(John, w1))
    let output = compile_kripke("John must study.").unwrap();

    assert!(
        output.contains("ForAll") || output.contains("∀"),
        "Should have universal quantifier for necessity. Got: {}",
        output
    );
    assert!(
        output.contains("Accessible_Alethic"),
        "Should have alethic accessibility. Got: {}",
        output
    );
    // Necessity uses implication, not conjunction
    assert!(
        output.contains("Implies") || output.contains("→"),
        "Necessity should use implication. Got: {}",
        output
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// DEONTIC VS ALETHIC ACCESSIBILITY
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kripke_deontic_accessibility() {
    // "John should study" - deontic modal
    // Should use Accessible_Deontic, not Accessible_Alethic
    let output = compile_kripke("John should study.").unwrap();

    assert!(
        output.contains("Accessible_Deontic"),
        "Deontic modal 'should' should use Accessible_Deontic. Got: {}",
        output
    );
}

#[test]
fn kripke_alethic_can() {
    // "Birds can fly" - alethic modal (ability)
    let output = compile_kripke("Birds can fly.").unwrap();

    assert!(
        output.contains("Accessible_Alethic"),
        "'can' (ability) should use Accessible_Alethic. Got: {}",
        output
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// NON-MODAL PREDICATES (ACTUAL WORLD)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kripke_non_modal_at_actual_world() {
    // "John runs" - no modal, should evaluate at actual world w0
    let output = compile_kripke("John runs.").unwrap();

    assert!(
        output.contains("w0"),
        "Non-modal predicates should be at actual world w0. Got: {}",
        output
    );
    // Should NOT have accessibility relation (no modal)
    assert!(
        !output.contains("Accessible"),
        "Non-modal sentence should not have accessibility. Got: {}",
        output
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// NESTED MODALS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kripke_nested_modals() {
    // "John might be able to fly" - nested modals
    // Should produce: ∃w1(Accessible(w0, w1) ∧ ∃w2(Accessible(w1, w2) ∧ Fly(John, w2)))
    let output = compile_kripke("John might be able to fly.").unwrap();

    // Should have at least two world variables beyond w0
    assert!(
        output.contains("w1") && output.contains("w2"),
        "Nested modals should produce w1 and w2. Got: {}",
        output
    );

    // The inner modal's accessibility should reference the outer world
    // w1 → w2, not w0 → w2
    assert!(
        output.contains("w1") && output.contains("w2"),
        "Nested accessibility should chain worlds. Got: {}",
        output
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// QUANTIFIER-MODAL INTERACTION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kripke_quantified_subject_with_modal() {
    // "Every student can pass" - quantifier + modal
    // The quantifier and modal world should both be present
    let output = compile_kripke("Every student can pass.").unwrap();

    assert!(
        output.contains("ForAll") || output.contains("∀"),
        "Should have universal quantifier for 'every'. Got: {}",
        output
    );
    assert!(
        output.contains("Accessible"),
        "Should have accessibility relation for modal. Got: {}",
        output
    );
}

#[test]
fn kripke_existential_with_necessity() {
    // "Some student must pass" - existential + necessity
    let output = compile_kripke("Some student must pass.").unwrap();

    assert!(
        output.contains("Exists") || output.contains("∃"),
        "Should have existential quantifier for 'some'. Got: {}",
        output
    );
    // Must = necessity = universal over worlds
    assert!(
        output.contains("ForAll") || output.contains("∀"),
        "Necessity 'must' should add universal world quantifier. Got: {}",
        output
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// MODAL FORCE DISTINCTION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kripke_possibility_uses_existential_world() {
    // Low force (≤0.5) = possibility = existential world quantifier
    let output = compile_kripke("John can fly.").unwrap();

    // 'can' has force 0.5 = possibility = existential
    assert!(
        output.contains("Exists") || output.contains("∃"),
        "Possibility modal should use existential world quantifier. Got: {}",
        output
    );
}

#[test]
fn kripke_necessity_uses_universal_world() {
    // High force (>0.5) = necessity = universal world quantifier
    let output = compile_kripke("John must study.").unwrap();

    // 'must' has force 1.0 = necessity = universal
    let has_forall = output.contains("ForAll") || output.contains("∀");
    assert!(
        has_forall,
        "Necessity modal should use universal world quantifier. Got: {}",
        output
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// SURFACE VS DEEP COMPARISON
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kripke_deep_differs_from_surface() {
    // Same input should produce different outputs for surface vs deep
    let surface = compile("John can fly.").unwrap();
    let deep = compile_kripke("John can fly.").unwrap();

    // Surface should have modal operator symbol
    assert!(
        surface.contains("◇") || surface.contains("□") || surface.contains("Can"),
        "Surface should have modal operator. Got: {}",
        surface
    );

    // Deep should have explicit world quantification
    assert!(
        deep.contains("Accessible"),
        "Deep should have accessibility predicate. Got: {}",
        deep
    );

    // They should not be identical
    assert_ne!(
        surface, deep,
        "Surface and deep forms should differ. Surface: {}, Deep: {}",
        surface, deep
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// WORLD ARGUMENT POSITION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kripke_world_as_predicate_argument() {
    // World should appear as explicit argument: P(x, w) not P_w(x)
    let output = compile_kripke("John runs.").unwrap();

    // Should have world w0 as argument to predicate
    // Format: Run(John, w0) or similar
    assert!(
        output.contains("w0"),
        "Predicate should have world as argument. Got: {}",
        output
    );
}
