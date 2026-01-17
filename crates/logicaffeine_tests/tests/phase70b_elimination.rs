//! =============================================================================
//! PHASE 70b: ELIMINATION (THE MATCH)
//! =============================================================================
//!
//! Teaching the Kernel to consume inductive types via pattern matching.
//!
//! The Match expression is the dependent eliminator for inductive types:
//! - match <discriminant> return <motive> with | <cases>
//!
//! This is the most complex typing rule in CIC.

use logicaffeine_kernel::{infer_type, Context, Term, Universe};

/// Helper to set up Nat with Zero and Succ
fn setup_nat_context() -> Context {
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

    ctx
}

#[test]
fn test_predecessor_function() {
    let ctx = setup_nat_context();
    let nat = Term::Global("Nat".to_string());

    // pred : Nat -> Nat
    // pred n = match n return (λ_. Nat) with
    //          | Zero => Zero
    //          | Succ k => k

    let motive = Term::Lambda {
        param: "_".to_string(),
        param_type: Box::new(nat.clone()),
        body: Box::new(nat.clone()),
    };

    let pred_body = Term::Match {
        discriminant: Box::new(Term::Var("n".to_string())),
        motive: Box::new(motive),
        cases: vec![
            // Case Zero: return Zero
            Term::Global("Zero".to_string()),
            // Case Succ k: return k (λk:Nat. k)
            Term::Lambda {
                param: "k".to_string(),
                param_type: Box::new(nat.clone()),
                body: Box::new(Term::Var("k".to_string())),
            },
        ],
    };

    let pred = Term::Lambda {
        param: "n".to_string(),
        param_type: Box::new(nat.clone()),
        body: Box::new(pred_body),
    };

    let result = infer_type(&ctx, &pred);
    assert!(result.is_ok(), "pred should type-check: {:?}", result);

    // Check it's Nat -> Nat (Pi type)
    let ty = result.unwrap();
    match &ty {
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            assert!(
                matches!(param_type.as_ref(), Term::Global(n) if n == "Nat"),
                "param should be Nat"
            );
            // body_type is (λ_.Nat) applied to Var("n"), but for type checking
            // we just need to verify it's well-formed
        }
        _ => panic!("Expected Pi type, got: {:?}", ty),
    }

    println!("✓ pred : Nat -> Nat");
}

#[test]
fn test_wrong_number_of_cases() {
    let ctx = setup_nat_context();
    let nat = Term::Global("Nat".to_string());

    // Missing Succ case - should fail
    let motive = Term::Lambda {
        param: "_".to_string(),
        param_type: Box::new(nat.clone()),
        body: Box::new(nat.clone()),
    };

    let bad_match = Term::Match {
        discriminant: Box::new(Term::Global("Zero".to_string())),
        motive: Box::new(motive),
        cases: vec![
            Term::Global("Zero".to_string()),
            // Missing second case!
        ],
    };

    let result = infer_type(&ctx, &bad_match);
    assert!(
        result.is_err(),
        "Should reject match with wrong number of cases"
    );

    println!("✓ Correctly rejected wrong number of cases");
}

#[test]
fn test_match_on_non_inductive() {
    let ctx = setup_nat_context();
    let nat = Term::Global("Nat".to_string());

    // Try to match on a Pi type (not an inductive)
    let func_type = Term::Pi {
        param: "x".to_string(),
        param_type: Box::new(nat.clone()),
        body_type: Box::new(nat.clone()),
    };

    let motive = Term::Lambda {
        param: "_".to_string(),
        param_type: Box::new(func_type.clone()),
        body: Box::new(nat.clone()),
    };

    let bad_match = Term::Match {
        discriminant: Box::new(func_type),
        motive: Box::new(motive),
        cases: vec![],
    };

    let result = infer_type(&ctx, &bad_match);
    assert!(result.is_err(), "Should reject match on non-inductive");

    println!("✓ Correctly rejected match on non-inductive");
}

