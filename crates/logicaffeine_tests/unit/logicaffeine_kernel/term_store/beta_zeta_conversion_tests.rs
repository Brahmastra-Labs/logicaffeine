use super::*;
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn lam(n: &str, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(g("A")),
        body: Box::new(b),
    }
}
#[test]
fn opaque_match_branches_compare_by_beta_without_unfolding() {
    let c = crate::Context::new();
    let make = |b| Term::Match {
        discriminant: Box::new(v("d")),
        motive: Box::new(lam("d", g("A"))),
        cases: vec![b],
    };
    let lhs = make(app(lam("x", v("x")), g("a")));
    assert!(beta_zeta_equivalent(&c, &lhs, &make(g("a")), 100));
    assert!(!beta_zeta_equivalent(&c, &lhs, &make(g("b")), 100));
}
#[test]
fn beta_graph_conversion_preserves_capture_and_refuses_holes() {
    let c = crate::Context::new();
    let lhs = app(lam("x", lam("y", v("x"))), v("y"));
    assert!(beta_zeta_equivalent(&c, &lhs, &lam("fresh", v("y")), 100));
    assert!(!beta_zeta_equivalent(&c, &lhs, &lam("y", v("y")), 100));
    let erased = app(lam("x", g("a")), Term::Hole);
    assert!(!beta_zeta_equivalent(&c, &erased, &g("a"), 100));
    assert!(!beta_zeta_equivalent(&c, &lhs, &lam("fresh", v("y")), 0));
}
#[test]
fn beta_graph_conversion_does_not_import_float_identity_or_delta() {
    let mut c = crate::Context::new();
    c.add_definition("alias".into(), g("A"), g("a"));
    assert!(!beta_zeta_equivalent(&c, &g("alias"), &g("a"), 100));
    let nan = Term::Lit(crate::Literal::Float(f64::NAN));
    assert!(!beta_zeta_equivalent(&c, &nan, &nan, 100));
}
