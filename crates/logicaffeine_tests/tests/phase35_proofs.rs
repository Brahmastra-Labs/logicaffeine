//! Phase 35: The Proof Bridge (Assertions & Invariants)
//!
//! Tests for Trust statements with justifications.

use logicaffeine_compile::compile::compile_to_rust;

#[test]
fn test_trust_statement() {
    let source = r#"
## Main
Let x be 10.
Trust that x is greater than 0 because "I set it to 10".
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("// TRUST: I set it to 10"), "Should have trust comment: {}", rust);
    assert!(rust.contains("debug_assert!((x > 0));"), "Should have assertion: {}", rust);
}

#[test]
fn test_trust_complex_proposition() {
    let source = r#"
## Main
Let a be 5.
Let b be 3.
Trust that a is greater than b because "a is always larger".
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("// TRUST: a is always larger"), "Should have trust comment: {}", rust);
    assert!(rust.contains("debug_assert!((a > b));"), "Should have assertion: {}", rust);
}

#[test]
fn test_trust_compound_logic() {
    let source = r#"
## Main
Let x be 5.
Trust that x is greater than 0 and x is less than 100 because "x is in valid range".
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("// TRUST: x is in valid range"), "Should have trust comment: {}", rust);
    assert!(rust.contains("debug_assert!(((x > 0) && (x < 100)));"), "Should have compound assertion: {}", rust);
}

#[test]
fn test_assert_unchanged() {
    let source = r#"
## Main
Let y be 42.
Assert that y is equal to 42.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("debug_assert!((y == 42));"), "Should have assertion: {}", rust);
    assert!(!rust.contains("// TRUST:"), "Should NOT have trust comment for Assert: {}", rust);
}

#[test]
fn test_trust_without_that() {
    let source = r#"
## Main
Let n be 1.
Trust n is greater than 0 because "positive".
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("// TRUST: positive"), "Should have trust comment: {}", rust);
}

#[test]
fn test_trust_with_causal_logic() {
    // Ensure "because" in logic doesn't break "because" in Trust
    let source = r#"
## Main
Let x be 10.
Trust that x is positive because x is greater than 0 because "reasoning".
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    // Prop=(x is positive because x > 0), Reason="reasoning"
    assert!(rust.contains("// TRUST: reasoning"), "Should have trust comment: {}", rust);
}

#[test]
fn test_assert_in_function() {
    let source = r#"
## To withdraw (amount: Int) from (balance: Int) -> Int:
    Assert that amount is greater than 0.
    Return balance - amount.

## Main
Let result be withdraw(50, 100).
Show result.
"#;
    let rust = compile_to_rust(source).expect("Should compile Assert in function");
    assert!(rust.contains("debug_assert!"), "Should have debug_assert: {}", rust);
}

#[test]
fn test_trust_in_function() {
    let source = r#"
## To process (n: Int) -> Int:
    Trust that n is greater than 0 because "caller guarantees positive input".
    Return n * 2.

## Main
Let doubled be process(5).
Show doubled.
"#;
    let rust = compile_to_rust(source).expect("Should compile Trust in function");
    assert!(rust.contains("// TRUST: caller guarantees positive input"), "Should have trust comment: {}", rust);
    assert!(rust.contains("debug_assert!"), "Should have debug_assert: {}", rust);
}
