use super::*;
use crate::prelude::StandardLibrary;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn nat(n: u64) -> Term {
    Term::Lit(Literal::Nat(crate::BigInt::from_u64(n)))
}
fn power(n: u64) -> Term {
    app(app(g("Nat_pow"), nat(2)), nat(n))
}
fn context() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c.add("n", g("Nat"));
    c
}
fn lam(n: &str, body: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(g("Nat")),
        body: Box::new(body),
    }
}
#[test]
fn original_closed_entry_and_compact_evaluator_remain_closed() {
    let c = context();
    let n = v("n");
    assert!(!equivalent(&c, &n, &n));
    let mut attempted = false;
    assert!(!compact_data(&c, &n, &g("true"), &mut attempted, 1 << 18));
    assert!(!attempted);
    assert!(equivalent_scoped(&c, &n, &n));
}
#[test]
fn actual_large_power_shape_is_decided_without_full_normalization() {
    let mut c = context();
    c.declare("ScopedGuard.size", g("Nat"), power(32)).unwrap();
    let prefix = app(g("natLt"), v("n"));
    let a = app(prefix.clone(), g("ScopedGuard.size"));
    let b = app(prefix, power(32));
    assert!(equivalent_scoped(&c, &a, &b));
    assert!(equivalent_scoped(&c, &b, &a));
}
#[test]
fn unknown_names_in_erased_values_and_annotations_refuse_scope() {
    let c = context();
    for bad in [v("ghost"), Term::Hole] {
        let erased = Term::Let {
            name: "unused".into(),
            ty: Box::new(g("Nat")),
            value: Box::new(bad.clone()),
            body: Box::new(v("n")),
        };
        assert!(!equivalent_scoped(&c, &erased, &erased));
        let annotated = Term::Lambda {
            param: "x".into(),
            param_type: Box::new(bad),
            body: Box::new(v("x")),
        };
        assert!(!equivalent_scoped(&c, &annotated, &annotated));
    }
}
#[test]
fn a_name_mentioned_only_in_a_local_type_is_not_authorized() {
    let mut c = context();
    c.add("w", v("ghost"));
    assert!(!equivalent_scoped(&c, &v("ghost"), &v("ghost")));
}
#[test]
fn context_or_lexical_binding_cannot_authorize_metavariables() {
    let mut c = context();
    c.add("?pending", g("Nat"));
    assert!(!equivalent_scoped(&c, &v("?pending"), &v("?pending")));
    let bound = lam("?pending", v("?pending"));
    assert!(!equivalent_scoped(&c, &bound, &bound));
}
#[test]
fn shadowing_is_lexical_and_capture_cannot_create_a_positive_result() {
    let c = context();
    let good = lam("n", v("n"));
    let alpha = lam("z", v("z"));
    assert!(equivalent_scoped(&c, &good, &alpha));
    let captures = app(lam("x", lam("n", v("x"))), v("n"));
    assert!(!equivalent_scoped(&c, &captures, &good));
    let free = lam("z", v("n"));
    assert!(!equivalent_scoped(&c, &free, &good));
}
#[test]
fn scoped_walk_shares_the_depth_and_work_limits() {
    let c = context();
    let n = v("n");
    let tr = Transparency::All;
    let mut fuel = 0;
    assert!(!same_scoped(
        &c, &n, &n, tr, &mut fuel, 0, false, &mut false
    ));
    let mut fuel = 8;
    assert!(!same_scoped(
        &c, &n, &n, tr, &mut fuel, 64, false, &mut false
    ));
    assert_eq!(fuel, 8);
    let a = app(g("Succ"), n.clone());
    let b = app(g("Succ"), g("Zero"));
    let mut fuel = 2;
    assert!(!same_scoped(
        &c, &a, &b, tr, &mut fuel, 0, false, &mut false
    ));
    assert_eq!(fuel, 0);
}
#[test]
fn oversized_originals_are_refused_before_scoped_alpha() {
    let mut c = context();
    let name = "n".repeat(257);
    c.add(&name, g("Nat"));
    assert!(!equivalent_scoped(&c, &v(&name), &v(&name)));
    let mut deep = v("n");
    for _ in 0..66 {
        deep = app(g("Succ"), deep);
    }
    assert!(!equivalent_scoped(&c, &deep, &deep));
}
#[test]
fn active_simp_rules_disable_scoped_positive_evidence() {
    let mut c = context();
    let pi = |a: Term, b: Term| Term::Pi {
        param: "x".into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    };
    c.declare("ScopedGuard.id", pi(g("Nat"), g("Nat")), lam("x", v("x")))
        .unwrap();
    let equality = app(
        app(app(g("Eq"), g("Nat")), app(g("ScopedGuard.id"), v("x"))),
        v("x"),
    );
    c.declare(
        "ScopedGuard.rule",
        pi(g("Nat"), equality),
        lam("x", app(app(g("refl"), g("Nat")), v("x"))),
    )
    .unwrap();
    c.add_simp_lemma("ScopedGuard.rule").unwrap();
    assert!(!equivalent_scoped(&c, &v("n"), &v("n")));
}
