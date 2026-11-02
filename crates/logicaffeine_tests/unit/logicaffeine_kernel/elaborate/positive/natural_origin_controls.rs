use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn nat(n: u64) -> Term {
    Term::Lit(Literal::Nat(crate::BigInt::from_u64(n)))
}
fn apps(f: Term, args: Vec<Term>) -> Term {
    args.into_iter().fold(f, app)
}
fn power(n: u64) -> Term {
    apps(g("Nat_pow"), vec![nat(2), nat(n)])
}
fn context() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c
}
fn ofnat(n: u64) -> Term {
    apps(
        Term::Const {
            name: "OfNat_ofNat".into(),
            levels: vec![Universe::Type(0)],
        },
        vec![g("Nat"), nat(n), app(g("instOfNatNat"), nat(n))],
    )
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, body: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(g("Nat")),
        body: Box::new(body),
    }
}
#[test]
fn renamed_checked_power_and_dictionary_preserve_both_origin_orientations() {
    let mut c = context();
    c.declare(
        "Origins.operation",
        pi("a", g("Nat"), pi("b", g("Nat"), g("Nat"))),
        g("Nat_pow"),
    )
    .unwrap();
    c.declare("Origins.bound", g("Nat"), ofnat(4294967296))
        .unwrap();
    let p = apps(g("Origins.operation"), vec![nat(2), nat(32)]);
    assert!(equivalent(&c, &g("Origins.bound"), &p));
    assert!(equivalent(&c, &p, &g("Origins.bound")));
}
#[test]
fn adjacent_large_dictionary_values_cannot_certify_the_power() {
    for n in [4294967295, 4294967297] {
        let mut c = context();
        c.declare("Origins.wrong", g("Nat"), ofnat(n)).unwrap();
        assert!(!equivalent(&c, &g("Origins.wrong"), &power(32)));
        assert!(!equivalent(&c, &power(32), &g("Origins.wrong")));
    }
}
#[test]
fn application_children_have_distinct_origins_and_cannot_certify_a_sibling() {
    let c = context();
    let eq = |a: Term, b: Term| apps(g("Eq"), vec![g("Nat"), a, b]);
    let a = eq(ofnat(4294967296), nat(0));
    let same = eq(power(32), nat(0));
    let bad = eq(power(32), nat(1));
    assert!(equivalent(&c, &a, &same));
    assert!(!equivalent(&c, &a, &bad));
}
#[test]
fn failed_child_congruence_keeps_its_attempt_spent_but_can_finish_by_beta() {
    let c = context();
    let f = lam("ignored", nat(256));
    let a = app(f.clone(), power(8));
    let b = app(f, nat(255));
    let mut fuel = 4096;
    let mut used = false;
    assert!(same_inner(
        &c,
        &a,
        &b,
        Transparency::All,
        &mut fuel,
        0,
        true,
        &mut used
    ));
    assert!(used);
    assert!(fuel < 4096);
    assert!(!compact_data(&c, &power(8), &nat(256), &mut used, 1 << 18));
}
#[test]
fn opaque_origin_is_not_replaced_by_its_stored_dictionary_body() {
    let mut c = context();
    c.declare("Origins.opaque", g("Nat"), ofnat(4294967296))
        .unwrap();
    c.set_reducibility("Origins.opaque", crate::Reducibility::Irreducible);
    assert!(!equivalent(&c, &g("Origins.opaque"), &power(32)));
    assert!(!equivalent(&c, &power(32), &g("Origins.opaque")));
}
#[test]
fn unknown_opposite_origin_never_becomes_equality_evidence() {
    let c = context();
    assert!(!equivalent(&c, &ofnat(256), &g("Origins.missing")));
    assert!(!equivalent(&c, &g("Origins.missing"), &ofnat(256)));
}

#[test]
fn independently_beta_exposed_literal_keeps_the_opposite_power_origin() {
    let c = context();
    let a = app(lam("ignored", ofnat(4294967296)), nat(7));
    let b = power(32);
    assert_eq!(crate::infer_type(&c, &a).unwrap(), g("Nat"));
    assert_eq!(crate::recheck(&c, &b).unwrap(), g("Nat"));
    assert!(equivalent(&c, &a, &b));
    assert!(equivalent(&c, &b, &a));
}
