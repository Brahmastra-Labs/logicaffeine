//! Phase 9: Structured Concurrency Tests
//!
//! Tests for concurrent (async, I/O-bound) and parallel (CPU-bound) execution blocks.

mod common;
use common::compile_to_rust;

// =============================================================================
// Concurrent Block Tests (Attempt all of the following)
// =============================================================================

#[test]
fn test_concurrent_basic() {
    let source = "## Main\nAttempt all of the following:\n    Let x be 1.\n    Let y be 2.";
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("tokio::join!"), "Should generate tokio::join! for concurrent block");
}

#[test]
fn test_concurrent_destructures_let_bindings() {
    let source = "## Main\nAttempt all of the following:\n    Let a be 1.\n    Let b be 2.";
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("let (a, b) = tokio::join!"), "Should destructure Let bindings into tuple");
}

#[test]
fn test_concurrent_with_single_statement() {
    let source = "## Main\nAttempt all of the following:\n    Let x be 42.";
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("tokio::join!"), "Should generate tokio::join! even for single task");
    assert!(rust.contains("async {"), "Should wrap statement in async block");
}

// =============================================================================
// Parallel Block Tests (Simultaneously)
// =============================================================================

#[test]
fn test_parallel_basic() {
    let source = "## Main\nSimultaneously:\n    Let x be 1.\n    Let y be 2.";
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("rayon::join"), "Should generate rayon::join for parallel block with 2 tasks");
}

#[test]
fn test_parallel_destructures_let_bindings() {
    let source = "## Main\nSimultaneously:\n    Let a be 1.\n    Let b be 2.";
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("let (a, b) = rayon::join"), "Should destructure Let bindings into tuple");
}

#[test]
fn test_parallel_three_tasks_uses_threads() {
    let source = "## Main\nSimultaneously:\n    Let a be 1.\n    Let b be 2.\n    Let c be 3.";
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("std::thread::spawn"), "Should use thread::spawn for 3+ tasks");
}

#[test]
fn test_parallel_uses_closure_syntax() {
    let source = "## Main\nSimultaneously:\n    Let x be 1.\n    Let y be 2.";
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("|| {"), "Should use closure syntax for rayon::join tasks");
}

// =============================================================================
// Error Cases
// =============================================================================

#[test]
fn test_concurrent_missing_all_fails() {
    let source = "## Main\nAttempt of the following:\n    Let x be 1.";
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail without 'all' keyword");
}

#[test]
fn test_concurrent_missing_colon_fails() {
    let source = "## Main\nAttempt all of the following\n    Let x be 1.";
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail without colon");
}

#[test]
fn test_parallel_missing_colon_fails() {
    let source = "## Main\nSimultaneously\n    Let x be 1.";
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail without colon");
}

// =============================================================================
// Integration with Other Constructs
// =============================================================================

#[test]
fn test_concurrent_in_function() {
    let source = r#"## To fetch_all:
    Attempt all of the following:
        Let x be 1.
        Let y be 2.
    Return.

## Main
Call fetch_all."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("fn fetch_all"), "Should have function definition");
    assert!(rust.contains("tokio::join!"), "Should have concurrent block in function");
}

#[test]
fn test_parallel_in_function() {
    let source = r#"## To compute_all:
    Simultaneously:
        Let x be 1.
        Let y be 2.
    Return.

## Main
Call compute_all."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("fn compute_all"), "Should have function definition");
    assert!(rust.contains("rayon::join"), "Should have parallel block in function");
}

#[test]
fn test_nested_parallel_blocks() {
    let source = r#"## Main
Simultaneously:
    Let a be 1.
    Let b be 2.
Simultaneously:
    Let c be 3.
    Let d be 4."#;
    let rust = compile_to_rust(source).expect("Should compile");
    // Should have two separate rayon::join calls
    let join_count = rust.matches("rayon::join").count();
    assert_eq!(join_count, 2, "Should have two parallel blocks");
}
