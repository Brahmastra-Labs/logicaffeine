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
fn cases(n: &str, domain: Term, branches: Vec<Term>) -> Term {
    Term::Match {
        discriminant: Box::new(v(n)),
        motive: Box::new(lam("_", domain, g("Tree"))),
        cases: branches,
    }
}
#[test]
fn native_nested_types_and_guarded_mutual_recursion_preserve_generic_representation() {
    let mut c = containers();
    let original = pi("children", app(g("Array"), g("Tree")), g("Tree"));
    c.add_mutual_inductives(&[MutualInductive {
        name: "Tree".into(),
        sort: ty(),
        num_params: 0,
        constructors: vec![
            ("leaf".into(), g("Tree")),
            ("node".into(), original.clone()),
        ],
    }])
    .expect("strictly positive native Array/List nesting");
    assert_eq!(c.get_global("node"), Some(&original));
    let list = app(g("List"), g("Tree"));
    let array = app(g("Array"), g("Tree"));
    let recursor = Term::MutualFix {
        index: 0,
        defs: vec![
            (
                "tree_rec".into(),
                lam(
                    "t",
                    g("Tree"),
                    cases(
                        "t",
                        g("Tree"),
                        vec![
                            g("leaf"),
                            lam("a", array.clone(), app(v("array_rec"), v("a"))),
                        ],
                    ),
                ),
            ),
            (
                "array_rec".into(),
                lam(
                    "a",
                    array.clone(),
                    cases(
                        "a",
                        array.clone(),
                        vec![lam("xs", list.clone(), app(v("list_rec"), v("xs")))],
                    ),
                ),
            ),
            (
                "list_rec".into(),
                lam(
                    "xs",
                    list.clone(),
                    cases(
                        "xs",
                        list.clone(),
                        vec![
                            g("leaf"),
                            lam(
                                "h",
                                g("Tree"),
                                lam("tail", list.clone(), app(v("tree_rec"), v("h"))),
                            ),
                        ],
                    ),
                ),
            ),
        ],
    };
    let generic = app(
        app(g("array"), g("Tree")),
        app(
            app(app(g("cons"), g("Tree")), g("leaf")),
            app(g("nil"), g("Tree")),
        ),
    );
    let value = app(g("node"), generic);
    assert_eq!(infer_type(&c, &value).unwrap(), g("Tree"));
    let computation = app(recursor, value);
    assert_eq!(infer_type(&c, &computation).unwrap(), g("Tree"));
    assert!(matches!(
        double_check(&c, &computation),
        DoubleCheck::Agreed
    ));
    assert_eq!(normalize_checked(&c, &computation).unwrap(), g("leaf"));
}
#[test]
fn native_nested_negative_container_parameter_is_rejected() {
    let mut c = containers();
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
                    "f",
                    pi("x", v("A"), app(g("List"), v("A"))),
                    app(g("Contra"), v("A")),
                ),
            ),
        )],
    }])
    .unwrap();
    assert!(c
        .add_mutual_inductives(&[MutualInductive {
            name: "Bad".into(),
            sort: ty(),
            num_params: 0,
            constructors: vec![(
                "bad".into(),
                pi("field", app(g("Contra"), g("Bad")), g("Bad"))
            )],
        }])
        .is_err());
    assert!(c.get_global("Bad").is_none());
    assert!(c.get_global("bad").is_none());
}

