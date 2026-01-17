//! =============================================================================
//! PHASE 70c: COMPUTATION (FIXPOINT & REDUCTION)
//! =============================================================================
//!
//! The fire is lit. Terms now COMPUTE.
//!
//! This phase implements:
//! - Iota reduction: match (Cᵢ args) ... → caseᵢ(args)
//! - Beta reduction: (λx. body) arg → body[x := arg]
//! - Fix unfolding: fix f. body → body[f := fix f. body] (guarded)
//! - Normalization: reduce terms to normal form

use logicaffeine_kernel::{infer_type, normalize, Context, Term, Universe};

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

fn nat() -> Term {
    Term::Global("Nat".to_string())
}

fn zero() -> Term {
    Term::Global("Zero".to_string())
}

fn succ(n: Term) -> Term {
    Term::App(
        Box::new(Term::Global("Succ".to_string())),
        Box::new(n),
    )
}

fn church_numeral(n: u32) -> Term {
    let mut result = zero();
    for _ in 0..n {
        result = succ(result);
    }
    result
}

// =============================================================================
// IOTA REDUCTION TESTS
// =============================================================================

#[test]
fn test_iota_reduction_zero_case() {
    let ctx = setup_nat_context();

    // match Zero return (λ_. Nat) with | Zero => Zero | Succ k => k
    // Should reduce to: Zero
    let motive = Term::Lambda {
        param: "_".to_string(),
        param_type: Box::new(nat()),
        body: Box::new(nat()),
    };

    let match_expr = Term::Match {
        discriminant: Box::new(zero()),
        motive: Box::new(motive),
        cases: vec![
            zero(), // Zero case
            Term::Lambda {
                // Succ case: λk. k
                param: "k".to_string(),
                param_type: Box::new(nat()),
                body: Box::new(Term::Var("k".to_string())),
            },
        ],
    };

    let result = normalize(&ctx, &match_expr);
    assert!(
        matches!(result, Term::Global(ref s) if s == "Zero"),
        "match Zero should reduce to Zero, got: {}",
        result
    );

    println!("✓ match Zero => Zero");
}

#[test]
fn test_iota_reduction_succ_case() {
    let ctx = setup_nat_context();

    // match (Succ Zero) return (λ_. Nat) with | Zero => Zero | Succ k => k
    // Should reduce to: Zero (predecessor of 1)
    let motive = Term::Lambda {
        param: "_".to_string(),
        param_type: Box::new(nat()),
        body: Box::new(nat()),
    };

    let match_expr = Term::Match {
        discriminant: Box::new(succ(zero())),
        motive: Box::new(motive),
        cases: vec![
            zero(),
            Term::Lambda {
                param: "k".to_string(),
                param_type: Box::new(nat()),
                body: Box::new(Term::Var("k".to_string())),
            },
        ],
    };

    let result = normalize(&ctx, &match_expr);
    assert!(
        matches!(result, Term::Global(ref s) if s == "Zero"),
        "pred(1) should be 0, got: {}",
        result
    );

    println!("✓ pred(Succ Zero) => Zero");
}

#[test]
fn test_iota_nested_succ() {
    let ctx = setup_nat_context();

    // pred(Succ(Succ(Zero))) = Succ(Zero)
    let motive = Term::Lambda {
        param: "_".to_string(),
        param_type: Box::new(nat()),
        body: Box::new(nat()),
    };

    let match_expr = Term::Match {
        discriminant: Box::new(succ(succ(zero()))),
        motive: Box::new(motive),
        cases: vec![
            zero(),
            Term::Lambda {
                param: "k".to_string(),
                param_type: Box::new(nat()),
                body: Box::new(Term::Var("k".to_string())),
            },
        ],
    };

    let result = normalize(&ctx, &match_expr);
    let expected = succ(zero());
    assert_eq!(
        format!("{}", result),
        format!("{}", expected),
        "pred(2) should be 1"
    );

    println!("✓ pred(Succ(Succ(Zero))) => Succ(Zero)");
}

// =============================================================================
// BETA REDUCTION TESTS
// =============================================================================

