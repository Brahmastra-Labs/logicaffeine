use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
#[test]
fn cyclic_publication_is_refused_and_pending_bodies_never_enter_readback() {
    let mut c = Context::new();
    c.set_auto_auxiliaries(false);
    c.add_inductive("Nat", Term::Sort(crate::Universe::Type(0)));
    c.add_constructor("Zero", "Nat", g("Nat"));
    c.add_constructor(
        "Succ",
        "Nat",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(g("Nat")),
            body_type: Box::new(g("Nat")),
        },
    );
    c.declare("loop_a", g("Nat"), g("loop_b")).unwrap();
    assert!(c
        .pending_obligations()
        .iter()
        .any(|(name, _)| *name == "loop_a"));
    assert!(c.get_global("loop_a").is_none());
    assert!(c.get_definition_body("loop_a").is_none());
    let error = c.declare("loop_b", g("Nat"), g("loop_a")).unwrap_err();
    assert!(matches!(error, crate::KernelError::CertificationError(_)));
    for name in ["loop_a", "loop_b"] {
        assert!(c.get_global(name).is_none());
        assert!(c.get_definition_body(name).is_none());
        assert!(crate::infer_type(&c, &g(name)).is_err());
        assert!(crate::recheck(&c, &g(name)).is_err());
    }
    // All publication/type preconditions are checked before observing anything.
    for name in ["loop_a", "loop_b"] {
        assert!(!natural_result_candidate(
            &c,
            &g(name),
            super::super::Transparency::All
        ));
        assert_eq!(
            natural_normal_form(&c, &g(name), super::super::Transparency::All, 1000).0,
            NatReadbackOutcome::Unsupported
        );
    }
}
