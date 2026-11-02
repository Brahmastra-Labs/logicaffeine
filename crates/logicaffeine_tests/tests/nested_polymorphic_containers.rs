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
fn k(n: &str, u: Universe) -> Term {
    Term::Const {
        name: n.into(),
        levels: vec![u],
    }
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
    let u = Universe::Var("u".into());
    let sort = Term::Sort(u.clone());
    let list = app(k("List", u.clone()), v("A"));
    c.add_universe_poly_inductives(
        &["u".into()],
        &[MutualInductive {
            name: "List".into(),
            sort: pi("A", sort.clone(), sort.clone()),
            num_params: 1,
            constructors: vec![
                ("nil".into(), pi("A", sort.clone(), list.clone())),
                (
                    "cons".into(),
                    pi(
                        "A",
                        sort.clone(),
                        pi("head", v("A"), pi("tail", list.clone(), list.clone())),
                    ),
                ),
            ],
        }],
    )
    .unwrap();
    let array = app(k("Array", u.clone()), v("A"));
    c.add_universe_poly_inductives(
        &["u".into()],
        &[MutualInductive {
            name: "Array".into(),
            sort: pi("A", sort.clone(), sort.clone()),
            num_params: 1,
            constructors: vec![("array".into(), pi("A", sort, pi("items", list, array)))],
        }],
    )
    .unwrap();
    c
}
fn declaration(levels: Vec<Universe>) -> NestedDecl {
    NestedDecl {
        name: "Tree".into(),
        sort: ty(),
        constructors: vec![
            ("leaf".into(), g("Tree")),
            (
                "node".into(),
                pi(
                    "children",
                    app(
                        Term::Const {
                            name: "Array".into(),
                            levels,
                        },
                        g("Tree"),
                    ),
                    g("Tree"),
                ),
            ),
        ],
    }
}
#[test]
fn explicit_universe_compound_container_preserves_checked_round_trips() {
    let mut c = containers();
    let info = c
        .add_nested_inductive(&declaration(vec![Universe::Type(0)]))
        .unwrap();
    assert_eq!(info.siblings.len(), 2);
    let generic = app(
        app(k("array", Universe::Type(0)), g("Tree")),
        app(
            app(app(k("cons", Universe::Type(0)), g("Tree")), g("leaf")),
            app(k("nil", Universe::Type(0)), g("Tree")),
        ),
    );
    let iso = info.isos.iter().find(|i| i.container == "Array").unwrap();
    let specialized = app(g(&iso.from_generic), generic.clone());
    let round = app(g(&iso.to_generic), specialized.clone());
    infer_type(&c, &round).unwrap();
    assert!(matches!(double_check(&c, &round), DoubleCheck::Agreed));
    assert_eq!(normalize_checked(&c, &round).unwrap(), generic);
    let other = app(
        g(&iso.from_generic),
        app(g(&iso.to_generic), specialized.clone()),
    );
    assert_eq!(
        normalize_checked(&c, &other).unwrap(),
        normalize_checked(&c, &specialized).unwrap()
    );
}
#[test]
fn malformed_or_incompatible_container_universes_leave_no_nested_artifacts() {
    for levels in [
        vec![],
        vec![Universe::Prop],
        vec![Universe::Type(0), Universe::Type(0)],
    ] {
        let mut c = containers();
        assert!(c.add_nested_inductive(&declaration(levels)).is_err());
        assert!(c.get_global("Tree").is_none());
        assert!(c.get_global("node").is_none());
    }
}

#[test]
fn specialization_does_not_lower_the_original_container_universe() {
    let mut c = containers();
    assert!(c.add_nested_inductive(&declaration(vec![Universe::Type(1)])).is_err(),
        "a Type 0 tree cannot store a Type 1 Array merely because specialization shrinks its fields");
    assert!(c.get_global("Tree").is_none());
    assert!(c.get_global("node").is_none());
}
