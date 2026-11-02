use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    axiom_dependencies, axiom_dependencies_of, defeq_for_test, infer_type, recheck, Context,
    Literal, Term,
};

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: &[Term]) -> Term {
    xs.iter().cloned().fold(f, app)
}
fn op(n: &str, x: Term, y: Term) -> Term {
    apps(g(n), &[x, y])
}
fn eq(t: &str, x: Term, y: Term) -> Term {
    apps(g("Eq"), &[g(t), x, y])
}
fn pi(n: &str, ty: &str, body: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(g(ty)),
        body_type: Box::new(body),
    }
}
fn lam(n: &str, ty: &str, body: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(g(ty)),
        body: Box::new(body),
    }
}
fn forall3(ty: &str, body: Term) -> Term {
    pi("a", ty, pi("b", ty, pi("c", ty, body)))
}
fn lambda3(ty: &str, body: Term) -> Term {
    lam("a", ty, lam("b", ty, lam("c", ty, body)))
}
fn context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
fn distrib(a: Term, b: Term, c: Term) -> Term {
    eq(
        "Int",
        op("mul", a.clone(), op("add", b.clone(), c.clone())),
        op("add", op("mul", a.clone(), b), op("mul", a, c)),
    )
}
fn expected(name: &str) -> Term {
    let diff = |x, y| op("Int_subNatNat", x, y);
    let pos = |x| app(g("Int_ofNat"), x);
    let neg = |x| app(g("Int_negOfNat"), x);
    match name {
        "IntLaw_subNatNat_add_add" => forall3(
            "Nat",
            eq(
                "Int",
                diff(op("Nat_add", v("a"), v("c")), op("Nat_add", v("b"), v("c"))),
                diff(v("a"), v("b")),
            ),
        ),
        "IntLaw_subNatNat_zero_left" => {
            pi("a", "Nat", eq("Int", diff(g("Zero"), v("a")), neg(v("a"))))
        }
        "IntLaw_add_ofNat_negOfNat" => pi(
            "a",
            "Nat",
            pi(
                "b",
                "Nat",
                eq(
                    "Int",
                    op("add", pos(v("a")), neg(v("b"))),
                    diff(v("a"), v("b")),
                ),
            ),
        ),
        "IntLaw_add_negOfNat_negOfNat" => pi(
            "a",
            "Nat",
            pi(
                "b",
                "Nat",
                eq(
                    "Int",
                    op("add", neg(v("a")), neg(v("b"))),
                    neg(op("Nat_add", v("a"), v("b"))),
                ),
            ),
        ),
        "IntLaw_mul_ofNat_subNatNat" | "IntLaw_mul_negOfNat_subNatNat" => {
            let negative = name == "IntLaw_mul_negOfNat_subNatNat";
            let p = op("Nat_mul", v("a"), v("b"));
            let n = op("Nat_mul", v("a"), v("c"));
            forall3(
                "Nat",
                eq(
                    "Int",
                    op(
                        "mul",
                        if negative { neg(v("a")) } else { pos(v("a")) },
                        diff(v("b"), v("c")),
                    ),
                    if negative { diff(n, p) } else { diff(p, n) },
                ),
            )
        }
        "mul_distrib_add" => forall3("Int", distrib(v("a"), v("b"), v("c"))),
        _ => unreachable!(),
    }
}
fn check_actual_body(name: &str) {
    let ctx = context();
    let required = expected(name);
    let body = ctx
        .get_definition_body(name)
        .expect("the law must have an actual checked proof body");
    assert_ne!(
        body,
        &g(name),
        "a theorem reference is not the original proof body"
    );
    assert!(
        defeq_for_test(&ctx, ctx.get_global(name).unwrap(), &required),
        "{name}"
    );
    assert!(
        defeq_for_test(&ctx, &infer_type(&ctx, body).unwrap(), &required),
        "{name}"
    );
    assert!(
        defeq_for_test(&ctx, &recheck(&ctx, body).unwrap(), &required),
        "{name}"
    );
    for usage in [
        axiom_dependencies(&ctx, body),
        axiom_dependencies_of(&ctx, name).unwrap(),
    ] {
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "{name}: {usage:?}"
        );
    }
}

