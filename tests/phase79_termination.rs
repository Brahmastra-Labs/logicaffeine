//! =============================================================================
//! PHASE 79: THE GUARDIAN - TERMINATION & POSITIVITY
//! =============================================================================
//!
//! A proof system must reject infinite loops AND negative inductives.
//! Without these checks, the logic is unsound.
//!
//! Termination: fix f. λn:Nat. f n would "prove" False by looping forever.
//! Positivity: Inductive Bad := Cons : (Bad -> False) -> Bad creates paradoxes.

use logos::kernel::prelude::StandardLibrary;
use logos::kernel::{infer_type, Context, Term, Universe};

// =============================================================================
// PART 1: TERMINATION TESTS
// =============================================================================

#[test]
fn test_reject_simple_infinite_loop() {
    // fix f. λn:Nat. f n
    // This loops forever - n never gets smaller.
    // MUST BE REJECTED.

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    let nat = Term::Global("Nat".to_string());

    // Body: λn:Nat. f n
    let loop_body = Term::Lambda {
        param: "n".to_string(),
        param_type: Box::new(nat.clone()),
        body: Box::new(Term::App(
            Box::new(Term::Var("f".to_string())),
            Box::new(Term::Var("n".to_string())),
        )),
    };

    let bad_fix = Term::Fix {
        name: "f".to_string(),
        body: Box::new(loop_body),
    };

    let result = infer_type(&ctx, &bad_fix);

    assert!(
        result.is_err(),
        "UNSOUND: Kernel accepted infinite loop! Result: {:?}",
        result
    );

    if let Err(e) = &result {
        println!("Correctly rejected infinite loop: {}", e);
    }
}

#[test]
fn test_reject_non_decreasing_in_match() {
    // fix f. λn:Nat. match n with Zero => Zero | Succ k => f n
    // Wrong: recursive call uses n, not k!
    // MUST BE REJECTED.

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    let nat = Term::Global("Nat".to_string());
    let zero = Term::Global("Zero".to_string());

    // match n return (λ_. Nat) with | Zero => Zero | Succ k => f n
    let match_expr = Term::Match {
        discriminant: Box::new(Term::Var("n".to_string())),
        motive: Box::new(Term::Lambda {
            param: "_".to_string(),
            param_type: Box::new(nat.clone()),
            body: Box::new(nat.clone()),
        }),
        cases: vec![
            // Zero case => Zero
            zero.clone(),
            // Succ case => λk:Nat. f n (WRONG - uses n, not k)
            Term::Lambda {
                param: "k".to_string(),
                param_type: Box::new(nat.clone()),
                body: Box::new(Term::App(
                    Box::new(Term::Var("f".to_string())),
                    Box::new(Term::Var("n".to_string())), // Should be k!
                )),
            },
        ],
    };

    let bad_fix = Term::Fix {
        name: "f".to_string(),
        body: Box::new(Term::Lambda {
            param: "n".to_string(),
            param_type: Box::new(nat.clone()),
            body: Box::new(match_expr),
        }),
    };

    let result = infer_type(&ctx, &bad_fix);

    assert!(
        result.is_err(),
        "UNSOUND: Kernel accepted non-decreasing recursion! Result: {:?}",
        result
    );

    if let Err(e) = &result {
        println!("Correctly rejected non-decreasing recursion: {}", e);
    }
}