#[test]
fn double_contravariance_is_not_strict_positivity() {
    let mut c = containers();
    let list_a = app(g("List"), v("A"));
    c.add_mutual_inductives(&[MutualInductive {
        name: "DoubleContra".into(),
        sort: pi("A", ty(), ty()),
        num_params: 1,
        constructors: vec![(
            "double_contra".into(),
            pi(
                "A",
                ty(),
                pi(
                    "f",
                    pi("consumer", pi("x", v("A"), list_a.clone()), list_a),
                    app(g("DoubleContra"), v("A")),
                ),
            ),
        )],
    }])
    .unwrap();
    assert!(c
        .add_mutual_inductives(&[MutualInductive {
            name: "Bad".into(),
            sort: ty(),
            num_params: 0,
            constructors: vec![(
                "bad".into(),
                pi("field", app(g("DoubleContra"), g("Bad")), g("Bad"))
            )],
        }])
        .is_err());
    assert!(c.get_global("Bad").is_none());
}
#[test]
fn native_nested_parameters_are_checked_in_every_constructor() {
    let mut c = containers();
    c.add_mutual_inductives(&[MutualInductive {
        name: "Mixed".into(),
        sort: pi("A", ty(), ty()),
        num_params: 1,
        constructors: vec![
            (
                "positive".into(),
                pi("A", ty(), pi("x", v("A"), app(g("Mixed"), v("A")))),
            ),
            (
                "negative".into(),
                pi(
                    "A",
                    ty(),
                    pi(
                        "f",
                        pi("x", v("A"), app(g("List"), v("A"))),
                        app(g("Mixed"), v("A")),
                    ),
                ),
            ),
        ],
    }])
    .unwrap();
    assert!(c
        .add_mutual_inductives(&[MutualInductive {
            name: "Bad".into(),
            sort: ty(),
            num_params: 0,
            constructors: vec![(
                "bad".into(),
                pi("field", app(g("Mixed"), g("Bad")), g("Bad"))
            )],
        }])
        .is_err());
    assert!(c.get_global("Bad").is_none());
}
#[test]
fn positive_functional_container_fields_remain_admissible() {
    let mut c = containers();
    c.add_mutual_inductives(&[MutualInductive {
        name: "Index".into(),
        sort: ty(),
        num_params: 0,
        constructors: vec![],
    }])
    .unwrap();
    c.add_mutual_inductives(&[MutualInductive {
        name: "Family".into(),
        sort: pi("A", ty(), ty()),
        num_params: 1,
        constructors: vec![(
            "family".into(),
            pi(
                "A",
                ty(),
                pi("f", pi("i", g("Index"), v("A")), app(g("Family"), v("A"))),
            ),
        )],
    }])
    .unwrap();
    c.add_mutual_inductives(&[MutualInductive {
        name: "Tree".into(),
        sort: ty(),
        num_params: 0,
        constructors: vec![(
            "node".into(),
            pi("field", app(g("Family"), g("Tree")), g("Tree")),
        )],
    }])
    .unwrap();
    assert!(c.get_global("node").is_some());
}

#[test]
fn a_positive_constructor_cycle_cannot_hide_a_later_negative_constructor() {
    let mut c = containers();
    c.add_mutual_inductives(&[MutualInductive {
        name: "MixedCycle".into(),
        sort: pi("A", ty(), ty()),
        num_params: 1,
        constructors: vec![
            (
                "cycle".into(),
                pi(
                    "A",
                    ty(),
                    pi(
                        "xs",
                        app(g("MixedCycle"), v("A")),
                        app(g("MixedCycle"), v("A")),
                    ),
                ),
            ),
            (
                "negative_cycle".into(),
                pi(
                    "A",
                    ty(),
                    pi(
                        "f",
                        pi("x", v("A"), app(g("List"), v("A"))),
                        app(g("MixedCycle"), v("A")),
                    ),
                ),
            ),
        ],
    }])
    .unwrap();
    assert!(c
        .add_mutual_inductives(&[MutualInductive {
            name: "Bad".into(),
            sort: ty(),
            num_params: 0,
            constructors: vec![(
                "bad".into(),
                pi("field", app(g("MixedCycle"), g("Bad")), g("Bad"))
            )],
        }])
        .is_err());
    assert!(c.get_global("Bad").is_none());
}

#[test]
fn checked_container_extension_cannot_invalidate_registered_nested_positivity() {
    let mut c = containers();
    c.add_mutual_inductives(&[MutualInductive {
        name: "Tree".into(),
        sort: ty(),
        num_params: 0,
        constructors: vec![(
            "node".into(),
            pi("field", app(g("Array"), g("Tree")), g("Tree")),
        )],
    }])
    .unwrap();
    let negative = pi(
        "A",
        ty(),
        pi(
            "consumer",
            pi("x", v("A"), app(g("List"), v("A"))),
            app(g("List"), v("A")),
        ),
    );
    assert!(
        c.add_constructor_checked("negative_list", "List", negative)
            .is_err(),
        "the checked extension must revalidate dependent nested datatypes"
    );
    assert!(c.get_global("negative_list").is_none());
    assert_eq!(c.get_constructors("List").len(), 2);
    assert!(c.get_global("node").is_some());
}

fn native_tree(mut c: Context, container: &str) -> Context {
    c.add_mutual_inductives(&[MutualInductive {
        name: "Tree".into(),
        sort: ty(),
        num_params: 0,
        constructors: vec![(
            "node".into(),
            pi("field", app(g(container), g("Tree")), g("Tree")),
        )],
    }])
    .unwrap();
    c
}

#[test]
fn replacing_a_nested_container_is_transactionally_refused() {
    let c = native_tree(containers(), "Array");
    let old = c.get_global("List").unwrap().clone();
    let replacement = MutualInductive {
        name: "List".into(),
        sort: pi("A", ty(), ty()),
        num_params: 1,
        constructors: vec![],
    };
    let mut mono = c.clone();
    assert!(mono.add_mutual_inductives(&[replacement.clone()]).is_err());
    let mut poly = c.clone();
    assert!(poly
        .add_universe_poly_inductives(&["u".into()], &[replacement])
        .is_err());
    for context in [&mono, &poly] {
        assert_eq!(context.get_global("List"), Some(&old));
        assert_eq!(context.get_constructors("List").len(), 2);
        assert!(context.poly_declaration("List").is_none());
        assert!(context.get_global("node").is_some());
    }
}

