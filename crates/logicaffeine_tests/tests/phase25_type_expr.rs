//! Phase 25: Type Expressions & Annotations
//!
//! This phase teaches the parser to read explicit type annotations:
//! - `Let x: Int be 5.` → `let x: i64 = 5;`
//! - `Let s: List of Int be empty.` → `let s: Vec<i64> = ...`
//!
//! Validates that TypeRegistry is working by parsing recursive generic types.

mod common;

use logicaffeine_compile::compile::compile_to_rust;

// =============================================================================
// Test 1: Simple Type Annotations
// =============================================================================

#[test]
fn let_with_int_annotation() {
    let source = "## Main\nLet x: Int be 5.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("let x: i64 = 5;"), "Should emit typed let: {}", rust);
}

#[test]
fn let_with_nat_annotation() {
    let source = "## Main\nLet count: Nat be 10.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse: {:?}", result);
    let rust = result.unwrap();
    // Spec §10.6.1: Nat → u64 (not usize, which varies by platform)
    assert!(rust.contains("let count: u64 = 10;"), "Should emit u64: {}", rust);
}

#[test]
fn let_with_text_annotation() {
    let source = "## Main\nLet name: Text be \"Alice\".";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("String"), "Should emit String type: {}", rust);
}

#[test]
fn let_with_bool_annotation() {
    let source = "## Main\nLet flag: Bool be true.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("let flag: bool = true;"), "Should emit bool: {}", rust);
}

#[test]
fn let_without_annotation_still_works() {
    let source = "## Main\nLet x be 5.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse untyped let: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("let x = 5;"), "Should emit untyped: {}", rust);
}

// =============================================================================
// Test 2: Generic Type Annotations
// =============================================================================

#[test]
fn let_with_list_of_int() {
    let source = "## Main\nLet xs: List of Int be empty.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse generic: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("Vec<i64>"), "Should emit Vec<i64>: {}", rust);
}

#[test]
fn let_with_option_of_text() {
    let source = "## Main\nLet maybe: Option of Text be nothing.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse Option: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("Option<String>"), "Should emit Option<String>: {}", rust);
}

#[test]
fn let_with_nested_generic() {
    // "List of List of Int" should parse as List<List<Int>> (right-associative)
    let source = "## Main\nLet matrix: List of List of Int be empty.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse nested generic: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("Vec<Vec<i64>>"), "Should emit Vec<Vec<i64>>: {}", rust);
}

#[test]
fn let_with_result_two_params() {
    // "Result of Int and Text" should parse as Result<i64, String>
    let source = "## Main\nLet res: Result of Int and Text be nothing.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse multi-param generic: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("Result<i64, String>"), "Should emit Result<i64, String>: {}", rust);
}

// =============================================================================
// Test 3: Mutable with Type Annotation
// =============================================================================

#[test]
fn mutable_with_int_annotation() {
    let source = "## Main\nLet mutable counter: Int be 0.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse mutable typed: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("let mut counter: i64 = 0;"), "Should emit mut typed: {}", rust);
}

#[test]
fn mutable_with_list_annotation() {
    // Note: "items" is reserved for slice syntax, use "values" instead
    let source = "## Main\nLet mutable values: List of Text be empty.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse mutable generic: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("let mut values: Vec<String>"), "Should emit mut Vec: {}", rust);
}

// =============================================================================
// Test 4: Maybe as Dual Syntax for Option
// =============================================================================

#[test]
fn maybe_of_int_annotation() {
    let source = "## Main\nLet x: Maybe of Int be nothing.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse Maybe of Int: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("Option<i64>"), "Maybe of Int should emit Option<i64>: {}", rust);
}

#[test]
fn maybe_int_direct_syntax() {
    let source = "## Main\nLet x: Maybe Int be nothing.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse Maybe Int (no 'of'): {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("Option<i64>"), "Maybe Int should emit Option<i64>: {}", rust);
}

#[test]
fn maybe_text_direct_syntax() {
    let source = "## Main\nLet x: Maybe Text be nothing.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse Maybe Text: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("Option<String>"), "Maybe Text should emit Option<String>: {}", rust);
}

#[test]
fn maybe_as_return_type() {
    let source = r#"## To f () -> Maybe Int:
    Return some 5.

## Main
Show f().
"#;
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse Maybe Int return type: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("Option<i64>"), "Maybe return type should emit Option<i64>: {}", rust);
}

#[test]
fn maybe_nested_generic() {
    let source = "## Main\nLet x: Maybe List of Int be nothing.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse Maybe List of Int: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("Option<Vec<i64>>"), "Maybe List of Int should emit Option<Vec<i64>>: {}", rust);
}

#[test]
fn e2e_maybe_some_value() {
    let source = r#"## To f () -> Maybe Int:
    Return some 42.

## Main
Show f().
"#;
    common::assert_exact_output(source, "42");
}

#[test]
fn e2e_maybe_nothing() {
    let source = r#"## To f () -> Maybe Int:
    Return none.

## Main
Show f().
"#;
    common::assert_exact_output(source, "nothing");
}
