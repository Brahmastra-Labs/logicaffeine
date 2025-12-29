//! Phase 44: Totality Checking (Termination Proofs) Tests
//!
//! Tests that loop termination is verified at compile time using decreasing variants.
//! Run with: cargo test --features verification --test phase_totality

#[cfg(feature = "verification")]
use logos::compile::compile_to_rust_verified;

#[test]
#[cfg(feature = "verification")]
fn test_decreasing_loop_valid() {
    let source = r#"## Main
Let x be 10.
While x > 0 (decreasing x):
    Set x to x - 1.
"#;
    let result = compile_to_rust_verified(source);
    assert!(result.is_ok(), "Valid decreasing loop should compile: {:?}", result.err());
}

#[test]
#[cfg(feature = "verification")]
fn test_decreasing_loop_invalid_increment() {
    let source = r#"## Main
Let x be 10.
While x > 0 (decreasing x):
    Set x to x + 1.
"#;
    let result = compile_to_rust_verified(source);
    assert!(result.is_err(), "Incrementing loop variant should fail");
    let err = format!("{:?}", result.unwrap_err());
    assert!(err.contains("terminat") || err.contains("decreas") || err.contains("Terminat"),
            "Error should mention termination: {}", err);
}

#[test]
#[cfg(feature = "verification")]
fn test_decreasing_loop_no_change() {
    let source = r#"## Main
Let x be 10.
While x > 0 (decreasing x):
    Let y be 5.
"#;
    let result = compile_to_rust_verified(source);
    assert!(result.is_err(), "Loop without variant change should fail");
}

#[test]
#[cfg(feature = "verification")]
fn test_decreasing_loop_valid_with_offset() {
    let source = r#"## Main
Let x be 100.
While x > 0 (decreasing x):
    Set x to x - 10.
"#;
    let result = compile_to_rust_verified(source);
    assert!(result.is_ok(), "Decrementing by 10 should be valid: {:?}", result.err());
}

#[test]
#[cfg(feature = "verification")]
fn test_loop_without_decreasing_skipped() {
    // Loops without decreasing clause should not be checked
    let source = r#"## Main
Let x be 10.
While x > 0:
    Set x to x + 1.
"#;
    let result = compile_to_rust_verified(source);
    // Should compile (no termination checking without decreasing clause)
    assert!(result.is_ok(), "Loop without decreasing clause should not be checked: {:?}", result.err());
}

#[test]
#[cfg(feature = "verification")]
fn test_decreasing_compound_body() {
    let source = r#"## Main
Let x be 10.
While x > 0 (decreasing x):
    Let temp be x - 1.
    Set x to temp.
"#;
    let result = compile_to_rust_verified(source);
    assert!(result.is_ok(), "Compound body with net decrease should pass: {:?}", result.err());
}
