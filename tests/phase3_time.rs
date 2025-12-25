//! Phase 3: Architecture of Time (Reichenbachian Temporal Logic)
//!
//! These tests verify the implementation of narrative progression where
//! sequential sentences in discourse are connected by temporal ordering.
//!
//! TDD Approach: These tests are written FIRST and should FAIL until
//! the implementation is complete.

use logos::compile_discourse;

// ═══════════════════════════════════════════════════════════════════
// NARRATIVE PROGRESSION TESTS
// ═══════════════════════════════════════════════════════════════════

#[test]
fn narrative_progression_two_sentences() {
    let result = compile_discourse(&["John entered.", "He sat."]).unwrap();

    eprintln!("DEBUG: Two sentences output: {}", result);

    assert!(
        result.contains("e1") && result.contains("e2"),
        "Should have unique event variables e1 and e2: got '{}'",
        result
    );

    assert!(
        result.contains("Precedes(e1, e2)"),
        "Should have temporal ordering Precedes(e1, e2): got '{}'",
        result
    );
}

#[test]
fn narrative_progression_three_sentences() {
    let result = compile_discourse(&["John entered.", "He sat.", "He read."]).unwrap();

    eprintln!("DEBUG: Three sentences output: {}", result);

    assert!(
        result.contains("Precedes(e1, e2)") && result.contains("Precedes(e2, e3)"),
        "Three sentences should chain with Precedes(e1, e2) and Precedes(e2, e3): got '{}'",
        result
    );
}

#[test]
fn single_sentence_no_precedes() {
    let result = compile_discourse(&["John ran."]).unwrap();

    eprintln!("DEBUG: Single sentence output: {}", result);

    assert!(
        !result.contains("Precedes"),
        "Single sentence should have no Precedes constraint: got '{}'",
        result
    );
}

#[test]
fn narrative_preserves_pronoun_resolution() {
    let result = compile_discourse(&["John ran.", "He stopped."]).unwrap();

    eprintln!("DEBUG: Pronoun resolution output: {}", result);

    assert!(
        result.contains("e1") && result.contains("e2"),
        "Should have unique event variables: got '{}'",
        result
    );

    assert!(
        result.contains("Precedes(e1, e2)"),
        "Should have temporal ordering: got '{}'",
        result
    );

    assert!(
        result.contains("Agent(e1,") && result.contains("Agent(e2,"),
        "Both events should have agents: got '{}'",
        result
    );
}

// ═══════════════════════════════════════════════════════════════════
// EVENT VARIABLE UNIQUENESS TESTS
// ═══════════════════════════════════════════════════════════════════

#[test]
fn event_variables_unique_across_discourse() {
    let result = compile_discourse(&["Mary walked.", "She ran."]).unwrap();

    eprintln!("DEBUG: Unique events output: {}", result);

    let e1_count = result.matches("e1").count();
    let e2_count = result.matches("e2").count();

    assert!(
        e1_count >= 2 && e2_count >= 2,
        "Each event var should appear multiple times (quantifier + roles): e1={}, e2={} in '{}'",
        e1_count, e2_count, result
    );
}

#[test]
fn four_sentence_chain() {
    let result = compile_discourse(&[
        "John arrived.",
        "He sat.",
        "He ate.",
        "He left."
    ]).unwrap();

    eprintln!("DEBUG: Four sentences output: {}", result);

    assert!(result.contains("e1"), "Should have e1: got '{}'", result);
    assert!(result.contains("e2"), "Should have e2: got '{}'", result);
    assert!(result.contains("e3"), "Should have e3: got '{}'", result);
    assert!(result.contains("e4"), "Should have e4: got '{}'", result);

    assert!(
        result.contains("Precedes(e1, e2)"),
        "Should have Precedes(e1, e2): got '{}'",
        result
    );
    assert!(
        result.contains("Precedes(e2, e3)"),
        "Should have Precedes(e2, e3): got '{}'",
        result
    );
    assert!(
        result.contains("Precedes(e3, e4)"),
        "Should have Precedes(e3, e4): got '{}'",
        result
    );
}
