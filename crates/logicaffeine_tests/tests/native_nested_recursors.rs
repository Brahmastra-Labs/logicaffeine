use logicaffeine_kernel::{
    double_check, infer_type, normalize_checked, Context, DoubleCheck, MutualInductive, Term,
    Universe,
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

fn lam(n: &str, a: Term, body: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(a),
        body: Box::new(body),
    }
}

use logicaffeine_kernel::inductive_compile::{derive_native_recursors, NestedDecl};
fn apps(f: Term, args: Vec<Term>) -> Term {
    args.into_iter().fold(f, app)
}
fn fixture() -> (Context, NestedDecl) {
    let mut c = containers();
    let decl = NestedDecl {
        name: "Tree".into(),
        sort: ty(),
        constructors: vec![
            ("leaf".into(), g("Tree")),
            (
                "node".into(),
                pi("children", app(g("Array"), g("Tree")), g("Tree")),
            ),
        ],
    };
    c.add_mutual_inductives(&[MutualInductive {
        name: decl.name.clone(),
        sort: decl.sort.clone(),
        num_params: 0,
        constructors: decl.constructors.clone(),
    }])
    .unwrap();
    (c, decl)
}

#[test]
fn reconstructed_native_recursors_typecheck_without_representation_conversions() {
    let (mut c, decl) = fixture();
    let recursors = derive_native_recursors(&c, &decl).unwrap();
    let carriers = vec![
        g("Tree"),
        app(g("Array"), g("Tree")),
        app(g("List"), g("Tree")),
    ];
    assert_eq!(
        recursors
            .iter()
            .map(|r| r.carrier.clone())
            .collect::<Vec<_>>(),
        carriers
    );
    for (i, rec) in recursors.into_iter().enumerate() {
        assert_eq!(rec.levels.len(), 3);
        assert!(matches!(double_check(&c, &rec.body), DoubleCheck::Agreed));
        c.declare_universe_poly(&format!("rec{i}"), rec.levels, rec.ty, rec.body)
            .unwrap();
    }
    let tree = g("Tree");
    let array = app(g("Array"), tree.clone());
    let list = app(g("List"), tree.clone());
    let motives = vec![
        lam("t", tree.clone(), tree.clone()),
        lam("a", array.clone(), tree.clone()),
        lam("xs", list.clone(), tree.clone()),
    ];
    // One minor for each original constructor, in source/container discovery order.
    let minors = vec![
        g("leaf"),
        lam("a", array.clone(), lam("ih", tree.clone(), v("ih"))),
        lam("xs", list.clone(), lam("ih", tree.clone(), v("ih"))),
        g("leaf"),
        lam(
            "h",
            tree.clone(),
            lam(
                "tail",
                list.clone(),
                lam(
                    "ih_head",
                    tree.clone(),
                    lam("ih_tail", tree.clone(), v("ih_head")),
                ),
            ),
        ),
    ];
    let xs = apps(
        g("cons"),
        vec![tree.clone(), g("leaf"), app(g("nil"), tree.clone())],
    );
    let ar = apps(g("array"), vec![tree.clone(), xs.clone()]);
    let tr = app(g("node"), ar.clone());
    for (i, value) in [tr, ar, xs].into_iter().enumerate() {
        let mut arguments = motives.clone();
        arguments.extend(minors.clone());
        arguments.push(value);
        let call = apps(
            Term::Const {
                name: format!("rec{i}"),
                levels: vec![Universe::Type(0); 3],
            },
            arguments,
        );
        assert_eq!(infer_type(&c, &call).unwrap(), tree);
        assert!(matches!(double_check(&c, &call), DoubleCheck::Agreed));
        assert_eq!(normalize_checked(&c, &call).unwrap(), g("leaf"));
    }
}
#[test]
fn native_recursor_derivation_requires_the_exact_registered_constructor_inventory() {
    let (c, mut decl) = fixture();
    decl.constructors.pop();
    assert!(derive_native_recursors(&c, &decl).is_err());
}

