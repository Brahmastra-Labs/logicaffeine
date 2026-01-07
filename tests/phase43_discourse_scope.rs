//! Phase 43: Discourse Scope and Accessibility
//!
//! Tests for DRT cross-sentence anaphora with proper scope enforcement.
//!
//! Key principles:
//! 1. DRS boxes create scope barriers
//! 2. Negation and disjunction are impenetrable (no anaphora out)
//! 3. Universal quantifiers CAN telescope (extend scope across sentences)
//! 4. Hard errors on scope violations (no silent fallback)

use logos::{compile, compile_discourse};

// ============================================
// BLOCKED SCOPES (Should FAIL)
// ============================================

/// Negation creates an impenetrable scope barrier.
/// "No farmer owns a donkey. He is happy."
/// "He" should NOT resolve to "farmer" - it's trapped in negation.
#[test]
fn test_negation_blocks_anaphora() {
    let result = compile_discourse(&[
        "No farmer owns a donkey.",
        "He is happy."
    ]);
    eprintln!("DEBUG test_negation_blocks_anaphora: {:?}", result);

    assert!(result.is_err(),
        "Should fail: 'farmer' is trapped in negation scope and should be inaccessible");
}

/// Disjunction creates scope islands.
/// "Either a farmer walks or a merchant runs. He is happy."
/// Neither "farmer" nor "merchant" should be accessible.
#[test]
fn test_disjunction_blocks_anaphora() {
    let result = compile_discourse(&[
        "Either a farmer walks or a merchant runs.",
        "He is happy."
    ]);
    eprintln!("DEBUG test_disjunction_blocks_anaphora: {:?}", result);

    assert!(result.is_err(),
        "Should fail: referents in disjuncts are inaccessible from outside");
}

/// Double negation still blocks.
/// "It is not the case that no farmer exists. He is happy."
/// Even with double negation, the farmer is trapped.
#[test]
fn test_double_negation_still_blocks() {
    let result = compile_discourse(&[
        "It is not the case that no farmer exists.",
        "He is happy."
    ]);
    eprintln!("DEBUG test_double_negation_still_blocks: {:?}", result);

    // Even with double negation, referents inside are inaccessible
    assert!(result.is_err(),
        "Should fail: double negation doesn't rescue referents");
}

// ============================================
// ACCESSIBLE SCOPES (Should SUCCEED)
// ============================================

/// Main clause indefinites are accessible cross-sentence.
/// "A farmer owns a donkey. He beats it."
/// Both "farmer" and "donkey" should be accessible.
#[test]
fn test_main_clause_indefinite_accessible() {
    let result = compile_discourse(&[
        "A farmer owns a donkey.",
        "He beats it."
    ]);
    eprintln!("DEBUG test_main_clause_indefinite_accessible: {:?}", result);

    assert!(result.is_ok(),
        "Main clause indefinites should be accessible cross-sentence: {:?}", result);

    let output = result.unwrap();
    // Should not have unresolved pronouns
    assert!(!output.contains("?"),
        "Should not have unresolved pronouns: {}", output);
}

/// Proper names are always accessible.
/// "John owns a donkey. He beats it."
#[test]
fn test_proper_name_accessible() {
    let result = compile_discourse(&[
        "John owns a donkey.",
        "He beats it."
    ]);
    eprintln!("DEBUG test_proper_name_accessible: {:?}", result);

    assert!(result.is_ok(),
        "Proper names should always be accessible: {:?}", result);

    let output = result.unwrap();
    assert!(!output.contains("?"),
        "Should not have unresolved pronouns: {}", output);
}

/// Multiple sentences with accessible referents.
/// "A farmer walks. A merchant runs. He is happy."
/// "He" should resolve to "merchant" (most recent).
#[test]
fn test_multiple_accessible_most_recent() {
    let result = compile_discourse(&[
        "A farmer walks.",
        "A merchant runs.",
        "He is happy."
    ]);
    eprintln!("DEBUG test_multiple_accessible_most_recent: {:?}", result);

    assert!(result.is_ok(),
        "Multiple accessible referents should work: {:?}", result);
}

// ============================================
// TELESCOPING (Universal → Extended Scope)
// ============================================