#[test]
fn legacy_mutation_wrappers_preserve_nested_dependencies_and_record_refusal() {
    let original = native_tree(containers(), "Array");
    for operation in 0..8 {
        let mut c = original.clone();
        let before = c.refused_declarations().len();
        match operation {
            0 => c.add_inductive("List", ty()),
            1 => c.add_indexed_inductive("List", ty(), 0),
            2 => c.add_universe_poly_inductive("List", vec!["u".into()], ty(), 0),
            3 => c.set_inductive_params("List", 0),
            4 => c.add_constructor("new_list", "List", g("List")),
            5 => c.add_declaration("List", ty()),
            6 => c.add_declaration("nil", ty()),
            7 => c.register_struct_info(
                "List",
                logicaffeine_kernel::StructInfo {
                    mk: "nil".into(),
                    num_params: 1,
                    projections: vec![],
                    params: vec![],
                    fields: vec![],
                    parents: vec![],
                    out_params: vec![],
                },
            ),
            _ => unreachable!(),
        }
        assert_eq!(
            c.get_global("List"),
            original.get_global("List"),
            "operation {operation}"
        );
        assert_eq!(
            c.get_global("nil"),
            original.get_global("nil"),
            "operation {operation}"
        );
        assert_eq!(c.get_constructors("List").len(), 2);
        assert_eq!(c.inductive_num_params("List"), 1);
        assert!(c.poly_declaration("List").is_none());
        assert!(c.get_global("new_list").is_none());
        assert!(c.struct_info("List").is_none());
        assert_eq!(
            c.refused_declarations().len(),
            before + 1,
            "operation {operation}"
        );
    }
}

#[test]
fn a_transparent_nested_alias_cannot_be_redefined_after_use() {
    let mut c = containers();
    let alias_ty = pi("A", ty(), ty());
    c.declare(
        "Alias",
        alias_ty.clone(),
        lam("A", ty(), app(g("List"), v("A"))),
    )
    .unwrap();
    c = native_tree(c, "Alias");
    let prior = c.get_definition_body("Alias").unwrap().clone();
    let negative = lam("A", ty(), pi("consume", v("A"), app(g("List"), v("A"))));
    assert!(c
        .declare("Alias", alias_ty.clone(), negative.clone())
        .is_err());
    assert!(c
        .declare_universe_poly("Alias", vec![], alias_ty, negative)
        .is_err());
    assert_eq!(c.get_definition_body("Alias"), Some(&prior));
    assert!(c
        .add_structure_at("Alias", Universe::Type(0), &[], &[])
        .is_err());
    assert_eq!(c.get_definition_body("Alias"), Some(&prior));
}

