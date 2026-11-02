use logicaffeine_kernel::{Context, KernelError, MutualInductive, Term, Universe};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn ty() -> Term {
    Term::Sort(Universe::Type(0))
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
fn env() -> Context {
    let mut c = Context::new();
    c.set_auto_auxiliaries(false);
    c.add_mutual_inductives(&[MutualInductive {
        name: "W".into(),
        sort: ty(),
        num_params: 0,
        constructors: vec![("w0".into(), g("W")), ("w1".into(), g("W"))],
    }])
    .unwrap();
    c.add_mutual_inductives(&[MutualInductive {
        name: "L".into(),
        sort: pi("A", ty(), ty()),
        num_params: 1,
        constructors: vec![("L.mk".into(), pi("A", ty(), app(g("L"), v("A"))))],
    }])
    .unwrap();
    c
}
fn block(field: Term) -> MutualInductive {
    MutualInductive {
        name: "E".into(),
        sort: pi("w", g("W"), ty()),
        num_params: 1,
        constructors: vec![(
            "E.mk".into(),
            pi("w", g("W"), pi("field", field, app(g("E"), v("w")))),
        )],
    }
}
fn rejects(field: Term) {
    let mut c = env();
    let result = c.add_mutual_inductives(&[block(field)]);
    assert!(
        matches!(result, Err(KernelError::InvalidDeclaration(_))),
        "{result:?}"
    );
    assert!(c.get_global("E").is_none());
    assert!(c.get_global("E.mk").is_none());
}
#[test]
fn direct_recursive_parameter_cannot_change() {
    rejects(app(g("E"), g("w0")));
}
#[test]
fn phantom_nested_parameter_cannot_change() {
    rejects(app(g("L"), app(g("E"), g("w0"))));
}
#[test]
fn shadowed_parameter_is_not_the_outer_uniform_parameter() {
    rejects(pi("w", g("W"), app(g("E"), v("w"))));
}
#[test]
fn valid_uniform_nested_parameter_still_registers() {
    let mut c = env();
    c.add_mutual_inductives(&[block(app(g("L"), app(g("E"), v("w"))))])
        .unwrap();
}
#[test]
fn definitionally_equal_recursive_parameter_still_registers() {
    let mut c = env();
    c.declare(
        "identity",
        pi("x", g("W"), g("W")),
        Term::Lambda {
            param: "x".into(),
            param_type: Box::new(g("W")),
            body: Box::new(v("x")),
        },
    )
    .unwrap();
    c.add_mutual_inductives(&[block(app(g("E"), app(g("identity"), v("w"))))])
        .unwrap();
}
#[test]
fn indices_may_change_while_uniform_parameters_stay_fixed() {
    let mut c = env();
    let ei = |w, n| app(app(g("E"), w), n);
    c.add_mutual_inductives(&[MutualInductive {
        name: "E".into(),
        sort: pi("w", g("W"), pi("n", g("W"), ty())),
        num_params: 1,
        constructors: vec![(
            "E.mk".into(),
            pi(
                "w",
                g("W"),
                pi("field", ei(v("w"), g("w0")), ei(v("w"), g("w1"))),
            ),
        )],
    }])
    .unwrap();
}
fn mutual(changed: bool) -> Vec<MutualInductive> {
    let mut e = block(app(g("Sibling"), if changed { g("w0") } else { v("w") }));
    e.name = "E".into();
    vec![
        e,
        MutualInductive {
            name: "Sibling".into(),
            sort: pi("p", g("W"), ty()),
            num_params: 1,
            constructors: vec![(
                "Sibling.mk".into(),
                pi(
                    "p",
                    g("W"),
                    pi("field", app(g("E"), v("p")), app(g("Sibling"), v("p"))),
                ),
            )],
        },
    ]
}
#[test]
fn mutual_sibling_recursive_parameter_cannot_change() {
    let mut c = env();
    let result = c.add_mutual_inductives(&mutual(true));
    assert!(
        matches!(result, Err(KernelError::InvalidDeclaration(_))),
        "{result:?}"
    );
    assert!(c.get_global("E").is_none());
    assert!(c.get_global("Sibling").is_none());
}
#[test]
fn mutual_siblings_preserve_alpha_renamed_uniform_parameters() {
    env().add_mutual_inductives(&mutual(false)).unwrap();
}
fn poly(changed: bool) -> MutualInductive {
    let e = Term::Const {
        name: "Poly".into(),
        levels: vec![Universe::Var("u".into())],
    };
    MutualInductive {
        name: "Poly".into(),
        sort: pi("w", g("W"), Term::Sort(Universe::Var("u".into()))),
        num_params: 1,
        constructors: vec![(
            "Poly.mk".into(),
            pi(
                "w",
                g("W"),
                pi(
                    "field",
                    app(e.clone(), if changed { g("w0") } else { v("w") }),
                    app(e, v("w")),
                ),
            ),
        )],
    }
}
#[test]
fn explicit_universe_constant_recursive_parameter_cannot_change() {
    let mut c = env();
    let result = c.add_universe_poly_inductives(&["u".into()], &[poly(true)]);
    assert!(
        matches!(result, Err(KernelError::InvalidDeclaration(_))),
        "{result:?}"
    );
    assert!(c.poly_declaration("Poly").is_none());
}
#[test]
fn explicit_universe_constant_uniform_parameter_still_registers() {
    env()
        .add_universe_poly_inductives(&["u".into()], &[poly(false)])
        .unwrap();
}
#[test]
fn adding_checked_constructor_cannot_bypass_recursive_uniformity() {
    let mut c = env();
    c.add_indexed_inductive("E", pi("w", g("W"), ty()), 1);
    let ct = block(app(g("L"), app(g("E"), g("w0"))))
        .constructors
        .remove(0)
        .1;
    let result = c.add_constructor_checked("E.mk", "E", ct);
    assert!(
        matches!(result, Err(KernelError::InvalidDeclaration(_))),
        "{result:?}"
    );
    assert!(c.get_global("E.mk").is_none());
}

#[test]
fn nested_binder_cannot_capture_the_freshened_outer_parameter() {
    let mut c = env();
    c.add_mutual_inductives(&[block(pi(
        "__constructor_local_0",
        g("W"),
        app(g("E"), v("w")),
    ))])
    .unwrap();
}

fn beta_head_field(argument: Term) -> Term {
    let function_type = pi("parameter", g("W"), ty());
    let apply_parameter = Term::Lambda {
        param: "family".into(),
        param_type: Box::new(function_type),
        body: Box::new(app(v("family"), argument)),
    };
    app(apply_parameter, g("E"))
}
#[test]
fn beta_head_may_supply_the_uniform_parameter_to_a_partial_family() {
    let mut c = env();
    c.add_mutual_inductives(&[block(beta_head_field(v("w")))])
        .unwrap();
}
#[test]
fn beta_head_cannot_change_the_uniform_parameter() {
    rejects(beta_head_field(g("w0")));
}

#[test]
fn erased_ill_typed_beta_argument_cannot_bypass_original_typing() {
    let mut c = env();
    let erase = Term::Lambda {
        param: "ignored_family".into(),
        param_type: Box::new(pi("p", g("W"), ty())),
        body: Box::new(app(g("E"), v("w"))),
    };
    // w0 : W, not W -> Type. Head beta would erase this malformed argument.
    let result = c.add_mutual_inductives(&[block(app(erase, g("w0")))]);
    assert!(result.is_err(), "{result:?}");
    assert!(c.get_global("E").is_none());
    assert!(c.get_global("E.mk").is_none());
}