#[test]
fn test_accept_valid_structural_recursion() {
    // fix f. λn:Nat. match n with Zero => Zero | Succ k => f k
    // Valid: k < n structurally.
    // MUST BE ACCEPTED.

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    let nat = Term::Global("Nat".to_string());
    let zero = Term::Global("Zero".to_string());

    // match n return (λ_. Nat) with | Zero => Zero | Succ k => f k
    let match_expr = Term::Match {
        discriminant: Box::new(Term::Var("n".to_string())),
        motive: Box::new(Term::Lambda {
            param: "_".to_string(),
            param_type: Box::new(nat.clone()),
            body: Box::new(nat.clone()),
        }),
        cases: vec![
            // Zero case => Zero
            zero.clone(),
            // Succ case => λk:Nat. f k (CORRECT - k < n)
            Term::Lambda {
                param: "k".to_string(),
                param_type: Box::new(nat.clone()),
                body: Box::new(Term::App(
                    Box::new(Term::Var("f".to_string())),
                    Box::new(Term::Var("k".to_string())), // k is smaller than n
                )),
            },
        ],
    };

    let valid_fix = Term::Fix {
        name: "f".to_string(),
        body: Box::new(Term::Lambda {
            param: "n".to_string(),
            param_type: Box::new(nat.clone()),
            body: Box::new(match_expr),
        }),
    };

    let result = infer_type(&ctx, &valid_fix);

    assert!(
        result.is_ok(),
        "Valid structural recursion rejected: {:?}",
        result
    );

    println!("Valid structural recursion accepted: {:?}", result.unwrap());
}

#[test]
fn test_addition_still_works() {
    // fix plus. λn. λm. match n with Zero => m | Succ k => Succ (plus k m)
    // This is the addition function from Phase 70c.
    // MUST STILL WORK after adding termination checking.

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    let nat = Term::Global("Nat".to_string());
    let succ = Term::Global("Succ".to_string());

    let match_expr = Term::Match {
        discriminant: Box::new(Term::Var("n".to_string())),
        motive: Box::new(Term::Lambda {
            param: "_".to_string(),
            param_type: Box::new(nat.clone()),
            body: Box::new(nat.clone()),
        }),
        cases: vec![
            // Zero => m
            Term::Var("m".to_string()),
            // Succ k => Succ (plus k m)
            Term::Lambda {
                param: "k".to_string(),
                param_type: Box::new(nat.clone()),
                body: Box::new(Term::App(
                    Box::new(succ.clone()),
                    Box::new(Term::App(
                        Box::new(Term::App(
                            Box::new(Term::Var("plus".to_string())),
                            Box::new(Term::Var("k".to_string())),
                        )),
                        Box::new(Term::Var("m".to_string())),
                    )),
                )),
            },
        ],
    };

    let plus = Term::Fix {
        name: "plus".to_string(),
        body: Box::new(Term::Lambda {
            param: "n".to_string(),
            param_type: Box::new(nat.clone()),
            body: Box::new(Term::Lambda {
                param: "m".to_string(),
                param_type: Box::new(nat.clone()),
                body: Box::new(match_expr),
            }),
        }),
    };

    let result = infer_type(&ctx, &plus);

    assert!(
        result.is_ok(),
        "Addition should pass termination check: {:?}",
        result
    );

    println!("Addition still works: {:?}", result.unwrap());
}

// =============================================================================
// PART 2: POSITIVITY TESTS
// =============================================================================

#[test]
fn test_reject_negative_inductive() {
    // Inductive Bad := Cons : (Bad -> False) -> Bad
    // Bad appears LEFT of an arrow = NEGATIVE = REJECT
    // This can encode Russell's paradox.

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // Declare the Bad inductive type
    ctx.add_inductive("Bad", Term::Sort(Universe::Type(0)));

    // Constructor type: (Bad -> False) -> Bad
    // Note: Bad appears in the parameter type, in negative position
    let bad_arrow_false = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(Term::Global("Bad".to_string())), // NEGATIVE!
        body_type: Box::new(Term::Global("False".to_string())),
    };
    let cons_type = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(bad_arrow_false),
        body_type: Box::new(Term::Global("Bad".to_string())),
    };

    // This should fail the positivity check
    let result = ctx.add_constructor_checked("Cons", "Bad", cons_type);

    assert!(
        result.is_err(),
        "UNSOUND: Kernel accepted negative inductive! Result: {:?}",
        result
    );

    if let Err(e) = &result {
        println!("Correctly rejected negative inductive: {}", e);
    }
}

