use logicaffeine_kernel::{
    double_check, infer_type, normalize_checked, Context, DoubleCheck, MutualInductive, NestedDecl,
    Term, Universe,
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
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn ty() -> Term {
    Term::Sort(Universe::Type(0))
}
fn containers() -> Context {
    let mut c = Context::new();
    c.add_mutual_inductives(&[MutualInductive {
        name: "List".into(),
        sort: pi("A", ty(), ty()),
        num_params: 1,
        constructors: vec![
            ("nil".into(), pi("A", ty(), app(g("List"), v("A")))),
            (
                "cons".into(),
                pi(
                    "A",
                    ty(),
                    pi(
                        "head",
                        v("A"),
                        pi("tail", app(g("List"), v("A")), app(g("List"), v("A"))),
                    ),
                ),
            ),
        ],
    }])
    .unwrap();
    c.add_mutual_inductives(&[MutualInductive {
        name: "Array".into(),
        sort: pi("A", ty(), ty()),
        num_params: 1,
        constructors: vec![(
            "array".into(),
            pi(
                "A",
                ty(),
                pi("items", app(g("List"), v("A")), app(g("Array"), v("A"))),
            ),
        )],
    }])
    .unwrap();
    c
}
#[test]
fn compound_container_field_is_specialized_and_round_trips_in_both_kernels() {
    let mut c = containers();
    let info = c
        .add_nested_inductive(&NestedDecl {
            name: "Tree".into(),
            sort: ty(),
            constructors: vec![
                ("leaf".into(), g("Tree")),
                (
                    "node".into(),
                    pi("children", app(g("Array"), g("Tree")), g("Tree")),
                ),
            ],
        })
        .expect("Array's List field must create the inner sibling");
    assert_eq!(info.siblings, vec!["Tree$List", "Tree$Array"]);
    let list = app(app(g("Tree$List_cons"), g("leaf")), g("Tree$List_nil"));
    let specialized = app(g("Tree$Array_array"), list);
    let generic = app(
        app(g("array"), g("Tree")),
        app(
            app(app(g("cons"), g("Tree")), g("leaf")),
            app(g("nil"), g("Tree")),
        ),
    );
    let outer = info.isos.iter().find(|i| i.container == "Array").unwrap();
    for (input, first, second) in [
        (specialized, &outer.to_generic, &outer.from_generic),
        (generic, &outer.from_generic, &outer.to_generic),
    ] {
        let round = app(g(second), app(g(first), input.clone()));
        infer_type(&c, &round).unwrap();
        assert!(matches!(double_check(&c, &round), DoubleCheck::Agreed));
        assert_eq!(normalize_checked(&c, &round).unwrap(), input);
    }
}
#[test]
fn compound_negative_field_is_rejected_transactionally() {
    let mut c = Context::new();
    c.add_mutual_inductives(&[MutualInductive {
        name: "Void".into(),
        sort: ty(),
        num_params: 0,
        constructors: vec![],
    }])
    .unwrap();
    c.add_mutual_inductives(&[MutualInductive {
        name: "Contra".into(),
        sort: pi("A", ty(), ty()),
        num_params: 1,
        constructors: vec![(
            "contra".into(),
            pi(
                "A",
                ty(),
                pi(
                    "consumer",
                    pi("x", v("A"), g("Void")),
                    app(g("Contra"), v("A")),
                ),
            ),
        )],
    }])
    .unwrap();
    assert!(c
        .add_nested_inductive(&NestedDecl {
            name: "Bad".into(),
            sort: ty(),
            constructors: vec![(
                "bad".into(),
                pi("field", app(g("Contra"), g("Bad")), g("Bad"))
            )],
        })
        .is_err());
    assert!(c.get_global("Bad").is_none());
    assert!(c.get_global("bad").is_none());
}
