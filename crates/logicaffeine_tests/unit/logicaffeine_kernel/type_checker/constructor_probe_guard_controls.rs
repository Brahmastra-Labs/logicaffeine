use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn pi(n: &str, t: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    }
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor("Zero", "Nat", g("Nat"));
    c.add_constructor("Succ", "Nat", pi("n", g("Nat"), g("Nat")));
    c
}
fn dependent_witness(c: &mut Context, carrier: Term, a: Term, b: Term) {
    assert!(def_eq(c, &a, &b));
    assert!(def_eq(c, &b, &a));
    c.add_declaration(
        "ProbeWitness",
        pi("x", carrier, Term::Sort(Universe::Type(0))),
    );
    c.add_declaration("probe_witness_a", app(g("ProbeWitness"), a));
    let term = app(
        Term::Lambda {
            param: "w".into(),
            param_type: Box::new(app(g("ProbeWitness"), b)),
            body: Box::new(Term::Sort(Universe::Prop)),
        },
        g("probe_witness_a"),
    );
    assert_eq!(infer_type(c, &term).unwrap(), Term::Sort(Universe::Type(0)));
    assert_eq!(
        crate::recheck(c, &term).unwrap(),
        Term::Sort(Universe::Type(0))
    );
}
#[test]
fn constructor_probe_requires_full_application_and_bounded_metadata() {
    let c = context();
    assert_eq!(monomorphic_constructor_parent(&c, &g("Zero")), Some("Nat"));
    assert_eq!(
        monomorphic_constructor_parent(&c, &app(g("Succ"), g("Zero"))),
        Some("Nat")
    );
    assert!(!can_have_unit_type(&c, &app(g("Succ"), g("Zero"))));
    assert!(is_monomorphic_data_constructor(
        &c,
        &app(g("Succ"), g("Zero"))
    ));
    assert_eq!(monomorphic_constructor_parent(&c, &g("Succ")), None);
    assert_eq!(
        monomorphic_constructor_parent(&c, &app(app(g("Succ"), g("Zero")), g("Zero"))),
        None
    );
    assert_eq!(monomorphic_constructor_parent(&c, &g("unknown")), None);
    let deep = (0..300).fold(g("Succ"), |f, _| app(f, g("Zero")));
    assert_eq!(monomorphic_constructor_parent(&c, &deep), None);
}
#[test]
fn raw_constructor_parent_cannot_suppress_actual_proof_irrelevance() {
    let mut c = context();
    c.add_inductive("ClaimedData", Term::Sort(Universe::Type(0)));
    c.add_declaration("ActualProposition", Term::Sort(Universe::Prop));
    c.add_constructor("raw_proof_a", "ClaimedData", g("ActualProposition"));
    c.add_constructor("raw_proof_b", "ClaimedData", g("ActualProposition"));
    dependent_witness(
        &mut c,
        g("ActualProposition"),
        g("raw_proof_a"),
        g("raw_proof_b"),
    );
    assert_eq!(monomorphic_constructor_parent(&c, &g("raw_proof_a")), None);
}
#[test]
fn raw_constructor_parent_cannot_suppress_actual_unit_eta() {
    let mut c = context();
    c.add_inductive("ActualUnit", Term::Sort(Universe::Type(0)));
    c.add_constructor("actual_unit", "ActualUnit", g("ActualUnit"));
    c.add_inductive("ClaimedData", Term::Sort(Universe::Type(0)));
    c.add_constructor("raw_unit_a", "ClaimedData", g("ActualUnit"));
    c.add_constructor("raw_unit_b", "ClaimedData", g("ActualUnit"));
    dependent_witness(&mut c, g("ActualUnit"), g("raw_unit_a"), g("raw_unit_b"));
    assert_eq!(monomorphic_constructor_parent(&c, &g("raw_unit_a")), None);
}
#[test]
fn dependent_raw_result_uses_actual_instantiated_proposition() {
    let mut c = context();
    c.add_inductive("ClaimedData", Term::Sort(Universe::Type(0)));
    c.add_declaration("ActualProposition", Term::Sort(Universe::Prop));
    let header = pi("P", Term::Sort(Universe::Prop), Term::Var("P".into()));
    c.add_constructor("raw_dependent_a", "ClaimedData", header.clone());
    c.add_constructor("raw_dependent_b", "ClaimedData", header);
    let a = app(g("raw_dependent_a"), g("ActualProposition"));
    let b = app(g("raw_dependent_b"), g("ActualProposition"));
    dependent_witness(&mut c, g("ActualProposition"), a.clone(), b);
    assert_eq!(monomorphic_constructor_parent(&c, &a), None);
}
#[test]
fn registered_rewrite_contexts_keep_inference_based_probes() {
    let mut c = context();
    c.add_declaration(
        "Eq",
        pi(
            "A",
            Term::Sort(Universe::Type(0)),
            pi(
                "x",
                Term::Var("A".into()),
                pi("y", Term::Var("A".into()), Term::Sort(Universe::Prop)),
            ),
        ),
    );
    c.add_declaration("opaque_id", pi("n", g("Nat"), g("Nat")));
    let statement = app(
        app(app(g("Eq"), g("Nat")), app(g("opaque_id"), g("Zero"))),
        g("Zero"),
    );
    assert_eq!(
        infer_type(&c, &statement).unwrap(),
        Term::Sort(Universe::Prop)
    );
    c.add_declaration("opaque_id_zero", statement);
    c.add_simp_lemma("opaque_id_zero").unwrap();
    assert!(!c.simp_rules().is_empty());
    assert_eq!(monomorphic_constructor_parent(&c, &g("Zero")), None);
    assert_eq!(
        monomorphic_constructor_parent(&c, &app(g("Succ"), g("Zero"))),
        None
    );
}
