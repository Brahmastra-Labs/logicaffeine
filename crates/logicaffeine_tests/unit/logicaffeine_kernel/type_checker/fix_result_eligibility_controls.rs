use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn lam(n: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
fn pi(n: &str, t: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    }
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_inductive("Input", Term::Sort(Universe::Type(0)));
    c.add_constructor("input_zero", "Input", g("Input"));
    c.add_constructor("input_succ", "Input", pi("x", g("Input"), g("Input")));
    c.add_inductive("Output", Term::Sort(Universe::Type(0)));
    c.add_constructor("output_left", "Output", g("Output"));
    c.add_constructor("output_right", "Output", g("Output"));
    c
}
fn fix(motive: Term) -> Term {
    Term::Fix {
        name: "Output".into(),
        body: Box::new(lam(
            "Output",
            g("Input"),
            lam(
                "unused",
                g("Input"),
                Term::Match {
                    discriminant: Box::new(v("Output")),
                    motive: Box::new(motive),
                    cases: vec![g("output_left"), lam("x", g("Input"), g("output_right"))],
                },
            ),
        )),
    }
}
fn applied(f: Term) -> Term {
    app(app(f, g("input_zero")), g("input_zero"))
}
#[test]
fn saturated_fix_result_eligibility_ignores_cosmetic_captured_names() {
    let c = context();
    let t = applied(fix(lam("Output", g("Input"), g("Output"))));
    assert!(!can_have_unit_type(&c, &t));
    let t = applied(fix(g("Output")));
    assert!(!can_have_unit_type(&c, &t));
}
#[test]
fn partial_extra_and_dependent_result_shapes_retain_inference() {
    let c = context();
    let f = fix(lam("q", g("Input"), g("Output")));
    assert!(can_have_unit_type(&c, &f));
    assert!(can_have_unit_type(&c, &app(f.clone(), g("input_zero"))));
    assert!(can_have_unit_type(&c, &app(applied(f), g("input_zero"))));
    assert!(can_have_unit_type(
        &c,
        &applied(fix(lam("Output", g("Input"), v("Output"))))
    ));
    assert!(can_have_unit_type(
        &c,
        &applied(fix(lam("q", g("Input"), lam("r", g("Input"), g("Output")))))
    ));
}
#[test]
fn visible_family_header_collision_polymorphism_and_aliases_retain_inference() {
    let term = applied(fix(lam("q", g("Input"), g("Output"))));
    let mut c = context();
    c.add_constructor("Output", "Input", g("Input"));
    assert!(can_have_unit_type(&c, &term));
    let mut c = context();
    c.add_definition("Output".into(), Term::Sort(Universe::Type(0)), g("Input"));
    assert!(can_have_unit_type(&c, &term));
    let mut c = context();
    c.add_universe_poly_inductive(
        "Output",
        vec!["u".into()],
        Term::Sort(Universe::Var("u".into())),
        0,
    );
    assert!(can_have_unit_type(&c, &term));
    let mut c = context();
    c.add_inductive("Output", Term::Sort(Universe::Var("unknown".into())));
    assert!(can_have_unit_type(&c, &term));
}
#[test]
fn registered_rewrites_retain_applied_fix_inference() {
    let mut c = context();
    c.add_declaration(
        "Eq",
        pi(
            "A",
            Term::Sort(Universe::Type(0)),
            pi("x", v("A"), pi("y", v("A"), Term::Sort(Universe::Prop))),
        ),
    );
    c.add_declaration("opaque", pi("x", g("Input"), g("Input")));
    let lhs = app(g("opaque"), g("input_zero"));
    let eq = app(app(app(g("Eq"), g("Input")), lhs), g("input_zero"));
    assert_eq!(infer_type(&c, &eq).unwrap(), Term::Sort(Universe::Prop));
    c.add_declaration("rewrite_zero", eq);
    c.add_simp_lemma("rewrite_zero").unwrap();
    assert!(can_have_unit_type(
        &c,
        &applied(fix(lam("q", g("Input"), g("Output"))))
    ));
}
#[test]
fn overlong_fix_application_spines_retain_inference() {
    let c = context();
    let t = (0..300).fold(fix(lam("q", g("Input"), g("Output"))), |f, _| {
        app(f, g("input_zero"))
    });
    assert!(can_have_unit_type(&c, &t));
}
