//! Phase 86: Kernel Primitives (The Verifier Upgrade)
//!
//! Teaches the Kernel to speak i64 natively.
//! - Term::Lit for literal values
//! - Int, Float, Text as opaque types
//! - Primitive arithmetic via ALU

use logos::interface::Repl;

// =============================================================================
// LITERAL PARSING & TYPE INFERENCE
// =============================================================================

#[test]
fn test_check_integer_literal() {
    let mut repl = Repl::new();

    // Small integer
    let result = repl.execute("Check 42.").expect("Check 42");
    assert_eq!(result, "42 : Int");
}

#[test]
fn test_check_large_integer() {
    let mut repl = Repl::new();

    // Large integer (would crash with Peano)
    let result = repl.execute("Check 123456789.").expect("Check large");
    assert_eq!(result, "123456789 : Int");
}

#[test]
fn test_check_negative_integer() {
    let mut repl = Repl::new();

    // Negative integer
    let result = repl.execute("Check -100.").expect("Check negative");
    assert_eq!(result, "-100 : Int");
}

// =============================================================================
// PRIMITIVE OPERATIONS
// =============================================================================

#[test]
fn test_primitive_add() {
    let mut repl = Repl::new();

    // Define using add builtin
    repl.execute("Definition sum : Int := add 2 3.").expect("Define sum");

    let result = repl.execute("Eval sum.").expect("Eval sum");
    assert_eq!(result, "5");
}

#[test]
fn test_primitive_mul() {
    let mut repl = Repl::new();

    // Large multiplication (instant via ALU)
    repl.execute("Definition big : Int := mul 10000 10000.").expect("Define big");

    let result = repl.execute("Eval big.").expect("Eval big");
    assert_eq!(result, "100000000");
}

#[test]
fn test_primitive_sub() {
    let mut repl = Repl::new();

    repl.execute("Definition diff : Int := sub 100 42.").expect("Define diff");

    let result = repl.execute("Eval diff.").expect("Eval diff");
    assert_eq!(result, "58");
}

#[test]
fn test_primitive_div() {
    let mut repl = Repl::new();

    repl.execute("Definition quot : Int := div 100 7.").expect("Define quot");

    let result = repl.execute("Eval quot.").expect("Eval quot");
    assert_eq!(result, "14"); // Integer division
}

#[test]
fn test_primitive_mod() {
    let mut repl = Repl::new();

    repl.execute("Definition rem : Int := mod 100 7.").expect("Define rem");

    let result = repl.execute("Eval rem.").expect("Eval rem");
    assert_eq!(result, "2");
}

// =============================================================================
// NESTED OPERATIONS
// =============================================================================

#[test]
fn test_nested_operations() {
    let mut repl = Repl::new();

    // (3 + 4) * 2 = 14
    repl.execute("Definition nested : Int := mul (add 3 4) 2.").expect("Define nested");

    let result = repl.execute("Eval nested.").expect("Eval nested");
    assert_eq!(result, "14");
}

#[test]
fn test_complex_expression() {
    let mut repl = Repl::new();

    // ((100 - 10) * 2) + 5 = 185
    repl.execute("Definition complex : Int := add (mul (sub 100 10) 2) 5.").expect("Define");

    let result = repl.execute("Eval complex.").expect("Eval");
    assert_eq!(result, "185");
}

// =============================================================================
// FUNCTION WITH PRIMITIVES
// =============================================================================

#[test]
fn test_function_with_int() {
    let mut repl = Repl::new();

    // Define a function that doubles an Int
    repl.execute("Definition double : Int -> Int := fun n : Int => add n n.")
        .expect("Define double");

    repl.execute("Definition four : Int := double 2.").expect("Define four");

    let result = repl.execute("Eval four.").expect("Eval four");
    assert_eq!(result, "4");
}

#[test]
fn test_function_with_two_ints() {
    let mut repl = Repl::new();

    // A function taking two Ints
    repl.execute("Definition add3 : Int -> Int -> Int := fun a : Int => fun b : Int => add (add a b) 1.")
        .expect("Define add3");

    repl.execute("Definition result : Int := add3 10 20.").expect("Define result");

    let result = repl.execute("Eval result.").expect("Eval result");
    assert_eq!(result, "31");
}

// =============================================================================
// TYPE CHECKING
// =============================================================================

#[test]
fn test_int_type_exists() {
    let mut repl = Repl::new();

    // Int should be a Type
    let result = repl.execute("Check Int.").expect("Check Int");
    assert_eq!(result, "Int : Type0");
}

#[test]
fn test_add_type() {
    let mut repl = Repl::new();

    // add : Int -> Int -> Int
    let result = repl.execute("Check add.").expect("Check add");
    assert_eq!(result, "add : Int -> Int -> Int");
}

#[test]
fn test_partial_application() {
    let mut repl = Repl::new();

    // (add 5) : Int -> Int
    let result = repl.execute("Check (add 5).").expect("Check partial");
    assert_eq!(result, "(add 5) : Int -> Int");
}

// =============================================================================
// ERROR CASES
// =============================================================================

#[test]
fn test_type_error_add_nat() {
    let mut repl = Repl::new();

    // Mixing Int and Nat should fail
    let result = repl.execute("Check (add Zero 1).");
    assert!(result.is_err(), "Should reject mixing Nat and Int");
}

// =============================================================================
// PERFORMANCE (INSTANT)
// =============================================================================

#[test]
fn test_instant_large_computation() {
    let mut repl = Repl::new();

    // This must complete instantly, not hang
    // 1000000 * 1000000 = 1000000000000
    repl.execute("Definition trillion : Int := mul 1000000 1000000.").expect("Define");

    let result = repl.execute("Eval trillion.").expect("Eval");
    assert_eq!(result, "1000000000000");
}
