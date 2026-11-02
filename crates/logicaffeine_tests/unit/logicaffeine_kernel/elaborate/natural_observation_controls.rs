use super::*;
use crate::Literal;
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
    crate::prelude::StandardLibrary::register(&mut c);
    c.add("n", g("Nat"));
    c
}
fn ofnat(n: u64) -> Term {
    let f = Term::Const {
        name: "OfNat_ofNat".into(),
        levels: vec![crate::Universe::Type(0)],
    };
    app(
        app(app(f, g("Nat")), nat(n)),
        app(g("instOfNatNat"), nat(n)),
    )
}
fn reset() {
    LAZY_UNIFY_FULL_NORMALIZATIONS.with(|n| n.set(0));
}
fn count() -> usize {
    LAZY_UNIFY_FULL_NORMALIZATIONS.with(|n| n.get())
}
#[test]
fn local_bound_with_literal_alias_and_computed_power_keeps_normalizer_cold() {
    let mut c = context();
    c.declare("NaturalGate.bound", g("Nat"), nat(256)).unwrap();
    let prefix = app(g("natLt"), v("n"));
    let a = app(prefix.clone(), g("NaturalGate.bound"));
    let b = app(prefix, power(8));
    assert_eq!(crate::infer_type(&c, &a).unwrap(), g("Bool"));
    assert_eq!(crate::recheck(&c, &b).unwrap(), g("Bool"));
    for (a, b) in [(&a, &b), (&b, &a)] {
        reset();
        assert!(unify(&c, &mut MetaCtx::new(), a, b));
        assert_eq!(count(), 0);
    }
}
#[test]
fn imported_shape_numeral_dictionary_alias_matches_computed_power() {
    let mut c = context();
    c.declare("NaturalGate.dictionaryBound", g("Nat"), ofnat(256))
        .unwrap();
    let prefix = app(g("natLt"), v("n"));
    let a = app(prefix.clone(), g("NaturalGate.dictionaryBound"));
    let b = app(prefix, power(8));
    reset();
    assert!(unify(&c, &mut MetaCtx::new(), &a, &b));
    assert_eq!(count(), 0);
}
#[test]
fn closed_natural_computation_uses_existing_observation_without_full_normalization() {
    let c = context();
    reset();
    assert!(unify(&c, &mut MetaCtx::new(), &power(8), &nat(256)));
    assert_eq!(count(), 0);
}
#[test]
fn adjacent_incorrect_literal_claims_remain_false() {
    let c = context();
    for expected in [255, 257] {
        assert!(!unify(&c, &mut MetaCtx::new(), &power(8), &nat(expected)));
    }
}

#[test]
fn irreducible_natural_alias_is_not_arithmetic_evidence() {
    let mut c = context();
    c.declare("NaturalGate.hidden", g("Nat"), nat(256)).unwrap();
    c.set_reducibility("NaturalGate.hidden", crate::Reducibility::Irreducible);
    assert!(!unify(
        &c,
        &mut MetaCtx::new(),
        &g("NaturalGate.hidden"),
        &power(8)
    ));
}
