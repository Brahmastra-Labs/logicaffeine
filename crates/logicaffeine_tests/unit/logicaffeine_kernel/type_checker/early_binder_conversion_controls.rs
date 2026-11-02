use super::*;
fn g(name: &str) -> Term {
    Term::Global(name.into())
}
fn v(name: &str) -> Term {
    Term::Var(name.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn lam(name: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: name.into(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn pi(name: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: name.into(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_inductive("Flag", Term::Sort(Universe::Type(0)));
    c.add_constructor("yes", "Flag", g("Flag"));
    c.add_constructor("no", "Flag", g("Flag"));
    c.add_declaration("A", Term::Sort(Universe::Type(0)));
    c.add_declaration("B", Term::Sort(Universe::Type(0)));
    c.add_declaration("a", g("A"));
    c
}
fn chosen_type() -> Term {
    Term::Match {
        discriminant: Box::new(g("yes")),
        motive: Box::new(lam("_", g("Flag"), Term::Sort(Universe::Type(0)))),
        cases: vec![g("A"), g("B")],
    }
}
fn telescope(depth: usize, prefix: &str, domain: Term, lambda: bool) -> Term {
    let mut out = if lambda {
        v(&format!("{prefix}0"))
    } else {
        g("A")
    };
    for i in 0..depth {
        let name = format!("{prefix}{i}");
        out = if lambda {
            lam(&name, domain.clone(), out)
        } else {
            pi(&name, domain.clone(), out)
        };
    }
    out
}
fn measured(depth: usize, lambda: bool) -> usize {
    let c = context();
    let lhs = telescope(depth, "x", chosen_type(), lambda);
    let rhs = telescope(depth, "y", g("A"), lambda);
    assert!(infer_type(&c, &lhs).is_ok());
    assert!(infer_type(&c, &rhs).is_ok());
    crate::term_store::deepen_stack_all(&[&lhs, &rhs], || {
        crate::term_store::reset_intern_visits();
        assert!(def_eq(&c, &lhs, &rhs));
        crate::term_store::intern_visits()
    })
    .expect("the test enters outside a kernel stack scope")
}
fn linear_work(lambda: bool) {
    let counts: Vec<_> = [8, 16, 32]
        .into_iter()
        .map(|n| measured(n, lambda))
        .collect();
    eprintln!("intern visits at 8/16/32 binders (lambda={lambda}): {counts:?}");
    for pair in counts.windows(2) {
        assert!(
            pair[1] <= 2 * pair[0] + 256,
            "doubling a telescope must not rebuild stores for every remaining suffix: {counts:?}"
        );
    }
}
#[test]
fn pi_telescope_conversion_has_linear_intern_work() {
    linear_work(false);
}
#[test]
fn lambda_telescope_conversion_has_linear_intern_work() {
    linear_work(true);
}
#[test]
fn early_binders_preserve_domains_and_universe_annotations() {
    let c = context();
    assert!(!def_eq(
        &c,
        &lam("x", g("A"), g("a")),
        &lam("x", g("B"), g("a"))
    ));
    assert!(!def_eq(
        &c,
        &pi("x", g("A"), g("A")),
        &pi("x", g("B"), g("A"))
    ));
    assert!(!def_eq(
        &c,
        &pi("T", Term::Sort(Universe::Type(0)), v("T")),
        &pi("T", Term::Sort(Universe::Type(1)), v("T"))
    ));
}
#[test]
fn early_binders_avoid_capture_and_preserve_dependent_domains() {
    let mut c = context().extend("__conversion", g("A"));
    let lhs = lam("x", chosen_type(), v("__conversion"));
    let equal = lam("renamed", g("A"), v("__conversion"));
    let captured = lam("__conversion", g("A"), v("__conversion"));
    assert!(def_eq(&c, &lhs, &equal));
    assert!(!def_eq(&c, &lhs, &captured));
    c.add_declaration("F", pi("x", g("A"), Term::Sort(Universe::Type(0))));
    let left = pi(
        "x",
        chosen_type(),
        pi("z", app(g("F"), v("x")), app(g("F"), v("x"))),
    );
    let right = pi(
        "y",
        g("A"),
        pi("q", app(g("F"), v("y")), app(g("F"), v("y"))),
    );
    assert!(infer_type(&c, &left).is_ok());
    assert!(infer_type(&c, &right).is_ok());
    assert!(def_eq(&c, &left, &right));
}
#[test]
fn early_binders_retain_proof_irrelevance() {
    let mut c = context();
    c.add_declaration("P", Term::Sort(Universe::Prop));
    c.add_declaration("p", g("P"));
    c.add_declaration("q", g("P"));
    assert!(def_eq(
        &c,
        &lam("x", chosen_type(), g("p")),
        &lam("y", g("A"), g("q"))
    ));
    assert!(!def_eq(
        &c,
        &lam("x", g("A"), g("p")),
        &lam("y", g("B"), g("q"))
    ));
}
#[test]
fn early_binders_leave_eta_and_head_reduction_fallbacks_available() {
    let mut c = context();
    c.add_declaration("f", pi("x", g("A"), g("A")));
    let eta = lam("x", g("A"), app(g("f"), v("x")));
    assert!(def_eq(&c, &eta, &g("f")));
    let identity = lam("x", g("A"), v("x"));
    c.declare("identityA", pi("x", g("A"), g("A")), identity.clone())
        .unwrap();
    assert!(def_eq(&c, &identity, &g("identityA")));
}
