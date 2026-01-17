//! Phase 84: Program Extraction (The Forge)
//!
//! Compiles verified kernel terms to executable Rust code.
//! - Inductive types → enum
//! - Fixpoints → recursive fn
//! - Pattern matching → match

use logicaffeine_compile::extraction::extract_program;
use logicaffeine_kernel::interface::Repl;

// =============================================================================
// BASIC EXTRACTION TESTS
// =============================================================================

#[test]
fn test_extract_nat_enum() {
    let mut repl = Repl::new();

    // StandardLibrary already has Nat, but we define MyNat for isolation
    repl.execute("Inductive MyNat := MZero : MyNat | MSucc : MyNat -> MyNat.")
        .expect("Define MyNat");

    let rust_code = extract_program(repl.context(), "MyNat").expect("Extract MyNat");

    // Should generate enum
    assert!(
        rust_code.contains("enum MyNat {"),
        "Should have enum declaration"
    );
    assert!(rust_code.contains("MZero,"), "Should have MZero constructor");
    assert!(
        rust_code.contains("MSucc(Box<MyNat>)"),
        "Should have MSucc with Box"
    );
}

#[test]
fn test_extract_simple_definition() {
    let mut repl = Repl::new();

    // Define a simple value
    repl.execute("Definition one : Nat := Succ Zero.")
        .expect("Define one");

    let rust_code = extract_program(repl.context(), "one").expect("Extract one");

    // Should reference Nat enum
    assert!(
        rust_code.contains("enum Nat {"),
        "Should include Nat dependency"
    );
    // Should have the definition
    assert!(rust_code.contains("Nat::Succ"), "Should use Nat::Succ");
    assert!(rust_code.contains("Nat::Zero"), "Should use Nat::Zero");
}

// =============================================================================
// FIXPOINT EXTRACTION TESTS
// =============================================================================

#[test]
fn test_extract_add_function() {
    let mut repl = Repl::new();

    // Define add using fix + nested lambdas
    // Note: motive must be a function (fun _ : Nat => ReturnType)
    let add_def = "Definition add : Nat -> Nat -> Nat := \
        fix rec => fun n : Nat => fun m : Nat => \
        match n return (fun _ : Nat => Nat) with \
        | Zero => m \
        | Succ k => Succ (rec k m).";
    repl.execute(add_def).expect("Define add");

    let rust_code = extract_program(repl.context(), "add").expect("Extract add");

    println!("Generated Rust:\n{}", rust_code);

    // Should have Nat enum
    assert!(rust_code.contains("enum Nat {"), "Should include Nat");

    // Should have add function
    assert!(rust_code.contains("fn add("), "Should have fn add");

    // Should have match
    assert!(rust_code.contains("match"), "Should have match expression");
    assert!(rust_code.contains("Nat::Zero"), "Should match Zero");
    assert!(rust_code.contains("Nat::Succ"), "Should match Succ");
}

#[test]
fn test_extract_double_function() {
    let mut repl = Repl::new();

    // add must be defined first
    let add_def = "Definition add : Nat -> Nat -> Nat := \
        fix rec => fun n : Nat => fun m : Nat => \
        match n return (fun _ : Nat => Nat) with \
        | Zero => m \
        | Succ k => Succ (rec k m).";
    repl.execute(add_def).expect("Define add");

    // double uses add
    let double_def = "Definition double : Nat -> Nat := fun n : Nat => add n n.";
    repl.execute(double_def).expect("Define double");

    let rust_code = extract_program(repl.context(), "double").expect("Extract double");

    // Should have all dependencies
    assert!(rust_code.contains("enum Nat {"), "Should include Nat");
    assert!(rust_code.contains("fn add("), "Should include add");
    assert!(rust_code.contains("fn double("), "Should have double");
}

// =============================================================================
// DEPENDENCY TESTS
// =============================================================================

#[test]
fn test_transitive_dependencies() {
    let mut repl = Repl::new();

    // Build a chain: triple -> double -> add -> Nat
    let add_def = "Definition add : Nat -> Nat -> Nat := \
        fix rec => fun n : Nat => fun m : Nat => \
        match n return (fun _ : Nat => Nat) with \
        | Zero => m \
        | Succ k => Succ (rec k m).";
    let double_def = "Definition double : Nat -> Nat := fun n : Nat => add n n.";
    let triple_def = "Definition triple : Nat -> Nat := fun n : Nat => add n (double n).";

    repl.execute(add_def).expect("Define add");
    repl.execute(double_def).expect("Define double");
    repl.execute(triple_def).expect("Define triple");

    let rust_code = extract_program(repl.context(), "triple").expect("Extract triple");

    // Should have all transitive dependencies
    assert!(rust_code.contains("enum Nat {"), "Should include Nat");
    assert!(rust_code.contains("fn add("), "Should include add");
    assert!(rust_code.contains("fn double("), "Should include double");
    assert!(rust_code.contains("fn triple("), "Should have triple");
}

// =============================================================================
// BOOL EXTRACTION TESTS
// =============================================================================

#[test]
fn test_extract_bool_enum() {
    let mut repl = Repl::new();

    repl.execute("Inductive MyBool := Yes : MyBool | No : MyBool.")
        .expect("Define MyBool");

    let rust_code = extract_program(repl.context(), "MyBool").expect("Extract MyBool");

    assert!(rust_code.contains("enum MyBool {"), "Should have enum");
    assert!(rust_code.contains("Yes,"), "Should have Yes");
    assert!(rust_code.contains("No,"), "Should have No");
    // No Box needed - not recursive
    assert!(!rust_code.contains("Box<MyBool>"), "Should not need Box");
}

#[test]
fn test_extract_is_zero() {
    let mut repl = Repl::new();

    repl.execute("Inductive MyBool := Yes : MyBool | No : MyBool.")
        .expect("Define MyBool");

    let is_zero_def = "Definition is_zero : Nat -> MyBool := \
        fun n : Nat => match n return (fun _ : Nat => MyBool) with \
        | Zero => Yes \
        | Succ k => No.";
    repl.execute(is_zero_def).expect("Define is_zero");

    let rust_code = extract_program(repl.context(), "is_zero").expect("Extract is_zero");

    assert!(rust_code.contains("enum Nat {"), "Should include Nat");
    assert!(rust_code.contains("enum MyBool {"), "Should include MyBool");
    assert!(rust_code.contains("fn is_zero("), "Should have is_zero");
}

// =============================================================================
// ERROR HANDLING TESTS
// =============================================================================

#[test]
fn test_extract_undefined_name() {
    let repl = Repl::new();

    let result = extract_program(repl.context(), "undefined_name");
    assert!(result.is_err(), "Should error on undefined name");
}

#[test]
fn test_extract_inductive_is_extractable() {
    let repl = Repl::new();

    // Nat is an inductive from StandardLibrary, so it IS extractable as an enum
    let result = extract_program(repl.context(), "Nat");
    assert!(result.is_ok(), "Inductives should be extractable");
}
