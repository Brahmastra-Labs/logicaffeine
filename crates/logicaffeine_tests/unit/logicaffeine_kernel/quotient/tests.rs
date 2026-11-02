use super::*;
use crate::{double_check, infer_type, normalize_checked, DoubleCheck};
fn environment() -> Context {
    let mut c = Context::new();
    let u = Universe::Var("u".into());
    c.add_universe_poly_inductive(
        "Eq",
        vec!["u".into()],
        pi(
            "A",
            Term::Sort(u.clone()),
            pi("a", v("A"), pi("b", v("A"), prop())),
        ),
        2,
    );
    c.add_constructor(
        "refl",
        "Eq",
        pi(
            "A",
            Term::Sort(u.clone()),
            pi(
                "a",
                v("A"),
                app(constant("Eq", vec![u]), [v("A"), v("a"), v("a")]),
            ),
        ),
    );
    for kind in [Kind::Type, Kind::Constructor, Kind::Lift, Kind::Induction] {
        let params = if kind == Kind::Lift {
            vec!["u".into(), "v".into()]
        } else {
            vec!["u".into()]
        };
        install(&mut c, kind, params, "Eq").unwrap();
    }
    c
}
fn lam(n: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: format!("quot.{n}"),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
#[test]
fn quotient_universe_instances_are_checked_in_both_kernels() {
    let c = environment();
    for u in [Universe::Prop, Universe::Type(0), Universe::Type(4)] {
        for target in [Universe::Prop, Universe::Type(0), Universe::Type(7)] {
            for kind in [Kind::Type, Kind::Constructor, Kind::Lift, Kind::Induction] {
                let levels = if kind == Kind::Lift {
                    vec![u.clone(), target.clone()]
                } else {
                    vec![u.clone()]
                };
                let term = constant(kind.name(), levels);
                infer_type(&c, &term).unwrap();
                assert_eq!(double_check(&c, &term), DoubleCheck::Agreed);
            }
        }
    }
}
#[test]
fn quotient_lift_reduction_is_independently_rechecked_at_a_higher_target_universe() {
    let mut c = environment();
    c.add_declaration("A", Term::Sort(Universe::Type(0)));
    c.add_declaration("a", Term::Global("A".into()));
    c.add_declaration("True", prop());
    let a = Term::Global("a".into());
    let carrier = Term::Global("A".into());
    let r = lam(
        "x",
        carrier.clone(),
        lam("y", carrier.clone(), Term::Global("True".into())),
    );
    let target = Term::Sort(Universe::Type(1));
    let value = Term::Sort(Universe::Type(0));
    let f = lam("x", carrier.clone(), value.clone());
    let refl = app(
        constant("refl", vec![Universe::Type(2)]),
        [target.clone(), value.clone()],
    );
    let respect = lam(
        "x",
        carrier.clone(),
        lam(
            "y",
            carrier.clone(),
            lam("h", Term::Global("True".into()), refl.clone()),
        ),
    );
    let q = app(
        constant(Kind::Constructor.name(), vec![Universe::Type(0)]),
        [carrier.clone(), r.clone(), a],
    );
    let lift = app(
        constant(
            Kind::Lift.name(),
            vec![Universe::Type(0), Universe::Type(2)],
        ),
        [carrier, r, target.clone(), f, respect, q],
    );
    infer_type(&c, &lift).unwrap();
    assert_eq!(normalize_checked(&c, &lift).unwrap(), value.clone());
    let goal = app(
        constant("Eq", vec![Universe::Type(2)]),
        [target, lift, value],
    );
    let checked = app(lam("p", goal, v("p")), [refl]);
    infer_type(&c, &checked).unwrap();
    assert_eq!(double_check(&c, &checked), DoubleCheck::Agreed);
}
#[test]
fn replacing_quotient_or_equality_dependencies_revokes_computation_certificates() {
    for name in [Kind::Type.name(), Kind::Lift.name(), "Eq", "refl"] {
        let mut c = environment();
        assert_eq!(
            kind(
                &c,
                Kind::Lift.name(),
                &[Universe::Type(0), Universe::Type(0)]
            ),
            Some(Kind::Lift)
        );
        // Where the stable-meaning gate refuses the replacement (something depends on
        // `name`), the refused attempt leaves the certificate valid.
        if let Some(kept) = c.refused_meaning_change_for_test(|c| {
            c.add_declaration(name, prop());
            Ok(())
        }) {
            assert_eq!(
                kind(
                    &kept,
                    Kind::Lift.name(),
                    &[Universe::Type(0), Universe::Type(0)]
                ),
                Some(Kind::Lift),
                "a refused replacement of `{name}` leaves the certificate valid"
            );
        }
        c.permit_meaning_changes_for_cache_tests();
        c.add_declaration(name, prop());
        assert_eq!(
            kind(
                &c,
                Kind::Lift.name(),
                &[Universe::Type(0), Universe::Type(0)]
            ),
            None
        );
    }
    let mut c = environment();
    assert!(
        c.refused_meaning_change_for_test(|c| {
            c.add_constructor("extra", "Eq", prop());
            Ok(())
        })
        .is_some(),
        "the stable-meaning gate keeps `Eq`'s constructors under its dependents"
    );
    c.permit_meaning_changes_for_cache_tests();
    c.add_constructor("extra", "Eq", prop());
    assert_eq!(
        kind(
            &c,
            Kind::Lift.name(),
            &[Universe::Type(0), Universe::Type(0)]
        ),
        None
    );
}
#[test]
fn quotient_induction_computes_and_is_independently_rechecked() {
    let mut c = environment();
    c.add_declaration("A", Term::Sort(Universe::Type(0)));
    let carrier = Term::Global("A".into());
    c.add_declaration("a", carrier.clone());
    c.add_declaration("True", prop());
    let truth = Term::Global("True".into());
    c.add_declaration("witness", truth.clone());
    let witness = Term::Global("witness".into());
    let r = lam(
        "x",
        carrier.clone(),
        lam("y", carrier.clone(), truth.clone()),
    );
    let qtype = app(
        constant(Kind::Type.name(), vec![Universe::Type(0)]),
        [carrier.clone(), r.clone()],
    );
    let q = app(
        constant(Kind::Constructor.name(), vec![Universe::Type(0)]),
        [carrier.clone(), r.clone(), Term::Global("a".into())],
    );
    let elimination = app(
        constant(Kind::Induction.name(), vec![Universe::Type(0)]),
        [
            carrier.clone(),
            r,
            lam("q", qtype, truth.clone()),
            lam("x", carrier, witness.clone()),
            q,
        ],
    );
    infer_type(&c, &elimination).unwrap();
    assert_eq!(normalize_checked(&c, &elimination).unwrap(), witness);
    let proof = app(
        constant("refl", vec![Universe::Prop]),
        [truth.clone(), witness.clone()],
    );
    let goal = app(
        constant("Eq", vec![Universe::Prop]),
        [truth, elimination, witness],
    );
    let checked = app(lam("p", goal, v("p")), [proof]);
    assert_eq!(double_check(&c, &checked), DoubleCheck::Agreed);
}

#[test]
fn forged_equality_is_not_accepted_as_a_quotient_respect_predicate() {
    let mut c = Context::new();
    install(&mut c, Kind::Type, vec!["u".into()], "FakeEq").unwrap();
    let u = Universe::Var("u".into());
    c.add_universe_poly_declaration(
        "FakeEq",
        vec!["u".into()],
        pi("A", Term::Sort(u), pi("a", v("A"), pi("b", v("A"), prop()))),
    )
    .unwrap();
    assert!(install(&mut c, Kind::Lift, vec!["u".into(), "v".into()], "FakeEq").is_err());
    assert!(c.get_global(Kind::Lift.name()).is_none());
}

#[test]
fn equality_certificates_do_not_hide_mutable_delta_dependencies() {
    let mut c = Context::new();
    c.add_definition("PropAlias".into(), Term::Sort(Universe::Type(0)), prop());
    let u = Universe::Var("u".into());
    c.add_universe_poly_inductive(
        "Eq",
        vec!["u".into()],
        pi(
            "A",
            Term::Sort(u.clone()),
            pi(
                "a",
                v("A"),
                pi("b", v("A"), Term::Global("PropAlias".into())),
            ),
        ),
        2,
    );
    c.add_constructor(
        "refl",
        "Eq",
        pi(
            "A",
            Term::Sort(u.clone()),
            pi(
                "a",
                v("A"),
                app(constant("Eq", vec![u]), [v("A"), v("a"), v("a")]),
            ),
        ),
    );
    install(&mut c, Kind::Type, vec!["u".into()], "Eq").unwrap();
    assert!(install(&mut c, Kind::Lift, vec!["u".into(), "v".into()], "Eq").is_err());
    assert!(
        c.refused_meaning_change_for_test(|c| c.declare(
            "PropAlias",
            Term::Sort(Universe::Type(1)),
            Term::Sort(Universe::Type(0)),
        ))
        .is_some(),
        "the stable-meaning gate keeps `PropAlias` under `Eq`"
    );
    c.permit_meaning_changes_for_cache_tests();
    c.add_definition(
        "PropAlias".into(),
        Term::Sort(Universe::Type(1)),
        Term::Sort(Universe::Type(0)),
    );
    assert_eq!(
        c.get_definition_body("PropAlias"),
        Some(&Term::Sort(Universe::Type(0)))
    );
    assert_eq!(
        kind(
            &c,
            Kind::Lift.name(),
            &[Universe::Type(0), Universe::Type(0)]
        ),
        None
    );
}

#[test]
fn malformed_universe_instances_have_no_primitive_reduction_identity() {
    let c = environment();
    assert_eq!(kind(&c, Kind::Lift.name(), &[]), None);
    assert_eq!(
        kind(
            &c,
            Kind::Constructor.name(),
            &[Universe::Type(0), Universe::Type(1)]
        ),
        None
    );
}
