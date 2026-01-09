// =============================================================================
// PHASE 69: THE KERNEL (CALCULUS OF CONSTRUCTIONS)
// =============================================================================
//
// "The True Core"
//
// In CIC: Proofs are Programs, Formulas are Types.
// The distinction between ProofExpr and ProofTerm is an illusion.
//
// This is the foundation. Everything is a Term:
// - Types are Terms
// - Values are Terms
// - Functions are Terms
// - Proofs are Terms

use logos::kernel::{infer_type, Context, Term, Universe};

// =============================================================================
// UNIVERSE HIERARCHY
// =============================================================================

#[test]
fn test_sort_hierarchy() {
    // Type 0 : Type 1
    // Type 1 : Type 2
    // The infinite tower of universes

    let ctx = Context::new();

    // Type 0 : Type 1
    let type0 = Term::Sort(Universe::Type(0));
    let result = infer_type(&ctx, &type0);

    assert!(result.is_ok(), "Type 0 should be well-typed: {:?}", result);
    let ty = result.unwrap();
    assert!(
        matches!(ty, Term::Sort(Universe::Type(1))),
        "Type 0 should have type Type 1, got: {}",
        ty
    );

    println!("✓ Type 0 : Type 1");

    // Type 1 : Type 2
    let type1 = Term::Sort(Universe::Type(1));
    let result = infer_type(&ctx, &type1);
    assert!(result.is_ok());
    let ty = result.unwrap();
    assert!(matches!(ty, Term::Sort(Universe::Type(2))));

    println!("✓ Type 1 : Type 2");
}

#[test]
fn test_prop_universe() {
    // Prop : Type 1
    let ctx = Context::new();

    let prop = Term::Sort(Universe::Prop);
    let result = infer_type(&ctx, &prop);

    assert!(result.is_ok());
    let ty = result.unwrap();
    assert!(
        matches!(ty, Term::Sort(Universe::Type(1))),
        "Prop should have type Type 1, got: {}",
        ty
    );

    println!("✓ Prop : Type 1");
}

// =============================================================================
// IDENTITY FUNCTION - The Classic Test
// =============================================================================

#[test]
fn test_identity_function_type() {
    // The polymorphic identity: λA:Type. λx:A. x
    // Expected type: ΠA:Type. Πx:A. A

    let id_func = Term::Lambda {
        param: "A".to_string(),
        param_type: Box::new(Term::Sort(Universe::Type(0))),
        body: Box::new(Term::Lambda {
            param: "x".to_string(),
            param_type: Box::new(Term::Var("A".to_string())),
            body: Box::new(Term::Var("x".to_string())),
        }),
    };

    let ctx = Context::new();
    let result = infer_type(&ctx, &id_func);

    assert!(
        result.is_ok(),
        "Kernel should type-check identity: {:?}",
        result
    );

    let ty = result.unwrap();

    // Should be: ΠA:Type. Πx:A. A
    match &ty {
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            assert_eq!(param, "A");
            assert!(matches!(param_type.as_ref(), Term::Sort(Universe::Type(0))));

            // Inner Pi: Πx:A. A
            match body_type.as_ref() {
                Term::Pi {
                    param: inner_param,
                    param_type: inner_type,
                    body_type: return_type,
                } => {
                    assert_eq!(inner_param, "x");
                    assert!(matches!(inner_type.as_ref(), Term::Var(v) if v == "A"));
                    assert!(matches!(return_type.as_ref(), Term::Var(v) if v == "A"));
                }
                _ => panic!("Expected inner Pi, got: {:?}", body_type),
            }
        }
        _ => panic!("Expected Pi type, got: {:?}", ty),
    }

    println!("✓ Identity has type: {}", ty);
}

// =============================================================================
// APPLICATION - Dependent Type Substitution
// =============================================================================

#[test]
fn test_application_type() {
    // Given id : ΠA:Type. Πx:A. A
    // And Nat : Type
    // Then (id Nat) : Πx:Nat. Nat

    let mut ctx = Context::new();

    // Add id to context with its type
    let id_type = Term::Pi {
        param: "A".to_string(),
        param_type: Box::new(Term::Sort(Universe::Type(0))),
        body_type: Box::new(Term::Pi {
            param: "x".to_string(),
            param_type: Box::new(Term::Var("A".to_string())),
            body_type: Box::new(Term::Var("A".to_string())),
        }),
    };
    ctx.add("id", id_type);

    // Add Nat to context
    ctx.add("Nat", Term::Sort(Universe::Type(0)));

    // (id Nat)
    let app = Term::App(
        Box::new(Term::Var("id".to_string())),
        Box::new(Term::Var("Nat".to_string())),
    );

    let result = infer_type(&ctx, &app);
    assert!(result.is_ok(), "Should type (id Nat): {:?}", result);

    let ty = result.unwrap();

    // Should be: Πx:Nat. Nat
    match &ty {
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            assert!(
                matches!(param_type.as_ref(), Term::Var(v) if v == "Nat"),
                "Expected Nat as param type, got: {}",
                param_type
            );
            assert!(
                matches!(body_type.as_ref(), Term::Var(v) if v == "Nat"),
                "Expected Nat as return type, got: {}",
                body_type
            );
        }
        _ => panic!("Expected Pi type, got: {:?}", ty),
    }

    println!("✓ (id Nat) : Πx:Nat. Nat");
}

