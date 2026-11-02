use super::*;
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn lam(n: &str, body: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(g("A")),
        body: Box::new(body),
    }
}
#[test]
fn lazy_head_exhaustion_is_inconclusive() {
    let c = Context::new();
    assert!(beta_iota_zeta_head(&c, &v("x"), &mut 0).is_none());
    let omega = lam("x", app(v("x"), v("x")));
    let mut budget = 20;
    assert!(beta_iota_zeta_head(&c, &app(omega.clone(), omega), &mut budget).is_none());
    assert_eq!(budget, 0);
}
#[test]
fn lazy_head_discards_divergent_arguments_and_does_not_unfold_globals() {
    let mut c = Context::new();
    c.add_definition("cycle".into(), g("A"), g("cycle"));
    let omega = lam("x", app(v("x"), v("x")));
    let discarded = app(lam("ignored", v("answer")), app(omega.clone(), omega));
    assert_eq!(
        beta_iota_zeta_head(&c, &discarded, &mut 20),
        Some(v("answer"))
    );
    assert_eq!(
        beta_iota_zeta_head(&c, &g("cycle"), &mut 20),
        Some(g("cycle"))
    );
}
#[test]
fn lazy_head_checks_tree_growth_before_substitution() {
    let c = Context::new();
    let mut value = v("y");
    for _ in 0..100 {
        value = app(v("f"), value);
    }
    let mut body = v("x");
    for _ in 0..100 {
        body = app(v("x"), body);
    }
    let redex = app(lam("x", body), value);
    assert!(beta_iota_zeta_head(&c, &redex, &mut 256).is_none());
}
#[test]
fn lazy_head_beta_and_zeta_preserve_free_variables() {
    let c = Context::new();
    let beta = app(lam("x", lam("y", v("x"))), v("y"));
    let zeta = Term::Let {
        name: "x".into(),
        ty: Box::new(g("A")),
        value: Box::new(v("y")),
        body: Box::new(lam("y", v("x"))),
    };
    for term in [beta, zeta] {
        let Some(Term::Lambda { param, body, .. }) = beta_iota_zeta_head(&c, &term, &mut 20) else {
            panic!("expected lambda")
        };
        assert_ne!(param, "y");
        assert_eq!(*body, v("y"));
    }
}
#[test]
fn lazy_head_iota_uses_registered_branches_and_leaves_missing_cases_stuck() {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    let make = |cases| Term::Match {
        discriminant: Box::new(g("Zero")),
        motive: Box::new(lam("n", g("Nat"))),
        cases,
    };
    assert_eq!(
        beta_iota_zeta_head(&c, &make(vec![g("Zero"), lam("n", v("n"))]), &mut 20),
        Some(g("Zero"))
    );
    let missing = make(vec![]);
    assert_eq!(beta_iota_zeta_head(&c, &missing, &mut 20), Some(missing));
}