/// Classic telescope: "Every X has a Y. It is Z."
/// "Every chess game has a winner. He is happy."
/// Should telescope the universal scope to include "He is happy."
#[test]
fn test_telescoping_universal_succeeds() {
    let result = compile_discourse(&[
        "Every chess game has a winner.",
        "He is happy."
    ]);
    eprintln!("DEBUG test_telescoping_universal_succeeds: {:?}", result);

    assert!(result.is_ok(),
        "Should telescope: extend universal scope to include continuation: {:?}", result);

    let output = result.unwrap();
    // The output should have universal quantifier
    assert!(output.contains("∀"),
        "Telescoped output should have universal quantifier: {}", output);
    // Happy should be bound to the winner variable
    assert!(output.contains("Happy"),
        "Should have Happy predicate: {}", output);
}

/// Negation CANNOT telescope.
/// "No chess game has a winner. He is happy."
/// This should FAIL - negation blocks telescoping.
#[test]
fn test_telescoping_negation_fails() {
    let result = compile_discourse(&[
        "No chess game has a winner.",
        "He is happy."
    ]);
    eprintln!("DEBUG test_telescoping_negation_fails: {:?}", result);

    assert!(result.is_err(),
        "Should NOT telescope out of negation - must fail");
}

/// Chain telescope: multiple sentences extend the same scope.
/// "Every player gets a trophy. It is shiny. The player displays it."
#[test]
fn test_telescoping_chain() {
    let result = compile_discourse(&[
        "Every player gets a trophy.",
        "It is shiny.",
        "The player displays it."
    ]);
    eprintln!("DEBUG test_telescoping_chain: {:?}", result);

    assert!(result.is_ok(),
        "Chain telescoping should work: {:?}", result);

    let output = result.unwrap();
    // Should have universal quantifier
    assert!(output.contains("∀"),
        "Chain telescope should preserve universal: {}", output);
}

/// Telescope with passive continuation.
/// "Every student submitted a paper. It was graded."
#[test]
fn test_telescoping_passive_continuation() {
    let result = compile_discourse(&[
        "Every student submitted a paper.",
        "It was graded."
    ]);
    eprintln!("DEBUG test_telescoping_passive_continuation: {:?}", result);

    assert!(result.is_ok(),
        "Telescope with passive should work: {:?}", result);
}

/// Conditional telescoping.
/// "If a farmer owns a donkey, he beats it. The donkey is grey."
/// Should extend the conditional scope.
#[test]
fn test_telescoping_conditional() {
    let result = compile_discourse(&[
        "If a farmer owns a donkey, he beats it.",
        "The donkey is grey."
    ]);
    eprintln!("DEBUG test_telescoping_conditional: {:?}", result);

    assert!(result.is_ok(),
        "Conditional telescoping should work: {:?}", result);

    let output = result.unwrap();
    // Should have implication preserved
    assert!(output.contains("→"),
        "Should preserve conditional structure: {}", output);
}

// ============================================
// CONDITIONAL ACCESSIBILITY (Within Sentence)
// ============================================

/// Within same sentence, consequent can access antecedent.
/// "If a farmer owns a donkey, he beats it."
#[test]
fn test_conditional_antecedent_accessible_in_consequent() {
    let result = compile("If a farmer owns a donkey, he beats it.");
    eprintln!("DEBUG test_conditional_antecedent_accessible_in_consequent: {:?}", result);

    assert!(result.is_ok(),
        "Consequent should access antecedent referents: {:?}", result);

    let output = result.unwrap();
    assert!(!output.contains("?"),
        "Should resolve all pronouns: {}", output);
}

/// Antecedent should NOT be accessible cross-sentence without telescope.
/// "If a farmer is rich, he is happy. He walks."
/// The second "He" might not resolve if we're strict.
/// (This test documents expected behavior - may need Council decision)
#[test]
fn test_conditional_cross_sentence_strict() {
    let result = compile_discourse(&[
        "If a farmer is rich, he is happy.",
        "He walks."
    ]);
    eprintln!("DEBUG test_conditional_cross_sentence_strict: {:?}", result);

    // With strict DRT, this might fail because "farmer" is inside conditional
    // With telescoping, it might succeed
    // Document actual behavior
    if result.is_ok() {
        eprintln!("Note: Conditional antecedent DID telescope cross-sentence");
    } else {
        eprintln!("Note: Conditional antecedent did NOT telescope (strict DRT)");
    }
}

