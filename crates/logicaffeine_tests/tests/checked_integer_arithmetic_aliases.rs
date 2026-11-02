use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    axiom_dependencies_of, defeq_for_test, eval_bool_tree, infer_type, int_lit,
    native_compile_decide, normalize_checked, recheck, BigInt, Context, KernelError, Literal,
    MeaningChange, Term,
};

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn apps(f: Term, xs: &[Term]) -> Term {
    xs.iter().cloned().fold(f, app)
}
fn op(n: &str, a: Term, b: Term) -> Term {
    apps(g(n), &[a, b])
}
fn integer(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
fn nat(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(n)))
}
fn lam(n: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn binary_type(carrier: &str) -> Term {
    Term::Pi {
        param: "a".into(),
        param_type: Box::new(g(carrier)),
        body_type: Box::new(Term::Pi {
            param: "b".into(),
            param_type: Box::new(g(carrier)),
            body_type: Box::new(g(carrier)),
        }),
    }
}
fn context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
fn check_all_paths(ctx: &Context, expression: Term, expected: Term) {
    let reduced = normalize_checked(ctx, &expression)
        .expect("literal computation remains within the existing budget");
    assert!(
        defeq_for_test(ctx, &reduced, &expected),
        "{expression} reduced to {reduced}, expected {expected}"
    );
    let equation = apps(g("Eq"), &[g("Int"), expression.clone(), expected.clone()]);
    let proof = apps(g("refl"), &[g("Int"), expected.clone()]);
    let consume = app(lam("h", equation, g("true")), proof);
    assert_eq!(infer_type(ctx, &consume).unwrap(), g("Bool"));
    assert_eq!(recheck(ctx, &consume).unwrap(), g("Bool"));
    for comparison in [
        op("le", expression.clone(), expected.clone()),
        op("le", expected, expression),
    ] {
        assert_eq!(eval_bool_tree(ctx, &comparison), Some(true));
        assert_eq!(native_compile_decide(ctx, &comparison), Some(true));
    }
}

/// A rebinding of a name the prelude's integer laws were proved about is refused by the
/// stable-meaning gate, typed, naming what depends on it. (The caches behind the gate — the
/// alias attestation that must notice such a change — are tested with the change permitted in
/// the kernel's own unit tests, `int_inductive::checked_alias_attestation_controls`.)
fn refused_as_a_change_of_meaning(result: Result<(), KernelError>, name: &str) {
    match result {
        Err(KernelError::StableMeaningViolation {
            name: refused,
            change: MeaningChange::Redeclared,
            dependents,
        }) => {
            assert_eq!(refused, name);
            assert!(!dependents.is_empty());
        }
        other => panic!("rebinding `{name}` under its dependents must be refused: {other:?}"),
    }
}

#[test]
fn public_integer_add_and_mul_are_checked_aliases_with_no_axiom_dependency() {
    let ctx = context();
    for (alias, target) in [("add", "Int_add"), ("mul", "Int_mul")] {
        assert_eq!(ctx.get_definition_body(alias), Some(&g(target)));
        assert!(defeq_for_test(
            &ctx,
            ctx.get_global(alias).unwrap(),
            &binary_type("Int")
        ));
        let body = ctx.get_definition_body(alias).unwrap();
        assert!(defeq_for_test(
            &ctx,
            &recheck(&ctx, body).unwrap(),
            &binary_type("Int")
        ));
        let usage = axiom_dependencies_of(&ctx, alias).unwrap();
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "{usage:?}"
        );
    }
}

