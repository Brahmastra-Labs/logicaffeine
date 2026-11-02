use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn lit(n: u64) -> Term {
    Term::Lit(Literal::Nat(crate::BigInt::from_u64(n)))
}
fn call(n: &str, a: Term, b: Term) -> Term {
    Term::App(
        Box::new(Term::App(Box::new(g(n)), Box::new(a))),
        Box::new(b),
    )
}
#[test]
fn large_power_uses_checked_recurrence_with_bounded_work() {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    let t = call("Nat_pow", lit(2), lit(32));
    assert!(constructor_equivalent(&c, &t, &lit(4_294_967_296), 1000));
    assert!(!constructor_equivalent(&c, &t, &lit(4_294_967_295), 1000));
}
