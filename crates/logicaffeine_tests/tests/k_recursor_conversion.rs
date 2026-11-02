use logicaffeine_kernel::{
    derive_recursor_universe_poly, infer_type, normalize_checked, recheck, Context,
    MutualInductive, Term, Universe,
};
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
fn same(x: Term, y: Term) -> Term {
    apps(g("Same"), vec![g("Nat"), x, y])
}
fn env() -> Context {
    let mut c = Context::new();
    c.add_mutual_inductives(&[MutualInductive {
        name: "Nat".into(),
        sort: ty(),
        num_params: 0,
        constructors: vec![
            ("Z".into(), g("Nat")),
            ("S".into(), pi("n", g("Nat"), g("Nat"))),
        ],
    }])
    .unwrap();
    c.add_mutual_inductives(&[MutualInductive {
        name: "Same".into(),
        sort: pi(
            "A",
            ty(),
            pi("x", v("A"), pi("y", v("A"), Term::Sort(Universe::Prop))),
        ),
        num_params: 2,
        constructors: vec![(
            "same_refl".into(),
            pi(
                "A",
                ty(),
                pi("x", v("A"), apps(g("Same"), vec![v("A"), v("x"), v("x")])),
            ),
        )],
    }])
    .unwrap();
    let (levels, ty, body) = derive_recursor_universe_poly(&c, "Same").unwrap();
    c.declare_universe_poly("same_rec", levels, ty, body)
        .unwrap();
    c.add_declaration("F", pi("n", g("Nat"), Term::Sort(Universe::Type(0))));
    c.add_declaration("value", app(g("F"), g("Z")));
    c
}
fn call(index: Term, proof: Term) -> Term {
    let motive = lam("y", g("Nat"), lam("h", same(g("Z"), v("y")), g("Nat")));
    apps(
        Term::Const {
            name: "same_rec".into(),
            levels: vec![Universe::Type(0)],
        },
        vec![g("Nat"), g("Z"), motive, g("Z"), index, proof],
    )
}
fn check_conversion(result: Term) -> Term {
    Term::Let {
        name: "checked".into(),
        ty: Box::new(app(g("F"), result)),
        value: Box::new(g("value")),
        body: Box::new(v("checked")),
    }
}
#[test]
fn k_reduction_computes_on_an_opaque_reflexive_proof() {
    let mut c = env();
    c.add_declaration("opaque", same(g("Z"), g("Z")));
    let result = call(g("Z"), g("opaque"));
    infer_type(&c, &result).unwrap();
    assert_eq!(normalize_checked(&c, &result).unwrap(), g("Z"));
    let checked = check_conversion(result);
    infer_type(&c, &checked).unwrap();
    recheck(&c, &checked).unwrap();
}
#[test]
fn k_reduction_requires_convertible_indices() {
    let mut c = env();
    let other = app(g("S"), g("Z"));
    c.add_declaration("opaque", same(g("Z"), other.clone()));
    let result = call(other, g("opaque"));
    infer_type(&c, &result).unwrap();
    assert_ne!(normalize_checked(&c, &result).unwrap(), g("Z"));
    let checked = check_conversion(result);
    assert!(infer_type(&c, &checked).is_err());
    assert!(recheck(&c, &checked).is_err());
}
#[test]
fn k_conversion_works_under_a_locally_bound_proof() {
    let c = env();
    let proof_ty = same(g("Z"), g("Z"));
    let term = lam("h", proof_ty.clone(), call(g("Z"), v("h")));
    assert_eq!(
        normalize_checked(&c, &term).unwrap(),
        lam("h", proof_ty.clone(), g("Z"))
    );
    let checked = lam("h", proof_ty, check_conversion(call(g("Z"), v("h"))));
    infer_type(&c, &checked).unwrap();
    recheck(&c, &checked).unwrap();
}
#[test]
fn ordinary_constructor_iota_still_computes() {
    let c = env();
    let proof = apps(g("same_refl"), vec![g("Nat"), g("Z")]);
    let result = call(g("Z"), proof);
    assert_eq!(normalize_checked(&c, &result).unwrap(), g("Z"));
    let checked = check_conversion(result);
    infer_type(&c, &checked).unwrap();
    recheck(&c, &checked).unwrap();
}

#[test]
fn fieldless_data_singleton_has_unit_eta_and_eliminator_reduction() {
    let mut c = env();
    c.add_mutual_inductives(&[MutualInductive {
        name: "Unit".into(),
        sort: ty(),
        num_params: 0,
        constructors: vec![("unit".into(), g("Unit"))],
    }])
    .unwrap();
    let (levels, ty, body) = derive_recursor_universe_poly(&c, "Unit").unwrap();
    c.declare_universe_poly("unit_rec", levels, ty, body)
        .unwrap();
    c.add_declaration("H", pi("x", g("Unit"), Term::Sort(Universe::Type(0))));
    c.add_declaration("h", app(g("H"), g("unit")));
    let conv = lam(
        "x",
        g("Unit"),
        Term::Let {
            name: "proof".into(),
            ty: Box::new(app(g("H"), v("x"))),
            value: Box::new(g("h")),
            body: Box::new(v("proof")),
        },
    );
    infer_type(&c, &conv).unwrap();
    recheck(&c, &conv).unwrap();
    let call = apps(
        Term::Const {
            name: "unit_rec".into(),
            levels: vec![Universe::Type(0)],
        },
        vec![lam("x", g("Unit"), g("Nat")), g("Z"), v("x")],
    );
    let t = lam("x", g("Unit"), check_conversion(call));
    infer_type(&c, &t).unwrap();
    recheck(&c, &t).unwrap();
}

#[test]
fn orphan_constructor_metadata_cannot_give_an_opaque_type_unit_eta() {
    let mut c = Context::new();
    c.add_declaration("Opaque", ty());
    c.add_constructor("orphan", "Opaque", g("Opaque"));
    c.add_declaration("a", g("Opaque"));
    c.add_declaration("b", g("Opaque"));
    c.add_declaration("F", pi("x", g("Opaque"), ty()));
    c.add_declaration("value", app(g("F"), g("a")));
    let t = Term::Let {
        name: "test".into(),
        ty: Box::new(app(g("F"), g("b"))),
        value: Box::new(g("value")),
        body: Box::new(v("test")),
    };
    assert!(infer_type(&c, &t).is_err());
    assert!(
        recheck(&c, &t).is_err(),
        "an opaque type is not a singleton inductive"
    );
}

#[test]
fn eta_short_indexed_motive_keeps_k_reduction() {
    let mut c = env();
    c.add_declaration("opaque", same(g("Z"), g("Z")));
    c.declare(
        "motive_tail",
        pi("y", g("Nat"), pi("h", same(g("Z"), v("y")), ty())),
        lam("y", g("Nat"), lam("h", same(g("Z"), v("y")), g("Nat"))),
    )
    .unwrap();
    let result = Term::Match {
        discriminant: Box::new(g("opaque")),
        motive: Box::new(lam("y", g("Nat"), app(g("motive_tail"), v("y")))),
        cases: vec![g("Z")],
    };
    infer_type(&c, &result).unwrap();
    assert_eq!(normalize_checked(&c, &result).unwrap(), g("Z"));
    let checked = check_conversion(result);
    infer_type(&c, &checked).unwrap();
    recheck(&c, &checked).unwrap();
}