// =============================================================================
// TYPE ERRORS - The Kernel Must Reject
// =============================================================================

#[test]
fn test_type_mismatch_fails() {
    // λx:Nat. x applied to Bool value should fail

    let mut ctx = Context::new();
    ctx.add("Nat", Term::Sort(Universe::Type(0)));
    ctx.add("Bool", Term::Sort(Universe::Type(0)));
    ctx.add("true", Term::Var("Bool".to_string()));

    // λx:Nat. x
    let nat_id = Term::Lambda {
        param: "x".to_string(),
        param_type: Box::new(Term::Var("Nat".to_string())),
        body: Box::new(Term::Var("x".to_string())),
    };

    // (λx:Nat. x) true  -- type error! true : Bool, not Nat
    let bad_app = Term::App(Box::new(nat_id), Box::new(Term::Var("true".to_string())));

    let result = infer_type(&ctx, &bad_app);
    assert!(result.is_err(), "Should reject type mismatch");

    println!("✓ Correctly rejected type mismatch");
}

#[test]
fn test_unbound_variable_fails() {
    // Reference to undefined variable should fail
    let ctx = Context::new();

    let bad_term = Term::Var("undefined".to_string());
    let result = infer_type(&ctx, &bad_term);

    assert!(result.is_err(), "Should reject unbound variable");

    println!("✓ Correctly rejected unbound variable");
}

#[test]
fn test_non_function_application_fails() {
    // Applying a non-function should fail
    let mut ctx = Context::new();
    ctx.add("Nat", Term::Sort(Universe::Type(0)));
    ctx.add("zero", Term::Var("Nat".to_string()));

    // zero zero -- zero is not a function!
    let bad_app = Term::App(
        Box::new(Term::Var("zero".to_string())),
        Box::new(Term::Var("zero".to_string())),
    );

    let result = infer_type(&ctx, &bad_app);
    assert!(result.is_err(), "Should reject non-function application");

    println!("✓ Correctly rejected non-function application");
}

// =============================================================================
// PI TYPE FORMATION
// =============================================================================

#[test]
fn test_pi_type_well_formed() {
    // Π(x:Nat). Nat should be a valid type in Type 0

    let mut ctx = Context::new();
    ctx.add("Nat", Term::Sort(Universe::Type(0)));

    let pi_type = Term::Pi {
        param: "x".to_string(),
        param_type: Box::new(Term::Var("Nat".to_string())),
        body_type: Box::new(Term::Var("Nat".to_string())),
    };

    let result = infer_type(&ctx, &pi_type);
    assert!(result.is_ok(), "Pi type should be well-formed: {:?}", result);

    let ty = result.unwrap();
    // Nat : Type 0, so Π(x:Nat).Nat : Type 0
    assert!(
        matches!(ty, Term::Sort(Universe::Type(0))),
        "Π(x:Nat).Nat should have type Type 0, got: {}",
        ty
    );

    println!("✓ Π(x:Nat).Nat : Type 0");
}

#[test]
fn test_dependent_pi_type() {
    // Π(A:Type). A → A should be in Type 1
    // Because A : Type 0, and (A → A) : Type 0, but the outer Pi quantifies over Type 0

    let ctx = Context::new();

    let poly_type = Term::Pi {
        param: "A".to_string(),
        param_type: Box::new(Term::Sort(Universe::Type(0))),
        body_type: Box::new(Term::Pi {
            param: "x".to_string(),
            param_type: Box::new(Term::Var("A".to_string())),
            body_type: Box::new(Term::Var("A".to_string())),
        }),
    };

    let result = infer_type(&ctx, &poly_type);
    assert!(result.is_ok(), "Dependent Pi should be well-formed: {:?}", result);

    let ty = result.unwrap();
    // ΠA:Type₀. (A → A) : Type₁
    assert!(
        matches!(ty, Term::Sort(Universe::Type(1))),
        "ΠA:Type₀.(A→A) should have type Type 1, got: {}",
        ty
    );

    println!("✓ ΠA:Type₀.(A→A) : Type₁");
}
