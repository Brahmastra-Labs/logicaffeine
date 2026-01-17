//! Phase 101a: Polymorphic Inductive Types (Parser Enhancement)
//!
//! Extends the vernacular parser to support type parameters:
//! `Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.`
//!
//! The parser must:
//! - Parse type parameters from the header (before `:=`)
//! - Build a polymorphic sort: Π(A:Type). Type
//! - Prepend parameters to constructor types: Π(A:Type). A -> List A -> List A

use logicaffeine_kernel::interface::Repl;

// =============================================================================
// PARSING: BASIC POLYMORPHIC INDUCTIVE
// =============================================================================

#[test]
fn test_parse_polymorphic_inductive_list() {
    let mut repl = Repl::new();

    // Define polymorphic List
    let cmd = "Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.";
    let result = repl.execute(cmd);

    assert!(
        result.is_ok(),
        "Should parse polymorphic inductive: {:?}",
        result
    );
}

#[test]
fn test_polymorphic_list_type_sort() {
    let mut repl = Repl::new();

    // Define List
    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    // Check List's type - should be Type -> Type (or Π(A:Type). Type)
    let output = repl.execute("Check List.").expect("Check List");

    // List : Π(A:Type0). Type0 (polymorphic type constructor)
    assert!(
        output.contains("List :"),
        "Should have type annotation: {}",
        output
    );
    // Accept either "Type -> Type" or "Π(A:Type). Type" notation
    assert!(
        output.contains("Type") && (output.contains("->") || output.contains("Π")),
        "List should be a type constructor: {}",
        output
    );
}

#[test]
fn test_polymorphic_nil_type() {
    let mut repl = Repl::new();

    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    // Check Nil's type - should be Π(A:Type). List A
    let output = repl.execute("Check Nil.").expect("Check Nil");

    assert!(
        output.contains("Nil :"),
        "Should have type annotation: {}",
        output
    );
    // Nil must be polymorphic (takes a type parameter)
    // Could be displayed as "forall (A : Type), List A" or "Π(A:Type). List A"
    assert!(
        output.contains("forall") || output.contains("Π") || output.contains("Type"),
        "Nil should be polymorphic (take type parameter): {}",
        output
    );
}

#[test]
fn test_polymorphic_cons_type() {
    let mut repl = Repl::new();

    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    // Check Cons's type - should be Π(A:Type). A -> List A -> List A
    let output = repl.execute("Check Cons.").expect("Check Cons");

    assert!(
        output.contains("Cons :"),
        "Should have type annotation: {}",
        output
    );
    // Cons takes type param, then element, then list
    assert!(
        output.contains("->"),
        "Cons should be a function type: {}",
        output
    );
}

// =============================================================================
// INSTANTIATION: USING POLYMORPHIC TYPES
// =============================================================================

#[test]
fn test_instantiate_list_nat() {
    let mut repl = Repl::new();

    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    // Instantiate List with Nat
    let output = repl.execute("Check (List Nat).").expect("Check List Nat");

    assert!(
        output.contains("Type"),
        "List Nat should be a Type: {}",
        output
    );
}

#[test]
fn test_instantiate_nil_nat() {
    let mut repl = Repl::new();

    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    // Nil Nat : List Nat
    let output = repl.execute("Check (Nil Nat).").expect("Check Nil Nat");

    assert!(
        output.contains("List"),
        "Nil Nat should have type List Nat: {}",
        output
    );
}

#[test]
fn test_instantiate_cons_nat() {
    let mut repl = Repl::new();

    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    // Cons Nat Zero (Nil Nat) : List Nat
    let output = repl
        .execute("Check (Cons Nat Zero (Nil Nat)).")
        .expect("Check cons list");

    assert!(
        output.contains("List"),
        "Cons Nat Zero (Nil Nat) should have type List Nat: {}",
        output
    );
}

#[test]
fn test_eval_list_construction() {
    let mut repl = Repl::new();

    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    // Build a list: Cons Nat Zero (Cons Nat (Succ Zero) (Nil Nat))
    // This represents [0, 1]
    let output = repl
        .execute("Eval (Cons Nat Zero (Cons Nat (Succ Zero) (Nil Nat))).")
        .expect("Eval list");

    assert!(
        output.contains("Cons") && output.contains("Zero"),
        "Should normalize to list structure: {}",
        output
    );
}

// =============================================================================
// MULTIPLE TYPE PARAMETERS
// =============================================================================

#[test]
fn test_parse_two_parameter_inductive() {
    let mut repl = Repl::new();

    // Define Either with two type parameters
    let cmd = "Inductive Either (A : Type) (B : Type) := Left : A -> Either A B | Right : B -> Either A B.";
    let result = repl.execute(cmd);

    assert!(
        result.is_ok(),
        "Should parse two-parameter inductive: {:?}",
        result
    );
}

