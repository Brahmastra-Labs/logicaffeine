//! =============================================================================
//! PHASE 72: KERNEL PRELUDE (EQUALITY & LOGIC)
//! =============================================================================
//!
//! The Library of Truth emerges. Equality gains meaning.

use logos::kernel::prelude::StandardLibrary;
use logos::kernel::{infer_type, Context, Term, Universe};

// =============================================================================
// NATURAL NUMBERS
// =============================================================================

#[test]
fn test_nat_in_prelude() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // Nat should be registered
    let nat_type = infer_type(&ctx, &Term::Global("Nat".to_string()));
    assert!(nat_type.is_ok(), "Nat should be defined: {:?}", nat_type);

    // Zero : Nat
    let zero_type = infer_type(&ctx, &Term::Global("Zero".to_string()));
    assert!(zero_type.is_ok(), "Zero should be defined: {:?}", zero_type);

    // Succ Zero : Nat
    let one = Term::App(
        Box::new(Term::Global("Succ".to_string())),
        Box::new(Term::Global("Zero".to_string())),
    );
    let one_type = infer_type(&ctx, &one);
    assert!(one_type.is_ok(), "Succ Zero should type-check: {:?}", one_type);

    println!("✓ Nat, Zero, Succ registered in prelude");
}

// =============================================================================
// EQUALITY
// =============================================================================

#[test]
fn test_eq_type() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // Eq : Π(A:Type 0). A → A → Prop
    let eq_type = infer_type(&ctx, &Term::Global("Eq".to_string()));
    assert!(eq_type.is_ok(), "Eq should be defined: {:?}", eq_type);

    println!("✓ Eq type registered");
}

#[test]
fn test_eq_nat_zero_zero() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // Build: Eq Nat Zero Zero
    let nat = Term::Global("Nat".to_string());
    let zero = Term::Global("Zero".to_string());

    let eq_nat = Term::App(
        Box::new(Term::Global("Eq".to_string())),
        Box::new(nat.clone()),
    );
    let eq_nat_zero = Term::App(Box::new(eq_nat), Box::new(zero.clone()));
    let eq_nat_zero_zero = Term::App(Box::new(eq_nat_zero), Box::new(zero.clone()));

    // This should be a Prop
    let result = infer_type(&ctx, &eq_nat_zero_zero);
    assert!(
        result.is_ok(),
        "Eq Nat Zero Zero should type-check: {:?}",
        result
    );

    let ty = result.unwrap();
    assert!(
        matches!(ty, Term::Sort(Universe::Prop)),
        "Eq Nat Zero Zero should have type Prop, got {:?}",
        ty
    );

    println!("✓ Eq Nat Zero Zero : Prop");
}

#[test]
fn test_refl_proves_equality() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    let nat = Term::Global("Nat".to_string());
    let zero = Term::Global("Zero".to_string());

    // Build proof: refl Nat Zero
    let proof = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("refl".to_string())),
            Box::new(nat.clone()),
        )),
        Box::new(zero.clone()),
    );

    // The type of (refl Nat Zero) should be (Eq Nat Zero Zero)
    let result = infer_type(&ctx, &proof);
    assert!(
        result.is_ok(),
        "refl Nat Zero should type-check: {:?}",
        result
    );

    // Build expected type: Eq Nat Zero Zero
    let expected = Term::App(
        Box::new(Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("Eq".to_string())),
                Box::new(nat.clone()),
            )),
            Box::new(zero.clone()),
        )),
        Box::new(zero.clone()),
    );

    let inferred = result.unwrap();
    // Types should be convertible (we'll check structurally for now)
    assert_eq!(
        format!("{}", inferred),
        format!("{}", expected),
        "refl Nat Zero should have type Eq Nat Zero Zero"
    );

    println!("✓ refl Nat Zero : Eq Nat Zero Zero");
}

#[test]
fn test_refl_one_equals_one() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    let nat = Term::Global("Nat".to_string());
    let one = Term::App(
        Box::new(Term::Global("Succ".to_string())),
        Box::new(Term::Global("Zero".to_string())),
    );

    // refl Nat (Succ Zero) : Eq Nat (Succ Zero) (Succ Zero)
    let proof = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("refl".to_string())),
            Box::new(nat.clone()),
        )),
        Box::new(one.clone()),
    );

    let result = infer_type(&ctx, &proof);
    assert!(
        result.is_ok(),
        "refl Nat (Succ Zero) should type-check: {:?}",
        result
    );

    println!("✓ refl Nat (Succ Zero) : Eq Nat (Succ Zero) (Succ Zero)");
}

// =============================================================================
// PROPOSITIONAL LOGIC
// =============================================================================

#[test]
fn test_true_and_false() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // True : Prop
    let true_type = infer_type(&ctx, &Term::Global("True".to_string()));
    assert!(true_type.is_ok());
    assert!(matches!(true_type.unwrap(), Term::Sort(Universe::Prop)));

    // I : True
    let i_type = infer_type(&ctx, &Term::Global("I".to_string()));
    assert!(i_type.is_ok());

    // False : Prop
    let false_type = infer_type(&ctx, &Term::Global("False".to_string()));
    assert!(false_type.is_ok());
    assert!(matches!(false_type.unwrap(), Term::Sort(Universe::Prop)));

    println!("✓ True, I, False registered");
}

#[test]
fn test_and_type() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // And : Prop → Prop → Prop
    let and_type = infer_type(&ctx, &Term::Global("And".to_string()));
    assert!(and_type.is_ok(), "And should be defined: {:?}", and_type);

    // And True True : Prop
    let and_true_true = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("And".to_string())),
            Box::new(Term::Global("True".to_string())),
        )),
        Box::new(Term::Global("True".to_string())),
    );

    let result = infer_type(&ctx, &and_true_true);
    assert!(result.is_ok());
    assert!(matches!(result.unwrap(), Term::Sort(Universe::Prop)));

    println!("✓ And True True : Prop");
}

#[test]
fn test_or_type() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // Or : Prop → Prop → Prop
    let or_type = infer_type(&ctx, &Term::Global("Or".to_string()));
    assert!(or_type.is_ok(), "Or should be defined: {:?}", or_type);

    // Or True False : Prop
    let or_true_false = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("Or".to_string())),
            Box::new(Term::Global("True".to_string())),
        )),
        Box::new(Term::Global("False".to_string())),
    );

    let result = infer_type(&ctx, &or_true_false);
    assert!(result.is_ok());
    assert!(matches!(result.unwrap(), Term::Sort(Universe::Prop)));

    println!("✓ Or True False : Prop");
}
