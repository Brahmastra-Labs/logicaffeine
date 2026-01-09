//! =============================================================================
//! PHASE 70a: INDUCTIVE TYPES (FORMATION & INTRODUCTION)
//! =============================================================================
//!
//! Teaching the Kernel to understand:
//! - Inductive type declarations (Nat : Type)
//! - Constructor typing (Zero : Nat, Succ : Nat -> Nat)
//!
//! This is the "I" in CIC - Calculus of Inductive Constructions.

use logos::kernel::{infer_type, Context, Term, Universe};

#[test]
fn test_nat_zero_type() {
    // Define context with Nat and its constructors
    let mut ctx = Context::new();

    // Nat : Type 0
    ctx.add_inductive("Nat", Term::Sort(Universe::Type(0)));

    // Zero : Nat
    ctx.add_constructor("Zero", "Nat", Term::Global("Nat".to_string()));

    // Check that Zero has type Nat
    let zero = Term::Global("Zero".to_string());
    let result = infer_type(&ctx, &zero);

    assert!(result.is_ok(), "Zero should type-check: {:?}", result);

    let ty = result.unwrap();
    match &ty {
        Term::Global(name) => assert_eq!(name, "Nat"),
        _ => panic!("Expected Nat, got: {:?}", ty),
    }

    println!("✓ Zero : Nat");
}

#[test]
fn test_nat_succ_type() {
    let mut ctx = Context::new();

    // Nat : Type 0
    ctx.add_inductive("Nat", Term::Sort(Universe::Type(0)));

    // Zero : Nat
    ctx.add_constructor("Zero", "Nat", Term::Global("Nat".to_string()));

    // Succ : Nat -> Nat (Π n:Nat. Nat)
    ctx.add_constructor(
        "Succ",
        "Nat",
        Term::Pi {
            param: "n".to_string(),
            param_type: Box::new(Term::Global("Nat".to_string())),
            body_type: Box::new(Term::Global("Nat".to_string())),
        },
    );

    // Check that Succ has type Nat -> Nat
    let succ = Term::Global("Succ".to_string());
    let result = infer_type(&ctx, &succ);

    assert!(result.is_ok(), "Succ should type-check: {:?}", result);

    let ty = result.unwrap();
    match &ty {
        Term::Pi { param_type, body_type, .. } => {
            assert!(matches!(param_type.as_ref(), Term::Global(n) if n == "Nat"));
            assert!(matches!(body_type.as_ref(), Term::Global(n) if n == "Nat"));
        }
        _ => panic!("Expected Pi type, got: {:?}", ty),
    }

    println!("✓ Succ : Nat -> Nat");
}

#[test]
fn test_succ_succ_zero() {
    let mut ctx = Context::new();

    // Nat : Type 0
    ctx.add_inductive("Nat", Term::Sort(Universe::Type(0)));

    // Zero : Nat
    ctx.add_constructor("Zero", "Nat", Term::Global("Nat".to_string()));

    // Succ : Nat -> Nat
    ctx.add_constructor(
        "Succ",
        "Nat",
        Term::Pi {
            param: "n".to_string(),
            param_type: Box::new(Term::Global("Nat".to_string())),
            body_type: Box::new(Term::Global("Nat".to_string())),
        },
    );

    // Build: Succ(Succ(Zero)) = 2
    let zero = Term::Global("Zero".to_string());
    let one = Term::App(Box::new(Term::Global("Succ".to_string())), Box::new(zero));
    let two = Term::App(Box::new(Term::Global("Succ".to_string())), Box::new(one));

    // Type-check
    let result = infer_type(&ctx, &two);
    assert!(
        result.is_ok(),
        "Succ(Succ(Zero)) should type-check: {:?}",
        result
    );

    let ty = result.unwrap();
    match &ty {
        Term::Global(name) => assert_eq!(name, "Nat"),
        _ => panic!("Expected Nat, got: {:?}", ty),
    }

    println!("✓ Succ(Succ(Zero)) : Nat");
}

#[test]
fn test_inductive_type_has_sort() {
    let mut ctx = Context::new();

    // Nat : Type 0
    ctx.add_inductive("Nat", Term::Sort(Universe::Type(0)));

    // Check that Nat itself has type Type 0
    let nat = Term::Global("Nat".to_string());
    let result = infer_type(&ctx, &nat);

    assert!(result.is_ok(), "Nat should have a type: {:?}", result);

    let ty = result.unwrap();
    assert!(
        matches!(&ty, Term::Sort(Universe::Type(0))),
        "Nat should have type Type 0, got: {:?}",
        ty
    );

    println!("✓ Nat : Type 0");
}

#[test]
fn test_type_mismatch_with_inductives() {
    let mut ctx = Context::new();

    // Nat : Type 0
    ctx.add_inductive("Nat", Term::Sort(Universe::Type(0)));

    // Bool : Type 0
    ctx.add_inductive("Bool", Term::Sort(Universe::Type(0)));

    // Zero : Nat
    ctx.add_constructor("Zero", "Nat", Term::Global("Nat".to_string()));

    // true : Bool
    ctx.add_constructor("true", "Bool", Term::Global("Bool".to_string()));

    // nat_id : Nat -> Nat
    let nat_id = Term::Lambda {
        param: "x".to_string(),
        param_type: Box::new(Term::Global("Nat".to_string())),
        body: Box::new(Term::Var("x".to_string())),
    };

    // (nat_id true) should fail - type mismatch
    let bad_app = Term::App(
        Box::new(nat_id),
        Box::new(Term::Global("true".to_string())),
    );

    let result = infer_type(&ctx, &bad_app);
    assert!(result.is_err(), "Should reject (nat_id true): {:?}", result);

    println!("✓ Correctly rejected type mismatch with inductives");
}

#[test]
fn test_constructor_belongs_to_inductive() {
    let mut ctx = Context::new();

    // Nat : Type 0
    ctx.add_inductive("Nat", Term::Sort(Universe::Type(0)));

    // Zero : Nat
    ctx.add_constructor("Zero", "Nat", Term::Global("Nat".to_string()));

    // Verify Zero is registered as a constructor of Nat
    assert!(ctx.is_constructor("Zero"));
    assert_eq!(ctx.constructor_inductive("Zero"), Some("Nat"));

    // Nat is not a constructor
    assert!(!ctx.is_constructor("Nat"));

    println!("✓ Constructor metadata correct");
}
