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

fn beta_pair() -> Db {
    app(lam(g("Nat"), g("Pair")), g("zero"))
}
fn valid_closed(c: &Context, term: Db) {
    let named = from_db(&term, 0);
    assert!(crate::infer_type(c, &named).is_ok());
    assert!(crate::recheck(c, &named).is_ok());
}
fn product_context() -> Context {
    let mut c = context();
    let sort = Term::Sort(Universe::Type(0));
    let pp = |a: Term, b: Term| tapp(tapp(t("Product"), a), b);
    c.add_mutual_inductives(&[crate::context::MutualInductive {
        name: "Product".into(),
        sort: pi("A", sort.clone(), pi("B", sort.clone(), sort.clone())),
        num_params: 2,
        constructors: vec![(
            "product".into(),
            pi(
                "A",
                sort.clone(),
                pi(
                    "B",
                    sort,
                    pi(
                        "a",
                        Term::Var("A".into()),
                        pi(
                            "b",
                            Term::Var("B".into()),
                            pp(Term::Var("A".into()), Term::Var("B".into())),
                        ),
                    ),
                ),
            ),
        )],
    }])
    .unwrap();
    c
}
fn product_type() -> Db {
    app(app(g("Product"), g("Nat")), g("Nat"))
}
fn product_value() -> Db {
    app(
        app(app(app(g("product"), g("Nat")), g("Nat")), g("zero")),
        g("zero"),
    )
}
#[test]
fn beta_wrapped_raw_and_function_motives_avoid_fieldful_k() {
    let c = context();
    for motive in [beta_pair(), lam(g("Flag"), beta_pair())] {
        let proof = neutral_pair(motive);
        assert_eq!(infer(&c, &[g("Flag")], &proof).unwrap(), g("Pair"));
        assert_eq!(attempted(&c, &[g("Flag")], &proof), (None, 0));
        valid_closed(&c, lam(g("Flag"), eliminate_pair(proof)));
    }
}
#[test]
fn exact_fix_with_beta_wrapped_result_avoids_k() {
    let c = context();
    let proof = app(
        Db::Fix(FixBody::new(lam(
            g("Nat"),
            match_on(Db::Var(0), beta_pair(), vec![pair(), lam(g("Nat"), pair())]),
        ))),
        Db::Var(0),
    );
    assert_eq!(infer(&c, &[g("Nat")], &proof).unwrap(), g("Pair"));
    assert_eq!(attempted(&c, &[g("Nat")], &proof), (None, 0));
    valid_closed(&c, lam(g("Nat"), eliminate_pair(proof)));
}
#[test]
fn nested_lambda_under_extra_apps_preserves_exact_residual_family_arity() {
    let c = product_context();
    let motive = app(
        app(
            app(
                lam(
                    g("Nat"),
                    lam(Db::Sort(Universe::Type(0)), app(g("Product"), Db::Var(0))),
                ),
                g("zero"),
            ),
            g("Nat"),
        ),
        g("Nat"),
    );
    let proof = match_on(Db::Var(0), motive, vec![product_value(), product_value()]);
    assert_eq!(infer(&c, &[g("Flag")], &proof).unwrap(), product_type());
    assert_eq!(attempted(&c, &[g("Flag")], &proof), (None, 0));
    valid_closed(&c, lam(g("Flag"), proof));
}
#[test]
fn partial_and_excess_family_applications_cannot_claim_exact_arity() {
    let c = product_context();
    for motive in [
        app(lam(g("Nat"), app(g("Product"), g("Nat"))), g("zero")),
        app(app(lam(g("Nat"), product_type()), g("zero")), g("zero")),
    ] {
        let proof = match_on(Db::Var(0), motive, vec![product_value(), product_value()]);
        assert!(!definitely_fieldful_k_family(&c, &[g("Flag")], &proof));
        rejected_by_both(&c, lam(g("Flag"), proof));
    }
}
#[test]
fn shadowed_binders_only_change_arguments_not_rigid_result_heads() {
    let c = context();
    // Both nested beta binders have Nat domains. The outer argument uses
    // the local Nat; the nested argument deliberately uses the inner binder.
    let motive = app(
        lam(g("Nat"), app(lam(g("Nat"), g("Pair")), Db::Var(0))),
        Db::Var(1),
    );
    let proof = match_on(Db::Var(0), motive, vec![pair(), pair()]);
    assert_eq!(
        infer(&c, &[g("Nat"), g("Flag")], &proof).unwrap(),
        g("Pair")
    );
    assert_eq!(attempted(&c, &[g("Nat"), g("Flag")], &proof), (None, 0));
    valid_closed(&c, lam(g("Nat"), lam(g("Flag"), proof)));
}
#[test]
fn substituted_and_capture_shifted_variable_heads_keep_full_fallback() {
    let c = context();
    let substituted = neutral_pair(app(lam(Db::Sort(Universe::Type(0)), Db::Var(0)), g("Pair")));
    assert_eq!(infer(&c, &[g("Flag")], &substituted).unwrap(), g("Pair"));
    assert!(!definitely_fieldful_k_family(
        &c,
        &[g("Flag")],
        &substituted
    ));
    assert_eq!(attempted(&c, &[g("Flag")], &substituted), (None, 1));
    valid_closed(&c, lam(g("Flag"), substituted));
    // A : Type, value : A, flag : Flag. The beta-bound Nat shifts A to Var(3).
    let captured = match_on(
        Db::Var(0),
        app(lam(g("Nat"), Db::Var(3)), g("zero")),
        vec![Db::Var(1), Db::Var(1)],
    );
    let locals = [Db::Sort(Universe::Type(0)), Db::Var(0), g("Flag")];
    assert!(!definitely_fieldful_k_family(&c, &locals, &captured));
    valid_closed(
        &c,
        lam(
            Db::Sort(Universe::Type(0)),
            lam(Db::Var(0), lam(g("Flag"), captured)),
        ),
    );
}
#[test]
fn malformed_beta_arguments_domains_codomain_and_coverage_still_reject() {
    let c = context();
    for (motive, cases) in [
        (app(lam(g("Nat"), g("Pair")), g("on")), vec![pair(), pair()]),
        (lam(g("Nat"), beta_pair()), vec![pair(), pair()]),
        (app(lam(g("Nat"), pair()), g("zero")), vec![pair(), pair()]),
        (beta_pair(), vec![]),
        (beta_pair(), vec![pair(), g("on")]),
        (
            lam(g("Flag"), lam(g("Nat"), beta_pair())),
            vec![pair(), pair()],
        ),
    ] {
        rejected_by_both(&c, lam(g("Flag"), neutral_pair_with_cases(motive, cases)));
    }
}
fn neutral_pair_with_cases(motive: Db, cases: Vec<Db>) -> Db {
    match_on(Db::Var(0), motive, cases)
}
#[test]
fn beta_wrapped_unit_and_indexed_prop_still_use_checked_k() {
    let mut c = context();
    let unit = match_on(
        Db::Var(0),
        app(lam(g("Nat"), g("Unit")), g("zero")),
        vec![g("star"), g("star")],
    );
    assert_eq!(infer(&c, &[g("Flag")], &unit).unwrap(), g("Unit"));
    assert_eq!(attempted(&c, &[g("Flag")], &unit), (Some(g("star")), 1));
    c.add_mutual_inductives(&[crate::context::MutualInductive {
        name: "Mark".into(),
        sort: pi("n", t("Nat"), Term::Sort(Universe::Prop)),
        num_params: 0,
        constructors: vec![("marked".into(), tapp(t("Mark"), t("zero")))],
    }])
    .unwrap();
    let motive = app(lam(g("Nat"), app(g("Mark"), Db::Var(0))), g("zero"));
    let proof = match_on(Db::Var(0), motive, vec![g("marked"), g("marked")]);
    assert_eq!(
        infer(&c, &[g("Flag")], &proof).unwrap(),
        app(g("Mark"), g("zero"))
    );
    assert_eq!(attempted(&c, &[g("Flag")], &proof), (Some(g("marked")), 1));
}
#[test]
fn indexed_discriminant_with_beta_wrapped_generic_motive_preserves_k_exclusion() {
    let c = indexed_context();
    let domain = app(g("Index"), g("zero"));
    let motive = lam(g("Nat"), lam(app(g("Index"), Db::Var(0)), beta_pair()));
    let proof = match_on(Db::Var(0), motive, vec![pair()]);
    assert_eq!(infer(&c, &[domain.clone()], &proof).unwrap(), g("Pair"));
    assert_eq!(attempted(&c, &[domain.clone()], &proof), (None, 0));
    valid_closed(&c, lam(domain, proof));
}
#[test]
fn shallow_beta_head_walk_keeps_shared_budget_and_context_guards() {
    let mut c = context();
    let mut motive = g("Pair");
    for _ in 0..260 {
        motive = app(lam(g("Nat"), motive), g("zero"));
    }
    assert!(!definitely_fieldful_k_family(
        &c,
        &[g("Flag")],
        &neutral_pair(motive)
    ));
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
    assert!(!definitely_fieldful_k_family(
        &c,
        &[g("Flag")],
        &neutral_pair(beta_pair())
    ));
}