#[test]
fn test_is_zero_function() {
    // Set up Bool in addition to Nat
    let mut ctx = setup_nat_context();

    ctx.add_inductive("Bool", Term::Sort(Universe::Type(0)));
    ctx.add_constructor("True", "Bool", Term::Global("Bool".to_string()));
    ctx.add_constructor("False", "Bool", Term::Global("Bool".to_string()));

    let nat = Term::Global("Nat".to_string());
    let bool_ty = Term::Global("Bool".to_string());

    // is_zero : Nat -> Bool
    // is_zero n = match n return (λ_. Bool) with
    //             | Zero => True
    //             | Succ _ => False

    let motive = Term::Lambda {
        param: "_".to_string(),
        param_type: Box::new(nat.clone()),
        body: Box::new(bool_ty.clone()),
    };

    let is_zero_body = Term::Match {
        discriminant: Box::new(Term::Var("n".to_string())),
        motive: Box::new(motive),
        cases: vec![
            Term::Global("True".to_string()),
            Term::Lambda {
                param: "_".to_string(),
                param_type: Box::new(nat.clone()),
                body: Box::new(Term::Global("False".to_string())),
            },
        ],
    };

    let is_zero = Term::Lambda {
        param: "n".to_string(),
        param_type: Box::new(nat.clone()),
        body: Box::new(is_zero_body),
    };

    let result = infer_type(&ctx, &is_zero);
    assert!(result.is_ok(), "is_zero should type-check: {:?}", result);

    println!("✓ is_zero : Nat -> Bool");
}

#[test]
fn test_case_type_mismatch() {
    let ctx = setup_nat_context();
    let nat = Term::Global("Nat".to_string());

    // Wrong case type: Zero case should return Nat, but we return a function
    let motive = Term::Lambda {
        param: "_".to_string(),
        param_type: Box::new(nat.clone()),
        body: Box::new(nat.clone()),
    };

    let bad_match = Term::Match {
        discriminant: Box::new(Term::Var("n".to_string())),
        motive: Box::new(motive),
        cases: vec![
            // Wrong! Zero case should have type Nat (i.e., Motive(Zero))
            // but we provide a function Nat -> Nat
            Term::Lambda {
                param: "x".to_string(),
                param_type: Box::new(nat.clone()),
                body: Box::new(Term::Var("x".to_string())),
            },
            Term::Lambda {
                param: "k".to_string(),
                param_type: Box::new(nat.clone()),
                body: Box::new(Term::Var("k".to_string())),
            },
        ],
    };

    let bad_pred = Term::Lambda {
        param: "n".to_string(),
        param_type: Box::new(nat.clone()),
        body: Box::new(bad_match),
    };

    let result = infer_type(&ctx, &bad_pred);
    assert!(result.is_err(), "Should reject case with wrong type");

    println!("✓ Correctly rejected case type mismatch");
}

#[test]
fn test_constructor_order_preserved() {
    let ctx = setup_nat_context();

    // Verify constructors are returned in registration order
    let constructors = ctx.get_constructors("Nat");

    assert_eq!(constructors.len(), 2);
    assert_eq!(constructors[0].0, "Zero");
    assert_eq!(constructors[1].0, "Succ");

    println!("✓ Constructor order preserved");
}

#[test]
fn test_match_with_identity_motive() {
    let ctx = setup_nat_context();
    let nat = Term::Global("Nat".to_string());

    // A simple function that just returns its input
    // identity n = match n return (λ_. Nat) with
    //              | Zero => Zero
    //              | Succ k => Succ k

    let motive = Term::Lambda {
        param: "_".to_string(),
        param_type: Box::new(nat.clone()),
        body: Box::new(nat.clone()),
    };

    let succ_k = Term::App(
        Box::new(Term::Global("Succ".to_string())),
        Box::new(Term::Var("k".to_string())),
    );

    let identity_body = Term::Match {
        discriminant: Box::new(Term::Var("n".to_string())),
        motive: Box::new(motive),
        cases: vec![
            Term::Global("Zero".to_string()),
            Term::Lambda {
                param: "k".to_string(),
                param_type: Box::new(nat.clone()),
                body: Box::new(succ_k),
            },
        ],
    };

    let identity = Term::Lambda {
        param: "n".to_string(),
        param_type: Box::new(nat.clone()),
        body: Box::new(identity_body),
    };

    let result = infer_type(&ctx, &identity);
    assert!(result.is_ok(), "identity should type-check: {:?}", result);

    println!("✓ identity : Nat -> Nat");
}
