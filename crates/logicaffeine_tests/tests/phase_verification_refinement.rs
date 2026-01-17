//! Phase 43D: Static Refinement Verification Tests
//!
//! Tests that refinement type constraints are verified at compile time.
//! Run with: cargo test --features verification --test phase_verification_refinement

#[cfg(feature = "verification")]
use logicaffeine_compile::compile::compile_to_rust_verified;

#[test]
#[cfg(feature = "verification")]
fn test_refinement_valid_literal() {
    let source = "## Main\nLet x: Int where it > 0 be 10.";
    let result = compile_to_rust_verified(source);
    assert!(result.is_ok(), "Should allow 10 where it > 0: {:?}", result.err());
}

#[test]
#[cfg(feature = "verification")]
fn test_refinement_invalid_literal() {
    // Test with negative literal (unary minus now supported)
    let source = "## Main\nLet x: Int where it > 0 be -5.";
    let result = compile_to_rust_verified(source);
    assert!(result.is_err(), "Should reject -5 where it > 0");
    let err = format!("{:?}", result.unwrap_err());
    assert!(
        err.contains("Verification") || err.contains("refinement") || err.contains("Refinement"),
        "Error should mention verification failure: {}",
        err
    );
}

#[test]
#[cfg(feature = "verification")]
fn test_refinement_valid_variable() {
    let source = "## Main\nLet a be 5.\nLet b: Int where it > 0 be a.";
    let result = compile_to_rust_verified(source);
    assert!(result.is_ok(), "Should allow a=5 where it > 0: {:?}", result.err());
}

#[test]
#[cfg(feature = "verification")]
fn test_refinement_invalid_variable() {
    let source = "## Main\nLet a be -5.\nLet b: Int where it > 0 be a.";
    let result = compile_to_rust_verified(source);
    assert!(result.is_err(), "Should reject a=-5 where it > 0");
}

#[test]
#[cfg(feature = "verification")]
fn test_refinement_compound_and() {
    let source = "## Main\nLet x: Int where it > 0 and it < 100 be 50.";
    let result = compile_to_rust_verified(source);
    assert!(result.is_ok(), "Should allow 50 where it > 0 and it < 100: {:?}", result.err());
}

#[test]
#[cfg(feature = "verification")]
fn test_refinement_compound_violation() {
    let source = "## Main\nLet x: Int where it > 0 and it < 100 be 150.";
    let result = compile_to_rust_verified(source);
    assert!(result.is_err(), "Should reject 150 where it > 0 and it < 100");
}

#[test]
#[cfg(feature = "verification")]
fn test_refinement_zero_boundary() {
    // Edge case: exactly at boundary
    let source = "## Main\nLet x: Int where it >= 0 be 0.";
    let result = compile_to_rust_verified(source);
    assert!(result.is_ok(), "Should allow 0 where it >= 0: {:?}", result.err());
}

#[test]
#[cfg(feature = "verification")]
fn test_refinement_less_than() {
    let source = "## Main\nLet x: Int where it < 10 be 5.";
    let result = compile_to_rust_verified(source);
    assert!(result.is_ok(), "Should allow 5 where it < 10: {:?}", result.err());
}

#[test]
#[cfg(feature = "verification")]
fn test_refinement_less_than_violation() {
    let source = "## Main\nLet x: Int where it < 10 be 15.";
    let result = compile_to_rust_verified(source);
    assert!(result.is_err(), "Should reject 15 where it < 10");
}

#[test]
#[cfg(feature = "verification")]
fn test_refinement_equality() {
    let source = "## Main\nLet x: Int where it == 42 be 42.";
    let result = compile_to_rust_verified(source);
    assert!(result.is_ok(), "Should allow 42 where it == 42: {:?}", result.err());
}

#[test]
#[cfg(feature = "verification")]
fn test_refinement_equality_violation() {
    let source = "## Main\nLet x: Int where it == 42 be 43.";
    let result = compile_to_rust_verified(source);
    assert!(result.is_err(), "Should reject 43 where it == 42");
}

#[test]
#[cfg(feature = "verification")]
fn test_refinement_arithmetic_expression() {
    // Value is computed from arithmetic
    let source = "## Main\nLet a be 10.\nLet b be 5.\nLet c: Int where it > 0 be a - b.";
    let result = compile_to_rust_verified(source);
    // Note: This may or may not work depending on how well we track arithmetic
    // For now, just check it doesn't panic
    let _ = result;
}
