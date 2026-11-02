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
fn eliminate_pair(d: Db) -> Db {
    Db::Match {
        disc: DbBox::new(d),
        motive: DbBox::new(g("Flag")),
        cases: vec![Db::Lam(
            DbBox::new(g("Nat")),
            DbBox::new(Db::Lam(DbBox::new(g("Nat")), DbBox::new(g("on")))),
        )],
    }
}
fn measured(c: &Context, locals: &dyn LocalTypes, term: &Db) -> (Db, usize) {
    K_PROBE_ATTEMPTS.with(|n| n.set(Some(0)));
    let result = whnf_in(c, locals, term);
    let count = K_PROBE_ATTEMPTS.with(|n| n.replace(None).unwrap());
    (result, count)
}
#[test]
fn neutral_local_pair_does_not_run_an_ineligible_k_inference() {
    let c = context();
    let term = eliminate_pair(Db::Var(0));
    assert_eq!(infer(&c, &[g("Pair")], &term).unwrap(), g("Flag"));
    let (result, count) = measured(&c, &[g("Pair")], &term);
    assert_eq!(result, term);
    assert_eq!(count, 0);
}
#[test]
fn applied_neutral_header_avoids_k_without_evaluating_arguments() {
    let c = context();
    let mut arg = g("zero");
    for _ in 0..24 {
        arg = app(Db::Lam(DbBox::new(g("Nat")), DbBox::new(Db::Var(0))), arg);
    }
    let term = eliminate_pair(app(g("maker"), arg));
    assert_eq!(infer(&c, &[], &term).unwrap(), g("Flag"));
    let (result, count) = measured(&c, &[], &term);
    assert_eq!(result, term);
    assert_eq!(count, 0);
}
#[test]
fn explicit_universe_header_uses_its_exact_telescope_arity() {
    let c = context();
    let head = Db::Const {
        name: "polyMaker".into(),
        levels: vec![Universe::Type(0)],
    };
    let term = eliminate_pair(app(head, g("zero")));
    assert_eq!(infer(&c, &[], &term).unwrap(), g("Flag"));
    assert_eq!(measured(&c, &[], &term).1, 0);
    for levels in [vec![], vec![Universe::Type(0), Universe::Type(1)]] {
        let bad = eliminate_pair(app(
            Db::Const {
                name: "polyMaker".into(),
                levels,
            },
            g("zero"),
        ));
        assert!(infer(&c, &[], &bad).is_err());
    }
}
#[test]
fn fieldful_prefilter_never_validates_bad_arguments_or_branch_inventories() {
    let c = context();
    for term in [
        eliminate_pair(app(g("maker"), g("on"))),
        Db::Match {
            disc: DbBox::new(app(g("maker"), g("zero"))),
            motive: DbBox::new(g("Flag")),
            cases: vec![],
        },
        Db::Match {
            disc: DbBox::new(app(g("maker"), g("zero"))),
            motive: DbBox::new(g("Flag")),
            cases: vec![g("on")],
        },
    ] {
        assert!(infer(&c, &[], &term).is_err());
        let named = from_db(&term, 0);
        assert!(crate::infer_type(&c, &named).is_err());
        assert!(crate::recheck(&c, &named).is_err());
    }
}
#[test]
fn aliases_partial_excess_and_local_indices_keep_conservative_fallback() {
    let mut c = context();
    c.with_equations_suspended(|c| {
        c.declare("PairAlias", Term::Sort(Universe::Type(0)), t("Pair"))
    })
    .unwrap();
    c.add_universe_poly_declaration("aliasMaker", vec![], pi("n", t("Nat"), t("PairAlias")))
        .unwrap();
    let aliased = eliminate_pair(app(g("aliasMaker"), g("zero")));
    assert_eq!(infer(&c, &[], &aliased).unwrap(), g("Flag"));
    assert_eq!(measured(&c, &[], &aliased).1, 1);
    for bad in [
        eliminate_pair(g("maker")),
        eliminate_pair(app(app(g("maker"), g("zero")), g("zero"))),
        eliminate_pair(Db::Var(2)),
    ] {
        assert!(infer(&c, &[g("Pair")], &bad).is_err());
    }
    let term = eliminate_pair(Db::Var(1));
    assert_eq!(
        infer(&c, &[g("Pair"), g("Unit")], &term).unwrap(),
        g("Flag")
    );
    assert_eq!(measured(&c, &[g("Pair"), g("Unit")], &term).1, 0);
}
#[test]
fn unit_and_indexed_proposition_keep_their_checked_k_reductions() {
    let mut c = context();
    let singleton = Db::Match {
        disc: DbBox::new(Db::Var(0)),
        motive: DbBox::new(g("Flag")),
        cases: vec![g("on")],
    };
    assert_eq!(infer(&c, &[g("Unit")], &singleton).unwrap(), g("Flag"));
    assert_eq!(measured(&c, &[g("Unit")], &singleton), (g("on"), 1));
    c.add_mutual_inductives(&[crate::context::MutualInductive {
        name: "Mark".into(),
        sort: pi("n", t("Nat"), Term::Sort(Universe::Prop)),
        num_params: 0,
        constructors: vec![("marked".into(), tapp(t("Mark"), t("zero")))],
    }])
    .unwrap();
    let good = app(g("Mark"), g("zero"));
    assert!(k_constructor_db(&c, &[good], &Db::Var(0)).is_some());
    let different = app(g("Mark"), app(g("succ"), g("zero")));
    assert!(k_constructor_db(&c, &[different], &Db::Var(0)).is_none());
}
#[test]
fn malformed_family_and_header_are_not_admission_certificates() {
    let mut c = context();
    assert!(c
        .add_mutual_inductives(&[crate::context::MutualInductive {
            name: "Counterfeit".into(),
            sort: Term::Sort(Universe::Type(0)),
            num_params: 0,
            constructors: vec![("counterfeit".into(), t("Nat"))],
        }])
        .is_err());
    c.add_declaration("badHeader", Term::Hole);
    let term = eliminate_pair(g("badHeader"));
    assert!(infer(&c, &[], &term).is_err());
    assert!(crate::recheck(&c, &from_db(&term, 0)).is_err());
}
#[test]
fn a_match_result_shape_rules_out_k_without_certifying_the_motive() {
    let c = context();
    let inner = Db::Match {
        disc: DbBox::new(Db::Var(0)),
        motive: DbBox::new(g("Pair")),
        cases: vec![
            app(app(g("pair"), g("zero")), g("zero")),
            app(app(g("pair"), g("zero")), g("zero")),
        ],
    };
    let outer = eliminate_pair(inner);
    assert_eq!(infer(&c, &[g("Flag")], &outer).unwrap(), g("Flag"));
    let (result, count) = measured(&c, &[g("Flag")], &outer);
    assert_eq!(result, outer);
    assert_eq!(
        count, 0,
        "a necessary fieldful result shape excludes K without validating the motive"
    );
}

