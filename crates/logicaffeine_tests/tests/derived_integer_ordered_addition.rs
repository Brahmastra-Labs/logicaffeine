use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    axiom_dependencies, axiom_dependencies_of, defeq_for_test, infer_type, int_lit,
    normalize_checked, recheck, BigInt, Context, Literal, Term,
};
use std::collections::BTreeSet;
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
fn pi(n: &str, t: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
fn integer(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
fn eq(carrier: &str, a: Term, b: Term) -> Term {
    apps(g("Eq"), &[g(carrier), a, b])
}
fn op(n: &str, a: Term, b: Term) -> Term {
    apps(g(n), &[a, b])
}
fn inc(a: Term) -> Term {
    op("add", a, integer(1))
}
fn fact(n: &str, a: Term, b: Term) -> Term {
    eq("Bool", op(n, a, b), g("true"))
}

fn expected(name: &str) -> Term {
    let zero = integer(0);
    let inv = |a: Term| app(g("Int_neg"), a);
    let translate = |a: Term, b: Term, c: Term| {
        eq(
            "Bool",
            op(
                "le",
                op("add", a.clone(), c.clone()),
                op("add", b.clone(), c),
            ),
            op("le", a, b),
        )
    };
    match name {
        "IntLaw_subNatNat_self" => pi(
            "n",
            g("Nat"),
            eq("Int", op("Int_subNatNat", v("n"), v("n")), zero),
        ),
        "IntLaw_add_neg_right" => pi(
            "a",
            g("Int"),
            eq("Int", op("add", v("a"), inv(v("a"))), zero),
        ),
        "IntLaw_le_add_ofNat" => pi(
            "n",
            g("Nat"),
            pi(
                "a",
                g("Int"),
                pi(
                    "b",
                    g("Int"),
                    translate(v("a"), v("b"), app(g("Int_ofNat"), v("n"))),
                ),
            ),
        ),
        "IntLaw_le_add_right" => pi(
            "a",
            g("Int"),
            pi(
                "b",
                g("Int"),
                pi("c", g("Int"), translate(v("a"), v("b"), v("c"))),
            ),
        ),

        "le_add_mono" => {
            let mut ty = fact("le", op("add", v("a"), v("c")), op("add", v("b"), v("d")));
            ty = pi("_", fact("le", v("c"), v("d")), ty);
            ty = pi("_", fact("le", v("a"), v("b")), ty);
            for name in ["d", "c", "b", "a"] {
                ty = pi(name, g("Int"), ty);
            }
            ty
        }
        "le_sub" => pi(
            "a",
            g("Int"),
            pi(
                "b",
                g("Int"),
                pi(
                    "_",
                    fact("le", v("a"), v("b")),
                    fact(
                        "le",
                        zero,
                        op("add", v("b"), op("mul", integer(-1), v("a"))),
                    ),
                ),
            ),
        ),
        _ => unreachable!(),
    }
}
fn context() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}
fn check(ctx: &Context, proof: &Term, want: &Term) {
    assert!(defeq_for_test(ctx, &infer_type(ctx, proof).unwrap(), want));
    assert!(defeq_for_test(ctx, &recheck(ctx, proof).unwrap(), want));
}
fn actual(name: &str) {
    let ctx = context();
    let body = ctx
        .get_definition_body(name)
        .expect("an actual checked proof body is required");
    assert_ne!(body, &g(name));
    let want = expected(name);
    assert!(defeq_for_test(&ctx, ctx.get_global(name).unwrap(), &want));
    check(&ctx, body, &want);
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
fn references(t: &Term) -> BTreeSet<String> {
    let mut pending = vec![t];
    let mut names = BTreeSet::new();
    while let Some(t) = pending.pop() {
        match t {
            Term::Global(n) | Term::Const { name: n, .. } => {
                names.insert(n.clone());
            }
            Term::App(f, a) => {
                pending.push(f);
                pending.push(a);
            }
            Term::Lambda {
                param_type, body, ..
            } => {
                pending.push(param_type);
                pending.push(body);
            }
            Term::Pi {
                param_type,
                body_type,
                ..
            } => {
                pending.push(param_type);
                pending.push(body_type);
            }
            Term::Let {
                ty, value, body, ..
            } => {
                pending.push(ty);
                pending.push(value);
                pending.push(body);
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                pending.push(discriminant);
                pending.push(motive);
                pending.extend(cases);
            }
            Term::Fix { body, .. } => pending.push(body),
            Term::MutualFix { defs, .. } => pending.extend(defs.iter().map(|(_, b)| b)),
            _ => {}
        }
    }
    names
}
fn acyclic(ctx: &Context, name: &str, active: &mut BTreeSet<String>, done: &mut BTreeSet<String>) {
    if done.contains(name) {
        return;
    }
    let Some(body) = ctx.get_definition_body(name) else {
        return;
    };
    assert!(
        active.insert(name.into()),
        "proof dependency cycle through {name}: {active:?}"
    );
    let mut names = references(body);
    names.extend(references(ctx.get_definition_type(name).unwrap()));
    for dependency in names {
        acyclic(ctx, &dependency, active, done);
    }
    active.remove(name);
    done.insert(name.into());
}

#[test]
fn signed_natural_self_difference_has_an_actual_checked_zero_proof() {
    actual("IntLaw_subNatNat_self");
}
#[test]
fn additive_inverse_cancellation_has_an_actual_checked_proof() {
    actual("IntLaw_add_neg_right");
}
#[test]
fn natural_translation_invariance_has_an_actual_checked_proof() {
    actual("IntLaw_le_add_ofNat");
}
#[test]
fn arbitrary_integer_translation_invariance_has_an_actual_checked_proof() {
    actual("IntLaw_le_add_right");
}
#[test]
fn addition_monotonicity_preserves_its_exact_public_header_and_has_a_checked_body() {
    let ctx = context();
    assert_eq!(
        ctx.get_global("le_add_mono"),
        Some(&expected("le_add_mono"))
    );
    drop(ctx);
    actual("le_add_mono");
}
#[test]
fn moving_an_inequality_to_zero_preserves_its_exact_public_header_and_has_a_checked_body() {
    let ctx = context();
    assert_eq!(ctx.get_global("le_sub"), Some(&expected("le_sub")));
    drop(ctx);
    actual("le_sub");
}
fn truth() -> Term {
    apps(g("refl"), &[g("Bool"), g("true")])
}
#[test]
fn translated_boolean_order_computes_real_proofs_across_signs_and_zero() {
    let ctx = context();
    assert!(ctx.is_definition("IntLaw_le_add_right"));
    for (a, b, c) in [
        (-3, -1, -2),
        (-1, 0, 1),
        (0, -1, -1),
        (2, 3, -3),
        (-2, 3, 2),
        (3, -2, -2),
        (0, 0, 0),
        (1, 1, -1),
        (-1, -1, 1),
    ] {
        let x = logicaffeine_kernel::int_inductive::canonical_int(a);
        let y = logicaffeine_kernel::int_inductive::canonical_int(b);
        let z = logicaffeine_kernel::int_inductive::canonical_int(c);
        let term = apps(g("IntLaw_le_add_right"), &[x.clone(), y.clone(), z.clone()]);
        let ty = eq(
            "Bool",
            op(
                "le",
                op("add", x.clone(), z.clone()),
                op("add", y.clone(), z),
            ),
            op("le", x, y),
        );
        check(&ctx, &term, &ty);
        check(&ctx, &normalize_checked(&ctx, &term).unwrap(), &ty);
    }
}
#[test]
fn order_addition_and_subtraction_proofs_handle_bigints_and_zero_crossings() {
    let ctx = context();
    assert!(ctx.is_definition("le_add_mono") && ctx.is_definition("le_sub"));
    let huge = BigInt::parse_decimal("1180591620717411303449").unwrap();
    for a in [
        huge.clone(),
        huge.negated(),
        BigInt::from_i64(i64::MIN),
        BigInt::from_i64(i64::MAX),
        BigInt::from_i64(-1),
        BigInt::from_i64(0),
    ] {
        let b = a.add(&BigInt::from_i64(1));
        let c = a.negated();
        let d = c.add(&BigInt::from_i64(1));
        let (a, b, c, d) = (
            Term::Lit(int_lit(a)),
            Term::Lit(int_lit(b)),
            Term::Lit(int_lit(c)),
            Term::Lit(int_lit(d)),
        );
        check(
            &ctx,
            &apps(
                g("le_add_mono"),
                &[a.clone(), b.clone(), c.clone(), d.clone(), truth(), truth()],
            ),
            &fact("le", op("add", a.clone(), c), op("add", b.clone(), d)),
        );
        check(
            &ctx,
            &apps(g("le_sub"), &[a.clone(), b.clone(), truth()]),
            &fact("le", integer(0), op("add", b, op("mul", integer(-1), a))),
        );
    }
}
#[test]
fn false_order_premises_and_false_witnesses_cannot_forge_translated_order() {
    let ctx = context();
    for (a, b) in [(-1, -2), (0, -1), (3, 2)] {
        let (a, b) = (integer(a), integer(b));
        for bad in [
            apps(g("le_sub"), &[a.clone(), b.clone(), truth()]),
            apps(
                g("le_add_mono"),
                &[
                    a.clone(),
                    b.clone(),
                    integer(0),
                    integer(0),
                    truth(),
                    truth(),
                ],
            ),
            apps(
                g("le_add_mono"),
                &[integer(0), integer(0), a, b, truth(), truth()],
            ),
        ] {
            assert!(infer_type(&ctx, &bad).is_err());
            assert!(recheck(&ctx, &bad).is_err());
        }
    }
    let false_witness = apps(g("refl"), &[g("Bool"), g("false")]);
    let bad = apps(g("le_sub"), &[integer(-2), integer(-1), false_witness]);
    assert!(infer_type(&ctx, &bad).is_err());
    assert!(recheck(&ctx, &bad).is_err());
}
#[test]
fn translation_does_not_turn_a_false_comparison_into_a_true_one() {
    let ctx = context();
    assert!(ctx.is_definition("IntLaw_le_add_right"));
    for c in [-5, 0, 5] {
        let proof = apps(
            g("IntLaw_le_add_right"),
            &[integer(-1), integer(-2), integer(c)],
        );
        let bad = eq(
            "Bool",
            op(
                "le",
                op("add", integer(-1), integer(c)),
                op("add", integer(-2), integer(c)),
            ),
            g("true"),
        );
        assert!(!defeq_for_test(
            &ctx,
            &infer_type(&ctx, &proof).unwrap(),
            &bad
        ));
        assert!(!defeq_for_test(&ctx, &recheck(&ctx, &proof).unwrap(), &bad));
    }
}
#[test]
fn downstream_additive_order_proofs_close_all_values_and_evidence() {
    let mut ctx = context();
    for name in ["le_add_mono", "le_sub"] {
        let mut variables = Vec::new();
        let mut cursor = expected(name);
        while let Term::Pi {
            param,
            param_type,
            body_type,
        } = cursor
        {
            let renamed = if param == "_" {
                format!("h{}", variables.len())
            } else {
                param
            };
            variables.push((renamed, *param_type));
            cursor = *body_type;
        }
        let args = variables.iter().map(|(n, _)| v(n)).collect::<Vec<_>>();
        let mut proof = apps(g(name), &args);
        for (n, ty) in variables.into_iter().rev() {
            proof = lam(&n, ty, proof);
        }
        let ty = expected(name);
        check(&ctx, &proof, &ty);
        let registered = format!("closed_{name}");
        ctx.declare(&registered, ty, proof).unwrap();
        let usage = axiom_dependencies_of(&ctx, &registered).unwrap();
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "{usage:?}"
        );
    }
}
#[test]
fn ordered_addition_definitions_are_acyclic_and_use_no_remaining_assumptions() {
    let ctx = context();
    for name in [
        "IntLaw_subNatNat_self",
        "IntLaw_add_neg_right",
        "IntLaw_le_add_ofNat",
        "IntLaw_le_add_right",
        "le_add_mono",
        "le_sub",
    ] {
        assert!(ctx.is_definition(name));
        acyclic(&ctx, name, &mut BTreeSet::new(), &mut BTreeSet::new());
        let usage = axiom_dependencies_of(&ctx, name).unwrap();
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "{name}: {usage:?}"
        );
    }
}
