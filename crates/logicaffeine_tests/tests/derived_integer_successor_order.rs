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
    match name {
        "IntLaw_add_one_negSucc" => pi(
            "n",
            g("Nat"),
            eq(
                "Int",
                inc(app(g("Int_negSucc"), v("n"))),
                app(g("Int_negOfNat"), v("n")),
            ),
        ),
        "IntLaw_le_add_one" => pi(
            "a",
            g("Int"),
            pi(
                "b",
                g("Int"),
                eq(
                    "Bool",
                    op("le", inc(v("a")), inc(v("b"))),
                    op("le", v("a"), v("b")),
                ),
            ),
        ),
        "lt_add1_le" => pi(
            "a",
            g("Int"),
            pi(
                "b",
                g("Int"),
                pi(
                    "_",
                    fact("lt", v("a"), inc(v("b"))),
                    fact("le", v("a"), v("b")),
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
#[test]
fn incrementing_a_negative_constructor_has_a_checked_zero_crossing_proof() {
    actual("IntLaw_add_one_negSucc");
}
#[test]
fn cancelling_both_integer_increments_is_a_checked_bool_equality() {
    actual("IntLaw_le_add_one");
}
#[test]
fn upper_strict_bound_cancellation_preserves_the_exact_public_header() {
    let ctx = context();
    assert_eq!(ctx.get_global("lt_add1_le"), Some(&expected("lt_add1_le")));
    drop(ctx);
    actual("lt_add1_le");
}
#[test]
fn downstream_upper_bound_proof_closes_values_and_premise_for_both_kernels() {
    let mut ctx = context();
    let proof = lam(
        "a",
        g("Int"),
        lam(
            "b",
            g("Int"),
            lam(
                "h",
                fact("lt", v("a"), inc(v("b"))),
                apps(g("lt_add1_le"), &[v("a"), v("b"), v("h")]),
            ),
        ),
    );
    let want = expected("lt_add1_le");
    check(&ctx, &proof, &want);
    ctx.declare("closed_upper_bound_cancellation", want, proof)
        .unwrap();
    let usage = axiom_dependencies_of(&ctx, "closed_upper_bound_cancellation").unwrap();
    assert!(
        usage.axioms.is_empty() && usage.unresolved.is_empty(),
        "{usage:?}"
    );
}
#[test]
fn successor_order_proof_computes_and_rechecks_all_nine_constructor_boundaries() {
    let ctx = context();
    assert!(ctx.is_definition("IntLaw_le_add_one"));
    for (a, b) in [
        (2, 3),
        (2, -1),
        (2, -3),
        (-1, 2),
        (-3, 2),
        (-1, -1),
        (-1, -3),
        (-3, -1),
        (-3, -4),
    ] {
        let x = logicaffeine_kernel::int_inductive::canonical_int(a);
        let y = logicaffeine_kernel::int_inductive::canonical_int(b);
        let proof = apps(g("IntLaw_le_add_one"), &[x.clone(), y.clone()]);
        let want = eq(
            "Bool",
            op("le", inc(x.clone()), inc(y.clone())),
            op("le", x, y),
        );
        check(&ctx, &proof, &want);
        let reduced = normalize_checked(&ctx, &proof).unwrap();
        check(&ctx, &reduced, &want);
    }
}
#[test]
fn successor_bound_cancellation_handles_bigints_negative_zero_and_unit_crossings() {
    let ctx = context();
    assert!(ctx.is_definition("lt_add1_le"));
    let huge = BigInt::parse_decimal("1180591620717411303449").unwrap();
    for b in [
        huge.clone(),
        huge.negated(),
        BigInt::from_i64(i64::MIN),
        BigInt::from_i64(i64::MAX),
        BigInt::from_i64(-1),
        BigInt::from_i64(0),
    ] {
        for a in [b.clone(), b.sub(&BigInt::from_i64(1))] {
            let x = Term::Lit(int_lit(a));
            let y = Term::Lit(int_lit(b.clone()));
            let proof = apps(
                g("lt_add1_le"),
                &[
                    x.clone(),
                    y.clone(),
                    apps(g("refl"), &[g("Bool"), g("true")]),
                ],
            );
            check(&ctx, &proof, &fact("le", x, y));
        }
    }
    let zero = app(g("Int_negOfNat"), g("Zero"));
    let proof = apps(
        g("lt_add1_le"),
        &[
            zero.clone(),
            integer(0),
            apps(g("refl"), &[g("Bool"), g("true")]),
        ],
    );
    check(&ctx, &proof, &fact("le", zero, integer(0)));
}
#[test]
fn a_false_strict_premise_cannot_certify_the_cancelled_upper_bound() {
    let ctx = context();
    for (a, b) in [
        (integer(2), integer(1)),
        (integer(0), integer(-1)),
        (
            Term::Lit(int_lit(
                BigInt::parse_decimal("1180591620717411303450").unwrap(),
            )),
            Term::Lit(int_lit(
                BigInt::parse_decimal("1180591620717411303449").unwrap(),
            )),
        ),
    ] {
        let bad = apps(
            g("lt_add1_le"),
            &[a, b, apps(g("refl"), &[g("Bool"), g("true")])],
        );
        assert!(infer_type(&ctx, &bad).is_err());
        assert!(recheck(&ctx, &bad).is_err());
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
fn successor_cancellation_has_an_acyclic_definition_graph_without_remaining_assumptions() {
    let ctx = context();
    for name in ["IntLaw_add_one_negSucc", "IntLaw_le_add_one", "lt_add1_le"] {
        assert!(ctx.is_definition(name));
        acyclic(&ctx, name, &mut BTreeSet::new(), &mut BTreeSet::new());
        let usage = axiom_dependencies_of(&ctx, name).unwrap();
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "{name}: {usage:?}"
        );
    }
}

#[test]
fn negative_strict_premises_and_false_witnesses_cannot_forge_order_evidence() {
    let ctx = context();
    let huge_negative = BigInt::parse_decimal("-1180591620717411303449").unwrap();
    for (a, b) in [
        (integer(-1), integer(-2)),
        (integer(-2), integer(-3)),
        (
            Term::Lit(int_lit(huge_negative.add(&BigInt::from_i64(1)))),
            Term::Lit(int_lit(huge_negative)),
        ),
    ] {
        let forged = apps(
            g("lt_add1_le"),
            &[a, b, apps(g("refl"), &[g("Bool"), g("true")])],
        );
        assert!(infer_type(&ctx, &forged).is_err());
        assert!(recheck(&ctx, &forged).is_err());
    }
    let false_witness = apps(
        g("lt_add1_le"),
        &[
            integer(-2),
            integer(-1),
            apps(g("refl"), &[g("Bool"), g("false")]),
        ],
    );
    assert!(infer_type(&ctx, &false_witness).is_err());
    assert!(recheck(&ctx, &false_witness).is_err());
}
