//! E2E Tests: Language Feature Gaps
//!
//! Tests for language features identified as missing E2E coverage:
//! - Recursive enums with boxed fields
//! - String concatenation (E2E, not interpreter)
//! - Float arithmetic
//! - Trust statements
//! - Assert statements
//! - Temporal arithmetic
//! - Concurrent dependency detection
//!
//! ## Test Results Summary
//! - 17 passing tests
//! - 6 failing tests (codegen bugs identified)
//!
//! ## Known Codegen Bugs Found:
//! 1. String concatenation: RHS needs &str not String for `+` operator
//! 2. Float type: `Float` should be `f64` in generated Rust
//! 3. Duration return type: `Duration` should be `std::time::Duration`
//! 4. Recursive enum with Box: Boxing works but there may be edge cases
//!    in recursive function param passing (needs investigation)

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::{assert_output, assert_runs, run_logos, assert_panics};

// =============================================================================
// TIER 1: Recursive Enums with Boxed Fields
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_recursive_enum_creation() {
    let source = r#"## A Tree is one of:
    A Leaf with value Int.
    A Node with left Tree and right Tree.

## Main
Let tree be a new Leaf with value 42.
Inspect tree:
    When Leaf (v): Show v.
    When Node (l, r): Show "node".
"#;
    assert_output(source, "42");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_recursive_enum_nested() {
    let source = r#"## A Tree is one of:
    A Leaf with value Int.
    A Node with left Tree and right Tree.

## Main
Let l be a new Leaf with value 1.
Let r be a new Leaf with value 2.
Let tree be a new Node with left l and right r.
Inspect tree:
    When Leaf (v): Show v.
    When Node (left, right): Show "is node".
"#;
    assert_output(source, "is node");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_recursive_enum_nested_inspect() {
    let source = r#"## A Tree is one of:
    A Leaf with value Int.
    A Node with left Tree and right Tree.

## Main
Let l be a new Leaf with value 10.
Let r be a new Leaf with value 20.
Let tree be a new Node with left l and right r.
Inspect tree:
    When Leaf (v): Show v.
    When Node (left, right):
        Inspect left:
            When Leaf (v): Show v.
            When Node (l, r): Show "nested node".
"#;
    assert_output(source, "10");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_recursive_enum_function_param() {
    let source = r#"## A Tree is one of:
    A Leaf with value Int.
    A Node with left Tree and right Tree.

## To sum_tree (t: Tree) -> Int:
    Inspect t:
        When Leaf (v): Return v.
        When Node (left, right):
            Let lsum be sum_tree(left).
            Let rsum be sum_tree(right).
            Return lsum + rsum.

## Main
Let l be a new Leaf with value 3.
Let r be a new Leaf with value 5.
Let tree be a new Node with left l and right r.
Show sum_tree(tree).
"#;
    assert_output(source, "8");
}

// =============================================================================
// TIER 1: String Concatenation (E2E Tests)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_string_concat_basic() {
    let source = r#"## Main
Let greeting be "Hello".
Let name be "World".
Let message be greeting + " " + name.
Show message.
"#;
    assert_output(source, "Hello World");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_string_concat_two_strings() {
    let source = r#"## Main
Let a be "foo".
Let b be "bar".
Show a + b.
"#;
    assert_output(source, "foobar");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_string_concat_chained() {
    let source = r#"## Main
Let result be "a" + "b" + "c" + "d".
Show result.
"#;
    assert_output(source, "abcd");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_string_length() {
    let source = r#"## Main
Let s be "Hello".
Show length of s.
"#;
    assert_output(source, "5");
}

// =============================================================================
// TIER 1: Float Arithmetic
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_float_literal() {
    let source = r#"## Main
Let pi be 3.14.
Show pi.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Float literal should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("3.14"), "Should output 3.14: {}", result.stdout);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_float_addition() {
    let source = r#"## Main
Let a be 1.5.
Let b be 2.5.
Show a + b.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Float addition should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("4"), "Should output 4: {}", result.stdout);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_float_multiplication() {
    let source = r#"## Main
Let r be 2.0.
Let area be 3.14 * r * r.
Show area.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Float multiplication should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("12.56"), "Should output 12.56: {}", result.stdout);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_float_division() {
    let source = r#"## Main
Let x be 10.0.
Let y be 4.0.
Show x / y.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Float division should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("2.5"), "Should output 2.5: {}", result.stdout);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_float_in_function() {
    let source = r#"## To double (x: Float) -> Float:
    Return x * 2.0.

## Main
Let result be double(3.5).
Show result.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Float in function should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("7"), "Should output 7: {}", result.stdout);
}

// =============================================================================
// TIER 2: Trust Statement
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_trust_passing() {
    let source = r#"## Main
Let x be 10.
Trust that x is greater than 0 because "Input was validated".
Show x.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Trust statement should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("10"), "Should output 10: {}", result.stdout);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_trust_with_reason() {
    let source = r#"## Main
Let items be [1, 2, 3].
Let n be length of items.
Trust that n is greater than 0 because "List is non-empty".
Show n.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Trust with reason should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("3"), "Should output 3: {}", result.stdout);
}

// =============================================================================
// TIER 2: Assert Statement
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_assert_pass() {
    let source = r#"## Main
Let x be 10.
Assert that x is greater than 5.
Show "passed".
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Assert should pass.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("passed"), "Should output passed: {}", result.stdout);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_assert_fail() {
    let source = r#"## Main
Let x be 3.
Assert that x is greater than 10.
Show "passed".
"#;
    assert_panics(source, "assertion");
}

// =============================================================================
// TIER 3: Temporal Arithmetic
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_duration_variable() {
    let source = r#"## Main
Let d be 100ms.
Show "ok".
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Duration variable should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("ok"), "Should output ok: {}", result.stdout);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_duration_in_function() {
    let source = r#"## To get_timeout -> Duration:
    Return 500ms.

## Main
Let t be get_timeout().
Show "ok".
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Duration in function should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("ok"), "Should output ok: {}", result.stdout);
}

// =============================================================================
// TIER 3: Concurrent Dependency Detection
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_concurrent_independent_vars() {
    let source = r#"## To compute (x: Int) -> Int:
    Sleep 10.
    Return x * 2.

## Main
    Attempt all of the following:
        Let a be compute(5).
        Let b be compute(10).
    Show a + b.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Concurrent independent should run.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("30"), "Should output 30: {}", result.stdout);
}

// =============================================================================
// Additional Edge Cases
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_empty_string() {
    let source = r#"## Main
Let s be "".
Show length of s.
"#;
    assert_output(source, "0");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_string_equality() {
    let source = r#"## Main
Let a be "hello".
Let b be "hello".
If a equals b:
    Show "equal".
Otherwise:
    Show "not equal".
"#;
    assert_output(source, "equal");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_recursive_function_with_enum() {
    let source = r#"## A List is one of:
    A Nil.
    A Cons with head Int and tail List.

## To length_of (l: List) -> Int:
    Inspect l:
        When Nil: Return 0.
        When Cons (h, t): Return 1 + length_of(t).

## Main
Let l1 be a new Nil.
Let l2 be a new Cons with head 1 and tail l1.
Let l3 be a new Cons with head 2 and tail l2.
Show length_of(l3).
"#;
    assert_output(source, "2");
}