#[test]
fn reconstructed_native_recursors_compute_in_both_kernels() {
    let (mut c, decl) = fixture();
    let recursors = derive_native_recursors(&c, &decl).unwrap();
    let carriers = vec![
        g("Tree"),
        app(g("Array"), g("Tree")),
        app(g("List"), g("Tree")),
    ];
    assert_eq!(
        recursors
            .iter()
            .map(|r| r.carrier.clone())
            .collect::<Vec<_>>(),
        carriers
    );
    for (i, rec) in recursors.into_iter().enumerate() {
        assert_eq!(rec.levels.len(), 3);
        assert!(matches!(double_check(&c, &rec.body), DoubleCheck::Agreed));
        c.declare_universe_poly(&format!("rec{i}"), rec.levels, rec.ty, rec.body)
            .unwrap();
    }
    let tree = g("Tree");
    let array = app(g("Array"), tree.clone());
    let list = app(g("List"), tree.clone());
    let motives = vec![
        lam("t", tree.clone(), tree.clone()),
        lam("a", array.clone(), tree.clone()),
        lam("xs", list.clone(), tree.clone()),
    ];
    // One minor for each original constructor, in source/container discovery order.
    let minors = vec![
        g("leaf"),
        lam("a", array.clone(), lam("ih", tree.clone(), v("ih"))),
        lam("xs", list.clone(), lam("ih", tree.clone(), v("ih"))),
        g("leaf"),
        lam(
            "h",
            tree.clone(),
            lam(
                "tail",
                list.clone(),
                lam(
                    "ih_head",
                    tree.clone(),
                    lam("ih_tail", tree.clone(), v("ih_head")),
                ),
            ),
        ),
    ];
    let xs = apps(
        g("cons"),
        vec![tree.clone(), g("leaf"), app(g("nil"), tree.clone())],
    );
    let ar = apps(g("array"), vec![tree.clone(), xs.clone()]);
    let tr = app(g("node"), ar.clone());
    for (i, value) in [tr, ar, xs].into_iter().enumerate() {
        let mut arguments = motives.clone();
        arguments.extend(minors.clone());
        arguments.push(value);
        let call = apps(
            Term::Const {
                name: format!("rec{i}"),
                levels: vec![Universe::Type(0); 3],
            },
            arguments,
        );
        assert_eq!(
            normalize_checked(&c, &infer_type(&c, &call).unwrap()).unwrap(),
            tree
        );
        assert!(matches!(double_check(&c, &call), DoubleCheck::Agreed));
        assert_eq!(normalize_checked(&c, &call).unwrap(), g("leaf"));
    }
}

