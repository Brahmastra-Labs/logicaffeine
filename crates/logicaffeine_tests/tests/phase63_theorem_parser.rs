// =============================================================================
// PHASE 63: THE THEOREM INTERFACE - TEST SUITE
// =============================================================================
// TDD RED: These tests define the specification for theorem parsing.
// Users write theorems in LOGOS syntax → Proof Engine verifies them.
//
// The Interface:
// ```logos
// ## Theorem: Name
// Given: Premise 1.
// Given: Premise 2.
// Prove: Goal.
// Proof: Auto.
// ```

use logicaffeine_language::compile_theorem;

// =============================================================================
// BASIC THEOREM PARSING
// =============================================================================

#[test]
fn test_parse_and_prove_socrates() {
    // The classic syllogism:
    // All men are mortal.
    // Socrates is a man.
    // ∴ Socrates is mortal.
    //
    // Phase 64: Noun canonicalization ensures "men" → "Man" so predicates match.

    let input = r#"
## Theorem: Socrates_Mortality
Given: All men are mortal.
Given: Socrates is a man.
Prove: Socrates is mortal.
Proof: Auto.
"#;

    let result = compile_theorem(input);

    assert!(result.is_ok(), "Failed to compile theorem: {:?}", result);

    let output = result.unwrap();
    println!("Proof output:\n{}", output);

    // Should contain proof steps indicating success
    assert!(
        output.contains("Proved") || output.contains("ModusPonens") || output.contains("PremiseMatch"),
        "Expected proof output, got: {}",
        output
    );
}

#[test]
fn test_theorem_with_universal_instantiation() {
    // Tests that universal quantifiers are properly instantiated.
    // "Every philosopher is wise" + "Plato is a philosopher"
    // → "Plato is wise"

    let input = r#"
## Theorem: Plato_Wisdom
Given: Every philosopher is wise.
Given: Plato is a philosopher.
Prove: Plato is wise.
Proof: Auto.
"#;

    let result = compile_theorem(input);
    assert!(result.is_ok(), "Failed to prove Plato's wisdom: {:?}", result);

    let output = result.unwrap();
    println!("Plato proof:\n{}", output);
}

// =============================================================================
// PROOF FAILURE TESTS
// =============================================================================

#[test]
fn test_theorem_fail_missing_premise() {
    // Should fail: we don't have "Socrates is a man" → cannot derive mortality.

    let input = r#"
## Theorem: Incomplete
Given: All men are mortal.
Prove: Socrates is mortal.
Proof: Auto.
"#;

    let result = compile_theorem(input);

    // Should fail with a helpful error
    assert!(result.is_err(), "Theorem should have failed - missing premise!");

    let error = result.unwrap_err();
    let error_msg = format!("{:?}", error);
    println!("Expected failure: {}", error_msg);

    // Error should mention what's missing or what failed
    assert!(
        error_msg.contains("proof") || error_msg.contains("Proof") || error_msg.contains("derive"),
        "Error should explain proof failure: {}",
        error_msg
    );
}

#[test]
fn test_theorem_fail_contradictory() {
    // Should fail: cannot prove something that doesn't follow from premises.

    let input = r#"
## Theorem: Impossible
Given: Socrates is a man.
Prove: Socrates is a stone.
Proof: Auto.
"#;

    let result = compile_theorem(input);
    assert!(result.is_err(), "Theorem should have failed - no logical path!");
}

// =============================================================================
// MULTIPLE PREMISES
// =============================================================================

#[test]
fn test_theorem_chain_reasoning() {
    // Test multi-step reasoning:
    // All men are mortal.
    // All mortals are doomed.
    // Socrates is a man.
    // ∴ Socrates is doomed.
    //
    // Phase 64: Noun canonicalization handles "men" → "Man", "mortals" → "Mortal"

    let input = r#"
## Theorem: Socrates_Doom
Given: All men are mortal.
Given: All mortals are doomed.
Given: Socrates is a man.
Prove: Socrates is doomed.
Proof: Auto.
"#;

    let result = compile_theorem(input);
    assert!(
        result.is_ok(),
        "Failed to prove chain reasoning: {:?}",
        result
    );

    let output = result.unwrap();
    println!("Chain proof:\n{}", output);
}

// =============================================================================
// DIRECT MATCH (NO REASONING NEEDED)
// =============================================================================

#[test]
fn test_theorem_direct_match() {
    // Trivial case: goal is identical to a premise.
    //
    // Note: Using "Socrates is mortal" instead of "The sky is blue"
    // because "the" introduces uniqueness constraints that produce complex expressions.
    // TODO: Phase X - Handle definite descriptions in theorem prover.

    let input = r#"
## Theorem: Trivial
Given: Socrates is mortal.
Prove: Socrates is mortal.
Proof: Auto.
"#;

    let result = compile_theorem(input);
    assert!(result.is_ok(), "Direct match should succeed: {:?}", result);

    let output = result.unwrap();
    // Should use PremiseMatch rule
    assert!(
        output.contains("PremiseMatch") || output.contains("Proved"),
        "Expected PremiseMatch rule, got: {}",
        output
    );
}

// =============================================================================
// SYNTAX TESTS
// =============================================================================

#[test]
fn test_theorem_name_extraction() {
    // Verify the theorem name is captured correctly.

    let input = r#"
## Theorem: My_Custom_Name_123
Given: P.
Prove: P.
Proof: Auto.
"#;

    let result = compile_theorem(input);
    assert!(result.is_ok(), "Named theorem should parse: {:?}", result);

    let output = result.unwrap();
    assert!(
        output.contains("My_Custom_Name_123"),
        "Theorem name should appear in output: {}",
        output
    );
}
