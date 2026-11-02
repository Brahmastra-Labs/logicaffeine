use super::*;
fn context() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c
}
#[test]
fn unrelated_application_heads_do_not_exhaust_arithmetic_certification() {
    let c = context();
    let mut r = HeadReducer::new(&c, 100_000);
    let allowance = r.arithmetic_attempts;
    for n in 0..128 {
        let head = r.store.intern(&app(
            g("Nat_pow"),
            Term::Lit(Literal::Nat(crate::BigInt::from_u64(n))),
        ));
        assert!(r.certified_arithmetic(head).is_none());
    }
    assert_eq!(
        r.arithmetic_attempts, allowance,
        "non-candidates must not consume recurrence probes"
    );
    let head = r.store.intern(&g("Nat_pow"));
    assert_eq!(r.certified_arithmetic(head), Some(Arithmetic::Pow));
}
#[test]
fn unrelated_declared_signatures_do_not_exhaust_arithmetic_certification() {
    let mut c = context();
    for i in 0..128 {
        c.declare(
            &format!("noncandidate.{i}"),
            Term::Pi {
                param: "x".into(),
                param_type: Box::new(g("Nat")),
                body_type: Box::new(g("Nat")),
            },
            Term::Lambda {
                param: "x".into(),
                param_type: Box::new(g("Nat")),
                body: Box::new(Term::Var("x".into())),
            },
        )
        .unwrap();
    }
    let mut r = HeadReducer::new(&c, 100_000);
    let allowance = r.arithmetic_attempts;
    for i in 0..128 {
        let head = r.store.intern(&g(&format!("noncandidate.{i}")));
        assert!(r.certified_arithmetic(head).is_none());
    }
    assert_eq!(r.arithmetic_attempts, allowance);
    let head = r.store.intern(&g("Nat_pow"));
    assert_eq!(r.certified_arithmetic(head), Some(Arithmetic::Pow));
}
