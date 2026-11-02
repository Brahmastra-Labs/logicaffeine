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
fn assoc(ty: &str, mul: &str, a: Term, b: Term, c: Term) -> Term {
    eq(
        ty,
        op(mul, op(mul, a.clone(), b.clone()), c.clone()),
        op(mul, a, op(mul, b, c)),
    )
}
fn context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
fn expected(name: &str) -> Term {
    let nsub = |x, y| op("Nat_sub", x, y);
    let diff = |x, y| op("Int_subNatNat", x, y);
    let suc = |x| app(g("Succ"), x);
    let pos = |x| app(g("Int_ofNat"), x);
    let neg = |x| app(g("Int_negSucc"), x);
    match name {
        "Nat_sub_zero_left" => pi("a", "Nat", eq("Nat", nsub(g("Zero"), v("a")), g("Zero"))),
        "Nat_sub_succ_succ" => pi(
            "a",
            "Nat",
            pi(
                "b",
                "Nat",
                eq("Nat", nsub(suc(v("a")), suc(v("b"))), nsub(v("a"), v("b"))),
            ),
        ),
        "IntLaw_subNatNat_succ_succ" => pi(
            "a",
            "Nat",
            pi(
                "b",
                "Nat",
                eq("Int", diff(suc(v("a")), suc(v("b"))), diff(v("a"), v("b"))),
            ),
        ),
        "IntLaw_subNatNat_add_left" => pi(
            "a",
            "Nat",
            pi(
                "b",
                "Nat",
                eq(
                    "Int",
                    diff(op("Nat_add", v("a"), v("b")), v("a")),
                    pos(v("b")),
                ),
            ),
        ),
        "IntLaw_subNatNat_add_right" => pi(
            "a",
            "Nat",
            pi(
                "b",
                "Nat",
                eq(
                    "Int",
                    diff(v("a"), op("Nat_add", v("a"), suc(v("b")))),
                    neg(v("b")),
                ),
            ),
        ),
        "IntLaw_subNatNat_add" => forall3(
            "Nat",
            eq(
                "Int",
                diff(op("Nat_add", v("a"), v("b")), v("c")),
                op("add", pos(v("a")), diff(v("b"), v("c"))),
            ),
        ),
        "IntLaw_subNatNat_add_negSucc" => forall3(
            "Nat",
            eq(
                "Int",
                op("add", diff(v("a"), v("b")), neg(v("c"))),
                diff(v("a"), op("Nat_add", v("b"), suc(v("c")))),
            ),
        ),
        "add_assoc" => forall3("Int", assoc("Int", "add", v("a"), v("b"), v("c"))),
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
fn natural_zero_subtraction_has_a_closed_proof() {
    check_actual_body("Nat_sub_zero_left");
}
#[test]
fn natural_successor_cancellation_has_a_closed_proof() {
    check_actual_body("Nat_sub_succ_succ");
}
#[test]
fn signed_difference_successor_cancellation_has_a_closed_proof() {
    check_actual_body("IntLaw_subNatNat_succ_succ");
}
#[test]
fn positive_signed_difference_has_a_closed_proof() {
    check_actual_body("IntLaw_subNatNat_add_left");
}
#[test]
fn negative_signed_difference_has_a_closed_proof() {
    check_actual_body("IntLaw_subNatNat_add_right");
}
#[test]
fn positive_addition_through_signed_difference_has_a_closed_proof() {
    check_actual_body("IntLaw_subNatNat_add");
}
#[test]
fn negative_addition_through_signed_difference_has_a_closed_proof() {
    check_actual_body("IntLaw_subNatNat_add_negSucc");
}
#[test]
fn integer_addition_associates_with_a_closed_proof() {
    check_actual_body("add_assoc");
}
#[test]
fn addition_associativity_covers_signs_negative_zero_and_literal_boundaries() {
    let ctx = context();
    assert!(ctx.is_definition("add_assoc"));
    let lit = |n| Term::Lit(Literal::Int(n));
    for a in [-2, 0, 3] {
        for b in [-2, 0, 3] {
            for c in [-2, 0, 3] {
                let proof = apps(g("add_assoc"), &[lit(a), lit(b), lit(c)]);
                let required = assoc("Int", "add", lit(a), lit(b), lit(c));
                assert!(defeq_for_test(
                    &ctx,
                    &infer_type(&ctx, &proof).unwrap(),
                    &required
                ));
                assert!(defeq_for_test(
                    &ctx,
                    &recheck(&ctx, &proof).unwrap(),
                    &required
                ));
            }
        }
    }
    for (a, b, c) in [
        (lit(i64::MIN), lit(-1), lit(i64::MAX)),
        (lit(i64::MAX), lit(i64::MAX), lit(-1)),
        (
            app(g("Int_negOfNat"), g("Zero")),
            app(g("Int_negSucc"), g("Zero")),
            app(g("Int_ofNat"), app(g("Succ"), g("Zero"))),
        ),
    ] {
        let proof = apps(g("add_assoc"), &[a.clone(), b.clone(), c.clone()]);
        let required = assoc("Int", "add", a, b, c);
        assert!(defeq_for_test(
            &ctx,
            &infer_type(&ctx, &proof).unwrap(),
            &required
        ));
        assert!(defeq_for_test(
            &ctx,
            &recheck(&ctx, &proof).unwrap(),
            &required
        ));
    }
}
#[test]
fn closed_downstream_addition_associativity_has_no_assumptions() {
    let mut ctx = context();
    let body = lambda3("Int", apps(g("add_assoc"), &[v("a"), v("b"), v("c")]));
    let required = expected("add_assoc");
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
    ctx.declare("closed_addition_associativity", required, body)
        .unwrap();
    let usage = axiom_dependencies_of(&ctx, "closed_addition_associativity").unwrap();
    assert!(
        usage.axioms.is_empty() && usage.unresolved.is_empty(),
        "{usage:?}"
    );
}
#[test]
fn constructor_addition_does_not_certify_an_off_by_one_associative_equation() {
    let ctx = context();
    let value = op("add", op("add", v("a"), v("b")), v("c"));
    let false_goal = forall3(
        "Int",
        eq(
            "Int",
            value.clone(),
            op(
                "add",
                op("add", v("a"), op("add", v("b"), v("c"))),
                Term::Lit(Literal::Int(1)),
            ),
        ),
    );
    let fake = lambda3("Int", apps(g("refl"), &[g("Int"), value]));
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