#[test]
fn beta_wrapped_explicit_levels_and_aliases_keep_existing_admission_boundaries() {
    let mut c = context();
    c.add_universe_poly_inductives(
        &["u".into()],
        &[crate::context::MutualInductive {
            name: "PolyPair".into(),
            sort: Term::Sort(Universe::Type(0)),
            num_params: 0,
            constructors: vec![(
                "polyPair".into(),
                pi(
                    "a",
                    t("Nat"),
                    pi(
                        "b",
                        t("Nat"),
                        Term::Const {
                            name: "PolyPair".into(),
                            levels: vec![Universe::Var("u".into())],
                        },
                    ),
                ),
            )],
        }],
    )
    .unwrap();
    for u in [Universe::Type(0), Universe::Type(1)] {
        let result = Db::Const {
            name: "PolyPair".into(),
            levels: vec![u.clone()],
        };
        let value = app(
            app(
                Db::Const {
                    name: "polyPair".into(),
                    levels: vec![u],
                },
                g("zero"),
            ),
            g("zero"),
        );
        let proof = match_on(
            Db::Var(0),
            app(lam(g("Nat"), result.clone()), g("zero")),
            vec![value.clone(), value],
        );
        assert_eq!(infer(&c, &[g("Flag")], &proof).unwrap(), result);
        assert_eq!(attempted(&c, &[g("Flag")], &proof), (None, 0));
        valid_closed(&c, lam(g("Flag"), proof));
    }
    for levels in [vec![], vec![Universe::Type(0), Universe::Type(1)]] {
        let proof = neutral_pair(app(
            lam(
                g("Nat"),
                Db::Const {
                    name: "PolyPair".into(),
                    levels,
                },
            ),
            g("zero"),
        ));
        assert!(!definitely_fieldful_k_family(&c, &[g("Flag")], &proof));
        rejected_by_both(&c, lam(g("Flag"), proof));
    }
    c.with_equations_suspended(|c| {
        c.declare("PairAlias", Term::Sort(Universe::Type(0)), t("Pair"))
    })
    .unwrap();
    let proof = neutral_pair(app(lam(g("Nat"), g("PairAlias")), g("zero")));
    assert_eq!(infer(&c, &[g("Flag")], &proof).unwrap(), g("Pair"));
    assert_eq!(attempted(&c, &[g("Flag")], &proof), (None, 1));
}
