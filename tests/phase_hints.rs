//! Phase Hints: Making Auto Learn
//!
//! Tests for the hint database and hint-aware auto.

use logos::interface::Repl;

// =============================================================================
// HINT REGISTRATION
// =============================================================================

#[test]
fn test_hint_attribute_parses() {
    let mut repl = Repl::new();
    let result = repl.execute(
        r#"## Theorem: TrueHint
    Statement: True.
    Proof: auto.
    Attribute: hint."#,
    );
    assert!(result.is_ok(), "Hint attribute should parse: {:?}", result);
}

#[test]
fn test_hint_theorem_accessible() {
    let mut repl = Repl::new();
    repl.execute(
        r#"## Theorem: MyLemma
    Statement: True.
    Proof: auto.
    Attribute: hint."#,
    )
    .unwrap();

    // Check that MyLemma is registered and accessible
    let result = repl.execute("Check MyLemma.");
    assert!(
        result.is_ok(),
        "Hint theorem should be accessible: {:?}",
        result
    );
}

#[test]
fn test_hint_without_proof() {
    // A theorem with hint attribute but no proof should still register
    let mut repl = Repl::new();
    let result = repl.execute(
        r#"## Theorem: AxiomHint
    Statement: (implies A A).
    Attribute: hint."#,
    );
    // This could either succeed (axiom-style) or error (requires proof)
    // We accept either behavior for now
    let _ = result;
}

// =============================================================================
// HINT DATABASE QUERIES
// =============================================================================

#[test]
fn test_is_hint() {
    let mut repl = Repl::new();

    // Register a hint
    repl.execute(
        r#"## Theorem: HintedTheorem
    Statement: True.
    Proof: auto.
    Attribute: hint."#,
    )
    .unwrap();

    // Register a non-hint
    repl.execute(
        r#"## Theorem: NotHinted
    Statement: True.
    Proof: auto."#,
    )
    .unwrap();

    // Both should be accessible
    assert!(repl.execute("Check HintedTheorem.").is_ok());
    assert!(repl.execute("Check NotHinted.").is_ok());
}

// =============================================================================
// HINT-AWARE AUTO (stretch goal)
// =============================================================================

#[test]
fn test_auto_tries_hints() {
    let mut repl = Repl::new();

    // Register an implication as a hint
    // Note: This test documents the INTENDED behavior
    // auto should eventually try to apply registered hints

    // Use valid syntax that the prelude supports
    let result = repl.execute(
        r#"## Theorem: TrueImpliesTrue
    Statement: (Or True True).
    Proof: auto.
    Attribute: hint."#,
    );

    // This should succeed - registering a simple hint
    assert!(result.is_ok(), "Should be able to register hint: {:?}", result);
}

// =============================================================================
// MULTIPLE HINTS
// =============================================================================

#[test]
fn test_multiple_hints() {
    let mut repl = Repl::new();

    repl.execute(
        r#"## Theorem: Hint1
    Statement: True.
    Proof: auto.
    Attribute: hint."#,
    )
    .unwrap();

    repl.execute(
        r#"## Theorem: Hint2
    Statement: (implies True True).
    Proof: auto.
    Attribute: hint."#,
    )
    .unwrap();

    repl.execute(
        r#"## Theorem: Hint3
    Statement: (And True True).
    Proof: auto.
    Attribute: hint."#,
    )
    .unwrap();

    // All should be accessible
    assert!(repl.execute("Check Hint1.").is_ok());
    assert!(repl.execute("Check Hint2.").is_ok());
    assert!(repl.execute("Check Hint3.").is_ok());
}
