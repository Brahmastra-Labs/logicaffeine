use logicaffeine_kernel::{
    derive_recursor_universe_poly, infer_type, Context, MutualInductive, Term, Universe,
};
fn c(n: &str, u: Universe) -> Term {
    Term::Const {
        name: n.into(),
        levels: vec![u],
    }
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn tree() -> MutualInductive {
    let u = Universe::Var("u".into());
    MutualInductive {
        name: "Tree".into(),
        sort: Term::Sort(u.succ()),
        num_params: 0,
        constructors: vec![
            ("leaf".into(), c("Tree", u.clone())),
            (
                "node".into(),
                pi("child", c("Tree", u.clone()), c("Tree", u)),
            ),
        ],
    }
}
#[test]
fn checked_polymorphic_block_registers_complete_constructor_telescopes() {
    let mut ctx = Context::new();
    ctx.add_universe_poly_inductives(&["u".into()], &[tree()])
        .unwrap();
    for n in ["Tree", "leaf", "node"] {
        assert_eq!(ctx.poly_declaration(n).unwrap().0, &["u".to_string()]);
        infer_type(&ctx, &c(n, Universe::Type(2))).unwrap();
    }
    let (levels, ty, body) = derive_recursor_universe_poly(&ctx, "Tree").unwrap();
    ctx.declare_universe_poly("Tree_rec", levels, ty, body)
        .unwrap();
}
#[test]
fn block_level_parameters_must_be_unique_and_closed() {
    for params in [vec![], vec!["u".into(), "u".into()], vec!["other".into()]] {
        let mut ctx = Context::new();
        assert!(ctx
            .add_universe_poly_inductives(&params, &[tree()])
            .is_err());
        assert!(ctx.get_global("Tree").is_none());
    }
}
#[test]
fn a_constructor_cannot_change_its_parents_universe_instance() {
    let mut ctx = Context::new();
    let mut t = tree();
    t.constructors = vec![("escape".into(), c("Tree", Universe::Prop))];
    assert!(ctx
        .add_universe_poly_inductives(&["u".into()], &[t])
        .is_err());
    assert!(ctx.get_global("Tree").is_none());
}
#[test]
fn a_recursive_field_cannot_change_the_blocks_universe_instance() {
    let mut ctx = Context::new();
    let mut t = tree();
    t.constructors[1].1 = pi(
        "child",
        c("Tree", Universe::Prop),
        c("Tree", Universe::Var("u".into())),
    );
    assert!(ctx
        .add_universe_poly_inductives(&["u".into()], &[t])
        .is_err());
    assert!(ctx.get_global("node").is_none());
}
#[test]
fn a_polymorphic_block_cannot_erase_universe_arguments_from_recursive_references() {
    let mut ctx = Context::new();
    let mut t = tree();
    t.constructors[1].1 = pi(
        "child",
        Term::Global("Tree".into()),
        c("Tree", Universe::Var("u".into())),
    );
    assert!(ctx
        .add_universe_poly_inductives(&["u".into()], &[t])
        .is_err());
}
#[test]
fn a_failed_block_preserves_the_existing_generation() {
    let mut ctx = Context::new();
    ctx.add_universe_poly_inductives(&["u".into()], &[tree()])
        .unwrap();
    let before = ctx.get_global("node").unwrap().clone();
    let mut bad = tree();
    bad.constructors[1].1 = Term::Sort(Universe::Prop);
    assert!(ctx
        .add_universe_poly_inductives(&["u".into()], &[bad])
        .is_err());
    assert_eq!(ctx.get_global("node"), Some(&before));
}

#[test]
fn a_polymorphic_unit_eliminates_at_an_independent_level_even_at_prop() {
    let mut ctx = Context::new();
    let block = MutualInductive {
        name: "UnitP".into(),
        sort: Term::Sort(Universe::Var("u".into())),
        num_params: 0,
        constructors: vec![("unit".into(), c("UnitP", Universe::Var("u".into())))],
    };
    ctx.add_universe_poly_inductives(&["u".into()], &[block])
        .unwrap();
    let (levels, _, body) = derive_recursor_universe_poly(&ctx, "UnitP").unwrap();
    assert_eq!(levels.len(), 2);
    let subst = levels
        .into_iter()
        .zip([Universe::Type(2), Universe::Prop])
        .collect();
    let body = logicaffeine_kernel::instantiate_universes(&body, &subst);
    infer_type(&ctx, &body).unwrap();
    logicaffeine_kernel::recheck(&ctx, &body).unwrap();
}

#[test]
fn fresh_motive_levels_cannot_capture_unused_inductive_parameters() {
    let mut ctx = Context::new();
    let empty = MutualInductive {
        name: "EmptyP".into(),
        sort: Term::Sort(Universe::Type(0)),
        num_params: 0,
        constructors: vec![],
    };
    ctx.add_universe_poly_inductives(&["u".into()], &[empty])
        .unwrap();
    let (levels, ty, body) = derive_recursor_universe_poly(&ctx, "EmptyP").unwrap();
    assert_eq!(levels.len(), 2);
    assert_ne!(levels[0], levels[1]);
    ctx.declare_universe_poly("EmptyP_rec", levels, ty, body)
        .unwrap();
}