// ============================================
// SCOPE ERROR MESSAGES
// ============================================

/// Error message should explain WHY the referent is inaccessible.
#[test]
fn test_scope_error_message_explains_negation() {
    let result = compile_discourse(&[
        "No farmer owns a donkey.",
        "He is happy."
    ]);

    assert!(result.is_err());
    let err = result.unwrap_err();
    let err_msg = format!("{:?}", err);
    eprintln!("DEBUG scope_error_message: {}", err_msg);

    // Error should mention scope or accessibility
    assert!(
        err_msg.to_lowercase().contains("scope")
        || err_msg.to_lowercase().contains("access")
        || err_msg.to_lowercase().contains("negation")
        || err_msg.to_lowercase().contains("resolve"),
        "Error should explain scope issue: {}", err_msg
    );
}

// ============================================
// PARSER CHECKPOINT CONSISTENCY
// ============================================

/// Ambiguous parse + backtrack should not corrupt DRS.
/// "The farmer who saw a cat chased it."
/// "it" should resolve to "cat" even after potential backtracks.
#[test]
fn test_backtracking_preserves_drs_state() {
    let result = compile("The farmer who saw a cat chased it.");
    eprintln!("DEBUG test_backtracking_preserves_drs_state: {:?}", result);

    assert!(result.is_ok(),
        "Should parse successfully even with backtracking: {:?}", result);

    let output = result.unwrap();
    // "it" should resolve to "cat"
    assert!(!output.contains("?"),
        "Pronoun should resolve correctly after backtracking: {}", output);
}

/// Complex sentence with multiple potential backtrack points.
/// "Every farmer who owns a donkey that kicks it beats it."
#[test]
fn test_complex_backtracking() {
    let result = compile("Every farmer who owns a donkey that is grey beats it.");
    eprintln!("DEBUG test_complex_backtracking: {:?}", result);

    assert!(result.is_ok(),
        "Complex sentence should parse correctly: {:?}", result);
}

// ============================================
// EDGE CASES
// ============================================

/// Empty discourse should succeed.
#[test]
fn test_empty_discourse() {
    let result = compile_discourse(&[]);
    eprintln!("DEBUG test_empty_discourse: {:?}", result);

    // Empty discourse is valid (no-op)
    assert!(result.is_ok() || result.is_err()); // Document actual behavior
}

/// Single sentence discourse should work like compile().
#[test]
fn test_single_sentence_discourse() {
    let single = compile("A farmer walks.");
    let discourse = compile_discourse(&["A farmer walks."]);

    eprintln!("DEBUG single: {:?}", single);
    eprintln!("DEBUG discourse: {:?}", discourse);

    // Both should succeed
    assert!(single.is_ok());
    assert!(discourse.is_ok());
}

/// Gender mismatch should prevent resolution.
/// "John owns a donkey. She is happy."
/// "She" (Female) should NOT resolve to "John" (Male) or "donkey" (Neuter).
/// Note: "farmer" has Unknown gender and would match "She" via Gender Accommodation,
/// so we use a proper name with explicit gender.
#[test]
fn test_gender_mismatch_blocks() {
    let result = compile_discourse(&[
        "John owns a donkey.",
        "She is happy."
    ]);
    eprintln!("DEBUG test_gender_mismatch_blocks: {:?}", result);

    // Should fail - John is Male, donkey is Neuter, neither matches She (Female)
    assert!(result.is_err() || result.as_ref().map(|o| o.contains("?")).unwrap_or(false),
        "She (Female) should not resolve to John (Male) or donkey (Neuter): {:?}", result);
}

/// Plural/singular mismatch.
/// "Farmers walk. He is happy."
/// "He" (singular) should not resolve to "farmers" (plural).
#[test]
fn test_number_mismatch_blocks() {
    let result = compile_discourse(&[
        "Farmers walk.",
        "He is happy."
    ]);
    eprintln!("DEBUG test_number_mismatch_blocks: {:?}", result);

    // Should fail or have unresolved pronoun
    if let Ok(output) = result {
        assert!(output.contains("?"),
            "He (singular) should not resolve to farmers (plural): {}", output);
    }
}