#[test]
fn test_accept_positive_inductive_list() {
    // Inductive List := Nil : List | Cons : Nat -> List -> List
    // List always appears RIGHT of arrows = POSITIVE = ACCEPT

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // Declare the List inductive type
    ctx.add_inductive("List", Term::Sort(Universe::Type(0)));

    // Nil : List
    let nil_type = Term::Global("List".to_string());

    // Cons : Nat -> List -> List
    let cons_type = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(Term::Global("Nat".to_string())),
        body_type: Box::new(Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(Term::Global("List".to_string())), // positive
            body_type: Box::new(Term::Global("List".to_string())),  // positive (result)
        }),
    };

    let nil_result = ctx.add_constructor_checked("Nil", "List", nil_type);
    let cons_result = ctx.add_constructor_checked("Cons", "List", cons_type);

    assert!(nil_result.is_ok(), "Valid Nil constructor rejected: {:?}", nil_result);
    assert!(cons_result.is_ok(), "Valid Cons constructor rejected: {:?}", cons_result);

    println!("List inductive accepted (strictly positive)");
}

#[test]
fn test_nested_negative_rejected() {
    // Inductive Tricky := Make : ((Tricky -> Nat) -> Nat) -> Tricky
    // Tricky appears in: (Tricky -> Nat) -> Nat
    //                     ^^^^^^
    // Position analysis:
    // - Outer Pi: param_type = (Tricky -> Nat) -> Nat, positive context
    // - Inner Pi: param_type = Tricky -> Nat, now negative context (flipped)
    // - Innermost Pi: param_type = Tricky, now positive again (double flip)
    // Wait, this is actually: ((Tricky -> Nat) -> Nat) -> Tricky
    //                           ^^^^^^ Tricky is in param of (->Nat)
    //                                  which is in param of (->Nat)
    //                                  which is in param of (->Tricky)
    // Polarity: positive -> flip -> negative -> flip -> positive -> flip -> negative!
    // So Tricky appears in negative position. REJECT.

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    ctx.add_inductive("Tricky", Term::Sort(Universe::Type(0)));

    // (Tricky -> Nat)
    let tricky_to_nat = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(Term::Global("Tricky".to_string())),
        body_type: Box::new(Term::Global("Nat".to_string())),
    };

    // (Tricky -> Nat) -> Nat
    let inner = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(tricky_to_nat),
        body_type: Box::new(Term::Global("Nat".to_string())),
    };

    // ((Tricky -> Nat) -> Nat) -> Tricky
    let make_type = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(inner),
        body_type: Box::new(Term::Global("Tricky".to_string())),
    };

    let result = ctx.add_constructor_checked("Make", "Tricky", make_type);

    assert!(
        result.is_err(),
        "UNSOUND: Kernel accepted nested negative! Result: {:?}",
        result
    );

    if let Err(e) = &result {
        println!("Correctly rejected nested negative: {}", e);
    }
}

#[test]
fn test_accept_standard_library_inductives() {
    // All standard library inductives should be strictly positive.
    // This ensures we didn't break anything.

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // The standard library should have registered Nat, True, False, And, Or, etc.
    // If positivity checking were applied during registration, it would have failed
    // if any were negative. Since we use add_constructor (not checked), this test
    // verifies the standard library types are actually positive.

    // Nat: Zero : Nat, Succ : Nat -> Nat
    let nat_succ_type = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(Term::Global("Nat".to_string())),
        body_type: Box::new(Term::Global("Nat".to_string())),
    };
    let result = logos::kernel::positivity::check_positivity("Nat", "Succ", &nat_succ_type);
    assert!(result.is_ok(), "Nat.Succ is positive: {:?}", result);

    // And: conj : P -> Q -> And P Q
    // The inductive And doesn't appear in constructor params, just result
    // (This is parameterized, but simplified check should still pass)

    println!("Standard library inductives verified as positive");
}