fn k(n: &str, u: Universe) -> Term {
    Term::Const {
        name: n.into(),
        levels: vec![u],
    }
}
fn polymorphic_containers() -> Context {
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

#[test]
fn reconstructed_native_recursors_preserve_explicit_container_universes() {
    let mut c = polymorphic_containers();
    let decl = NestedDecl {
        name: "Tree".into(),
        sort: ty(),
        constructors: vec![
            ("leaf".into(), g("Tree")),
            (
                "node".into(),
                pi(
                    "children",
                    app(k("Array", Universe::Type(0)), g("Tree")),
                    g("Tree"),
                ),
            ),
        ],
    };
    c.add_mutual_inductives(&[MutualInductive {
        name: decl.name.clone(),
        sort: decl.sort.clone(),
        num_params: 0,
        constructors: decl.constructors.clone(),
    }])
    .unwrap();
    let recursors = derive_native_recursors(&c, &decl).unwrap();
    assert_eq!(
        recursors
            .iter()
            .map(|r| r.carrier.clone())
            .collect::<Vec<_>>(),
        vec![
            g("Tree"),
            app(k("Array", Universe::Type(0)), g("Tree")),
            app(k("List", Universe::Type(0)), g("Tree")),
        ]
    );
    for (i, rec) in recursors.into_iter().enumerate() {
        assert!(matches!(double_check(&c, &rec.body), DoubleCheck::Agreed));
        c.declare_universe_poly(&format!("poly_rec{i}"), rec.levels, rec.ty, rec.body)
            .unwrap();
    }
}

#[test]
fn native_nested_open_recursive_rule_matches_its_sibling_recursor() {
    let (mut c, decl) = fixture();
    let recursors = derive_native_recursors(&c, &decl).unwrap();
    for (i, rec) in recursors.into_iter().enumerate() {
        c.declare_universe_poly(&format!("open_rec{i}"), rec.levels, rec.ty, rec.body)
            .unwrap();
    }
    let reference = |i| Term::Const {
        name: format!("open_rec{i}"),
        levels: vec![Universe::Type(0); 3],
    };
    let mut telescope = infer_type(&c, &reference(0)).unwrap();
    let mut arguments = Vec::new();
    // Three motives and five constructor minors precede the major premise.
    for i in 0..8 {
        let Term::Pi {
            param,
            param_type,
            body_type,
        } = telescope
        else {
            panic!("recursor telescope")
        };
        let name = format!("rule_arg_{i}");
        let arg = v(&name);
        c = c.extend(&name, *param_type.clone());
        telescope =
            logicaffeine_kernel::instantiate_codomain(&param, &param_type, &body_type, &arg);
        arguments.push(arg);
    }
    c = c.extend("children", app(g("Array"), g("Tree")));
    let children = v("children");
    let lhs = app(
        apps(reference(0), arguments.clone()),
        app(g("node"), children.clone()),
    );
    let recursive = app(apps(reference(1), arguments.clone()), children.clone());
    let rhs = apps(arguments[4].clone(), vec![children, recursive]);
    assert!(matches!(double_check(&c, &lhs), DoubleCheck::Agreed));
    assert!(matches!(double_check(&c, &rhs), DoubleCheck::Agreed));
    assert!(
        logicaffeine_kernel::defeq_for_test(&c, &lhs, &rhs),
        "native recursive equation must agree on an open container argument"
    );
}

#[test]
fn native_nested_closed_recursive_rule_matches_its_sibling_recursor() {
    let (mut c, decl) = fixture();
    let recursors = derive_native_recursors(&c, &decl).unwrap();
    for (i, rec) in recursors.into_iter().enumerate() {
        c.declare_universe_poly(&format!("open_rec{i}"), rec.levels, rec.ty, rec.body)
            .unwrap();
    }
    let reference = |i| Term::Const {
        name: format!("open_rec{i}"),
        levels: vec![Universe::Type(0); 3],
    };
    let mut telescope = infer_type(&c, &reference(0)).unwrap();
    let mut arguments = Vec::new();
    let mut binders = Vec::new();
    // Three motives and five constructor minors precede the major premise.
    for i in 0..8 {
        let Term::Pi {
            param,
            param_type,
            body_type,
        } = telescope
        else {
            panic!("recursor telescope")
        };
        let name = format!("rule_arg_{i}");
        let arg = v(&name);
        binders.push((name.clone(), *param_type.clone()));
        c = c.extend(&name, *param_type.clone());
        telescope =
            logicaffeine_kernel::instantiate_codomain(&param, &param_type, &body_type, &arg);
        arguments.push(arg);
    }
    c = c.extend("children", app(g("Array"), g("Tree")));
    let children = v("children");
    let mut lhs = app(
        apps(reference(0), arguments.clone()),
        app(g("node"), children.clone()),
    );
    let recursive = app(apps(reference(1), arguments.clone()), children.clone());
    let mut rhs = apps(arguments[4].clone(), vec![children, recursive]);
    binders.push(("children".into(), app(g("Array"), g("Tree"))));
    for (name, ty) in binders.into_iter().rev() {
        lhs = lam(&name, ty.clone(), lhs);
        rhs = lam(&name, ty, rhs);
    }
    assert!(matches!(double_check(&c, &lhs), DoubleCheck::Agreed));
    assert!(matches!(double_check(&c, &rhs), DoubleCheck::Agreed));
    assert!(
        logicaffeine_kernel::defeq_for_test(&c, &lhs, &rhs),
        "native recursive equation must agree on an open container argument"
    );
}
