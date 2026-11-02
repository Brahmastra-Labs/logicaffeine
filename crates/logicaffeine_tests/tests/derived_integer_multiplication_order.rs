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
fn fact(n: &str, a: Term, b: Term) -> Term {
    eq("Bool", op(n, a, b), g("true"))
}

fn scaled(k: Term, a: Term, b: Term) -> Term {
    fact("le", op("mul", k.clone(), a), op("mul", k, b))
}
fn expected(name: &str) -> Term {
    match name {
        "IntLaw_mul_ofNat_succ" => pi(
            "n",
            g("Nat"),
            pi(
                "a",
                g("Int"),
                eq(
                    "Int",
                    op("mul", app(g("Int_ofNat"), app(g("Succ"), v("n"))), v("a")),
                    op(
                        "add",
                        op("mul", app(g("Int_ofNat"), v("n")), v("a")),
                        v("a"),
                    ),
                ),
            ),
        ),
        "IntLaw_le_mul_ofNat" => pi(
            "n",
            g("Nat"),
            pi(
                "a",
                g("Int"),
                pi(
                    "b",
                    g("Int"),
                    pi(
                        "_",
                        fact("le", v("a"), v("b")),
                        scaled(app(g("Int_ofNat"), v("n")), v("a"), v("b")),
                    ),
                ),
            ),
        ),
        "le_mul_nonneg" => pi(
            "k",
            g("Int"),
            pi(
                "a",
                g("Int"),
                pi(
                    "b",
                    g("Int"),
                    pi(
                        "_",
                        fact("le", integer(0), v("k")),
                        pi(
                            "_",
                            fact("le", v("a"), v("b")),
                            scaled(v("k"), v("a"), v("b")),
                        ),
                    ),
                ),
            ),
        ),
        _ => unreachable!(),
    }
}
fn truth() -> Term {
    apps(g("refl"), &[g("Bool"), g("true")])
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
fn successor_multiplication_has_an_actual_checked_signed_integer_proof() {
    actual("IntLaw_mul_ofNat_succ");
}
#[test]
fn natural_multiplier_order_has_an_actual_checked_inductive_proof() {
    actual("IntLaw_le_mul_ofNat");
}
#[test]
fn nonnegative_multiplication_preserves_exact_public_header_with_a_checked_body() {
    let ctx = context();
    assert_eq!(
        ctx.get_global("le_mul_nonneg"),
        Some(&expected("le_mul_nonneg"))
    );
    drop(ctx);
    actual("le_mul_nonneg");
}
#[test]
fn small_nonnegative_scaling_computes_real_proofs_across_zero_and_signs() {
    let ctx = context();
    assert!(ctx.is_definition("le_mul_nonneg"));
    for (k, a, b) in [
        (0, -3, 2),
        (0, 2, 3),
        (1, -3, -1),
        (1, -1, 0),
        (2, -2, 3),
        (3, 0, 2),
        (2, 3, 3),
    ] {
        let (k, a, b) = (
            logicaffeine_kernel::int_inductive::canonical_int(k),
            logicaffeine_kernel::int_inductive::canonical_int(a),
            logicaffeine_kernel::int_inductive::canonical_int(b),
        );
        let term = apps(
            g("le_mul_nonneg"),
            &[k.clone(), a.clone(), b.clone(), truth(), truth()],
        );
        let ty = scaled(k, a, b);
        check(&ctx, &term, &ty);
        check(&ctx, &normalize_checked(&ctx, &term).unwrap(), &ty);
    }
}
#[test]
fn multiplier_order_supports_large_positive_factors_and_mixed_sign_boundaries() {
    let ctx = context();
    assert!(ctx.is_definition("le_mul_nonneg"));
    let huge = BigInt::parse_decimal("1180591620717411303449").unwrap();
    for k in [
        BigInt::from_i64(0),
        BigInt::from_i64(1),
        BigInt::from_i64(i64::MAX),
        huge.clone(),
    ] {
        for a in [
            huge.negated(),
            BigInt::from_i64(i64::MIN),
            BigInt::from_i64(-1),
            BigInt::from_i64(0),
            huge.clone(),
        ] {
            let b = a.add(&BigInt::from_i64(1));
            let (k, a, b) = (
                Term::Lit(int_lit(k.clone())),
                Term::Lit(int_lit(a)),
                Term::Lit(int_lit(b)),
            );
            check(
                &ctx,
                &apps(
                    g("le_mul_nonneg"),
                    &[k.clone(), a.clone(), b.clone(), truth(), truth()],
                ),
                &scaled(k, a, b),
            );
        }
    }
    let z = app(g("Int_negOfNat"), g("Zero"));
    check(
        &ctx,
        &apps(
            g("le_mul_nonneg"),
            &[z.clone(), integer(-1), integer(1), truth(), truth()],
        ),
        &scaled(z, integer(-1), integer(1)),
    );
}
#[test]
fn a_negative_multiplier_cannot_supply_nonnegative_evidence_even_for_true_conclusions() {
    let ctx = context();
    for k in [
        integer(-1),
        integer(-3),
        Term::Lit(int_lit(
            BigInt::parse_decimal("-1180591620717411303449").unwrap(),
        )),
    ] {
        for (a, b) in [(integer(-2), integer(-1)), (integer(0), integer(0))] {
            let bad = apps(g("le_mul_nonneg"), &[k.clone(), a, b, truth(), truth()]);
            assert!(infer_type(&ctx, &bad).is_err());
            assert!(recheck(&ctx, &bad).is_err());
        }
    }
}
#[test]
fn false_order_or_false_boolean_witnesses_cannot_hide_behind_zero_scaling() {
    let ctx = context();
    for k in [integer(0), integer(1), integer(4)] {
        for (a, b) in [(integer(-1), integer(-2)), (integer(1), integer(0))] {
            let bad = apps(g("le_mul_nonneg"), &[k.clone(), a, b, truth(), truth()]);
            assert!(infer_type(&ctx, &bad).is_err());
            assert!(recheck(&ctx, &bad).is_err());
        }
    }
    let false_witness = apps(g("refl"), &[g("Bool"), g("false")]);
    for (hk, hab) in [(false_witness.clone(), truth()), (truth(), false_witness)] {
        let bad = apps(
            g("le_mul_nonneg"),
            &[integer(1), integer(-2), integer(-1), hk, hab],
        );
        assert!(infer_type(&ctx, &bad).is_err());
        assert!(recheck(&ctx, &bad).is_err());
    }
}
#[test]
fn downstream_multiplication_order_closes_multiplier_values_and_both_premises() {
    let mut ctx = context();
    let proof = lam(
        "k",
        g("Int"),
        lam(
            "a",
            g("Int"),
            lam(
                "b",
                g("Int"),
                lam(
                    "hk",
                    fact("le", integer(0), v("k")),
                    lam(
                        "hab",
                        fact("le", v("a"), v("b")),
                        apps(
                            g("le_mul_nonneg"),
                            &[v("k"), v("a"), v("b"), v("hk"), v("hab")],
                        ),
                    ),
                ),
            ),
        ),
    );
    let ty = expected("le_mul_nonneg");
    check(&ctx, &proof, &ty);
    ctx.declare("closed_nonnegative_scaling", ty, proof)
        .unwrap();
    let usage = axiom_dependencies_of(&ctx, "closed_nonnegative_scaling").unwrap();
    assert!(
        usage.axioms.is_empty() && usage.unresolved.is_empty(),
        "{usage:?}"
    );
}
#[test]
fn multiplication_order_proofs_are_acyclic_and_have_no_remaining_assumptions() {
    let ctx = context();
    for name in [
        "IntLaw_mul_ofNat_succ",
        "IntLaw_le_mul_ofNat",
        "le_mul_nonneg",
    ] {
        assert!(ctx.is_definition(name));
        acyclic(&ctx, name, &mut BTreeSet::new(), &mut BTreeSet::new());
        let usage = axiom_dependencies_of(&ctx, name).unwrap();
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "{name}:{usage:?}"
        );
    }
}
