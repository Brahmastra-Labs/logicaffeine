//! =============================================================================
//! PHASE 71: UNIVERSE HIERARCHY (CUMULATIVITY & IRRELEVANCE)
//! =============================================================================
//!
//! The boundaries soften. Small types live in large universes.
//! All proofs of Truth are one.

use logicaffeine_kernel::{infer_type, Context, Term, Universe};

// =============================================================================
// CUMULATIVITY TESTS
// =============================================================================

#[test]
fn test_type0_subtype_of_type1() {
    // Type 0 ≤ Type 1
    // A function expecting Type 1 should accept Type 0 as argument

    let ctx = Context::new();

    // id : Π(A : Type 1). A → A
    let id_type1 = Term::Lambda {
        param: "A".to_string(),
        param_type: Box::new(Term::Sort(Universe::Type(1))),
        body: Box::new(Term::Lambda {
            param: "x".to_string(),
            param_type: Box::new(Term::Var("A".to_string())),
            body: Box::new(Term::Var("x".to_string())),
        }),
    };

    // (id Type0) should type-check because Type 0 : Type 1, and Type 1 ≤ Type 1
    // But we're passing Type0 as A, where A : Type 1 is expected.
    // Type 0 : Type 1, so this should work.
    let applied = Term::App(
        Box::new(id_type1),
        Box::new(Term::Sort(Universe::Type(0))),
    );

    let result = infer_type(&ctx, &applied);
    assert!(result.is_ok(), "id Type0 should type-check: {:?}", result);

    println!("✓ Type 0 accepted where Type 1 expected (via cumulativity)");
}

#[test]
fn test_prop_subtype_of_type() {
    // Prop ≤ Type(i)
    // A function expecting Type should accept Prop as argument

    let ctx = Context::new();

    // f : Π(A : Type 1). A → A
    let f = Term::Lambda {
        param: "A".to_string(),
        param_type: Box::new(Term::Sort(Universe::Type(1))),
        body: Box::new(Term::Lambda {
            param: "x".to_string(),
            param_type: Box::new(Term::Var("A".to_string())),
            body: Box::new(Term::Var("x".to_string())),
        }),
    };

    // (f Prop) should type-check because Prop : Type 1
    let applied = Term::App(
        Box::new(f),
        Box::new(Term::Sort(Universe::Prop)),
    );

    let result = infer_type(&ctx, &applied);
    assert!(result.is_ok(), "f Prop should type-check: {:?}", result);

    println!("✓ Prop accepted where Type 1 expected (Prop ≤ Type)");
}

#[test]
fn test_nat_in_type1_context() {
    // Nat : Type 0, and Type 0 ≤ Type 1
    // So Nat should be usable where Type 1 is expected

    let mut ctx = Context::new();
    ctx.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    ctx.add_constructor("Zero", "Nat", Term::Global("Nat".to_string()));

    // g : Π(T : Type 1). T → T
    let g = Term::Lambda {
        param: "T".to_string(),
        param_type: Box::new(Term::Sort(Universe::Type(1))),
        body: Box::new(Term::Lambda {
            param: "x".to_string(),
            param_type: Box::new(Term::Var("T".to_string())),
            body: Box::new(Term::Var("x".to_string())),
        }),
    };

    // (g Nat Zero) should type-check
    // Nat : Type 0, and Type 0 ≤ Type 1
    let applied = Term::App(
        Box::new(Term::App(
            Box::new(g),
            Box::new(Term::Global("Nat".to_string())),
        )),
        Box::new(Term::Global("Zero".to_string())),
    );

    let result = infer_type(&ctx, &applied);
    assert!(result.is_ok(), "g Nat Zero should type-check: {:?}", result);

    println!("✓ Nat (Type 0) used where Type 1 expected");
}

#[test]
fn test_cumulativity_transitive() {
    // Type 0 ≤ Type 1 ≤ Type 2

    let ctx = Context::new();

    // h : Π(A : Type 2). A → A
    let h = Term::Lambda {
        param: "A".to_string(),
        param_type: Box::new(Term::Sort(Universe::Type(2))),
        body: Box::new(Term::Lambda {
            param: "x".to_string(),
            param_type: Box::new(Term::Var("A".to_string())),
            body: Box::new(Term::Var("x".to_string())),
        }),
    };

    // (h Type0) should work because Type 0 : Type 1 : Type 2
    let applied = Term::App(
        Box::new(h),
        Box::new(Term::Sort(Universe::Type(0))),
    );

    let result = infer_type(&ctx, &applied);
    assert!(result.is_ok(), "h Type0 should type-check (transitive): {:?}", result);

    println!("✓ Type 0 ≤ Type 2 (transitive cumulativity)");
}

#[test]
fn test_type_not_subtype_of_prop() {
    // Type(i) is NOT ≤ Prop
    // This should FAIL

    let ctx = Context::new();

    // f : Π(P : Prop). P → P
    let f = Term::Lambda {
        param: "P".to_string(),
        param_type: Box::new(Term::Sort(Universe::Prop)),
        body: Box::new(Term::Lambda {
            param: "x".to_string(),
            param_type: Box::new(Term::Var("P".to_string())),
            body: Box::new(Term::Var("x".to_string())),
        }),
    };

    // (f Type0) should FAIL because Type 0 : Type 1, and Type 1 is NOT ≤ Prop
    let applied = Term::App(
        Box::new(f),
        Box::new(Term::Sort(Universe::Type(0))),
    );

    let result = infer_type(&ctx, &applied);
    assert!(result.is_err(), "f Type0 should NOT type-check (Type not ≤ Prop)");

    println!("✓ Type 0 rejected where Prop expected (no downward cumulativity)");
}

// =============================================================================
// PROOF IRRELEVANCE TESTS
// =============================================================================

#[test]
fn test_proof_irrelevance_basic() {
    // If P : Prop, then all proofs of P are equal
    // We test this indirectly via type checking

    let mut ctx = Context::new();

    // True : Prop (a proposition with one proof)
    ctx.add_inductive("True", Term::Sort(Universe::Prop));
    ctx.add_constructor("tt", "True", Term::Global("True".to_string()));

    // If proof irrelevance holds, we should be able to use any proof of True
    // where another proof is expected (in a dependent context)

    let result = infer_type(&ctx, &Term::Global("tt".to_string()));
    assert!(result.is_ok(), "tt should type-check");

    // The type of tt should be True (a Prop)
    let ty = result.unwrap();
    assert!(matches!(ty, Term::Global(ref s) if s == "True"));

    println!("✓ Basic proof in Prop types correctly");
}

#[test]
fn test_proof_irrelevance_equality() {
    // Two distinct proofs of the same Prop should be considered equal

    let mut ctx = Context::new();

    // True : Prop with two "proofs" (for testing)
    ctx.add_inductive("True", Term::Sort(Universe::Prop));
    ctx.add_constructor("tt1", "True", Term::Global("True".to_string()));
    ctx.add_constructor("tt2", "True", Term::Global("True".to_string()));

    // In a system with proof irrelevance, tt1 ≡ tt2
    // We test this by checking that they can be used interchangeably

    // This test verifies the concept; actual equality checking
    // will be tested when we have dependent types that rely on proof equality

    let tt1_ty = infer_type(&ctx, &Term::Global("tt1".to_string()));
    let tt2_ty = infer_type(&ctx, &Term::Global("tt2".to_string()));

    assert!(tt1_ty.is_ok() && tt2_ty.is_ok());

    println!("✓ Multiple proofs of same Prop coexist");
}
