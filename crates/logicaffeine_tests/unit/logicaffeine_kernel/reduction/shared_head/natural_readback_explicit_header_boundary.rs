use super::*;
#[test]
fn registered_empty_universe_constructors_typecheck_but_keep_readback_fallback() {
    let nat = Term::Global("Nat".into());
    let mut c = Context::new();
    c.set_auto_auxiliaries(false);
    c.add_universe_poly_inductives(
        &[],
        &[crate::context::MutualInductive {
            name: "Nat".into(),
            sort: Term::Sort(crate::Universe::Type(0)),
            num_params: 0,
            constructors: vec![
                ("Zero".into(), nat.clone()),
                (
                    "Succ".into(),
                    Term::Pi {
                        param: "n".into(),
                        param_type: Box::new(nat.clone()),
                        body_type: Box::new(nat.clone()),
                    },
                ),
            ],
        }],
    )
    .unwrap();
    for name in ["Nat", "Zero", "Succ"] {
        assert!(c.poly_declaration(name).unwrap().0.is_empty());
    }
    assert!(!crate::int_inductive::canonical_nat_environment(&c));
    let explicit = |n: &str| Term::Const {
        name: n.into(),
        levels: vec![],
    };
    for term in [
        explicit("Zero"),
        Term::App(Box::new(explicit("Succ")), Box::new(explicit("Zero"))),
    ] {
        assert_eq!(crate::infer_type(&c, &term).unwrap(), nat);
        assert_eq!(crate::recheck(&c, &term).unwrap(), nat);
        assert_eq!(
            natural_normal_form(&c, &term, super::super::Transparency::All, 1000),
            (NatReadbackOutcome::Unsupported, 1000)
        );
    }
}
