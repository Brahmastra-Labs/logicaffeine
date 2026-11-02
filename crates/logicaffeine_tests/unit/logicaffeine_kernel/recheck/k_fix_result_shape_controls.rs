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
fn function(motive: Db, cases: Vec<Db>) -> Db {
    Db::Fix(FixBody::new(lam(
        g("Nat"),
        Db::Match {
            disc: DbBox::new(Db::Var(0)),
            motive: DbBox::new(motive),
            cases,
        },
    )))
}
fn pair_function(motive: Db) -> Db {
    function(motive, vec![pair(), lam(g("Nat"), pair())])
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
#[test]
fn exact_applied_fix_with_raw_type_motive_skips_ineligible_k() {
    let c = context();
    let proof = app(pair_function(g("Pair")), Db::Var(0));
    assert_eq!(infer(&c, &[g("Nat")], &proof).unwrap(), g("Pair"));
    assert_eq!(attempted(&c, &[g("Nat")], &proof), (None, 0));
    let term = lam(g("Nat"), eliminate_pair(proof));
    assert!(crate::recheck(&c, &from_db(&term, 0)).is_ok());
}
#[test]
fn exact_applied_fix_with_function_motive_skips_ineligible_k() {
    let c = context();
    let proof = app(pair_function(lam(g("Nat"), g("Pair"))), Db::Var(0));
    assert_eq!(infer(&c, &[g("Nat")], &proof).unwrap(), g("Pair"));
    assert_eq!(attempted(&c, &[g("Nat")], &proof), (None, 0));
}
#[test]
fn malformed_fix_branches_and_coverage_still_fail_full_checking() {
    let c = context();
    for cases in [vec![], vec![pair()], vec![g("on"), lam(g("Nat"), pair())]] {
        let proof = app(function(g("Pair"), cases), Db::Var(0));
        rejected_by_both(&c, lam(g("Nat"), eliminate_pair(proof)));
    }
}
#[test]
fn nondecreasing_recursive_calls_are_not_certified_by_result_shape() {
    let c = context();
    let looping = function(
        g("Pair"),
        vec![
            app(Db::Var(1), Db::Var(0)),
            lam(g("Nat"), app(Db::Var(2), Db::Var(1))),
        ],
    );
    rejected_by_both(&c, lam(g("Nat"), eliminate_pair(app(looping, Db::Var(0)))));
}
#[test]
fn wrong_motive_domain_and_non_sort_codomain_still_fail() {
    let c = context();
    for motive in [lam(g("Flag"), g("Pair")), lam(g("Nat"), pair())] {
        let proof = app(pair_function(motive), Db::Var(0));
        rejected_by_both(&c, lam(g("Nat"), eliminate_pair(proof)));
    }
}
#[test]
fn fieldless_fix_result_keeps_ordinary_k_reconstruction() {
    let c = context();
    let f = function(g("Unit"), vec![g("star"), lam(g("Nat"), g("star"))]);
    let proof = app(f, Db::Var(0));
    assert_eq!(infer(&c, &[g("Nat")], &proof).unwrap(), g("Unit"));
    assert_eq!(attempted(&c, &[g("Nat")], &proof), (Some(g("star")), 1));
}
#[test]
fn partial_excess_and_computed_motive_heads_keep_fallback() {
    let mut c = context();
    c.with_equations_suspended(|c| {
        c.declare("PairAlias", Term::Sort(Universe::Type(0)), t("Pair"))
    })
    .unwrap();
    let f = pair_function(g("Pair"));
    for proof in [f.clone(), app(app(f, g("zero")), g("zero"))] {
        assert!(!definitely_fieldful_k_family(&c, &[], &proof));
    }
    let aliased = app(pair_function(g("PairAlias")), Db::Var(0));
    assert_eq!(infer(&c, &[g("Nat")], &aliased).unwrap(), g("Pair"));
    assert_eq!(attempted(&c, &[g("Nat")], &aliased), (None, 1));
    let computed = app(
        pair_function(Db::Let(
            DbBox::new(Db::Sort(Universe::Type(0))),
            DbBox::new(g("Pair")),
            DbBox::new(Db::Var(0)),
        )),
        Db::Var(0),
    );
    assert!(!definitely_fieldful_k_family(&c, &[g("Nat")], &computed));
}
#[test]
fn bound_variable_result_head_is_not_guessed_or_captured() {
    let c = context();
    let motive = lam(g("Nat"), Db::Var(0));
    let proof = app(pair_function(motive), Db::Var(0));
    assert!(!definitely_fieldful_k_family(&c, &[g("Nat")], &proof));
    rejected_by_both(&c, lam(g("Nat"), eliminate_pair(proof)));
}
#[test]
fn raw_constructor_remap_and_carrier_alias_disable_fix_admission() {
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
    let proof = app(pair_function(g("Pair")), Db::Var(0));
    assert!(!definitely_fieldful_k_family(&c, &[g("Nat")], &proof));
    assert!(infer(&c, &[g("Nat")], &proof).is_err());
}
#[test]
fn oversized_fix_and_motive_telescope_is_inconclusive() {
    let c = context();
    let mut motive = g("Pair");
    for _ in 0..260 {
        motive = lam(g("Nat"), motive);
    }
    let proof = app(pair_function(motive), g("zero"));
    K_PROBE_ATTEMPTS.with(|n| n.set(Some(0)));
    assert!(!definitely_fieldful_k_family(&c, &[], &proof));
    assert_eq!(K_PROBE_ATTEMPTS.with(|n| n.replace(None).unwrap()), 0);
}

#[test]
fn indexed_fieldless_proposition_result_keeps_k_and_exact_indices() {
    let mut c = context();
    c.add_mutual_inductives(&[crate::context::MutualInductive {
        name: "Mark".into(),
        sort: pi("n", t("Nat"), Term::Sort(Universe::Prop)),
        num_params: 0,
        constructors: vec![("marked".into(), tapp(t("Mark"), t("zero")))],
    }])
    .unwrap();
    let f = function(
        app(g("Mark"), g("zero")),
        vec![g("marked"), lam(g("Nat"), g("marked"))],
    );
    let proof = app(f, Db::Var(0));
    assert_eq!(
        infer(&c, &[g("Nat")], &proof).unwrap(),
        app(g("Mark"), g("zero"))
    );
    assert_eq!(attempted(&c, &[g("Nat")], &proof), (Some(g("marked")), 1));
}
#[test]
fn explicit_polymorphic_result_preserves_level_arity_boundaries() {
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
        let proof = app(
            function(result.clone(), vec![value.clone(), lam(g("Nat"), value)]),
            Db::Var(0),
        );
        assert_eq!(infer(&c, &[g("Nat")], &proof).unwrap(), result);
        assert_eq!(attempted(&c, &[g("Nat")], &proof), (None, 0));
    }
    for levels in [vec![], vec![Universe::Type(0), Universe::Type(1)]] {
        let proof = app(
            pair_function(Db::Const {
                name: "PolyPair".into(),
                levels,
            }),
            Db::Var(0),
        );
        assert!(!definitely_fieldful_k_family(&c, &[g("Nat")], &proof));
        assert!(infer(&c, &[g("Nat")], &proof).is_err());
    }
}

