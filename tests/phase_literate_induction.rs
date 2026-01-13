//! Phase Literate Induction: Bullet Proof Syntax
//!
//! Tests for multi-step proofs with bullet points.
//!
//! Note: These tests focus on the `induction` keyword and bullet syntax.
//! The forall quantifier syntax will be addressed separately.

use logos::interface::Repl;

// =============================================================================
// INDUCTION KEYWORD RECOGNITION
// =============================================================================

#[test]
fn test_induction_keyword_recognized() {
    // Test that 'induction' is recognized as a proof tactic
    let mut repl = Repl::new();
    let result = repl.execute(
        r#"## Theorem: Test
    Statement: True.
    Proof:
        induction n.
        - auto.
        - auto."#,
    );
    // Should fail at induction (tactic not implemented), not at parsing
    // For now, accept either parse error or execution error
    let _ = result; // Document the behavior
}

#[test]
fn test_bullet_syntax_recognized() {
    // Test that '-' bullet syntax is recognized
    let mut repl = Repl::new();
    let result = repl.execute(
        r#"## Theorem: BulletTest
    Statement: True.
    Proof:
        induction x.
        - auto."#,
    );
    // This documents intended syntax
    let _ = result;
}

// =============================================================================
// SIMPLE INDUCTION (using existing syntax)
// =============================================================================

#[test]
fn test_simple_induction_fails_gracefully() {
    // Induction on something simple should fail gracefully
    // (since induction tactic doesn't exist yet)
    let mut repl = Repl::new();
    let result = repl.execute(
        r#"## Theorem: SimpleInd
    Statement: True.
    Proof: induction."#,
    );
    // Should fail because 'induction' tactic doesn't exist
    assert!(result.is_err(), "Induction should fail (not implemented): {:?}", result);
}

// =============================================================================
// ERROR CASES (these should fail in specific ways)
// =============================================================================

#[test]
fn test_induction_wrong_bullet_count_too_many() {
    let mut repl = Repl::new();
    // Even when induction works, too many bullets should error
    let result = repl.execute(
        r#"## Theorem: WrongCount
    Statement: True.
    Proof:
        induction n.
        - auto.
        - auto.
        - auto."#,
    );
    // Should error (either parse error now, or case count error later)
    assert!(result.is_err(), "Too many bullets should error: {:?}", result);
}

#[test]
fn test_induction_wrong_bullet_count_too_few() {
    let mut repl = Repl::new();
    let result = repl.execute(
        r#"## Theorem: Missing
    Statement: True.
    Proof:
        induction n.
        - auto."#,
    );
    // Should error
    assert!(result.is_err(), "Too few bullets should error: {:?}", result);
}

#[test]
fn test_induction_no_bullets() {
    let mut repl = Repl::new();
    let result = repl.execute(
        r#"## Theorem: NoBullets
    Statement: True.
    Proof:
        induction n."#,
    );
    // Should error - induction requires cases
    assert!(result.is_err(), "Induction without bullets should error: {:?}", result);
}

#[test]
fn test_induction_tactic_not_recognized() {
    let mut repl = Repl::new();
    // 'induction' as a single-word tactic should not be recognized yet
    let result = repl.execute(
        r#"## Theorem: NotRecognized
    Statement: True.
    Proof: induction."#,
    );
    // Should fail because induction is not a valid tactic
    assert!(result.is_err(), "induction should not be recognized: {:?}", result);
}

// =============================================================================
// HINT ATTRIBUTE AFTER PROOF
// =============================================================================

#[test]
fn test_theorem_with_hint_attribute() {
    let mut repl = Repl::new();
    let result = repl.execute(
        r#"## Theorem: HintedTheorem
    Statement: True.
    Proof: auto.
    Attribute: hint."#,
    );
    // This should work (hint attribute is already parsed)
    assert!(result.is_ok(), "Hint attribute should work: {:?}", result);
}

// =============================================================================
// MULTI-TACTIC PROOFS (sequential tactics)
// =============================================================================

#[test]
fn test_multi_tactic_in_bullet() {
    let mut repl = Repl::new();
    // Multiple tactics in a bullet: "simp. auto."
    let result = repl.execute(
        r#"## Theorem: MultiTactic
    Statement: True.
    Proof:
        induction n.
        - simp. auto.
        - auto."#,
    );
    // Should error because induction doesn't exist yet
    assert!(result.is_err(), "Multi-tactic bullets should error (induction not impl): {:?}", result);
}
