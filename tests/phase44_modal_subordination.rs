//! Phase 44: Modal Subordination
//!
//! Tests for cross-sentence modal continuation and pronoun resolution
//! in hypothetical worlds.
//!
//! Key principle: Modal subordination allows a second modal (like "would")
//! to continue the hypothetical world introduced by a first modal (like "might").
//!
//! Example: "A wolf might walk in. It would eat you."
//! - Sentence 1: Introduces "wolf" in a possible world via "might"
//! - Sentence 2: "It" resolves to "wolf", "would" continues the same world
//!
//! CRITICAL: The pronoun must bind to the variable introduced in the modal,
//! not be treated as deictic (proper noun "X").

use logos::compile_discourse;

// ============================================
// BASIC MODAL SUBORDINATION (Should SUCCEED)
// ============================================

/// Classic modal subordination: might → would
/// "A wolf might walk in. It would eat you."
/// "It" must resolve to the wolf variable, not be treated as deictic "X".
#[test]
fn test_might_would_subordination() {
    let result = compile_discourse(&[
        "A wolf might walk in.",
        "It would eat you."
    ]);
    eprintln!("DEBUG test_might_would_subordination: {:?}", result);

    assert!(result.is_ok(),
        "Modal subordination should allow pronoun resolution: {:?}", result);

    let output = result.unwrap();

    // CRITICAL: "It" must resolve to the wolf variable (x), not deictic "X"
    // The output should NOT contain "X" as a separate entity
    assert!(!output.contains("Agent(e2, X)"),
        "Should resolve 'It' to wolf variable, not deictic X: {}", output);

    // The wolf variable should appear in the second event
    // Looking for pattern like "Agent(e2, x)" where x is the wolf variable
    assert!(output.contains("Agent(e") && output.matches("Wolf").count() >= 1,
        "Wolf variable should be agent of eating: {}", output);
}

/// Modal subordination with "may" → "would"
/// "A thief may enter. He would steal everything."
#[test]
fn test_may_would_subordination() {
    let result = compile_discourse(&[
        "A thief may enter.",
        "He would steal everything."
    ]);
    eprintln!("DEBUG test_may_would_subordination: {:?}", result);

    assert!(result.is_ok(),
        "may → would should subordinate: {:?}", result);

    let output = result.unwrap();
    // "He" should bind to thief, not be deictic
    assert!(!output.contains("Agent(e2, He)") && !output.contains("Agent(e2, X)"),
        "Should resolve 'He' to thief variable, not deictic: {}", output);
}

/// Modal subordination with "could" → "would"
/// "A fire could start. It would spread quickly."
#[test]
fn test_could_would_subordination() {
    let result = compile_discourse(&[
        "A fire could start.",
        "It would spread quickly."
    ]);
    eprintln!("DEBUG test_could_would_subordination: {:?}", result);

    assert!(result.is_ok(),
        "could → would should subordinate: {:?}", result);

    let output = result.unwrap();
    // "It" should bind to fire, not be deictic
    assert!(!output.contains("Agent(e2, X)"),
        "Should resolve 'It' to fire variable, not deictic: {}", output);
}

// ============================================
// MULTI-SENTENCE CHAINS
// ============================================

/// Chain of modal subordination: might → would → would
/// "A wolf might walk in. It would growl. It would attack."
#[test]
fn test_chain_subordination() {
    let result = compile_discourse(&[
        "A wolf might walk in.",
        "It would growl.",
        "It would attack."
    ]);
    eprintln!("DEBUG test_chain_subordination: {:?}", result);

    assert!(result.is_ok(),
        "Chain subordination should work: {:?}", result);

    let output = result.unwrap();
    // All three sentences should be connected in the same modal context
    assert!(output.contains("∧"), "Should have conjunction: {}", output);

    // "It" in sentences 2 and 3 should bind to wolf, not deictic X
    assert!(!output.contains("Agent(e2, X)") && !output.contains("Agent(e3, X)"),
        "Pronouns should resolve to wolf, not deictic: {}", output);
}

/// Multiple referents in modal context
/// "A wolf might chase a rabbit. It would catch it."
#[test]
fn test_multiple_referents_in_modal() {
    let result = compile_discourse(&[
        "A wolf might chase a rabbit.",
        "It would catch it."
    ]);
    eprintln!("DEBUG test_multiple_referents_in_modal: {:?}", result);

    assert!(result.is_ok(),
        "Multiple referents in modal should work: {:?}", result);

    let output = result.unwrap();
    // Both pronouns should bind to referents, not deictic
    assert!(!output.contains("Agent(e2, X)") && !output.contains("Theme(e2, X)"),
        "Pronouns should resolve to wolf/rabbit, not deictic: {}", output);
}

// ============================================
// MODAL SCOPE BREAKS
// ============================================

/// Different modal base does not subordinate
/// "A wolf might walk in. A bear must appear."
/// These are separate modal contexts (possibility vs necessity)
#[test]
fn test_different_modal_no_subordination() {
    let result = compile_discourse(&[
        "A wolf might walk in.",
        "A bear must appear."
    ]);
    eprintln!("DEBUG test_different_modal_no_subordination: {:?}", result);

    // These are separate modal contexts - both should succeed independently
    assert!(result.is_ok());
}

/// "would" without prior modal context should still parse
/// (but uses counterfactual/hypothetical reading)
#[test]
fn test_would_standalone() {
    let result = compile_discourse(&[
        "A wolf would eat you."
    ]);
    eprintln!("DEBUG test_would_standalone: {:?}", result);

    assert!(result.is_ok());
}

// ============================================
// MIXED MODAL DOMAINS
// ============================================

/// Deontic subordination: should → would
/// "A user should register. He would have access."
#[test]
fn test_deontic_subordination() {
    let result = compile_discourse(&[
        "A user should register.",
        "He would have access."
    ]);
    eprintln!("DEBUG test_deontic_subordination: {:?}", result);

    assert!(result.is_ok(),
        "Deontic subordination should work: {:?}", result);

    let output = result.unwrap();
    // "He" should bind to user, not deictic
    assert!(!output.contains("Agent(e2, He)") && !output.contains("Agent(e2, X)"),
        "Should resolve 'He' to user variable, not deictic: {}", output);
}

// ============================================
// NEGATION INSIDE MODAL
// ============================================

/// Negation inside modal is complex
/// "A wolf might not walk in. It would eat you."
/// The wolf exists in a world where it doesn't walk in.
/// This is an edge case - document behavior.
#[test]
fn test_negation_inside_modal() {
    let result = compile_discourse(&[
        "A wolf might not walk in.",
        "It would eat you."
    ]);
    eprintln!("DEBUG test_negation_inside_modal: {:?}", result);

    // Document current behavior - may need design decision
}
