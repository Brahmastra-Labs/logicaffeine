use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn t(n: &str) -> Term {
    Term::Global(n.into())
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn app(f: Db, a: Db) -> Db {
    Db::App(DbBox::new(f), DbBox::new(a))
}
fn tapp(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn context() -> Context {
    let mut c = Context::new();
    c.set_auto_auxiliaries(false);
    c.add_mutual_inductives(&[crate::context::MutualInductive {
        name: "Nat".into(),
        sort: Term::Sort(Universe::Type(0)),
        num_params: 0,
        constructors: vec![
            ("zero".into(), t("Nat")),
            ("succ".into(), pi("n", t("Nat"), t("Nat"))),
        ],
    }])
    .unwrap();
    for (name, constructors) in [
        (
            "Flag",
            vec![("on".into(), t("Flag")), ("off".into(), t("Flag"))],
        ),
        ("Unit", vec![("star".into(), t("Unit"))]),
        (
            "Pair",
            vec![(
                "pair".into(),
                pi("first", t("Nat"), pi("second", t("Nat"), t("Pair"))),
            )],
        ),
    ] {
        c.add_mutual_inductives(&[crate::context::MutualInductive {
            name: name.into(),
            sort: Term::Sort(Universe::Type(0)),
            num_params: 0,
            constructors,
        }])
        .unwrap();
    }
    c.add_universe_poly_declaration("maker", vec![], pi("n", t("Nat"), t("Pair")))
        .unwrap();
    c.add_universe_poly_declaration("polyMaker", vec!["u".into()], pi("n", t("Nat"), t("Pair")))
        .unwrap();
    c
}

fn lam(d: Db, b: Db) -> Db {
    Db::Lam(DbBox::new(d), DbBox::new(b))
}
fn pair() -> Db {
    app(app(g("pair"), g("zero")), g("zero"))
}
fn eliminate_pair(d: Db) -> Db {
    Db::Match {
        disc: DbBox::new(d),
        motive: DbBox::new(g("Flag")),
        cases: vec![lam(g("Nat"), lam(g("Nat"), g("on")))],
    }
}
fn attempted(c: &Context, locals: &dyn LocalTypes, proof: &Db) -> (Option<Db>, usize) {
    K_PROBE_ATTEMPTS.with(|n| n.set(Some(0)));
    let result = k_constructor_db(c, locals, proof);
    let count = K_PROBE_ATTEMPTS.with(|n| n.replace(None).unwrap());
    (result, count)
}
fn rejected_by_both(c: &Context, term: Db) {
    assert!(infer(c, &[], &term).is_err());
    let named = from_db(&term, 0);
    assert!(crate::infer_type(c, &named).is_err());
    assert!(crate::recheck(c, &named).is_err());
}

fn match_on(d: Db, m: Db, cases: Vec<Db>) -> Db {
    Db::Match {
        disc: DbBox::new(d),
        motive: DbBox::new(m),
        cases,
    }
}
fn neutral_pair(motive: Db) -> Db {
    match_on(Db::Var(0), motive, vec![pair(), pair()])
}
fn indexed_context() -> Context {
    let mut c = context();
    c.add_mutual_inductives(&[crate::context::MutualInductive {
        name: "Index".into(),
        sort: pi("n", t("Nat"), Term::Sort(Universe::Type(0))),
        num_params: 0,
        constructors: vec![("atZero".into(), tapp(t("Index"), t("zero")))],
    }])
    .unwrap();
    c
}
#[test]
fn raw_and_function_motives_avoid_fieldful_direct_match_k() {
    let c = context();
    for motive in [g("Pair"), lam(g("Flag"), g("Pair"))] {
        let proof = neutral_pair(motive);
        assert_eq!(infer(&c, &[g("Flag")], &proof).unwrap(), g("Pair"));
        assert_eq!(attempted(&c, &[g("Flag")], &proof), (None, 0));
        let closed = lam(g("Flag"), eliminate_pair(proof));
        assert!(crate::infer_type(&c, &from_db(&closed, 0)).is_ok());
        assert!(crate::recheck(&c, &from_db(&closed, 0)).is_ok());
    }
}
#[test]
fn indexed_discriminant_generic_motive_avoids_fieldful_k() {
    let c = indexed_context();
    let domain = app(g("Index"), g("zero"));
    let motive = lam(g("Nat"), lam(app(g("Index"), Db::Var(0)), g("Pair")));
    let proof = match_on(Db::Var(0), motive, vec![pair()]);
    assert_eq!(infer(&c, &[domain.clone()], &proof).unwrap(), g("Pair"));
    assert_eq!(attempted(&c, &[domain.clone()], &proof), (None, 0));
    let closed = lam(domain, eliminate_pair(proof));
    assert!(crate::infer_type(&c, &from_db(&closed, 0)).is_ok());
    assert!(crate::recheck(&c, &from_db(&closed, 0)).is_ok());
}
#[test]
fn missing_extra_and_ill_typed_branches_still_reject() {
    let c = context();
    for cases in [
        vec![],
        vec![pair()],
        vec![pair(), pair(), pair()],
        vec![g("on"), pair()],
    ] {
        let proof = match_on(Db::Var(0), g("Pair"), cases);
        assert_eq!(attempted(&c, &[g("Flag")], &proof), (None, 0));
        rejected_by_both(&c, lam(g("Flag"), eliminate_pair(proof)));
    }
}
#[test]
fn wrong_domain_value_codomain_and_residual_lambda_motives_reject() {
    let c = context();
    for motive in [
        lam(g("Nat"), g("Pair")),
        lam(g("Flag"), pair()),
        lam(g("Flag"), lam(g("Nat"), g("Pair"))),
    ] {
        let proof = neutral_pair(motive);
        assert!(attempted(&c, &[g("Flag")], &proof).0.is_none());
        rejected_by_both(&c, lam(g("Flag"), eliminate_pair(proof)));
    }
}
#[test]
fn indexed_missing_major_and_wrong_generic_domain_are_not_certified() {
    let c = indexed_context();
    let domain = app(g("Index"), g("zero"));
    for motive in [
        lam(g("Nat"), g("Pair")),
        lam(g("Flag"), lam(app(g("Index"), g("zero")), g("Pair"))),
    ] {
        let proof = match_on(Db::Var(0), motive, vec![pair()]);
        assert_eq!(attempted(&c, &[domain.clone()], &proof), (None, 0));
        rejected_by_both(&c, lam(domain.clone(), eliminate_pair(proof)));
    }
}
#[test]
fn fieldless_unit_and_indexed_proposition_still_reconstruct_k() {
    let mut c = context();
    let unit = match_on(Db::Var(0), g("Unit"), vec![g("star"), g("star")]);
    assert_eq!(infer(&c, &[g("Flag")], &unit).unwrap(), g("Unit"));
    assert_eq!(attempted(&c, &[g("Flag")], &unit), (Some(g("star")), 1));
    let outer = match_on(unit, g("Flag"), vec![g("on")]);
    assert_eq!(whnf_in(&c, &[g("Flag")], &outer), g("on"));
    c.add_mutual_inductives(&[crate::context::MutualInductive {
        name: "Mark".into(),
        sort: pi("n", t("Nat"), Term::Sort(Universe::Prop)),
        num_params: 0,
        constructors: vec![("marked".into(), tapp(t("Mark"), t("zero")))],
    }])
    .unwrap();
    let proposition = match_on(
        Db::Var(0),
        app(g("Mark"), g("zero")),
        vec![g("marked"), g("marked")],
    );
    assert_eq!(
        infer(&c, &[g("Flag")], &proposition).unwrap(),
        app(g("Mark"), g("zero"))
    );
    assert_eq!(
        attempted(&c, &[g("Flag")], &proposition),
        (Some(g("marked")), 1)
    );
}
#[test]
fn aliases_variable_and_applied_match_heads_keep_fallback() {
    let mut c = context();
    c.with_equations_suspended(|c| {
        c.declare("PairAlias", Term::Sort(Universe::Type(0)), t("Pair"))
    })
    .unwrap();
    let alias = neutral_pair(g("PairAlias"));
    assert_eq!(infer(&c, &[g("Flag")], &alias).unwrap(), g("Pair"));
    assert_eq!(attempted(&c, &[g("Flag")], &alias), (None, 1));
    let var = neutral_pair(lam(g("Flag"), Db::Var(0)));
    assert!(!definitely_fieldful_k_family(&c, &[g("Flag")], &var));
    rejected_by_both(&c, lam(g("Flag"), eliminate_pair(var)));
    let applied = app(neutral_pair(g("Pair")), g("zero"));
    assert!(!definitely_fieldful_k_family(&c, &[g("Flag")], &applied));
    rejected_by_both(&c, lam(g("Flag"), eliminate_pair(applied)));
}
#[test]
fn direct_motive_observation_keeps_same_bounded_allowance() {
    let c = context();
    let mut motive = g("Pair");
    for _ in 0..260 {
        motive = lam(g("Flag"), motive);
    }
    assert!(!definitely_fieldful_k_family(
        &c,
        &[g("Flag")],
        &neutral_pair(motive)
    ));
    let mut c = context();
    c.add_inductive("OtherPair", Term::Sort(Universe::Type(0)));
    let other = pi("a", t("Nat"), pi("b", t("Nat"), t("OtherPair")));
    assert!(
        c.refused_meaning_change_for_test(|c| {
            c.add_constructor("pair", "OtherPair", other.clone());
            Ok(())
        })
        .is_some(),
        "the gate keeps `pair` in `Pair` under its dependents"
    );
    c.permit_meaning_changes_for_cache_tests();
    c.add_constructor("pair", "OtherPair", other);
    let proof = neutral_pair(g("Pair"));
    assert!(!definitely_fieldful_k_family(&c, &[g("Flag")], &proof));
    assert!(infer(&c, &[g("Flag")], &proof).is_err());
}
