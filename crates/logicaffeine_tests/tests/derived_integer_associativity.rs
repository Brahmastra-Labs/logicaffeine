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
    match name {
        "Nat_mul_add" => forall3(
            "Nat",
            eq(
                "Nat",
                op("Nat_mul", v("a"), op("Nat_add", v("b"), v("c"))),
                op(
                    "Nat_add",
                    op("Nat_mul", v("a"), v("b")),
                    op("Nat_mul", v("a"), v("c")),
                ),
            ),
        ),
        "Nat_mul_assoc" => forall3("Nat", assoc("Nat", "Nat_mul", v("a"), v("b"), v("c"))),
        "mul_assoc" => forall3("Int", assoc("Int", "mul", v("a"), v("b"), v("c"))),
        "IntLaw_mul_ofNat_negOfNat"
        | "IntLaw_mul_negOfNat_ofNat"
        | "IntLaw_mul_negOfNat_negOfNat" => {
            let negative_left = name != "IntLaw_mul_ofNat_negOfNat";
            let negative_right = name != "IntLaw_mul_negOfNat_ofNat";
            let repr = |negative: bool, x: Term| {
                app(
                    g(if negative {
                        "Int_negOfNat"
                    } else {
                        "Int_ofNat"
                    }),
                    x,
                )
            };
            pi(
                "a",
                "Nat",
                pi(
                    "b",
                    "Nat",
                    eq(
                        "Int",
                        op(
                            "mul",
                            repr(negative_left, v("a")),
                            repr(negative_right, v("b")),
                        ),
                        repr(
                            negative_left != negative_right,
                            op("Nat_mul", v("a"), v("b")),
                        ),
                    ),
                ),
            )
        }
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
fn natural_multiplication_distributes_with_a_closed_proof() {
    check_actual_body("Nat_mul_add");
}
#[test]
fn natural_multiplication_associates_with_a_closed_proof() {
    check_actual_body("Nat_mul_assoc");
}
#[test]
fn positive_times_negative_magnitude_has_a_closed_proof() {
    check_actual_body("IntLaw_mul_ofNat_negOfNat");
}
#[test]
fn negative_times_positive_magnitude_has_a_closed_proof() {
    check_actual_body("IntLaw_mul_negOfNat_ofNat");
}
#[test]
fn two_negative_magnitudes_have_a_closed_product_proof() {
    check_actual_body("IntLaw_mul_negOfNat_negOfNat");
}
#[test]
fn public_integer_multiplication_associativity_has_a_closed_proof() {
    check_actual_body("mul_assoc");
}

#[test]
fn derived_associativity_covers_signs_zero_and_large_literal_instances() {
    let ctx = context();
    assert!(ctx.is_definition("mul_assoc"));
    let lit = |n| Term::Lit(Literal::Int(n));
    for a in [-2, 0, 3] {
        for b in [-2, 0, 3] {
            for c in [-2, 0, 3] {
                let proof = apps(g("mul_assoc"), &[lit(a), lit(b), lit(c)]);
                let required = assoc("Int", "mul", lit(a), lit(b), lit(c));
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
    for (a, b, c) in [(i64::MIN, -1, 2), (i64::MAX, i64::MAX, -1)] {
        let proof = apps(g("mul_assoc"), &[lit(a), lit(b), lit(c)]);
        let required = assoc("Int", "mul", lit(a), lit(b), lit(c));
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
fn downstream_associativity_no_longer_inherits_an_assumption() {
    let mut ctx = context();
    let body = lambda3("Int", apps(g("mul_assoc"), &[v("a"), v("b"), v("c")]));
    let required = expected("mul_assoc");
    assert!(defeq_for_test(
        &ctx,
        &recheck(&ctx, &body).unwrap(),
        &required
    ));
    ctx.declare("closed_multiplication_associativity", required, body)
        .unwrap();
    let usage = axiom_dependencies_of(&ctx, "closed_multiplication_associativity").unwrap();
    assert!(
        usage.axioms.is_empty() && usage.unresolved.is_empty(),
        "{usage:?}"
    );
}
#[test]
fn constructor_induction_does_not_certify_an_incorrect_associative_equation() {
    let ctx = context();
    let value = op("mul", op("mul", v("a"), v("b")), v("c"));
    let false_goal = forall3(
        "Int",
        eq(
            "Int",
            value.clone(),
            op(
                "add",
                op("mul", v("a"), op("mul", v("b"), v("c"))),
                Term::Lit(Literal::Int(1)),
            ),
        ),
    );
    let fake_proof = lambda3("Int", apps(g("refl"), &[g("Int"), value]));
    let consumer = app(
        Term::Lambda {
            param: "h".into(),
            param_type: Box::new(false_goal),
            body: Box::new(g("true")),
        },
        fake_proof,
    );
    assert!(infer_type(&ctx, &consumer).is_err());
    assert!(recheck(&ctx, &consumer).is_err());
}