#[test]
fn raw_constructor_remapping_keeps_the_original_probe_fallback() {
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
    assert_eq!(c.constructor_inductive("pair"), Some("OtherPair"));
    K_PROBE_ATTEMPTS.with(|n| n.set(Some(0)));
    assert!(k_constructor_db(&c, &[g("Pair")], &Db::Var(0)).is_none());
    let count = K_PROBE_ATTEMPTS.with(|n| n.replace(None).unwrap());
    assert_eq!(
        count, 1,
        "inconsistent legacy constructor-parent mapping is not a prefilter certificate"
    );
}
#[test]
fn oversized_syntactic_spines_preserve_the_old_checked_probe() {
    let c = context();
    let mut ty = g("Pair");
    let mut proof = Db::Var(0);
    for _ in 0..260 {
        ty = Db::Pi(DbBox::new(g("Nat")), DbBox::new(ty));
        proof = app(proof, g("zero"));
    }
    K_PROBE_ATTEMPTS.with(|n| n.set(Some(0)));
    assert!(!definitely_fieldful_k_family(&c, &[ty], &proof));
    let count = K_PROBE_ATTEMPTS.with(|n| n.replace(None).unwrap());
    assert_eq!(
        count, 0,
        "observer exhaustion is inconclusive and never calls the full K probe itself"
    );
}
