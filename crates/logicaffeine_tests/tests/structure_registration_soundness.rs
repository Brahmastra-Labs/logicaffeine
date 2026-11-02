use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, normalize, Context, Term, Universe};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn arrow(a: Term, b: Term) -> Term {
    Term::Pi {
        param: "_".into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn ctx() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}

#[test]
fn inferred_structure_registration_does_not_bypass_a_failed_check() {
    let mut c = ctx();
    c.add_structure("Malformed", &[], &[("field", g("Zero"))]);
    assert!(c.get_global("Malformed").is_none());
    assert!(c.get_global("Malformed_mk").is_none());
    assert!(c
        .refused_declarations()
        .iter()
        .any(|(n, _)| *n == "Malformed"));
}

#[test]
fn explicit_prop_structure_checks_the_field_is_a_type() {
    let mut c = ctx();
    assert!(c
        .add_structure_at("NotAProp", Universe::Prop, &[], &[("field", g("Zero"))])
        .is_err());
    assert!(c.get_global("NotAProp").is_none());
}

#[test]
fn recursive_structures_cannot_store_a_negative_occurrence() {
    let mut c = ctx();
    let negative = arrow(g("BadRecord"), g("False"));
    assert!(c
        .add_structure_at(
            "BadRecord",
            Universe::Type(0),
            &[],
            &[("run", negative.clone())]
        )
        .is_err());
    assert!(c.get_global("BadRecord").is_none());
    c.add_structure("BadRecord", &[], &[("run", negative)]);
    assert!(
        c.get_global("BadRecord").is_none(),
        "the inferred-sort API must not bypass positivity either"
    );
}

#[test]
fn a_refused_structure_replacement_preserves_the_previous_generation() {
    let mut c = ctx();
    c.add_structure("Boxed", &[], &[("value", g("Nat"))]);
    let old = c.get_global("Boxed_mk").unwrap().clone();
    assert!(c
        .add_structure_at("Boxed", Universe::Prop, &[], &[("bad", g("Zero"))])
        .is_err());
    assert_eq!(c.get_global("Boxed_mk"), Some(&old));
    assert!(c.get_global("Boxed_bad").is_none());
}

#[test]
fn structure_parameter_domains_must_also_be_types() {
    let mut c = ctx();
    assert!(c
        .add_structure_at("BadParameter", Universe::Prop, &[("a", g("Zero"))], &[])
        .is_err());
    assert!(c.get_global("BadParameter").is_none());
}

#[test]
fn valid_data_and_proof_records_still_register_and_project() {
    let mut c = ctx();
    c.add_structure("NumberBox", &[], &[("value", g("Nat"))]);
    let app = |f: Term, a: Term| Term::App(Box::new(f), Box::new(a));
    let value = app(g("NumberBox_mk"), g("Zero"));
    let projection = app(g("NumberBox_value"), value);
    assert_eq!(infer_type(&c, &projection).unwrap(), g("Nat"));
    assert_eq!(normalize(&c, &projection), g("Zero"));
    c.add_structure_at("TrueBox", Universe::Prop, &[], &[("proof", g("True"))])
        .unwrap();
    assert!(c.get_global("TrueBox_mk").is_some());
}

#[test]
fn structure_levels_must_be_bound_in_the_declared_telescope() {
    let mut c = ctx();
    assert!(c
        .add_structure_poly_at("OpenSort", vec![], Universe::Var("u".into()), &[], &[])
        .is_err());
    assert!(c.get_global("OpenSort").is_none());
    assert!(c
        .add_structure_poly_at(
            "OpenParameter",
            vec![],
            Universe::Prop,
            &[("A", Term::Sort(Universe::Var("u".into())))],
            &[]
        )
        .is_err());
    assert!(c.get_global("OpenParameter").is_none());
    assert!(c
        .add_structure_poly_at(
            "OpenField",
            vec![],
            Universe::Prop,
            &[],
            &[("A", Term::Sort(Universe::Var("u".into())))]
        )
        .is_err());
    assert!(c.get_global("OpenField").is_none());
}

#[test]
fn structure_level_parameters_cannot_repeat() {
    let mut c = ctx();
    assert!(c
        .add_structure_poly_at(
            "DuplicateLevels",
            vec!["u".into(), "u".into()],
            Universe::Type(0),
            &[],
            &[("value", g("Nat"))]
        )
        .is_err());
    assert!(c.get_global("DuplicateLevels").is_none());
}

#[test]
fn bound_structure_levels_still_instantiate_independently() {
    let mut c = ctx();
    c.add_structure_poly_at(
        "PolyBox",
        vec!["u".into()],
        Universe::Var("u".into()),
        &[("A", Term::Sort(Universe::Var("u".into())))],
        &[("value", Term::Var("A".into()))],
    )
    .unwrap();
    for level in [Universe::Prop, Universe::Type(0), Universe::Type(2)] {
        assert!(infer_type(
            &c,
            &Term::Const {
                name: "PolyBox".into(),
                levels: vec![level]
            }
        )
        .is_ok());
    }
}
