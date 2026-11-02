use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    axiom_dependencies, axiom_dependencies_of, defeq_for_test, infer_type, int_lit,
    normalize_checked, recheck, BigInt, Context, Literal, Term,
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
fn natural(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(n)))
}
fn fact(op: &str, a: Term, b: Term) -> Term {
    apps(g("Eq"), &[g("Bool"), apps(g(op), &[a, b]), g("true")])
}
fn disj(op: &str, a: Term, b: Term) -> Term {
    apps(g("Or"), &[fact(op, a.clone(), b.clone()), fact(op, b, a)])
}
fn expected(carrier: &str, op: &str) -> Term {
    pi(
        "a",
        g(carrier),
        pi("b", g(carrier), disj(op, v("a"), v("b"))),
    )
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
fn actual(name: &str, carrier: &str, op: &str) {
    let ctx = context();
    let body = ctx
        .get_definition_body(name)
        .expect("totality must be a checked definition");
    assert_ne!(body, &g(name));
    let want = expected(carrier, op);
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
fn natural_order_totality_has_an_actual_closed_axiom_free_proof() {
    actual("Nat_le_total", "Nat", "natLe");
}
#[test]
fn integer_order_totality_preserves_the_public_or_contract_with_a_checked_body() {
    actual("le_total", "Int", "le");
}
#[test]
fn downstream_totality_closes_both_integer_variables_for_independent_rechecking() {
    let mut ctx = context();
    let proof = lam(
        "a",
        g("Int"),
        lam("b", g("Int"), apps(g("le_total"), &[v("a"), v("b")])),
    );
    let want = expected("Int", "le");
    check(&ctx, &proof, &want);
    ctx.declare("closed_integer_totality", want, proof).unwrap();
    let usage = axiom_dependencies_of(&ctx, "closed_integer_totality").unwrap();
    assert!(
        usage.axioms.is_empty() && usage.unresolved.is_empty(),
        "{usage:?}"
    );
}
#[test]
fn totality_instantiates_at_all_signs_zero_and_large_integer_boundaries() {
    let ctx = context();
    assert!(ctx.is_definition("le_total"));
    for a in [-3, 0, 4] {
        for b in [-4, 0, 3] {
            let first = logicaffeine_kernel::int_inductive::canonical_int(a);
            let second = integer(b);
            check(
                &ctx,
                &apps(g("le_total"), &[first.clone(), second.clone()]),
                &disj("le", first, second),
            );
        }
    }
    let zero = app(g("Int_negOfNat"), g("Zero"));
    check(
        &ctx,
        &apps(g("le_total"), &[zero.clone(), integer(0)]),
        &disj("le", zero, integer(0)),
    );
    let huge = BigInt::parse_decimal("1180591620717411303449").unwrap();
    for a in [
        huge.clone(),
        huge.negated(),
        BigInt::from_i64(i64::MIN),
        BigInt::from_i64(i64::MAX),
    ] {
        let x = Term::Lit(int_lit(a.clone()));
        let y = Term::Lit(int_lit(a.add(&BigInt::from_i64(1))));
        for (a, b) in [(x.clone(), y.clone()), (y, x)] {
            check(
                &ctx,
                &apps(g("le_total"), &[a.clone(), b.clone()]),
                &disj("le", a, b),
            );
        }
    }
}
#[test]
fn small_natural_totality_computes_a_real_or_witness_at_zero_and_successors() {
    let ctx = context();
    assert!(ctx.is_definition("Nat_le_total"));
    for (a, b) in [(0, 0), (0, 5), (5, 0), (3, 5), (5, 3), (3, 3)] {
        let term = apps(g("Nat_le_total"), &[natural(a), natural(b)]);
        let reduced = normalize_checked(&ctx, &term).unwrap();
        let mut head = &reduced;
        while let Term::App(f, _) = head {
            head = f;
        }
        assert!(
            matches!(head,Term::Global(n) if n=="left" || n=="right"),
            "{reduced}"
        );
        check(&ctx, &reduced, &disj("natLe", natural(a), natural(b)));
    }
}
#[test]
fn small_integer_totality_computes_real_witnesses_in_all_constructor_cases() {
    let ctx = context();
    assert!(ctx.is_definition("le_total"));
    for (a, b) in [(-3, -2), (-2, -3), (-1, 0), (0, -1), (2, 3), (3, 2), (0, 0)] {
        let term = apps(g("le_total"), &[integer(a), integer(b)]);
        let reduced = normalize_checked(&ctx, &term).unwrap();
        let mut head = &reduced;
        while let Term::App(f, _) = head {
            head = f;
        }
        assert!(
            matches!(head,Term::Global(n) if n=="left" || n=="right"),
            "{reduced}"
        );
        check(&ctx, &reduced, &disj("le", integer(a), integer(b)));
    }
}
#[test]
fn totality_does_not_certify_either_arbitrarily_chosen_false_order_witness() {
    let ctx = context();
    for (carrier, op, a, b) in [
        ("Int", "le", integer(2), integer(1)),
        ("Int", "le", integer(-1), integer(-2)),
        ("Nat", "natLe", natural(2), natural(1)),
    ] {
        let false_fact = fact(op, a.clone(), b.clone());
        let true_fact = fact(op, b, a);
        let bad = apps(
            g("left"),
            &[
                false_fact,
                true_fact,
                apps(g("refl"), &[g("Bool"), g("true")]),
            ],
        );
        assert!(
            infer_type(&ctx, &bad).is_err(),
            "{carrier} main checker accepted false branch"
        );
        assert!(
            recheck(&ctx, &bad).is_err(),
            "{carrier} independent checker accepted false branch"
        );
    }
}