#[test]
fn common_offset_signed_difference_cancellation_has_a_closed_proof() {
    check_actual_body("IntLaw_subNatNat_add_add");
}
#[test]
fn zero_minus_magnitude_including_negative_zero_has_a_closed_proof() {
    check_actual_body("IntLaw_subNatNat_zero_left");
}
#[test]
fn mixed_sign_addition_matches_signed_difference_with_a_closed_proof() {
    check_actual_body("IntLaw_add_ofNat_negOfNat");
}
#[test]
fn negative_magnitude_addition_has_a_closed_proof() {
    check_actual_body("IntLaw_add_negOfNat_negOfNat");
}
#[test]
fn positive_multiplication_respects_signed_difference_with_a_closed_proof() {
    check_actual_body("IntLaw_mul_ofNat_subNatNat");
}
#[test]
fn negative_multiplication_reverses_signed_difference_with_a_closed_proof() {
    check_actual_body("IntLaw_mul_negOfNat_subNatNat");
}
#[test]
fn integer_multiplication_distributes_with_an_actual_closed_proof() {
    check_actual_body("mul_distrib_add");
}
#[test]
fn closed_downstream_distributivity_has_no_assumptions() {
    let mut ctx = context();
    let body = lambda3("Int", apps(g("mul_distrib_add"), &[v("a"), v("b"), v("c")]));
    let required = expected("mul_distrib_add");
    assert!(defeq_for_test(
        &ctx,
        &infer_type(&ctx, &body).unwrap(),
        &required
    ));
    assert!(defeq_for_test(
        &ctx,
        &recheck(&ctx, &body).unwrap(),
        &required
    ));
    ctx.declare("closed_multiplication_distributivity", required, body)
        .unwrap();
    let usage = axiom_dependencies_of(&ctx, "closed_multiplication_distributivity").unwrap();
    assert!(
        usage.axioms.is_empty() && usage.unresolved.is_empty(),
        "{usage:?}"
    );
}
#[test]
fn distributivity_covers_all_signs_cancellation_negative_zero_and_large_literals() {
    let ctx = context();
    assert!(ctx.is_definition("mul_distrib_add"));
    let lit = |n| Term::Lit(Literal::Int(n));
    for a in [-2, 0, 3] {
        for b in [-3, 0, 3] {
            for c in [-3, 0, 3] {
                let body = apps(g("mul_distrib_add"), &[lit(a), lit(b), lit(c)]);
                let required = distrib(lit(a), lit(b), lit(c));
                assert!(defeq_for_test(
                    &ctx,
                    &infer_type(&ctx, &body).unwrap(),
                    &required
                ));
                assert!(defeq_for_test(
                    &ctx,
                    &recheck(&ctx, &body).unwrap(),
                    &required
                ));
            }
        }
    }
    for (a, b, c) in [
        (lit(i64::MIN), lit(i64::MAX), lit(-1)),
        (lit(i64::MAX), lit(i64::MAX), lit(i64::MAX)),
        (
            app(g("Int_negOfNat"), g("Zero")),
            app(g("Int_negSucc"), g("Zero")),
            app(g("Int_ofNat"), app(g("Succ"), g("Zero"))),
        ),
        (
            app(g("Int_negSucc"), g("Zero")),
            app(g("Int_negOfNat"), g("Zero")),
            app(g("Int_ofNat"), g("Zero")),
        ),
    ] {
        let body = apps(g("mul_distrib_add"), &[a.clone(), b.clone(), c.clone()]);
        let required = distrib(a, b, c);
        assert!(defeq_for_test(
            &ctx,
            &infer_type(&ctx, &body).unwrap(),
            &required
        ));
        assert!(defeq_for_test(
            &ctx,
            &recheck(&ctx, &body).unwrap(),
            &required
        ));
    }
}
#[test]
fn constructor_arithmetic_rejects_a_missing_distributive_summand() {
    let ctx = context();
    let lhs = op("mul", v("a"), op("add", v("b"), v("c")));
    let false_goal = forall3("Int", eq("Int", lhs.clone(), op("mul", v("a"), v("b"))));
    let fake = lambda3("Int", apps(g("refl"), &[g("Int"), lhs]));
    let consumer = app(
        Term::Lambda {
            param: "h".into(),
            param_type: Box::new(false_goal),
            body: Box::new(g("true")),
        },
        fake,
    );
    assert!(infer_type(&ctx, &consumer).is_err());
    assert!(recheck(&ctx, &consumer).is_err());
}
