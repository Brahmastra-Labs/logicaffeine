use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn pi(n: &str, ty: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(ty),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, ty: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(ty),
        body: Box::new(b),
    }
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor("Zero", "Nat", g("Nat"));
    c.add_constructor("Succ", "Nat", pi("n", g("Nat"), g("Nat")));
    c.add_declaration("mix", pi("a", g("Nat"), pi("b", g("Nat"), g("Nat"))));
    c
}
#[test]
fn visible_open_definition_body_cannot_supply_closed_identity() {
    let mut c = context().extend("outside", g("Nat"));
    let body = lam("x", g("Nat"), v("outside"));
    c.with_equations_suspended(|c| c.declare("open", pi("x", g("Nat"), g("Nat")), body.clone()))
        .unwrap();
    assert!(c.get_definition_body("open").is_some());
    assert!(crate::reduction::bounded_delta_head(&c, &g("open")).is_none());
    let mut budget = 256;
    assert!(!bounded_conversion(&c, &body, &g("open"), &mut budget));
}
#[test]
fn oversized_closed_body_is_inconclusive_before_specialization() {
    fn tree(depth: usize) -> Term {
        if depth == 0 {
            g("Zero")
        } else {
            app(app(g("mix"), tree(depth - 1)), tree(depth - 1))
        }
    }
    let mut c = context();
    let body = lam("x", g("Nat"), tree(11));
    c.with_equations_suspended(|c| c.declare("big", pi("x", g("Nat"), g("Nat")), body.clone()))
        .unwrap();
    assert!(c.get_definition_body("big").is_some());
    assert!(crate::reduction::bounded_delta_head(&c, &g("big")).is_none());
    let mut budget = 256;
    assert!(!bounded_conversion(&c, &body, &g("big"), &mut budget));
}
#[test]
fn partial_application_keeps_argument_and_residual_binder_obligations() {
    let mut c = context();
    let ty = pi("x", g("Nat"), pi("y", g("Nat"), g("Nat")));
    c.with_equations_suspended(|c| {
        c.declare(
            "a",
            ty.clone(),
            lam(
                "x",
                g("Nat"),
                lam("y", g("Nat"), app(app(g("mix"), v("x")), v("y"))),
            ),
        )
        .unwrap();
        c.declare(
            "b",
            ty,
            lam(
                "m",
                g("Nat"),
                lam("n", g("Nat"), app(app(g("mix"), v("m")), v("n"))),
            ),
        )
        .unwrap();
    });
    let a = app(g("a"), g("Zero"));
    let b = app(g("b"), g("Zero"));
    crate::reduction::take_bounded_head_clone_nodes();
    let mut budget = 256;
    assert!(bounded_conversion(&c, &a, &b, &mut budget));
    assert_eq!(crate::reduction::take_bounded_head_clone_nodes(), 0);
    let different = app(g("b"), app(g("Succ"), g("Zero")));
    let mut budget = 256;
    assert!(!bounded_conversion(&c, &a, &different, &mut budget));
}
