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
fn pi(n: &str, t: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
fn base() -> Context {
    let mut c = Context::new();
    c.add_inductive("Input", Term::Sort(Universe::Type(0)));
    c.add_constructor("input", "Input", g("Input"));
    c
}
fn witness(c: &mut Context, carrier: Term, a: Term, b: Term) {
    c.add_declaration(
        "DependentFamily",
        pi("f", carrier, Term::Sort(Universe::Type(0))),
    );
    c.add_declaration("left_witness", app(g("DependentFamily"), a));
    let term = Term::Let {
        name: "checked".into(),
        ty: Box::new(app(g("DependentFamily"), b)),
        value: Box::new(g("left_witness")),
        body: Box::new(v("checked")),
    };
    infer_type(c, &term).unwrap();
    crate::recheck(c, &term).unwrap();
}
#[test]
fn a_lambda_cannot_require_a_unit_family_probe() {
    let c = base();
    assert!(!can_have_unit_type(&c, &lam("x", g("Input"), v("x"))));
}
#[test]
fn proposition_valued_lambdas_retain_proof_irrelevance() {
    for sort in [Universe::Prop, Universe::SProp] {
        let mut c = base();
        c.add_inductive("Proposition", Term::Sort(sort));
        c.add_declaration("first_proof", g("Proposition"));
        c.add_declaration("second_proof", g("Proposition"));
        let a = lam("n", g("Input"), g("first_proof"));
        let b = lam("m", g("Input"), g("second_proof"));
        assert!(proof_irrelevant(&c, &a, &b));
        assert!(def_eq(&c, &a, &b));
        witness(&mut c, pi("n", g("Input"), g("Proposition")), a, b);
    }
}
#[test]
fn unit_valued_lambdas_remain_extensionally_convertible() {
    let mut c = base();
    c.add_inductive("LocalUnit", Term::Sort(Universe::Type(0)));
    c.add_constructor("local_unit", "LocalUnit", g("LocalUnit"));
    c.add_declaration("neutral_unit", g("LocalUnit"));
    let a = lam("n", g("Input"), g("local_unit"));
    let b = lam("m", g("Input"), g("neutral_unit"));
    assert!(def_eq(&c, &a, &b));
    assert!(def_eq(&c, &b, &a));
    witness(&mut c, pi("n", g("Input"), g("LocalUnit")), a, b);
}
#[test]
fn lambda_rewrite_contexts_preserve_original_probe_eligibility() {
    let mut c = base();
    c.add_declaration(
        "Eq",
        pi(
            "A",
            Term::Sort(Universe::Type(0)),
            pi("x", v("A"), pi("y", v("A"), Term::Sort(Universe::Prop))),
        ),
    );
    c.add_declaration("opaque", pi("x", g("Input"), g("Input")));
    let eq = app(
        app(app(g("Eq"), g("Input")), app(g("opaque"), g("input"))),
        g("input"),
    );
    assert_eq!(infer_type(&c, &eq).unwrap(), Term::Sort(Universe::Prop));
    c.add_declaration("rewrite_input", eq);
    c.add_simp_lemma("rewrite_input").unwrap();
    assert!(can_have_unit_type(&c, &lam("x", g("Input"), v("x"))));
}

#[test]
fn alias_hidden_proposition_function_results_retain_inference() {
    let mut c = base();
    c.add_inductive("ActualProp", Term::Sort(Universe::Prop));
    c.add_declaration("first", g("ActualProp"));
    c.add_declaration("second", g("ActualProp"));
    c.add_definition(
        "HiddenProp".into(),
        Term::Sort(Universe::Prop),
        g("ActualProp"),
    );
    let f = Term::Fix {
        name: "proof_function".into(),
        body: Box::new(lam(
            "n",
            g("Input"),
            Term::Match {
                discriminant: Box::new(v("n")),
                motive: Box::new(lam("_", g("Input"), g("HiddenProp"))),
                cases: vec![g("first")],
            },
        )),
    };
    let a = lam("n", g("Input"), app(f, v("n")));
    let b = lam("m", g("Input"), g("second"));
    assert!(fixed_applied_match_result_family(&c, &a).is_none());
    assert!(proof_irrelevant(&c, &a, &b));
    witness(&mut c, pi("n", g("Input"), g("ActualProp")), a, b);
}
#[test]
fn lambda_data_results_preserve_equal_and_different_outputs() {
    let mut c = base();
    c.add_inductive("Output", Term::Sort(Universe::Type(0)));
    c.add_constructor("left", "Output", g("Output"));
    c.add_constructor("right", "Output", g("Output"));
    let a = lam("n", g("Input"), g("left"));
    let equal = lam("renamed", g("Input"), g("left"));
    let different = lam("n", g("Input"), g("right"));
    assert!(def_eq(&c, &a, &equal));
    assert!(!def_eq(&c, &a, &different));
    assert!(!proof_irrelevant(&c, &a, &different));
    let carrier = pi("n", g("Input"), g("Output"));
    witness(&mut c, carrier.clone(), a.clone(), equal);
    c.add_declaration(
        "DifferentFamily",
        pi("f", carrier, Term::Sort(Universe::Type(0))),
    );
    c.add_declaration("different_witness", app(g("DifferentFamily"), a));
    let term = Term::Let {
        name: "checked".into(),
        ty: Box::new(app(g("DifferentFamily"), different)),
        value: Box::new(g("different_witness")),
        body: Box::new(v("checked")),
    };
    assert!(infer_type(&c, &term).is_err());
    assert!(crate::recheck(&c, &term).is_err());
}
