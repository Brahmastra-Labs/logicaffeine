use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn pair() -> (Term, Term) {
    (
        Term::App(
            Box::new(Term::Lambda {
                param: "x".into(),
                param_type: Box::new(g("Nat")),
                body: Box::new(Term::Var("x".into())),
            }),
            Box::new(g("Zero")),
        ),
        g("Zero"),
    )
}
fn context() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c
}
#[test]
fn repeated_exact_conversion_reuses_the_checked_result() {
    let c = context();
    let (a, b) = pair();
    scope(&c, || {
        assert!(crate::type_checker::def_eq(&c, &a, &b));
        let work = COMPUTATIONS.with(|n| n.get());
        assert!(work > 0);
        assert!(crate::type_checker::def_eq(&c, &b, &a));
        assert_eq!(
            COMPUTATIONS.with(|n| n.get()),
            work,
            "a symmetric repeat in the same immutable context must not rerun conversion"
        );
        assert!(known(&c, &a, &b));
    });
    assert!(
        !known(&c, &a, &b),
        "evidence must die with its checking scope"
    );
}
