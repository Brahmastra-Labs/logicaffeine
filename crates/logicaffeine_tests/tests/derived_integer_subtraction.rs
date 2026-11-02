use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    axiom_dependencies_of, defeq_for_test, eval_bool_tree, infer_type, int_lit,
    native_compile_decide, normalize_checked, recheck, BigInt, Context, Literal, Term,
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

/// A rebinding of a name the prelude's integer laws rest on is refused by the stable-meaning
/// gate, typed, naming what depends on it. (The attestation caches that must notice such a
/// change are tested with the change permitted in the kernel's own unit tests,
/// `int_inductive::subtraction_alias_adversaries` and `checked_order_alias_adversaries`.)
fn refused_as_a_change_of_meaning(
    result: Result<(), logicaffeine_kernel::KernelError>,
    name: &str,
) {
    match result {
        Err(logicaffeine_kernel::KernelError::StableMeaningViolation {
            name: refused,
            dependents,
            ..
        }) => {
            assert_eq!(refused, name);
            assert!(!dependents.is_empty());
        }
        other => panic!("rebinding `{name}` under its dependents must be refused: {other:?}"),
    }
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

fn eq(x: Term, y: Term) -> Term {
    apps(g("Eq"), &[g("Int"), x, y])
}
fn pi(n: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn expected(name: &str) -> Term {
    match name {
        "IntLaw_neg_eq_mul_neg_one" => pi(
            "b",
            g("Int"),
            eq(app(g("Int_neg"), v("b")), op("mul", integer(-1), v("b"))),
        ),
        "sub_elim" => pi(
            "a",
            g("Int"),
            pi(
                "b",
                g("Int"),
                eq(
                    op("sub", v("a"), v("b")),
                    op("add", v("a"), op("mul", integer(-1), v("b"))),
                ),
            ),
        ),
        _ => unreachable!(),
    }
}
fn check_body(name: &str) {
    let ctx = context();
    let want = expected(name);
    let body = ctx
        .get_definition_body(name)
        .expect("actual constructor-derived proof body required");
    assert_ne!(body, &g(name));
    assert!(defeq_for_test(&ctx, ctx.get_global(name).unwrap(), &want));
    assert!(defeq_for_test(
        &ctx,
        &infer_type(&ctx, body).unwrap(),
        &want
    ));
    assert!(defeq_for_test(&ctx, &recheck(&ctx, body).unwrap(), &want));
    for usage in [
        logicaffeine_kernel::axiom_dependencies(&ctx, body),
        axiom_dependencies_of(&ctx, name).unwrap(),
    ] {
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "{name}: {usage:?}"
        );
    }
}
#[test]
fn public_subtraction_is_the_checked_canonical_alias() {
    let ctx = context();
    let body = ctx
        .get_definition_body("sub")
        .expect("public sub must have a body");
    assert_eq!(body, &g("Int_sub"));
    assert!(defeq_for_test(
        &ctx,
        ctx.get_global("sub").unwrap(),
        &binary_type("Int")
    ));
    assert!(defeq_for_test(
        &ctx,
        &infer_type(&ctx, body).unwrap(),
        &binary_type("Int")
    ));
    assert!(defeq_for_test(
        &ctx,
        &recheck(&ctx, body).unwrap(),
        &binary_type("Int")
    ));
    let usage = axiom_dependencies_of(&ctx, "sub").unwrap();
    assert!(
        usage.axioms.is_empty() && usage.unresolved.is_empty(),
        "{usage:?}"
    );
}
#[test]
fn negation_equals_negative_one_product_by_closed_constructor_proof() {
    check_body("IntLaw_neg_eq_mul_neg_one");
}
#[test]
fn subtraction_elimination_has_an_actual_closed_axiom_free_proof() {
    check_body("sub_elim");
}
#[test]
fn closed_downstream_subtraction_elimination_has_no_assumptions() {
    let mut ctx = context();
    let body = lam(
        "a",
        g("Int"),
        lam("b", g("Int"), apps(g("sub_elim"), &[v("a"), v("b")])),
    );
    let want = expected("sub_elim");
    assert!(defeq_for_test(
        &ctx,
        &infer_type(&ctx, &body).unwrap(),
        &want
    ));
    assert!(defeq_for_test(&ctx, &recheck(&ctx, &body).unwrap(), &want));
    ctx.declare("closed_subtraction_elimination", want, body)
        .unwrap();
    let usage = axiom_dependencies_of(&ctx, "closed_subtraction_elimination").unwrap();
    assert!(
        usage.axioms.is_empty() && usage.unresolved.is_empty(),
        "{usage:?}"
    );
}
#[test]
fn checked_subtraction_keeps_all_four_paths_exact_at_sign_and_bigint_boundaries() {
    let ctx = context();
    assert!(ctx.is_definition("sub"));
    for a in [-3, 0, 5] {
        for b in [-4, 0, 2] {
            check_all_paths(
                &ctx,
                op(
                    "sub",
                    logicaffeine_kernel::int_inductive::canonical_int(a),
                    integer(b),
                ),
                integer(a - b),
            );
        }
    }
    check_all_paths(
        &ctx,
        op(
            "sub",
            app(g("Int_negOfNat"), g("Zero")),
            app(g("Int_negSucc"), g("Zero")),
        ),
        integer(1),
    );
    let huge = BigInt::parse_decimal("1180591620717411303449").unwrap();
    for n in [
        huge.clone(),
        huge.negated(),
        BigInt::from_i64(i64::MIN),
        BigInt::from_i64(i64::MAX),
    ] {
        let value = Term::Lit(int_lit(n.clone()));
        let rhs = Term::Lit(int_lit(n.negated()));
        check_all_paths(
            &ctx,
            op("sub", value.clone(), rhs),
            Term::Lit(int_lit(n.mul(&BigInt::from_i64(2)))),
        );
        check_all_paths(
            &ctx,
            op(
                "sub",
                op("add", value.clone(), integer(3)),
                op("sub", value, integer(2)),
            ),
            integer(5),
        );
    }
}
#[test]
fn rebinding_public_subtraction_is_refused_and_its_native_meaning_is_unchanged() {
    let mut ctx = context();
    check_all_paths(&ctx, op("sub", integer(7), integer(2)), integer(5));
    refused_as_a_change_of_meaning(
        ctx.declare(
            "sub",
            binary_type("Int"),
            lam("a", g("Int"), lam("b", g("Int"), v("b"))),
        ),
        "sub",
    );
    check_all_paths(&ctx, op("sub", integer(7), integer(2)), integer(5));
}
#[test]
fn unique_and_cloned_context_negation_rebindings_are_refused_and_subtraction_is_unchanged() {
    let mut parent = context();
    parent
        .declare("sub", binary_type("Int"), g("Int_sub"))
        .unwrap();
    assert!(
        parent.refused_declarations().is_empty(),
        "{:?}",
        parent.refused_declarations()
    );
    check_all_paths(&parent, op("sub", integer(7), integer(2)), integer(5));
    let mut child = parent.clone();
    refused_as_a_change_of_meaning(
        child.declare(
            "Int_neg",
            pi("x", g("Int"), g("Int")),
            lam("x", g("Int"), v("x")),
        ),
        "Int_neg",
    );
    check_all_paths(&child, op("sub", integer(7), integer(2)), integer(5));
    check_all_paths(&parent, op("sub", integer(7), integer(2)), integer(5));
    drop(child);
    refused_as_a_change_of_meaning(
        parent.declare(
            "Int_neg",
            pi("x", g("Int"), g("Int")),
            lam("x", g("Int"), integer(0)),
        ),
        "Int_neg",
    );
    check_all_paths(&parent, op("sub", integer(7), integer(2)), integer(5));
}
#[test]
fn subtraction_preserves_every_dependency_unfolding_boundary() {
    use logicaffeine_kernel::{normalize_checked_at, Transparency};
    let mut ctx = context();
    ctx.declare("sub", binary_type("Int"), g("Int_sub"))
        .unwrap();
    for blocked in [
        "sub",
        "Int_sub",
        "Int_neg",
        "Int_negOfNat",
        "Int_add",
        "Int_subNatNat",
        "Nat_sub",
        "Nat_pred",
        "Nat_add",
    ] {
        let b = if blocked == "Nat_add" { -2 } else { 2 };
        let opaque = |name: &str| name == blocked;
        let expression = op("sub", integer(5), integer(b));
        let reduced =
            normalize_checked_at(&ctx, &expression, Transparency::Excluding(&opaque)).unwrap();
        assert!(
            reduced.to_string().contains(blocked),
            "{blocked}: {reduced}"
        );
    }
}
#[test]
fn polynomial_semantics_accept_checked_subtraction_and_a_rebinding_is_refused() {
    let mut ctx = context();
    ctx.declare("sub", binary_type("Int"), g("Int_sub"))
        .unwrap();
    assert!(logicaffeine_kernel::int_inductive::supports_integer_polynomial_operations(&ctx));
    refused_as_a_change_of_meaning(
        ctx.declare(
            "sub",
            binary_type("Int"),
            lam("a", g("Int"), lam("b", g("Int"), v("a"))),
        ),
        "sub",
    );
    assert!(logicaffeine_kernel::int_inductive::supports_integer_polynomial_operations(&ctx));
}
#[test]
fn both_kernels_reject_subtraction_with_the_wrong_sign() {
    let ctx = context();
    let false_goal = pi(
        "a",
        g("Int"),
        pi(
            "b",
            g("Int"),
            eq(op("sub", v("a"), v("b")), op("add", v("a"), v("b"))),
        ),
    );
    let fake = lam(
        "a",
        g("Int"),
        lam(
            "b",
            g("Int"),
            apps(g("refl"), &[g("Int"), op("sub", v("a"), v("b"))]),
        ),
    );
    let consumer = app(lam("h", false_goal, g("true")), fake);
    assert!(infer_type(&ctx, &consumer).is_err());
    assert!(recheck(&ctx, &consumer).is_err());
}
