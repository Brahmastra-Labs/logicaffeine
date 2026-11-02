use logicaffeine_kernel::{infer_type, normalize_checked, recheck, Context, Term, Universe};
fn sort(u: Universe) -> Term {
    Term::Sort(u)
}
fn constant(n: &str, u: Universe) -> Term {
    Term::Const {
        name: n.into(),
        levels: vec![u],
    }
}
#[test]
fn polymorphic_assumption_instantiates_in_both_kernels_and_stays_opaque() {
    let mut c = Context::new();
    c.add_universe_poly_declaration("A", vec!["u".into()], sort(Universe::Var("u".into())))
        .unwrap();
    assert!(!c.is_inductive("A"));
    assert!(!c.is_universe_poly_inductive("A"));
    for level in [Universe::Prop, Universe::Type(0), Universe::Type(2)] {
        let a = constant("A", level.clone());
        assert_eq!(infer_type(&c, &a).unwrap(), sort(level));
        recheck(&c, &a).unwrap();
        assert_eq!(normalize_checked(&c, &a).unwrap(), a);
    }
}
#[test]
fn invalid_assumption_is_transactional() {
    let mut c = Context::new();
    c.add_declaration("A", sort(Universe::Type(0)));
    for (params, ty) in [
        (
            vec!["u".into(), "u".into()],
            sort(Universe::Var("u".into())),
        ),
        (vec!["u".into()], sort(Universe::Var("v".into()))),
        (vec!["u".into()], Term::Global("missing".into())),
    ] {
        assert!(c.add_universe_poly_declaration("A", params, ty).is_err());
        assert_eq!(c.get_global("A"), Some(&sort(Universe::Type(0))));
        assert!(c.poly_declaration("A").is_none());
    }
}
#[test]
fn unused_parameters_remain_ordered_and_arity_checked() {
    let mut c = Context::new();
    c.add_universe_poly_declaration(
        "A",
        vec!["unused".into(), "u".into()],
        sort(Universe::Var("u".into())),
    )
    .unwrap();
    let a = Term::Const {
        name: "A".into(),
        levels: vec![Universe::Type(2), Universe::Prop],
    };
    assert_eq!(infer_type(&c, &a).unwrap(), sort(Universe::Prop));
    recheck(&c, &a).unwrap();
    assert!(infer_type(&c, &constant("A", Universe::Prop)).is_err());
    assert!(recheck(&c, &constant("A", Universe::Prop)).is_err());
}
#[test]
fn monomorphic_replacement_clears_polymorphic_assumption_metadata() {
    let mut c = Context::new();
    c.add_universe_poly_declaration("A", vec!["u".into()], sort(Universe::Var("u".into())))
        .unwrap();
    c.add_declaration("A", sort(Universe::Type(0)));
    assert!(c.poly_declaration("A").is_none());
}

#[test]
fn checked_assumption_cannot_replace_a_name_and_invalidate_existing_types() {
    let mut c = Context::new();
    c.add_declaration("A", sort(Universe::Type(0)));
    c.add_declaration("a", Term::Global("A".into()));
    assert!(c
        .add_universe_poly_declaration("A", vec![], Term::Global("A".into()))
        .is_err());
    assert_eq!(c.get_global("A"), Some(&sort(Universe::Type(0))));
    assert_eq!(
        infer_type(&c, &Term::Global("a".into())).unwrap(),
        Term::Global("A".into())
    );
}
