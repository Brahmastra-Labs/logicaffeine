//! Phase 45: Native Ownership Analysis Tests
//!
//! Tests for control-flow-aware ownership tracking.
//! These tests use compile_to_rust_checked() which runs the ownership analysis pass.

use logos::compile::compile_to_rust_checked;

#[test]
fn test_use_after_move_in_branch() {
    let source = r#"## Main
Let x be 5.
If x > 0:
    Give x to processor.
Show x to console.
"#;
    let result = compile_to_rust_checked(source);
    assert!(result.is_err(), "Use after potential move should fail");
    let err = format!("{:?}", result.unwrap_err());
    assert!(
        err.contains("move") || err.contains("Move") || err.contains("give") || err.contains("Give"),
        "Error should mention move/ownership: {}", err
    );
}

#[test]
fn test_double_move() {
    let source = r#"## Main
Let x be 5.
Give x to processor1.
Give x to processor2.
"#;
    let result = compile_to_rust_checked(source);
    assert!(result.is_err(), "Double move should fail");
}

#[test]
fn test_valid_show_then_give() {
    let source = r#"## Main
Let x be 5.
Show x to console.
Give x to processor.
"#;
    let result = compile_to_rust_checked(source);
    assert!(result.is_ok(), "Show then Give is valid: {:?}", result.err());
}

#[test]
fn test_move_in_both_branches_then_use() {
    let source = r#"## Main
Let x be 5.
If x > 0:
    Give x to processor1.
Otherwise:
    Give x to processor2.
Show x to console.
"#;
    let result = compile_to_rust_checked(source);
    assert!(result.is_err(), "Move in both branches should fail");
}

#[test]
fn test_move_in_one_branch_ok_if_not_used() {
    let source = r#"## Main
Let x be 5.
If x > 0:
    Give x to processor.
"#;
    let result = compile_to_rust_checked(source);
    assert!(result.is_ok(), "Move in branch without later use is valid: {:?}", result.err());
}

#[test]
fn test_nested_control_flow() {
    let source = r#"## Main
Let x be 5.
While x > 0:
    If x = 1:
        Give x to processor.
    Set x to x - 1.
Show x to console.
"#;
    let result = compile_to_rust_checked(source);
    assert!(result.is_err(), "Nested control flow should track moves");
}

#[test]
fn test_linear_use_after_move() {
    // Basic linear case - should be caught
    let source = r#"## Main
Let x be 5.
Give x to processor.
Show x to console.
"#;
    let result = compile_to_rust_checked(source);
    assert!(result.is_err(), "Linear use after move should fail");
}

#[test]
fn test_borrow_does_not_move() {
    // Show (borrow) should not consume ownership
    let source = r#"## Main
Let x be 5.
Show x to console1.
Show x to console2.
Show x to console3.
"#;
    let result = compile_to_rust_checked(source);
    assert!(result.is_ok(), "Multiple Shows should be valid: {:?}", result.err());
}
