//! Phase 83: The Vernacular (Command Interface)
//!
//! Teaches the Kernel to speak text.
//! - Definition: name : T := v
//! - Check: query type
//! - Eval: compute/normalize

use logicaffeine_kernel::interface::Repl;

// =============================================================================
// DEFINITION TESTS
// =============================================================================

#[test]
fn test_definition_command() {
    let mut repl = Repl::new();

    // Define: one : Nat := Succ Zero
    let result = repl.execute("Definition one : Nat := Succ Zero.");
    assert!(result.is_ok(), "Definition should parse: {:?}", result);
}

#[test]
fn test_definition_without_type() {
    let mut repl = Repl::new();

    // Definition with inferred type
    let result = repl.execute("Definition two := Succ (Succ Zero).");
    assert!(result.is_ok(), "Definition without type annotation should work");
}

// =============================================================================
// CHECK TESTS
// =============================================================================

#[test]
fn test_check_constructor() {
    let mut repl = Repl::new();

    let output = repl.execute("Check Zero.").expect("Check should work");
    assert_eq!(output, "Zero : Nat");
}

#[test]
fn test_check_application() {
    let mut repl = Repl::new();

    let output = repl.execute("Check (Succ Zero).").expect("Check should work");
    assert_eq!(output, "(Succ Zero) : Nat");
}

#[test]
fn test_check_definition() {
    let mut repl = Repl::new();

    repl.execute("Definition one : Nat := Succ Zero.").unwrap();
    let output = repl.execute("Check one.").expect("Check should work");
    assert_eq!(output, "one : Nat");
}

// =============================================================================
// EVAL TESTS
// =============================================================================

#[test]
fn test_eval_constructor() {
    let mut repl = Repl::new();

    let output = repl.execute("Eval Zero.").expect("Eval should work");
    assert_eq!(output, "Zero");
}

#[test]
fn test_eval_definition() {
    let mut repl = Repl::new();

    repl.execute("Definition two : Nat := Succ (Succ Zero).").unwrap();
    let output = repl.execute("Eval two.").expect("Eval should work");
    // Definition unfolds via delta reduction
    assert_eq!(output, "(Succ (Succ Zero))");
}

#[test]
fn test_eval_nested_application() {
    let mut repl = Repl::new();

    let output = repl
        .execute("Eval (Succ (Succ (Succ Zero))).")
        .expect("Eval should work");
    assert_eq!(output, "(Succ (Succ (Succ Zero)))");
}

// =============================================================================
// LAMBDA TESTS
// =============================================================================

#[test]
fn test_definition_with_lambda() {
    let mut repl = Repl::new();

    // Define increment function
    let result = repl.execute("Definition inc : Nat -> Nat := fun n : Nat => Succ n.");
    assert!(result.is_ok(), "Lambda definition should parse: {:?}", result);
}

#[test]
fn test_eval_lambda_application() {
    let mut repl = Repl::new();

    repl.execute("Definition inc : Nat -> Nat := fun n : Nat => Succ n.")
        .unwrap();
    let output = repl.execute("Eval (inc Zero).").expect("Eval should work");
    // Beta reduces: (fun n => Succ n) Zero => Succ Zero
    assert_eq!(output, "(Succ Zero)");
}

// =============================================================================
// TERM PARSING TESTS
// =============================================================================

#[test]
fn test_parse_arrow_type() {
    let mut repl = Repl::new();

    let output = repl.execute("Check Succ.").expect("Check should work");
    // Succ : Nat -> Nat (non-dependent Pi with arrow notation)
    assert_eq!(output, "Succ : Nat -> Nat");
}

#[test]
fn test_parse_nested_application() {
    let mut repl = Repl::new();

    // Nested applications: (Succ (Succ Zero))
    let output = repl
        .execute("Eval (Succ (Succ Zero)).")
        .expect("Eval should work");
    assert_eq!(output, "(Succ (Succ Zero))");
}

// =============================================================================
// INDUCTIVE TESTS
// =============================================================================

#[test]
fn test_inductive_explicit_constructors() {
    let mut repl = Repl::new();

    // Define MyBool with explicit constructor types
    // Using MyBool/Yes/No to avoid collision with StandardLibrary's True/False
    let cmd = "Inductive MyBool := Yes : MyBool | No : MyBool.";
    assert!(repl.execute(cmd).is_ok(), "Inductive should parse");

    // Check constructor type
    let output = repl.execute("Check Yes.").expect("Check should work");
    assert_eq!(output, "Yes : MyBool");
}

#[test]
fn test_inductive_with_arguments() {
    let mut repl = Repl::new();

    // Define custom Nat (separate from StandardLibrary)
    let cmd = "Inductive MyNat := MZero : MyNat | MSucc : MyNat -> MyNat.";
    assert!(repl.execute(cmd).is_ok(), "Inductive with args should parse");

    // Check constructor types
    let zero_out = repl.execute("Check MZero.").expect("Check MZero");
    assert_eq!(zero_out, "MZero : MyNat");

    let succ_out = repl.execute("Check MSucc.").expect("Check MSucc");
    // Succ has function type
    assert!(succ_out.contains("MSucc :"), "MSucc should have type");
}

// =============================================================================
// ERROR HANDLING TESTS
// =============================================================================

#[test]
fn test_undefined_name_error() {
    let mut repl = Repl::new();

    let result = repl.execute("Check undefined_name.");
    assert!(result.is_err(), "Should error on undefined name");
}

#[test]
fn test_type_error() {
    let mut repl = Repl::new();

    // Succ expects Nat, not a type
    let result = repl.execute("Check (Succ Nat).");
    assert!(result.is_err(), "Should error on type mismatch");
}