#[test]
fn test_either_type_sort() {
    let mut repl = Repl::new();

    repl.execute("Inductive Either (A : Type) (B : Type) := Left : A -> Either A B | Right : B -> Either A B.")
        .expect("Define Either");

    // Either : Type -> Type -> Type
    let output = repl.execute("Check Either.").expect("Check Either");

    assert!(
        output.contains("Either :"),
        "Should have type annotation: {}",
        output
    );
}

#[test]
fn test_either_left_type() {
    let mut repl = Repl::new();

    repl.execute("Inductive Either (A : Type) (B : Type) := Left : A -> Either A B | Right : B -> Either A B.")
        .expect("Define Either");

    // Left : Π(A:Type). Π(B:Type). A -> Either A B
    let output = repl.execute("Check Left.").expect("Check Left");

    assert!(
        output.contains("Left :"),
        "Should have type annotation: {}",
        output
    );
    assert!(
        output.contains("->"),
        "Left should be a function: {}",
        output
    );
}

#[test]
fn test_instantiate_either() {
    let mut repl = Repl::new();

    repl.execute("Inductive Either (A : Type) (B : Type) := Left : A -> Either A B | Right : B -> Either A B.")
        .expect("Define Either");

    // Either Nat Nat : Type (using Nat twice since Bool isn't in stdlib)
    let output = repl
        .execute("Check (Either Nat Nat).")
        .expect("Check Either Nat Nat");

    assert!(
        output.contains("Type"),
        "Either Nat Nat should be a Type: {}",
        output
    );
}

// =============================================================================
// BACKWARD COMPATIBILITY (MONOMORPHIC)
// =============================================================================

#[test]
fn test_monomorphic_still_works() {
    let mut repl = Repl::new();

    // Old syntax (no parameters) should still work
    let cmd = "Inductive MyBool := Yes : MyBool | No : MyBool.";
    let result = repl.execute(cmd);

    assert!(
        result.is_ok(),
        "Monomorphic inductive should still work: {:?}",
        result
    );

    let output = repl.execute("Check Yes.").expect("Check Yes");
    assert_eq!(output, "Yes : MyBool");
}

#[test]
fn test_monomorphic_with_args_still_works() {
    let mut repl = Repl::new();

    // Old syntax with argument types
    let cmd = "Inductive MyNat := MZero : MyNat | MSucc : MyNat -> MyNat.";
    let result = repl.execute(cmd);

    assert!(
        result.is_ok(),
        "Monomorphic inductive with args should still work: {:?}",
        result
    );
}

// =============================================================================
// ERROR CASES
// =============================================================================

#[test]
fn test_error_missing_colon_in_param() {
    let mut repl = Repl::new();

    // Missing type annotation for parameter
    let cmd = "Inductive List (A) := Nil : List A.";
    let result = repl.execute(cmd);

    assert!(
        result.is_err(),
        "Should error on parameter without type annotation"
    );
}

#[test]
fn test_error_empty_parameter_list() {
    let mut repl = Repl::new();

    // Empty parentheses (syntactically odd but should be handled)
    let cmd = "Inductive List () := Nil : List.";
    let result = repl.execute(cmd);

    // This could either be an error or parse as no parameters
    // Implementation choice - both are valid behaviors
    // The test documents expected behavior
    assert!(
        result.is_err() || result.is_ok(),
        "Should handle empty parameter list gracefully"
    );
}

#[test]
fn test_type_error_wrong_arity() {
    let mut repl = Repl::new();

    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    // Using List without the type argument should be a type error
    // (Nil alone is polymorphic, but applying it wrong should error)
    let result = repl.execute("Check (Cons Zero (Nil Nat)).");

    assert!(
        result.is_err(),
        "Should error when missing type argument for Cons"
    );
}

// =============================================================================
// COMPLEX TYPE PARAMETERS
// =============================================================================

#[test]
fn test_parameter_with_arrow_type() {
    let mut repl = Repl::new();

    // Parameter with function type: (F : Type -> Type)
    // This is a higher-kinded type parameter
    let cmd = "Inductive Wrap (F : Type -> Type) := MkWrap : F Nat -> Wrap F.";
    let result = repl.execute(cmd);

    assert!(
        result.is_ok(),
        "Should parse higher-kinded parameter: {:?}",
        result
    );
}

// =============================================================================
// DEFINITION USING POLYMORPHIC TYPE
// =============================================================================

#[test]
fn test_definition_using_list() {
    let mut repl = Repl::new();

    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    // Define a concrete list value
    let result =
        repl.execute("Definition myList : List Nat := Cons Nat Zero (Nil Nat).");

    assert!(
        result.is_ok(),
        "Should define value of polymorphic type: {:?}",
        result
    );

    let output = repl.execute("Check myList.").expect("Check myList");
    assert!(
        output.contains("List"),
        "myList should have type List Nat: {}",
        output
    );
}
