use logicaffeine_kernel::{
    derive_recursor_universe_poly, infer_type, normalize_checked, recheck, Context, Term, Universe,
};
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn sort(u: Universe) -> Term {
    Term::Sort(u)
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn lam(n: &str, ty: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(ty),
        body: Box::new(b),
    }
}
fn con(n: &str, levels: Vec<Universe>) -> Term {
    Term::Const {
        name: n.into(),
        levels,
    }
}
fn poly_box() -> Context {
    let mut c = Context::new();
    let u = Universe::Var("u".into()).succ();
    c.add_structure_poly_at(
        "Box",
        vec!["u".into()],
        u.clone(),
        &[("A", sort(u))],
        &[("value", v("A"))],
    )
    .unwrap();
    c
}
#[test]
fn derived_recursor_binds_inductive_and_motive_levels() {
    let c = poly_box();
    let (levels, ty, body) = derive_recursor_universe_poly(&c, "Box").unwrap();
    assert_eq!(
        levels.len(),
        2,
        "the motive level is independent of the carrier level"
    );
    assert_eq!(levels[1], "u");
    assert_ne!(levels[0], levels[1]);
    assert!(infer_type(&c, &body).is_ok());
    assert!(recheck(&c, &body).is_ok());
    let mut registered = c.clone();
    registered
        .declare_universe_poly("Box_rec", levels, ty, body)
        .unwrap();
}
#[test]
fn instantiated_polymorphic_recursor_computes_at_two_carrier_levels() {
    let mut c = poly_box();
    let (levels, ty, body) = derive_recursor_universe_poly(&c, "Box").unwrap();
    c.declare_universe_poly("Box_rec", levels, ty, body)
        .unwrap();
    // At carrier parameter u, Sort u itself inhabits the parameter type Sort (u+1).
    for u in [Universe::Type(0), Universe::Type(2)] {
        let a = sort(u.clone());
        let value = sort(match u {
            Universe::Type(n) if n > 0 => Universe::Type(n - 1),
            _ => Universe::Prop,
        });
        let box_a = app(con("Box", vec![u.clone()]), a.clone());
        let boxed = app(
            app(con("Box_mk", vec![u.clone()]), a.clone()),
            value.clone(),
        );
        let motive = lam("box", box_a, a.clone());
        let minor = lam("x", a.clone(), v("x"));
        let call = [a, motive, minor, boxed]
            .into_iter()
            .fold(con("Box_rec", vec![u.succ(), u]), app);
        infer_type(&c, &call).unwrap();
        recheck(&c, &call).unwrap();
        assert_eq!(normalize_checked(&c, &call).unwrap(), value);
    }
}

#[test]
fn recursor_body_rechecks_after_explicit_universe_substitution() {
    let c = poly_box();
    let (levels, _, body) = derive_recursor_universe_poly(&c, "Box").unwrap();
    let subst = levels
        .into_iter()
        .zip([Universe::Type(0), Universe::Type(2)])
        .collect();
    let instantiated = logicaffeine_kernel::instantiate_universes(&body, &subst);
    infer_type(&c, &instantiated).unwrap();
    recheck(&c, &instantiated).unwrap();
}

fn pi(n: &str, ty: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(ty),
        body_type: Box::new(b),
    }
}
#[test]
fn constructor_telescope_preserves_order_and_unused_parameters() {
    let mut c = Context::new();
    let levels = vec!["z".into(), "unused".into(), "a".into()];
    let z = Universe::Var("z".into());
    let a = Universe::Var("a".into());
    c.add_structure_poly_at(
        "Packed",
        levels.clone(),
        z.max(&a).succ(),
        &[],
        &[("first", sort(z)), ("second", sort(a))],
    )
    .unwrap();
    assert_eq!(c.poly_declaration("Packed_mk").unwrap().0, levels);
    let ctor = con(
        "Packed_mk",
        vec![Universe::Type(2), Universe::Prop, Universe::Type(0)],
    );
    let ty = infer_type(&c, &ctor).unwrap();
    let Term::Pi {
        param_type,
        body_type,
        ..
    } = ty
    else {
        panic!("constructor telescope")
    };
    assert_eq!(*param_type, sort(Universe::Type(2)));
    let Term::Pi { param_type, .. } = *body_type else {
        panic!("second field")
    };
    assert_eq!(*param_type, sort(Universe::Type(0)));
    recheck(&c, &ctor).unwrap();
    let short = con("Packed_mk", vec![Universe::Type(2), Universe::Type(0)]);
    assert!(infer_type(&c, &short).is_err());
    assert!(recheck(&c, &short).is_err());
}
#[test]
fn instantiated_field_domains_are_checked_in_both_kernels() {
    let mut c = Context::new();
    let u = Universe::Var("u".into());
    c.add_structure_poly_at(
        "Packed",
        vec!["u".into()],
        u.succ(),
        &[],
        &[("value", sort(u))],
    )
    .unwrap();
    let packed = con("Packed", vec![Universe::Type(0)]);
    let motive = lam("p", packed.clone(), sort(Universe::Type(0)));
    let build = |domain: Universe| {
        lam(
            "p",
            packed.clone(),
            Term::Match {
                discriminant: Box::new(v("p")),
                motive: Box::new(motive.clone()),
                cases: vec![lam("A", sort(domain), v("A"))],
            },
        )
    };
    let good = build(Universe::Type(0));
    infer_type(&c, &good).unwrap();
    recheck(&c, &good).unwrap();
    let wrong = build(Universe::Prop);
    assert!(infer_type(&c, &wrong).is_err());
    assert!(recheck(&c, &wrong).is_err());
}
#[test]
fn an_indexed_recursor_body_preserves_constructor_levels() {
    let mut c = Context::new();
    let u = Universe::Var("u".into());
    let a_sort = sort(u.succ());
    let indexed = |a: Term, x: Term| app(app(con("Indexed", vec![u.clone()]), a), x);
    c.add_universe_poly_inductive(
        "Indexed",
        vec!["u".into()],
        pi("A", a_sort.clone(), pi("x", v("A"), a_sort.clone())),
        1,
    );
    c.add_constructor_checked(
        "Indexed_mk",
        "Indexed",
        pi("A", a_sort, pi("x", v("A"), indexed(v("A"), v("x")))),
    )
    .unwrap();
    let (levels, _, body) = derive_recursor_universe_poly(&c, "Indexed").unwrap();
    assert_eq!(levels.len(), 2);
    let subst = levels
        .into_iter()
        .zip([Universe::Type(0), Universe::Type(2)])
        .collect();
    let body = logicaffeine_kernel::instantiate_universes(&body, &subst);
    infer_type(&c, &body).unwrap();
    recheck(&c, &body).unwrap();
}
#[test]
fn a_polymorphic_structural_parameter_does_not_legalize_unchanged_recursion() {
    let c = poly_box();
    let u = Universe::Type(0);
    let box_a = app(con("Box", vec![u.clone()]), sort(u));
    let loop_body = Term::Fix {
        name: "loop".into(),
        body: Box::new(lam("x", box_a, app(v("loop"), v("x")))),
    };
    assert!(infer_type(&c, &loop_body).is_err());
    assert!(recheck(&c, &loop_body).is_err());
}
