use super::*;
#[test]
fn admitted_failures_still_spend_once_and_exhaustion_does_not_reset() {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    let pi = |n: &str, b: Term| Term::Pi {
        param: n.into(),
        param_type: Box::new(g("Nat")),
        body_type: Box::new(b),
    };
    let lam = |n: &str, b: Term| Term::Lambda {
        param: n.into(),
        param_type: Box::new(g("Nat")),
        body: Box::new(b),
    };
    c.declare(
        "constant.program",
        pi("x", pi("y", g("Nat"))),
        lam("x", lam("y", g("Zero"))),
    )
    .unwrap();
    let mut r = HeadReducer::new(&c, 100_000);
    r.arithmetic_attempts = 1;
    let bad = r.store.intern(&g("constant.program"));
    let good = r.store.intern(&g("Nat_pow"));
    assert_eq!(r.certified_arithmetic(bad), None);
    assert_eq!(r.arithmetic_attempts, 0);
    assert_eq!(r.certified_arithmetic(bad), None);
    assert_eq!(r.arithmetic_attempts, 0);
    assert_eq!(r.certified_arithmetic(good), None);
    assert_eq!(r.arithmetic_attempts, 0);
    let mut fresh = HeadReducer::new(&c, 100_000);
    let good = fresh.store.intern(&g("Nat_pow"));
    assert_eq!(fresh.certified_arithmetic(good), Some(Arithmetic::Pow));
    fresh.arithmetic_attempts = 0;
    assert_eq!(fresh.certified_arithmetic(good), Some(Arithmetic::Pow));
}
