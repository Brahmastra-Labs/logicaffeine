use logicaffeine_kernel::interface::lean as adapter;
use logicaffeine_kernel::{Context, Term, Universe};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, a: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(a),
        body: Box::new(b),
    }
}
fn env() -> Context {
    let mut c = Context::new();
    c.add_declaration("A", Term::Sort(Universe::Type(0)));
    c.add_declaration("B", Term::Sort(Universe::Type(0)));
    c.add_declaration("a", g("A"));
    c.add_declaration(
        "take",
        pi(
            "fa",
            pi("x", g("A"), g("A")),
            pi("fb", pi("x", g("B"), g("B")), g("A")),
        ),
    );
    c
}
#[test]
fn repeated_named_variables_keep_their_local_binder_types() {
    let c = env();
    let t = app(
        app(g("take"), lam("x", g("A"), v("x"))),
        lam("x", g("B"), v("x")),
    );
    assert_eq!(adapter::lean_type(&c, &t).unwrap(), g("A"));
    let bad = app(
        app(g("take"), lam("x", g("A"), v("x"))),
        lam("x", g("B"), g("a")),
    );
    assert!(adapter::lean_type(&c, &bad).is_err());
}
#[test]
fn a_new_check_does_not_reuse_a_previous_global_environment() {
    let mut c = env();
    c.add_declaration("value", g("A"));
    assert_eq!(adapter::lean_type(&c, &g("value")).unwrap(), g("A"));
    c.add_declaration("value", g("B"));
    assert_eq!(adapter::lean_type(&c, &g("value")).unwrap(), g("B"));
}
#[test]
fn dependent_local_types_keep_outer_scope_identity() {
    let c = env();
    let t = lam(
        "T",
        Term::Sort(Universe::Type(0)),
        lam(
            "x",
            v("T"),
            lam(
                "T",
                pi("ignored", v("T"), Term::Sort(Universe::Type(0))),
                v("x"),
            ),
        ),
    );
    assert!(adapter::lean_type(&c, &t).is_ok());
}