#[test]
fn checked_aliases_keep_compact_overflow_and_large_mixed_constructor_computation() {
    let ctx = context();
    for name in ["add", "mul"] {
        assert!(ctx.is_definition(name));
    }
    let large = BigInt::from_i64(i64::MAX).add(&BigInt::from_i64(1));
    let constructor = app(g("Int_ofNat"), Term::Lit(Literal::Nat(large.clone())));
    check_all_paths(
        &ctx,
        op("add", constructor.clone(), integer(-1)),
        integer(i64::MAX),
    );
    check_all_paths(
        &ctx,
        op("mul", constructor, integer(-2)),
        Term::Lit(int_lit(large.mul(&BigInt::from_i64(-2)))),
    );
    check_all_paths(
        &ctx,
        op("add", integer(i64::MAX), integer(1)),
        Term::Lit(int_lit(large)),
    );
    check_all_paths(
        &ctx,
        op("mul", integer(i64::MIN), integer(-1)),
        Term::Lit(int_lit(BigInt::from_i64(i64::MIN).negated())),
    );
}

#[test]
fn checked_aliases_agree_in_every_sign_case_on_both_kernels_and_evaluators() {
    let ctx = context();
    for a in [-3, -1, 0, 1, 4] {
        for b in [-2, 0, 2, 5] {
            for (name, want) in [("add", a + b), ("mul", a * b)] {
                let x = logicaffeine_kernel::int_inductive::canonical_int(a);
                check_all_paths(&ctx, op(name, x, integer(b)), integer(want));
            }
        }
    }
}

#[test]
fn rebinding_the_public_alias_is_refused_and_its_arithmetic_is_unchanged() {
    let mut ctx = context();
    refused_as_a_change_of_meaning(
        ctx.declare(
            "add",
            binary_type("Int"),
            lam("a", g("Int"), lam("b", g("Int"), v("a"))),
        ),
        "add",
    );
    check_all_paths(&ctx, op("add", integer(7), integer(5)), integer(12));
}

#[test]
fn rebinding_a_canonical_int_body_is_refused_and_the_alias_keeps_computing() {
    let mut ctx = context();
    // Redeclaring the alias at its own meaning (its binders renamed) changes nothing.
    ctx.declare("add", binary_type("Int"), g("Int_add"))
        .unwrap();
    assert!(
        ctx.refused_declarations().is_empty(),
        "{:?}",
        ctx.refused_declarations()
    );
    refused_as_a_change_of_meaning(
        ctx.declare(
            "Int_add",
            binary_type("Int"),
            lam("a", g("Int"), lam("b", g("Int"), v("b"))),
        ),
        "Int_add",
    );
    check_all_paths(&ctx, op("add", integer(7), integer(5)), integer(12));
}

#[test]
fn rebinding_a_transitive_nat_dependency_under_warmed_acceleration_is_refused() {
    let mut ctx = context();
    ctx.declare("add", binary_type("Int"), g("Int_add"))
        .unwrap();
    check_all_paths(&ctx, op("add", integer(2), integer(3)), integer(5));
    refused_as_a_change_of_meaning(
        ctx.declare(
            "Nat_add",
            binary_type("Nat"),
            lam("a", g("Nat"), lam("b", g("Nat"), nat(0))),
        ),
        "Nat_add",
    );
    check_all_paths(&ctx, op("add", integer(2), integer(3)), integer(5));
}

#[test]
fn a_refused_mutation_of_a_cloned_context_leaves_both_verdicts_intact() {
    let mut parent = context();
    parent
        .declare("mul", binary_type("Int"), g("Int_mul"))
        .unwrap();
    check_all_paths(&parent, op("mul", integer(2), integer(3)), integer(6));
    let mut child = parent.clone();
    refused_as_a_change_of_meaning(
        child.declare(
            "Nat_mul",
            binary_type("Nat"),
            lam("a", g("Nat"), lam("b", g("Nat"), nat(1))),
        ),
        "Nat_mul",
    );
    check_all_paths(&child, op("mul", integer(2), integer(3)), integer(6));
    check_all_paths(&parent, op("mul", integer(2), integer(3)), integer(6));
}

