use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    axiom_dependencies, axiom_dependencies_of, defeq_for_test, eval_bool_tree, infer_type, int_lit,
    native_compile_decide, normalize_checked, normalize_checked_at, recheck, BigInt, Context,
    Literal, Term, Transparency,
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
fn op(n: &str, a: Term, b: Term) -> Term {
    apps(g(n), &[a, b])
}
fn pi(n: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn lam(n: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn integer(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
fn relation(name: &str, a: Term, b: Term) -> Term {
    apps(g("Eq"), &[g("Bool"), op(name, a, b), g("true")])
}
fn order_type(carrier: &str) -> Term {
    pi("a", g(carrier), pi("b", g(carrier), g("Bool")))
}
fn refl_type(carrier: &str, op: &str) -> Term {
    pi("a", g(carrier), relation(op, v("a"), v("a")))
}
fn trans_type(carrier: &str, name: &str) -> Term {
    pi(
        "a",
        g(carrier),
        pi(
            "b",
            g(carrier),
            pi(
                "c",
                g(carrier),
                pi(
                    "ab",
                    relation(name, v("a"), v("b")),
                    pi(
                        "bc",
                        relation(name, v("b"), v("c")),
                        relation(name, v("a"), v("c")),
                    ),
                ),
            ),
        ),
    )
}
fn context() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
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
fn body_has_contract(ctx: &Context, name: &str, expected: Term) {
    let body = ctx
        .get_definition_body(name)
        .expect("a checked proof body is required");
    assert_ne!(body, &g(name));
    assert!(defeq_for_test(
        ctx,
        ctx.get_global(name).unwrap(),
        &expected
    ));
    assert!(defeq_for_test(
        ctx,
        &infer_type(ctx, body).unwrap(),
        &expected
    ));
    assert!(defeq_for_test(ctx, &recheck(ctx, body).unwrap(), &expected));
    for usage in [
        axiom_dependencies(ctx, body),
        axiom_dependencies_of(ctx, name).unwrap(),
    ] {
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "{name}: {usage:?}"
        );
    }
}
fn all_paths(ctx: &Context, expression: Term, expected: bool) {
    let want = g(if expected { "true" } else { "false" });
    assert_eq!(normalize_checked(ctx, &expression).unwrap(), want);
    let equation = apps(g("Eq"), &[g("Bool"), expression.clone(), want.clone()]);
    let proof = apps(g("refl"), &[g("Bool"), want]);
    let consume = app(lam("h", equation, g("true")), proof);
    assert_eq!(infer_type(ctx, &consume).unwrap(), g("Bool"));
    assert_eq!(recheck(ctx, &consume).unwrap(), g("Bool"));
    assert_eq!(eval_bool_tree(ctx, &expression), Some(expected));
    assert_eq!(native_compile_decide(ctx, &expression), Some(expected));
}

#[test]
fn public_order_is_a_checked_bool_valued_canonical_definition() {
    let ctx = context();
    assert_eq!(ctx.get_definition_body("le"), Some(&g("Int_le")));
    body_has_contract(&ctx, "Int_le", order_type("Int"));
    body_has_contract(&ctx, "le", order_type("Int"));
}
#[test]
fn natural_order_reflexivity_is_checked_in_both_kernels_without_axioms() {
    body_has_contract(&context(), "Nat_le_refl", refl_type("Nat", "natLe"));
}
#[test]
fn natural_order_transitivity_is_checked_in_both_kernels_without_axioms() {
    body_has_contract(&context(), "Nat_le_trans", trans_type("Nat", "natLe"));
}
#[test]
fn integer_order_reflexivity_is_checked_in_both_kernels_without_axioms() {
    body_has_contract(&context(), "le_refl", refl_type("Int", "le"));
}
#[test]
fn integer_order_transitivity_is_checked_in_both_kernels_without_axioms() {
    body_has_contract(&context(), "le_trans", trans_type("Int", "le"));
}
#[test]
fn downstream_transitivity_closes_all_values_and_evidence_before_rechecking() {
    let mut ctx = context();
    let body = lam(
        "a",
        g("Int"),
        lam(
            "b",
            g("Int"),
            lam(
                "c",
                g("Int"),
                lam(
                    "ab",
                    relation("le", v("a"), v("b")),
                    lam(
                        "bc",
                        relation("le", v("b"), v("c")),
                        apps(g("le_trans"), &[v("a"), v("b"), v("c"), v("ab"), v("bc")]),
                    ),
                ),
            ),
        ),
    );
    let want = trans_type("Int", "le");
    assert!(defeq_for_test(
        &ctx,
        &infer_type(&ctx, &body).unwrap(),
        &want
    ));
    assert!(defeq_for_test(&ctx, &recheck(&ctx, &body).unwrap(), &want));
    ctx.declare("closed_order_transitivity", want, body)
        .unwrap();
    let usage = axiom_dependencies_of(&ctx, "closed_order_transitivity").unwrap();
    assert!(
        usage.axioms.is_empty() && usage.unresolved.is_empty(),
        "{usage:?}"
    );
}
#[test]
fn checked_order_preserves_sign_zero_constructor_and_bigint_semantics_on_all_paths() {
    let ctx = context();
    assert!(ctx.is_definition("le"));
    for a in [-3, 0, 4] {
        for b in [-4, 0, 3] {
            all_paths(
                &ctx,
                op(
                    "le",
                    logicaffeine_kernel::int_inductive::canonical_int(a),
                    integer(b),
                ),
                a <= b,
            );
        }
    }
    all_paths(
        &ctx,
        op("le", app(g("Int_negOfNat"), g("Zero")), integer(0)),
        true,
    );
    let huge = BigInt::parse_decimal("1180591620717411303449").unwrap();
    for a in [
        huge.clone(),
        huge.negated(),
        BigInt::from_i64(i64::MIN),
        BigInt::from_i64(i64::MAX),
    ] {
        let value = Term::Lit(int_lit(a.clone()));
        all_paths(
            &ctx,
            op(
                "le",
                value.clone(),
                Term::Lit(int_lit(a.add(&BigInt::from_i64(1)))),
            ),
            true,
        );
        all_paths(
            &ctx,
            op(
                "le",
                value.clone(),
                Term::Lit(int_lit(a.sub(&BigInt::from_i64(1)))),
            ),
            false,
        );
        all_paths(
            &ctx,
            op(
                "le",
                op("sub", op("add", value.clone(), integer(2)), integer(2)),
                value,
            ),
            true,
        );
    }
}
#[test]
fn strict_and_reversed_order_consume_the_checked_le_definition() {
    let ctx = context();
    assert!(ctx.is_definition("le"));
    for a in [-2, 0, 2] {
        for b in [-3, 0, 3] {
            for (name, want) in [("lt", a < b), ("ge", a >= b), ("gt", a > b)] {
                all_paths(&ctx, op(name, integer(a), integer(b)), want);
            }
        }
    }
}
#[test]
fn rebinding_le_or_its_checked_helpers_is_refused_and_every_native_meaning_is_unchanged() {
    let parent = context();
    assert!(parent.is_definition("le"));
    all_paths(&parent, op("le", integer(5), integer(2)), false);
    for dependency in ["le", "Int_le", "natLe"] {
        let mut child = parent.clone();
        let carrier = if dependency == "natLe" { "Nat" } else { "Int" };
        refused_as_a_change_of_meaning(
            child.declare(
                dependency,
                order_type(carrier),
                lam("a", g(carrier), lam("b", g(carrier), g("true"))),
            ),
            dependency,
        );
        all_paths(&child, op("le", integer(5), integer(2)), false);
        all_paths(&child, op("lt", integer(5), integer(2)), false);
        all_paths(&child, op("ge", integer(2), integer(5)), false);
        all_paths(&child, op("gt", integer(2), integer(5)), false);
        all_paths(&parent, op("le", integer(5), integer(2)), false);
    }
}
#[test]
fn checked_order_respects_each_transitive_unfolding_boundary() {
    let ctx = context();
    assert!(ctx.is_definition("le"));
    for blocked in ["le", "Int_le", "natLe"] {
        let opaque = |name: &str| name == blocked;
        let expression = op("le", integer(2), integer(3));
        let reduced =
            normalize_checked_at(&ctx, &expression, Transparency::Excluding(&opaque)).unwrap();
        assert!(
            reduced.to_string().contains(blocked),
            "{blocked}: {reduced}"
        );
        assert_ne!(reduced, g("true"));
    }
}
#[test]
fn false_order_and_false_transitivity_evidence_are_rejected_by_both_kernels() {
    let ctx = context();
    let bad = app(
        lam("h", relation("le", integer(2), integer(1)), g("true")),
        apps(g("refl"), &[g("Bool"), g("true")]),
    );
    assert!(infer_type(&ctx, &bad).is_err());
    assert!(recheck(&ctx, &bad).is_err());
    let false_transitivity = apps(
        g("le_trans"),
        &[
            integer(2),
            integer(1),
            integer(0),
            apps(g("refl"), &[g("Bool"), g("true")]),
            apps(g("refl"), &[g("Bool"), g("true")]),
        ],
    );
    assert!(infer_type(&ctx, &false_transitivity).is_err());
    assert!(recheck(&ctx, &false_transitivity).is_err());
}
