//! Phase: Interpreter Policy Support
//!
//! Tests for security policy checks in the interpreter.
//! These currently fail because the interpreter returns "not supported".

mod common;
use common::run_interpreter;

// =============================================================================
// Predicate Tests (A User is admin if ...)
// =============================================================================

#[test]
fn interpreter_policy_predicate_passes() {
    let source = r#"## Definition
A User has:
    a role, which is Text.

## Policy
A User is admin if the user's role equals "admin".

## Main
Let u be a new User with role "admin".
Check that the u is admin.
Show "passed".
"#;
    let result = run_interpreter(source);
    assert!(result.success, "Should run without error: {}", result.error);
    assert!(result.output.contains("passed"), "Should output 'passed', got: {}", result.output);
}

#[test]
fn interpreter_policy_predicate_fails() {
    let source = r#"## Definition
A User has:
    a role, which is Text.

## Policy
A User is admin if the user's role equals "admin".

## Main
Let u be a new User with role "guest".
Check that the u is admin.
Show "should not reach".
"#;
    let result = run_interpreter(source);
    assert!(!result.success, "Should fail security check");
    assert!(result.error.contains("Security") || result.error.contains("Check"),
        "Should have security error, got: {}", result.error);
}

#[test]
fn interpreter_policy_predicate_different_values() {
    let source = r#"## Definition
A User has:
    a role, which is Text.

## Policy
A User is editor if the user's role equals "editor".

## Main
Let u be a new User with role "editor".
Check that the u is editor.
Show "editor verified".
"#;
    let result = run_interpreter(source);
    assert!(result.success, "Should run without error: {}", result.error);
    assert!(result.output.contains("editor verified"), "Should output 'editor verified', got: {}", result.output);
}

// =============================================================================
// Boolean Field Tests
// =============================================================================

#[test]
fn interpreter_policy_bool_field() {
    let source = r#"## Definition
A User has:
    a verified, which is Bool.

## Policy
A User is trusted if the user's verified equals true.

## Main
Let u be a new User with verified true.
Check that the u is trusted.
Show "trusted".
"#;
    let result = run_interpreter(source);
    assert!(result.success, "Should run without error: {}", result.error);
    assert!(result.output.contains("trusted"), "Should output 'trusted', got: {}", result.output);
}

#[test]
fn interpreter_policy_bool_field_fails() {
    let source = r#"## Definition
A User has:
    a verified, which is Bool.

## Policy
A User is trusted if the user's verified equals true.

## Main
Let u be a new User with verified false.
Check that the u is trusted.
Show "should not reach".
"#;
    let result = run_interpreter(source);
    assert!(!result.success, "Should fail security check");
}

// =============================================================================
// Multiple Predicates
// =============================================================================

#[test]
fn interpreter_policy_multiple_predicates() {
    let source = r#"## Definition
A User has:
    a role, which is Text.
    a verified, which is Bool.

## Policy
A User is admin if the user's role equals "admin".
A User is verified_user if the user's verified equals true.

## Main
Let admin be a new User with role "admin" and verified true.
Check that the admin is admin.
Check that the admin is verified_user.
Show "both passed".
"#;
    let result = run_interpreter(source);
    assert!(result.success, "Should run without error: {}", result.error);
    assert!(result.output.contains("both passed"), "Should output 'both passed', got: {}", result.output);
}
