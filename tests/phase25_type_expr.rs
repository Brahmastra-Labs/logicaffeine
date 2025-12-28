//! Phase 25: Type Expressions & Annotations
//!
//! This phase teaches the parser to read explicit type annotations:
//! - `Let x: Int be 5.` → `let x: i64 = 5;`
//! - `Let s: List of Int be empty.` → `let s: Vec<i64> = ...`
//!
//! Validates that TypeRegistry is working by parsing recursive generic types.

use logos::compile::compile_to_rust;

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
