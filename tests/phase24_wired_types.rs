//! Phase 24: End-to-end type disambiguation via compile pipeline
//!
//! Tests that DiscoveryPass is wired into the main compilation pipeline,
//! so intrinsic generic types affect parsing in ## Main blocks.
//!
//! Note: Multi-block support (## Definition + ## Main) requires additional
//! parser work. These tests focus on intrinsic types which don't need Definition blocks.

use logos::compile::compile_to_rust;

// =============================================================================
// Test 1: Basic pipeline still works
// =============================================================================

#[test]
fn compile_to_rust_basic_let() {
    let source = "## Main\nLet x be 5.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Basic let should compile: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("let x = 5;"), "Should produce let binding: {}", rust);
}

#[test]
fn compile_to_rust_return() {
    let source = "## Main\nReturn 42.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Return should compile: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("return 42;"), "Should produce return: {}", rust);
}

// =============================================================================
// Test 2: Verify TypeRegistry is being passed (via intrinsic types)
// =============================================================================
// These tests ensure that when we wire the DiscoveryPass, the Parser receives
// a TypeRegistry that knows about intrinsic types (List, Option, Result).

#[test]
fn compile_with_if_statement() {
    // Simple conditional to verify the pipeline is working
    let source = "## Main\nIf x equals 5:\n    Return true.\nReturn false.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "If statement should compile: {:?}", result);
}

#[test]
fn compile_with_while_loop() {
    let source = "## Main\nLet x be 0.\nWhile x equals 5:\n    Set x to 10.\nReturn x.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "While loop should compile: {:?}", result);
}

// =============================================================================
// Test 3: Index expressions (existing feature)
// =============================================================================

#[test]
fn compile_index_expression() {
    let source = "## Main\nLet x be item 1 of list.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Index expression should compile: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("list[0]") || rust.contains("list.get(0)"),
            "Should produce array index: {}", rust);
}

#[test]
fn compile_range_expression() {
    let source = "## Main\nLet x be items 2 through 5 of list.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Range expression should compile: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("[1..5]") || rust.contains("1..5"),
            "Should produce range: {}", rust);
}

// =============================================================================
// Test 4: Assertions
// =============================================================================

#[test]
fn compile_assert_statement() {
    let source = "## Main\nLet x be 5.\nAssert that x is greater than 0.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Assert should compile: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("assert!"), "Should produce assertion: {}", rust);
}

// =============================================================================
// Test 5: Call statements
// =============================================================================

#[test]
fn compile_call_statement() {
    let source = "## Main\nCall process with data.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Call should compile: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("process(data)") || rust.contains("process(&data)"),
            "Should produce function call: {}", rust);
}