#[test]
fn test_beta_reduction_identity() {
    let ctx = setup_nat_context();

    // (λx:Nat. x) Zero → Zero
    let identity = Term::Lambda {
        param: "x".to_string(),
        param_type: Box::new(nat()),
        body: Box::new(Term::Var("x".to_string())),
    };

    let applied = Term::App(Box::new(identity), Box::new(zero()));

    let result = normalize(&ctx, &applied);
    assert!(
        matches!(result, Term::Global(ref s) if s == "Zero"),
        "identity(Zero) should be Zero, got: {}",
        result
    );

    println!("✓ (λx. x) Zero => Zero");
}

#[test]
fn test_beta_reduction_const() {
    let ctx = setup_nat_context();

    // (λx:Nat. λy:Nat. x) Zero (Succ Zero) → Zero
    let const_fn = Term::Lambda {
        param: "x".to_string(),
        param_type: Box::new(nat()),
        body: Box::new(Term::Lambda {
            param: "y".to_string(),
            param_type: Box::new(nat()),
            body: Box::new(Term::Var("x".to_string())),
        }),
    };

    let applied = Term::App(
        Box::new(Term::App(Box::new(const_fn), Box::new(zero()))),
        Box::new(succ(zero())),
    );

    let result = normalize(&ctx, &applied);
    assert!(
        matches!(result, Term::Global(ref s) if s == "Zero"),
        "const Zero (Succ Zero) should be Zero, got: {}",
        result
    );

    println!("✓ const Zero (Succ Zero) => Zero");
}

// =============================================================================
// FIXPOINT TESTS
// =============================================================================

/// Helper to build the plus function
fn make_plus() -> Term {
    // plus = fix plus. λn. λm. match n return (λ_. Nat) with
    //        | Zero => m
    //        | Succ k => Succ (plus k m)

    let plus_body = Term::Lambda {
        param: "n".to_string(),
        param_type: Box::new(nat()),
        body: Box::new(Term::Lambda {
            param: "m".to_string(),
            param_type: Box::new(nat()),
            body: Box::new(Term::Match {
                discriminant: Box::new(Term::Var("n".to_string())),
                motive: Box::new(Term::Lambda {
                    param: "_".to_string(),
                    param_type: Box::new(nat()),
                    body: Box::new(nat()),
                }),
                cases: vec![
                    // Zero => m
                    Term::Var("m".to_string()),
                    // Succ k => Succ (plus k m)
                    Term::Lambda {
                        param: "k".to_string(),
                        param_type: Box::new(nat()),
                        body: Box::new(succ(Term::App(
                            Box::new(Term::App(
                                Box::new(Term::Var("plus".to_string())),
                                Box::new(Term::Var("k".to_string())),
                            )),
                            Box::new(Term::Var("m".to_string())),
                        ))),
                    },
                ],
            }),
        }),
    };

    Term::Fix {
        name: "plus".to_string(),
        body: Box::new(plus_body),
    }
}

#[test]
fn test_addition_zero_plus_zero() {
    let ctx = setup_nat_context();
    let plus = make_plus();

    // plus 0 0 = 0
    let expr = Term::App(
        Box::new(Term::App(Box::new(plus), Box::new(zero()))),
        Box::new(zero()),
    );

    let result = normalize(&ctx, &expr);
    assert!(
        matches!(result, Term::Global(ref s) if s == "Zero"),
        "0 + 0 should be 0, got: {}",
        result
    );

    println!("✓ 0 + 0 = 0");
}

#[test]
fn test_addition_zero_plus_n() {
    let ctx = setup_nat_context();
    let plus = make_plus();

    // plus 0 3 = 3
    let three = church_numeral(3);
    let expr = Term::App(
        Box::new(Term::App(Box::new(plus), Box::new(zero()))),
        Box::new(three.clone()),
    );

    let result = normalize(&ctx, &expr);
    assert_eq!(
        format!("{}", result),
        format!("{}", three),
        "0 + 3 should equal 3"
    );

    println!("✓ 0 + 3 = 3");
}

#[test]
fn test_addition_one_plus_zero() {
    let ctx = setup_nat_context();
    let plus = make_plus();

    // plus 1 0 = 1
    let one = succ(zero());
    let expr = Term::App(
        Box::new(Term::App(Box::new(plus), Box::new(one.clone()))),
        Box::new(zero()),
    );

    let result = normalize(&ctx, &expr);
    assert_eq!(
        format!("{}", result),
        format!("{}", one),
        "1 + 0 should equal 1"
    );

    println!("✓ 1 + 0 = 1");
}

