use super::*;
fn lit(n: u64) -> Term {
    Term::Lit(Literal::Nat(crate::BigInt::from_u64(n)))
}
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn context() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c
}
#[test]
fn completed_arithmetic_decides_both_directions_and_inequality() {
    let c = context();
    let t = app(app(g("Nat_pow"), lit(2)), lit(64));
    let expected = Term::Lit(Literal::Nat(
        crate::BigInt::parse_decimal("18446744073709551616").unwrap(),
    ));
    assert_eq!(
        natural_literal_conversion(&c, &t, &expected, 1000),
        Some(true)
    );
    assert_eq!(
        natural_literal_conversion(&c, &expected, &t, 1000),
        Some(true)
    );
    assert_eq!(
        natural_literal_conversion(&c, &t, &lit(u64::MAX), 1000),
        Some(false)
    );
    assert!(!crate::type_checker::def_eq(&c, &t, &lit(u64::MAX)));
}
#[test]
fn opaque_unknown_open_and_erased_holes_remain_inconclusive() {
    let mut c = context();
    c.add_declaration("opaque.nat", g("Nat"));
    let erased = Term::Let {
        name: "ignored".into(),
        ty: Box::new(g("Nat")),
        value: Box::new(Term::Hole),
        body: Box::new(lit(1)),
    };
    for t in [g("opaque.nat"), Term::Hole, Term::Var("x".into()), erased] {
        assert_eq!(natural_literal_conversion(&c, &t, &lit(1), 1000), None);
    }
    let t = app(app(g("Nat_pow"), lit(2)), lit(32));
    assert_eq!(natural_literal_conversion(&c, &t, &lit(1), 0), None);
    assert_eq!(
        natural_literal_conversion(&c.order_recurrence_context(), &t, &lit(1), 1000),
        None
    );
}
#[test]
fn peano_literal_bridge_checks_remaining_tails() {
    let mut c = context();
    c.add_declaration("unknown.tail", g("Nat"));
    let concrete = app(g("Succ"), app(g("Succ"), g("Zero")));
    assert_eq!(
        natural_literal_conversion(&c, &concrete, &lit(2), 1000),
        Some(true)
    );
    assert_eq!(
        natural_literal_conversion(&c, &concrete, &lit(3), 1000),
        Some(false)
    );
    let unknown = app(g("Succ"), g("unknown.tail"));
    assert_eq!(
        natural_literal_conversion(&c, &unknown, &lit(1), 1000),
        None
    );
    assert!(!constructor_equivalent(&c, &unknown, &lit(1), 1000));
}
