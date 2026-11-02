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
fn power(n: u64) -> Term {
    app(app(g("Nat_pow"), nat(2)), nat(n))
}
fn context() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c
}
fn ofnat(n: u64) -> Term {
    let f = Term::Const {
        name: "OfNat_ofNat".into(),
        levels: vec![Universe::Type(0)],
    };
    app(
        app(app(f, g("Nat")), nat(n)),
        app(g("instOfNatNat"), nat(n)),
    )
}
#[test]
fn large_natural_literal_equality_uses_the_existing_bounded_observer() {
    let c = context();
    let p = power(32);
    let n = nat(4294967296);
    for (a, b) in [(&p, &n), (&n, &p)] {
        let mut used = false;
        assert!(compact_data(&c, a, b, &mut used, 1 << 18));
        assert!(used);
    }
}
#[test]
fn original_numeral_dictionary_shape_under_a_local_is_positive() {
    let mut c = context();
    c.declare("NaturalGuard.size", g("Nat"), ofnat(4294967296))
        .unwrap();
    c.add("n", g("Nat"));
    let prefix = app(g("natLt"), Term::Var("n".into()));
    let a = app(prefix.clone(), g("NaturalGuard.size"));
    let b = app(prefix, power(32));
    assert!(equivalent_scoped(&c, &a, &b));
    assert!(equivalent_scoped(&c, &b, &a));
}
#[test]
fn completed_inequality_is_never_positive_and_consumes_the_one_attempt() {
    let c = context();
    for n in [255, 257] {
        let mut used = false;
        assert!(!compact_data(&c, &power(8), &nat(n), &mut used, 1 << 18));
        assert!(used);
        assert!(!compact_data(&c, &power(8), &nat(256), &mut used, 1 << 18));
    }
}
#[test]
fn zero_and_exhausted_fuel_cannot_authorize_natural_equality() {
    let c = context();
    let mut used = false;
    assert!(!compact_data(
        &c,
        &power(32),
        &nat(4294967296),
        &mut used,
        0
    ));
    assert!(!used);
    assert!(!compact_data(
        &c,
        &power(32),
        &nat(4294967296),
        &mut used,
        1
    ));
    assert!(used);
}
#[test]
fn unrelated_irreducibility_keeps_full_transparency_evaluator_ineligible() {
    let mut c = context();
    c.set_reducibility("unrelated", crate::Reducibility::Irreducible);
    let mut used = false;
    assert!(!compact_data(&c, &power(8), &nat(256), &mut used, 1 << 18));
    assert!(!used);
}
#[test]
fn unknown_or_noncanonical_inputs_are_inconclusive() {
    let empty = Context::new();
    let mut used = false;
    assert!(!compact_data(
        &empty,
        &power(8),
        &nat(256),
        &mut used,
        1 << 18
    ));
    assert!(used);
    let c = context();
    used = false;
    assert!(!compact_data(
        &c,
        &g("missing.operation"),
        &nat(256),
        &mut used,
        1 << 18
    ));
    assert!(used);
}
#[test]
fn free_names_holes_negative_and_oversized_literals_are_not_endpoints() {
    let mut c = context();
    c.add("n", g("Nat"));
    for t in [Term::Var("n".into()), Term::Hole] {
        let mut used = false;
        assert!(!compact_data(&c, &t, &nat(256), &mut used, 1 << 18));
        assert!(!used);
    }
    let negative = Term::Lit(Literal::Nat(crate::BigInt::from_i64(-1)));
    let mut used = false;
    assert!(!compact_data(&c, &power(8), &negative, &mut used, 1 << 18));
    assert!(!used);
    let large = nat(u64::MAX);
    assert!(!compact_data(&c, &power(8), &large, &mut used, 1 << 18));
    assert!(!used);
}