#[test]
fn two_parameter_fieldful_result_and_two_fix_arguments_skip_k() {
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
    let result = app(app(g("Product"), g("Nat")), g("Nat"));
    let value = app(
        app(app(app(g("product"), g("Nat")), g("Nat")), g("zero")),
        g("zero"),
    );
    let f = Db::Fix(FixBody::new(lam(
        g("Nat"),
        lam(
            g("Nat"),
            Db::Match {
                disc: DbBox::new(Db::Var(0)),
                motive: DbBox::new(result.clone()),
                cases: vec![value.clone(), lam(g("Nat"), value)],
            },
        ),
    )));
    let proof = app(app(f, Db::Var(1)), Db::Var(0));
    assert_eq!(infer(&c, &[g("Nat"), g("Nat")], &proof).unwrap(), result);
    assert_eq!(attempted(&c, &[g("Nat"), g("Nat")], &proof), (None, 0));
    let closed = lam(g("Nat"), lam(g("Nat"), proof));
    assert!(crate::recheck(&c, &from_db(&closed, 0)).is_ok());
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
fn indexed_discriminant_generic_motive_has_rigid_fieldful_result() {
    let c = indexed_context();
    let domain = app(g("Index"), g("zero"));
    let motive = lam(g("Nat"), lam(app(g("Index"), Db::Var(0)), g("Pair")));
    let f = Db::Fix(FixBody::new(lam(
        domain.clone(),
        Db::Match {
            disc: DbBox::new(Db::Var(0)),
            motive: DbBox::new(motive),
            cases: vec![pair()],
        },
    )));
    let proof = app(f, Db::Var(0));
    assert_eq!(infer(&c, &[domain.clone()], &proof).unwrap(), g("Pair"));
    assert_eq!(attempted(&c, &[domain.clone()], &proof), (None, 0));
    assert!(crate::recheck(&c, &from_db(&lam(domain, proof), 0)).is_ok());
}
#[test]
fn residual_and_overapplied_motives_remain_invalid_after_negative_admission() {
    let c = context();
    let residual = app(
        pair_function(lam(g("Nat"), lam(g("Nat"), g("Pair")))),
        Db::Var(0),
    );
    assert!(
        definitely_fieldful_k_family(&c, &[g("Nat")], &residual),
        "this proves only K ineligibility, not motive validity"
    );
    rejected_by_both(&c, lam(g("Nat"), eliminate_pair(residual)));
    let c = indexed_context();
    let domain = app(g("Index"), g("zero"));
    let f = Db::Fix(FixBody::new(lam(
        domain.clone(),
        Db::Match {
            disc: DbBox::new(Db::Var(0)),
            motive: DbBox::new(lam(g("Nat"), g("Pair"))),
            cases: vec![pair()],
        },
    )));
    let overapplied = app(f, Db::Var(0));
    assert!(
        definitely_fieldful_k_family(&c, &[domain.clone()], &overapplied),
        "an extra motive application cannot create a fieldless family"
    );
    rejected_by_both(&c, lam(domain, eliminate_pair(overapplied)));
}
