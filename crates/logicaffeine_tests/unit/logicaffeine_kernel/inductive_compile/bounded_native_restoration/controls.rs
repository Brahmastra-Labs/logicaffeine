use super::*;
fn g(s: &str) -> Term {
    Term::Global(s.into())
}
fn v(s: &str) -> Term {
    Term::Var(s.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn sort() -> Term {
    Term::Sort(Universe::Prop)
}
fn lam(n: &str, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(sort()),
        body: Box::new(b),
    }
}
fn budget() -> RecursorBudget {
    RecursorBudget {
        max_depth: 128,
        max_tree_nodes: 100_000,
        max_store_nodes: 100_000,
        work: 1_000_000_000,
        allocation: 10_000_000,
        materialized_nodes: 100_000,
    }
}
fn equivalent(
    input: &Term,
    images: &HashMap<String, Term>,
    params: &[String],
    levels: &[Universe],
) -> Term {
    let expected = super::super::restore_native_term(input, images, params, levels)
        .expect("ordinary restoration");
    let actual = restore_native_term_bounded(input, images, params, levels, &mut budget())
        .expect("bounded restoration");
    let mut compare = TermStore::new();
    let a = compare.intern(&expected);
    let b = compare.intern(&actual);
    assert!(
        compare.alpha_same(a, b),
        "bounded restoration differs from ordinary restoration"
    );
    actual
}

#[test]
fn simultaneous_parameters_do_not_resubstitute_each_others_arguments() {
    let images = HashMap::from([("special".into(), app(v("A"), v("B")))]);
    let input = app(app(g("special"), v("B")), v("A"));
    let actual = equivalent(&input, &images, &["A".into(), "B".into()], &[]);
    assert_eq!(actual, app(v("B"), v("A")));
}

#[test]
fn image_binders_freshen_without_capturing_restored_arguments() {
    let images = HashMap::from([("special".into(), lam("x", app(v("A"), v("x"))))]);
    let actual = equivalent(&app(g("special"), v("x")), &images, &["A".into()], &[]);
    let mut store = TermStore::new();
    let root = store.intern(&actual);
    assert!(store.occurs_free(root, "x"));
    assert!(!store.occurs_free(root, "A"));
}

#[test]
fn restored_arguments_trailing_applications_and_unvisited_images_match_ordinary() {
    let images = HashMap::from([
        ("special".into(), app(g("source"), v("A"))),
        ("nested".into(), app(g("container"), v("A"))),
        // The result image intentionally mentions another mapped name. It must stay literal.
        ("source".into(), g("must_not_replace_image")),
    ]);
    let input = app(app(g("special"), app(g("nested"), v("z"))), v("tail"));
    let actual = equivalent(&input, &images, &["A".into()], &[]);
    assert_eq!(
        actual,
        app(app(g("source"), app(g("container"), v("z"))), v("tail"))
    );
}

#[test]
fn every_native_binder_shape_preserves_ordinary_restoration() {
    let images = HashMap::from([("special".into(), app(g("source"), v("A")))]);
    let target = || app(g("special"), v("free"));
    let input = Term::Let {
        name: "local".into(),
        ty: Box::new(Term::Pi {
            param: "p".into(),
            param_type: Box::new(sort()),
            body_type: Box::new(target()),
        }),
        value: Box::new(lam("x", target())),
        body: Box::new(Term::Match {
            discriminant: Box::new(target()),
            motive: Box::new(target()),
            cases: vec![
                Term::Fix {
                    name: "f".into(),
                    body: Box::new(target()),
                },
                Term::MutualFix {
                    defs: vec![("f".into(), target()), ("g".into(), target())],
                    index: 1,
                },
            ],
        }),
    };
    equivalent(&input, &images, &["A".into()], &[]);
}

#[test]
fn globals_and_exact_universe_instances_keep_the_original_distinction() {
    let u = Universe::Var("u".into());
    let images = HashMap::from([(
        "special".into(),
        Term::Const {
            name: "source".into(),
            levels: vec![u.clone()],
        },
    )]);
    let input = Term::Const {
        name: "special".into(),
        levels: vec![u.clone()],
    };
    let actual = equivalent(&input, &images, &[], &[u.clone()]);
    assert_eq!(
        actual,
        Term::Const {
            name: "source".into(),
            levels: vec![u.clone()]
        }
    );
    // A bare Global is deliberately not restored when the family has a universe instance.
    assert_eq!(
        equivalent(&g("special"), &images, &[], &[u.clone()]),
        g("special")
    );
    // A definitionally equal but structurally different universe still fails ordinary semantics.
    let wrong = Term::Const {
        name: "special".into(),
        levels: vec![Universe::Max(Box::new(u.clone()), Box::new(u.clone()))],
    };
    assert!(super::super::restore_native_term(&wrong, &images, &[], &[u.clone()]).is_err());
    assert!(matches!(
        restore_native_term_bounded(&wrong, &images, &[], &[u], &mut budget()),
        Err(RecursorGenerationError::Kernel(_))
    ));
}

#[test]
fn partial_shared_parameter_spines_are_not_restored() {
    let images = HashMap::from([("special".into(), app(v("A"), v("B")))]);
    let input = app(g("special"), v("x"));
    let params = vec!["A".into(), "B".into()];
    assert!(super::super::restore_native_term(&input, &images, &params, &[]).is_err());
    assert!(matches!(
        restore_native_term_bounded(&input, &images, &params, &[], &mut budget()),
        Err(RecursorGenerationError::Kernel(_))
    ));
}

#[test]
fn repeated_restoration_duplication_refuses_before_any_tree_materialization() {
    let images = HashMap::from([("special".into(), app(v("A"), v("A")))]);
    let mut input = v("leaf");
    for _ in 0..20 {
        input = app(g("special"), input);
    }
    let mut limits = budget();
    limits.max_tree_nodes = 1000;
    let before = limits.materialized_nodes;
    assert!(matches!(
        restore_native_term_bounded(&input, &images, &["A".into()], &[], &mut limits),
        Err(RecursorGenerationError::ResourceLimit)
    ));
    assert_eq!(
        limits.materialized_nodes, before,
        "no generated owned tree was extracted"
    );
}

#[test]
fn generated_depth_refuses_before_tree_materialization() {
    let mut image = v("A");
    for _ in 0..8 {
        image = lam("ignored", image);
    }
    let images = HashMap::from([("special".into(), image)]);
    let mut input = v("leaf");
    for _ in 0..8 {
        input = app(g("special"), input);
    }
    let mut limits = budget();
    limits.max_depth = 32;
    let before = limits.materialized_nodes;
    assert!(matches!(
        restore_native_term_bounded(&input, &images, &["A".into()], &[], &mut limits),
        Err(RecursorGenerationError::ResourceLimit)
    ));
    assert_eq!(limits.materialized_nodes, before);
}

#[test]
fn work_allocation_and_shared_materialization_allowances_are_not_reset() {
    let images = HashMap::new();
    let mut no_work = budget();
    no_work.work = 0;
    assert!(matches!(
        restore_native_term_bounded(&g("x"), &images, &[], &[], &mut no_work),
        Err(RecursorGenerationError::ResourceLimit)
    ));
    let mut no_allocation = budget();
    no_allocation.allocation = 0;
    assert!(matches!(
        restore_native_term_bounded(&g("x"), &images, &[], &[], &mut no_allocation),
        Err(RecursorGenerationError::ResourceLimit)
    ));
    let mut shared = budget();
    shared.materialized_nodes = 1;
    assert_eq!(
        restore_native_term_bounded(&g("x"), &images, &[], &[], &mut shared).unwrap(),
        g("x")
    );
    assert_eq!(shared.materialized_nodes, 0);
    assert!(matches!(
        restore_native_term_bounded(&g("y"), &images, &[], &[], &mut shared),
        Err(RecursorGenerationError::ResourceLimit)
    ));
}
