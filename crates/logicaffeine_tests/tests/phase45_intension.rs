//! Phase 45: Intensionality & Opacity - Temperature Paradox
//!
//! Tests for intensional predicates like "rising", "changing", "increasing".
//! These predicates take intensions as arguments, NOT current values.
//!
//! The Temperature Paradox:
//! - "The temperature is ninety." (extensional identity)
//! - "The temperature is rising." (intensional predicate)
//! Standard FOL would allow substituting 90 for temperature, yielding Rising(90) - absurd.
//! Solution: "rising" takes ^Temperature (intension), not the value.

use logicaffeine_language::compile;
use logicaffeine_language::lexer::Lexer;

// ═══════════════════════════════════════════════════════════════════
// TEMPERATURE PARADOX TESTS
// ═══════════════════════════════════════════════════════════════════

#[test]
fn temperature_is_rising_uses_intension() {
    let output = compile("The temperature is rising.").unwrap();
    // Should use ^Temperature (intension), NOT just Temperature
    assert!(
        output.contains("^Temperature") || output.contains("^T"),
        "Should predicate over intension ^Temperature, not extension. Got: {}",
        output
    );
    assert!(
        output.contains("Rise") || output.contains("Rising"),
        "Should have Rising predicate. Got: {}",
        output
    );
}

#[test]
fn price_is_changing_uses_intension() {
    let output = compile("The price is changing.").unwrap();
    assert!(
        output.contains("^Price") || output.contains("^P"),
        "Should predicate over intension ^Price. Got: {}",
        output
    );
}

#[test]
fn speed_is_increasing_uses_intension() {
    let output = compile("The speed is increasing.").unwrap();
    assert!(
        output.contains("^Speed") || output.contains("^S"),
        "Should predicate over intension ^Speed. Got: {}",
        output
    );
}

#[test]
fn value_is_decreasing_uses_intension() {
    let output = compile("The value is decreasing.").unwrap();
    assert!(
        output.contains("^Value") || output.contains("^V"),
        "Should predicate over intension ^Value. Got: {}",
        output
    );
}

// ═══════════════════════════════════════════════════════════════════
// NON-INTENSIONAL PREDICATES (CONTROL)
// ═══════════════════════════════════════════════════════════════════

#[test]
fn dog_is_running_is_extensional() {
    let output = compile("The dog is running.").unwrap();
    // "running" as activity (not property change) is extensional
    assert!(
        !output.contains("^Dog"),
        "Activity 'running' should be extensional. Got: {}",
        output
    );
}

#[test]
fn man_is_sleeping_is_extensional() {
    let output = compile("The man is sleeping.").unwrap();
    // "sleeping" is a distributive activity, not intensional
    assert!(
        !output.contains("^Man"),
        "Activity 'sleeping' should be extensional. Got: {}",
        output
    );
}

// ═══════════════════════════════════════════════════════════════════
// INTENSIONAL PREDICATE LEXER TESTS
// ═══════════════════════════════════════════════════════════════════

#[test]
fn rising_is_intensional_predicate() {
    assert!(
        Lexer::is_intensional_predicate("rising"),
        "rising should be intensional predicate"
    );
    assert!(
        Lexer::is_intensional_predicate("rise"),
        "rise should be intensional predicate"
    );
    assert!(
        Lexer::is_intensional_predicate("rises"),
        "rises should be intensional predicate"
    );
}

#[test]
fn changing_is_intensional_predicate() {
    assert!(
        Lexer::is_intensional_predicate("changing"),
        "changing should be intensional predicate"
    );
    assert!(
        Lexer::is_intensional_predicate("change"),
        "change should be intensional predicate"
    );
}

#[test]
fn increasing_is_intensional_predicate() {
    assert!(
        Lexer::is_intensional_predicate("increasing"),
        "increasing should be intensional predicate"
    );
    assert!(
        Lexer::is_intensional_predicate("increase"),
        "increase should be intensional predicate"
    );
}

#[test]
fn decreasing_is_intensional_predicate() {
    assert!(
        Lexer::is_intensional_predicate("decreasing"),
        "decreasing should be intensional predicate"
    );
    assert!(
        Lexer::is_intensional_predicate("decrease"),
        "decrease should be intensional predicate"
    );
}

#[test]
fn running_is_not_intensional_predicate() {
    assert!(
        !Lexer::is_intensional_predicate("running"),
        "running should NOT be intensional predicate"
    );
    assert!(
        !Lexer::is_intensional_predicate("run"),
        "run should NOT be intensional predicate"
    );
}

#[test]
fn sleeping_is_not_intensional_predicate() {
    assert!(
        !Lexer::is_intensional_predicate("sleeping"),
        "sleeping should NOT be intensional predicate"
    );
    assert!(
        !Lexer::is_intensional_predicate("sleep"),
        "sleep should NOT be intensional predicate"
    );
}

// ═══════════════════════════════════════════════════════════════════
// TRIPARTITE CLASSIFICATION (OPAQUE vs INTENSIONAL vs EXTENSIONAL)
// ═══════════════════════════════════════════════════════════════════

#[test]
fn opaque_and_intensional_are_distinct() {
    // Opaque verbs: de re/de dicto ambiguity on NP complement
    assert!(Lexer::is_opaque_verb("seek"), "seek is opaque");
    assert!(!Lexer::is_intensional_predicate("seek"), "seek is NOT intensional predicate");

    // Intensional predicates: inherently require intension as subject
    assert!(Lexer::is_intensional_predicate("rise"), "rise is intensional predicate");
    assert!(!Lexer::is_opaque_verb("rise"), "rise is NOT opaque");
}