#[test]
fn excluding_any_skipped_dependency_preserves_its_unfolding_boundary() {
    use logicaffeine_kernel::{normalize_checked_at, Transparency};
    let mut ctx = context();
    for (name, target) in [("add", "Int_add"), ("mul", "Int_mul")] {
        ctx.declare(name, binary_type("Int"), g(target)).unwrap();
    }
    for (operation, blocked, a, b) in [
        ("add", "add", 2, 3),
        ("add", "Int_add", 2, 3),
        ("add", "Nat_add", 2, 3),
        ("add", "Int_subNatNat", 2, -1),
        ("add", "Nat_sub", 2, -1),
        ("add", "Nat_pred", 2, -1),
        ("mul", "mul", 2, 3),
        ("mul", "Int_mul", 2, 3),
        ("mul", "Nat_mul", 2, 3),
        ("mul", "Nat_add", 2, 3),
        ("mul", "Int_negOfNat", 2, -1),
    ] {
        let opaque = |name: &str| name == blocked;
        let expression = op(operation, integer(a), integer(b));
        let reduced =
            normalize_checked_at(&ctx, &expression, Transparency::Excluding(&opaque)).unwrap();
        assert!(
            reduced.to_string().contains(blocked),
            "excluding {blocked} must retain that dependency in {expression}, got {reduced}"
        );
    }
}

#[test]
fn rebinding_the_nat_predecessor_is_refused_and_mixed_sign_addition_is_unchanged() {
    let mut ctx = context();
    ctx.declare("add", binary_type("Int"), g("Int_add"))
        .unwrap();
    check_all_paths(&ctx, op("add", integer(5), integer(-1)), integer(4));
    let ty = Term::Pi {
        param: "n".into(),
        param_type: Box::new(g("Nat")),
        body_type: Box::new(g("Nat")),
    };
    refused_as_a_change_of_meaning(
        ctx.declare("Nat_pred", ty, lam("n", g("Nat"), nat(0))),
        "Nat_pred",
    );
    check_all_paths(&ctx, op("add", integer(5), integer(-1)), integer(4));
}

#[test]
fn rebinding_the_subtraction_helper_is_refused_and_mixed_sign_addition_is_unchanged() {
    let mut ctx = context();
    ctx.declare("add", binary_type("Int"), g("Int_add"))
        .unwrap();
    check_all_paths(&ctx, op("add", integer(5), integer(-1)), integer(4));
    let ty = Term::Pi {
        param: "m".into(),
        param_type: Box::new(g("Nat")),
        body_type: Box::new(Term::Pi {
            param: "n".into(),
            param_type: Box::new(g("Nat")),
            body_type: Box::new(g("Int")),
        }),
    };
    refused_as_a_change_of_meaning(
        ctx.declare(
            "Int_subNatNat",
            ty,
            lam("m", g("Nat"), lam("n", g("Nat"), integer(9))),
        ),
        "Int_subNatNat",
    );
    check_all_paths(&ctx, op("add", integer(5), integer(-1)), integer(4));
}

#[test]
fn rebinding_the_negative_constructor_helper_is_refused_and_negative_multiplication_is_unchanged() {
    let mut ctx = context();
    ctx.declare("mul", binary_type("Int"), g("Int_mul"))
        .unwrap();
    check_all_paths(&ctx, op("mul", integer(2), integer(-3)), integer(-6));
    let ty = Term::Pi {
        param: "n".into(),
        param_type: Box::new(g("Nat")),
        body_type: Box::new(g("Int")),
    };
    refused_as_a_change_of_meaning(
        ctx.declare(
            "Int_negOfNat",
            ty,
            lam("n", g("Nat"), app(g("Int_ofNat"), v("n"))),
        ),
        "Int_negOfNat",
    );
    check_all_paths(&ctx, op("mul", integer(2), integer(-3)), integer(-6));
}

#[test]
fn nested_large_alias_operands_reduce_before_structural_peano_expansion() {
    let ctx = context();
    let huge = BigInt::parse_decimal("1180591620717411303449").unwrap();
    for value in [huge.clone(), huge.negated()] {
        let literal = Term::Lit(int_lit(value.clone()));
        let expression = op(
            "add",
            op("mul", literal.clone(), integer(-2)),
            op("add", literal, integer(3)),
        );
        let expected = Term::Lit(int_lit(value.negated().add(&BigInt::from_i64(3))));
        check_all_paths(&ctx, expression, expected);
    }
}