/// The seal a native nesting puts on its container belongs to the snapshot that made the nesting.
/// The snapshot taken before it may still redeclare the container, and an unrelated name in the
/// sealed snapshot is not frozen with it.
///
/// The `before` snapshot holds `List` alone. In `containers()` the `Array` container already
/// depends on `List`, and a declared name never changes meaning under what depends on it
/// (`Context::admit_meaning`), so there `List` is fixed for a reason that has nothing to do with
/// any seal. That refusal is asserted first, so the premise below is the one that isolates the seal.
#[test]
fn native_dependency_sealing_is_snapshot_local_and_does_not_freeze_unrelated_names() {
    let empty_list = || MutualInductive {
        name: "List".into(),
        sort: pi("A", ty(), ty()),
        num_params: 1,
        constructors: vec![],
    };
    let mut depended_on = containers();
    let refusal = depended_on.add_mutual_inductives(&[empty_list()]);
    assert!(
        matches!(&refusal,Err(logicaffeine_kernel::KernelError::StableMeaningViolation {
            name,change:logicaffeine_kernel::MeaningChange::Redeclared,dependents,
        }) if name=="List" && dependents.iter().any(|d| d=="Array")),
        "`Array` is declared over `List`, so `List` keeps its meaning: {refusal:?}"
    );
    assert_eq!(
        depended_on.get_constructors("List").len(),
        2,
        "and the refusal changed nothing"
    );

    let mut before = Context::new();
    before
        .add_mutual_inductives(&[MutualInductive {
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
    let mut sealed = native_tree(before.clone(), "List");
    assert!(
        sealed
            .clone()
            .add_mutual_inductives(&[empty_list()])
            .is_err(),
        "the snapshot that nests `Tree` through `List` may not redeclare it"
    );
    let mut independent = before;
    independent.add_mutual_inductives(&[empty_list()]).unwrap();
    assert_eq!(independent.get_constructors("List").len(), 0);
    assert_eq!(sealed.get_constructors("List").len(), 2);
    sealed
        .add_mutual_inductives(&[MutualInductive {
            name: "Unrelated".into(),
            sort: ty(),
            num_params: 0,
            constructors: vec![],
        }])
        .unwrap();
    sealed.add_inductive("Unrelated", Term::Sort(Universe::Type(1)));
    assert_eq!(
        sealed.get_global("Unrelated"),
        Some(&Term::Sort(Universe::Type(1)))
    );
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
fn native_nested_explicit_universe_container_retains_original_constructor_type() {
    let mut c = polymorphic_containers();
    let original = pi(
        "children",
        app(k("Array", Universe::Type(0)), g("Tree")),
        g("Tree"),
    );
    c.add_mutual_inductives(&[MutualInductive {
        name: "Tree".into(),
        sort: ty(),
        num_params: 0,
        constructors: vec![
            ("leaf".into(), g("Tree")),
            ("node".into(), original.clone()),
        ],
    }])
    .unwrap();
    assert_eq!(c.get_global("node"), Some(&original));
    let value = app(
        g("node"),
        app(
            app(k("array", Universe::Type(0)), g("Tree")),
            app(k("nil", Universe::Type(0)), g("Tree")),
        ),
    );
    assert_eq!(infer_type(&c, &value).unwrap(), g("Tree"));
    assert!(matches!(double_check(&c, &value), DoubleCheck::Agreed));
}

fn unsupported_parameter_container(value_parameter: bool) -> Context {
    let mut c = Context::new();
    c.add_mutual_inductives(&[MutualInductive {
        name: "Tag".into(),
        sort: ty(),
        num_params: 0,
        constructors: vec![("tag".into(), g("Tag"))],
    }])
    .unwrap();
    let instance = app(app(g("Box"), v("A")), v("tag"));
    c.add_mutual_inductives(&[MutualInductive {
        name: "Box".into(),
        sort: pi("A", ty(), pi("tag", g("Tag"), ty())),
        num_params: if value_parameter { 2 } else { 1 },
        constructors: vec![(
            "box".into(),
            pi(
                "A",
                ty(),
                pi("tag", g("Tag"), pi("value", v("A"), instance)),
            ),
        )],
    }])
    .unwrap();
    c
}
/// Nesting `Tree` through `Box Tree tag`: with `tag` an index (`value_parameter` false) or a value
/// parameter (true). Both are valid Lean and are now certified natively; the block member occurs
/// only in the container's TYPE parameter.
fn native_container_registration(
    value_parameter: bool,
    field: Term,
) -> (Context, Result<(), logicaffeine_kernel::KernelError>) {
    let mut c = unsupported_parameter_container(value_parameter);
    let result = c.add_mutual_inductives(&[MutualInductive {
        name: "Tree".into(),
        sort: ty(),
        num_params: 0,
        constructors: vec![("node".into(), pi("field", field, g("Tree")))],
    }]);
    (c, result)
}
fn native_container_is_certified(value_parameter: bool) {
    let field = app(app(g("Box"), g("Tree")), g("tag"));
    let (c, result) = native_container_registration(value_parameter, field.clone());
    result.expect("a positive occurrence through an indexed or value-parameter container");
    assert_eq!(c.get_global("node"), Some(&pi("field", field, g("Tree"))));
    // A negative neighbour through the same container is still a proven positivity violation.
    let negative = app(app(g("Box"), pi("f", g("Tree"), g("Tag"))), g("tag"));
    let (c, result) = native_container_registration(value_parameter, negative);
    assert!(
        matches!(
            result,
            Err(logicaffeine_kernel::KernelError::PositivityViolation { .. })
        ),
        "{result:?}"
    );
    assert!(c.get_global("Tree").is_none());
    assert!(c.get_global("node").is_none());
}
#[test]
fn native_indexed_container_is_certified_and_a_negative_neighbour_is_refused() {
    native_container_is_certified(false);
}
#[test]
fn native_value_parameter_container_is_certified_and_a_negative_neighbour_is_refused() {
    native_container_is_certified(true);
}
#[test]
fn native_known_negative_container_is_a_proven_positivity_violation() {
    let mut c = containers();
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
                    "f",
                    pi("x", v("A"), app(g("List"), v("A"))),
                    app(g("Contra"), v("A")),
                ),
            ),
        )],
    }])
    .unwrap();
    let error = c
        .add_mutual_inductives(&[MutualInductive {
            name: "Bad".into(),
            sort: ty(),
            num_params: 0,
            constructors: vec![(
                "bad".into(),
                pi("field", app(g("Contra"), g("Bad")), g("Bad")),
            )],
        }])
        .unwrap_err();
    assert!(matches!(
        error,
        logicaffeine_kernel::KernelError::PositivityViolation { .. }
    ));
}