#[test]
fn test_addition_one_plus_one() {
    let ctx = setup_nat_context();
    let plus = make_plus();

    // plus 1 1 = 2
    let one = succ(zero());
    let two = succ(succ(zero()));

    let expr = Term::App(
        Box::new(Term::App(Box::new(plus), Box::new(one.clone()))),
        Box::new(one),
    );

    let result = normalize(&ctx, &expr);
    assert_eq!(
        format!("{}", result),
        format!("{}", two),
        "1 + 1 should equal 2"
    );

    println!("✓ 1 + 1 = 2");
}

#[test]
fn test_addition_two_plus_three() {
    let ctx = setup_nat_context();
    let plus = make_plus();

    // plus 2 3 = 5
    let two = church_numeral(2);
    let three = church_numeral(3);
    let five = church_numeral(5);

    let expr = Term::App(
        Box::new(Term::App(Box::new(plus), Box::new(two))),
        Box::new(three),
    );

    let result = normalize(&ctx, &expr);
    assert_eq!(
        format!("{}", result),
        format!("{}", five),
        "2 + 3 should equal 5"
    );

    println!("✓ 2 + 3 = 5");
}

// =============================================================================
// TYPE CHECKING TESTS
// =============================================================================

#[test]
fn test_fix_type_checks() {
    let ctx = setup_nat_context();
    let plus = make_plus();

    // plus should type-check as Nat -> Nat -> Nat
    let result = infer_type(&ctx, &plus);
    assert!(result.is_ok(), "plus should type-check: {:?}", result);

    // Verify it's Π(n:Nat). Π(m:Nat). Nat
    let ty = result.unwrap();
    if let Term::Pi {
        param_type,
        body_type,
        ..
    } = &ty
    {
        assert!(
            matches!(param_type.as_ref(), Term::Global(s) if s == "Nat"),
            "First param should be Nat"
        );
        if let Term::Pi {
            param_type: inner_param,
            ..
        } = body_type.as_ref()
        {
            assert!(
                matches!(inner_param.as_ref(), Term::Global(s) if s == "Nat"),
                "Second param should be Nat"
            );
        } else {
            panic!("Expected nested Pi type");
        }
    } else {
        panic!("Expected Pi type, got: {:?}", ty);
    }

    println!("✓ plus : Nat -> Nat -> Nat");
}

// =============================================================================
// MULTIPLICATION (STRESS TEST)
// =============================================================================

#[test]
fn test_multiplication_two_times_three() {
    let ctx = setup_nat_context();

    // mult = fix mult. λn. λm. match n return (λ_. Nat) with
    //        | Zero => Zero
    //        | Succ k => plus m (mult k m)

    // We need plus for mult
    let plus = make_plus();

    let mult_body = Term::Lambda {
        param: "n".to_string(),
        param_type: Box::new(nat()),
        body: Box::new(Term::Lambda {
            param: "m".to_string(),
            param_type: Box::new(nat()),
            body: Box::new(Term::Match {
                discriminant: Box::new(Term::Var("n".to_string())),
                motive: Box::new(Term::Lambda {
                    param: "_".to_string(),
                    param_type: Box::new(nat()),
                    body: Box::new(nat()),
                }),
                cases: vec![
                    // Zero => Zero
                    zero(),
                    // Succ k => plus m (mult k m)
                    Term::Lambda {
                        param: "k".to_string(),
                        param_type: Box::new(nat()),
                        body: Box::new(Term::App(
                            Box::new(Term::App(
                                Box::new(plus.clone()),
                                Box::new(Term::Var("m".to_string())),
                            )),
                            Box::new(Term::App(
                                Box::new(Term::App(
                                    Box::new(Term::Var("mult".to_string())),
                                    Box::new(Term::Var("k".to_string())),
                                )),
                                Box::new(Term::Var("m".to_string())),
                            )),
                        )),
                    },
                ],
            }),
        }),
    };

    let mult = Term::Fix {
        name: "mult".to_string(),
        body: Box::new(mult_body),
    };

    // mult 2 3 = 6
    let two = church_numeral(2);
    let three = church_numeral(3);
    let six = church_numeral(6);

    let expr = Term::App(
        Box::new(Term::App(Box::new(mult), Box::new(two))),
        Box::new(three),
    );

    let result = normalize(&ctx, &expr);
    assert_eq!(
        format!("{}", result),
        format!("{}", six),
        "2 * 3 should equal 6"
    );

    println!("✓ 2 * 3 = 6");
}
