use logicaffeine_kernel::{infer_type, recheck, Context, MutualInductive, Term, Universe};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn apps(f: Term, args: Vec<Term>) -> Term {
    args.into_iter().fold(f, app)
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
fn ty() -> Term {
    Term::Sort(Universe::Type(0))
}
fn example(indexed: bool, external: &str, wrong: bool) -> (Context, Term) {
    let mut c = Context::new();
    c.add_mutual_inductives(&[MutualInductive {
        name: "N".into(),
        sort: ty(),
        num_params: 0,
        constructors: vec![("z".into(), g("N"))],
    }])
    .unwrap();
    let result = apps(g("Pair"), vec![v("A"), v("B")]);
    let result = if indexed { app(result, g("z")) } else { result };
    let arity = if indexed { pi("n", g("N"), ty()) } else { ty() };
    c.add_mutual_inductives(&[MutualInductive {
        name: "Pair".into(),
        sort: pi("A", ty(), pi("B", ty(), arity)),
        num_params: 2,
        constructors: vec![(
            "mk".into(),
            pi(
                "A",
                ty(),
                pi("B", ty(), pi("x", v("A"), pi("y", v("B"), result))),
            ),
        )],
    }])
    .unwrap();
    c = c.extend(external, ty()).extend("C", ty());
    let dtype = apps(g("Pair"), vec![v(external), v("C")]);
    let at_index = if indexed {
        app(dtype.clone(), v("n"))
    } else {
        dtype.clone()
    };
    let motive = lam("p", at_index, v(external));
    let motive = if indexed {
        lam("n", g("N"), motive)
    } else {
        motive
    };
    let dtype = if indexed { app(dtype, g("z")) } else { dtype };
    c = c.extend("pair", dtype.clone());
    let branch = lam(
        "left",
        v(external),
        lam("right", v("C"), if wrong { v("right") } else { v("left") }),
    );
    let term = Term::Match {
        discriminant: Box::new(v("pair")),
        motive: Box::new(motive),
        cases: vec![branch],
    };
    (
        c,
        lam(external, ty(), lam("C", ty(), lam("pair", dtype, term))),
    )
}
#[test]
fn simultaneous_parameter_instantiation_preserves_external_variables() {
    let (c, t) = example(false, "B", false);
    infer_type(&c, &t).unwrap();
    recheck(&c, &t).unwrap();
}
#[test]
fn indexed_parameter_instantiation_preserves_external_variables() {
    let (c, t) = example(true, "B", false);
    infer_type(&c, &t).unwrap();
    recheck(&c, &t).unwrap();
}
#[test]
fn generated_case_binders_do_not_capture_context_names() {
    for indexed in [false, true] {
        let (c, t) = example(indexed, "__arg0", false);
        infer_type(&c, &t).unwrap();
        recheck(&c, &t).unwrap();
    }
}
#[test]
fn parameter_capture_cannot_certify_the_other_field() {
    for indexed in [false, true] {
        let (c, t) = example(indexed, "B", true);
        assert!(infer_type(&c, &t).is_err());
        assert!(recheck(&c, &t).is_err());
    }
}
