use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn lit(n: i64) -> Db {
    Db::Lit(Literal::Nat(n.into()))
}
fn succ(t: Db) -> Db {
    Db::App(DbBox::new(g("Succ")), DbBox::new(t))
}
fn chain(n: usize, tail: Db) -> Db {
    (0..n).fold(tail, |t, _| succ(t))
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor("Zero", "Nat", Term::Global("Nat".into()));
    c.add_constructor(
        "Succ",
        "Nat",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(Term::Global("Nat".into())),
            body_type: Box::new(Term::Global("Nat".into())),
        },
    );
    c
}
fn compare(c: &Context, locals: &dyn LocalTypes, a: &Db, b: &Db, expected: bool) {
    let previous = WHNF_EXHAUSTED.with(|v| v.replace(false));
    assert_eq!(def_eq(c, locals, a, b), expected);
    let exhausted = WHNF_EXHAUSTED.with(|v| v.replace(previous));
    assert!(
        !exhausted,
        "finite canonical Nat prefix must receive a completed verdict"
    );
}
#[test]
fn deep_literal_constructor_equality_is_symmetric() {
    let c = context();
    let a = chain(300, g("Zero"));
    let b = lit(300);
    compare(&c, &[], &a, &b, true);
    compare(&c, &[], &b, &a, true);
}
#[test]
fn deep_literal_constructor_inequality_is_symmetric() {
    let c = context();
    let a = chain(300, g("Zero"));
    for b in [lit(299), lit(301)] {
        compare(&c, &[], &a, &b, false);
        compare(&c, &[], &b, &a, false);
    }
}
#[test]
fn deep_constructor_prefixes_compare_computed_and_literal_tails() {
    let c = context();
    let computed = Db::Let(
        DbBox::new(g("Nat")),
        DbBox::new(lit(7)),
        DbBox::new(Db::Var(0)),
    );
    let a = chain(300, computed);
    let b = chain(300, lit(7));
    compare(&c, &[], &a, &b, true);
    compare(&c, &[], &b, &a, true);
    compare(&c, &[], &a, &chain(300, lit(8)), false);
}
#[test]
fn deep_prefixes_leave_neutral_tails_in_their_actual_local_scope() {
    let c = context();
    let locals = [g("Nat"), g("Nat")];
    let a = chain(300, Db::Var(0));
    let b = chain(
        300,
        Db::Let(
            DbBox::new(g("Nat")),
            DbBox::new(Db::Var(0)),
            DbBox::new(Db::Var(0)),
        ),
    );
    compare(&c, &locals, &a, &b, true);
    compare(&c, &locals, &b, &a, true);
    compare(&c, &locals, &a, &chain(300, Db::Var(1)), false);
    compare(&c, &locals, &lit(300), &a, false);
}
#[test]
fn natural_bridge_preserves_zero_and_malformed_literal_boundaries() {
    let c = context();
    for n in [-1, 0] {
        compare(&c, &[], &lit(n), &g("Zero"), true);
        compare(&c, &[], &g("Zero"), &lit(n), true);
        compare(&c, &[], &lit(n), &succ(g("Zero")), false);
    }
    compare(&c, &[], &lit(-1), &lit(0), false);
    compare(&c, &[], &lit(i64::MAX), &chain(3, lit(i64::MAX - 3)), true);
}
#[test]
fn unfinished_prefix_comparison_shares_fuel_and_reports_exhaustion() {
    struct Restore(usize, usize, bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            WHNF_DEPTH.with(|v| v.set(self.0));
            WHNF_FUEL.with(|v| v.set(self.1));
            WHNF_EXHAUSTED.with(|v| v.set(self.2));
        }
    }
    let _restore = Restore(
        WHNF_DEPTH.with(|v| v.get()),
        WHNF_FUEL.with(|v| v.get()),
        WHNF_EXHAUSTED.with(|v| v.get()),
    );
    WHNF_DEPTH.with(|v| v.set(1));
    WHNF_FUEL.with(|v| v.set(10));
    WHNF_EXHAUSTED.with(|v| v.set(false));
    assert!(!def_eq(&context(), &[], &chain(300, g("Zero")), &lit(300)));
    assert!(WHNF_EXHAUSTED.with(|v| v.get()));
    assert_eq!(WHNF_FUEL.with(|v| v.get()), 0);
    assert_eq!(WHNF_DEPTH.with(|v| v.get()), 1);
}

#[test]
fn earlier_incompleteness_does_not_erase_a_completed_prefix_comparison() {
    let previous = WHNF_EXHAUSTED.with(|v| v.replace(true));
    let equal = def_eq(&context(), &[], &chain(3, g("Zero")), &lit(3));
    let preserved = WHNF_EXHAUSTED.with(|v| v.replace(previous));
    assert!(equal);
    assert!(
        preserved,
        "this comparison must not clear an earlier unsupported probe"
    );
}

#[test]
fn rebound_nat_with_two_remaining_constructors_keeps_delta_and_distinction() {
    let mut c = context();
    let nat = Term::Global("Nat".into());
    c.permit_meaning_changes_for_cache_tests();
    c.declare(
        "Succ",
        Term::Pi {
            param: "x".into(),
            param_type: Box::new(nat.clone()),
            body_type: Box::new(nat.clone()),
        },
        Term::Lambda {
            param: "x".into(),
            param_type: Box::new(nat.clone()),
            body: Box::new(Term::Global("Zero".into())),
        },
    )
    .unwrap();
    c.add_constructor_checked(
        "OtherSucc",
        "Nat",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(nat.clone()),
            body_type: Box::new(nat.clone()),
        },
    )
    .unwrap();
    let inventory: Vec<_> = c
        .get_constructors("Nat")
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert_eq!(inventory, vec!["Zero", "OtherSucc"]);
    assert!(c.unit_constructor("Nat").is_none());
    assert!(!crate::int_inductive::canonical_nat_environment(&c));
    c.add_declaration("unrelated_natural", nat);
    for (a, b, expected) in [
        (succ(lit(5)), g("Zero"), true),
        (succ(lit(5)), lit(0), true),
        (succ(lit(5)), g("unrelated_natural"), false),
        (
            succ(lit(5)),
            Db::App(DbBox::new(g("OtherSucc")), DbBox::new(g("Zero"))),
            false,
        ),
    ] {
        compare(&c, &[], &a, &b, expected);
        compare(&c, &[], &b, &a, expected);
    }
}
#[test]
fn rebound_nat_singleton_keeps_delta_and_existing_unit_eta() {
    let mut c = context();
    let nat = Term::Global("Nat".into());
    c.declare(
        "Succ",
        Term::Pi {
            param: "x".into(),
            param_type: Box::new(nat.clone()),
            body_type: Box::new(nat.clone()),
        },
        Term::Lambda {
            param: "x".into(),
            param_type: Box::new(nat.clone()),
            body: Box::new(Term::Global("Zero".into())),
        },
    )
    .unwrap();
    let inventory: Vec<_> = c
        .get_constructors("Nat")
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert_eq!(inventory, vec!["Zero"]);
    assert!(c.unit_constructor("Nat").is_some());
    assert!(!crate::int_inductive::canonical_nat_environment(&c));
    c.add_declaration("singleton_neutral", nat);
    compare(&c, &[], &succ(lit(5)), &g("Zero"), true);
    compare(&c, &[], &succ(lit(5)), &lit(0), true);
    compare(&c, &[], &succ(lit(5)), &g("singleton_neutral"), true);
}

#[test]
fn a_successor_declared_again_is_no_successor_to_the_literal_bridge() {
    assert!(
        def_eq(&context(), &[], &lit(1), &succ(g("Zero"))),
        "`1 ≡ Succ Zero` while `Succ` is `Nat`'s successor"
    );
    let mut c = context();
    c.add_inductive("Bool", Term::Sort(Universe::Type(0)));
    c.add_constructor("false", "Bool", Term::Global("Bool".into()));
    c.add_constructor("true", "Bool", Term::Global("Bool".into()));
    c.declare(
        "Succ",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(Term::Global("Nat".into())),
            body_type: Box::new(Term::Global("Bool".into())),
        },
        Term::Lambda {
            param: "n".into(),
            param_type: Box::new(Term::Global("Nat".into())),
            body: Box::new(Term::Global("false".into())),
        },
    )
    .expect("`Succ` declared again");
    assert!(
        !def_eq(&c, &[], &lit(1), &succ(g("Zero"))),
        "`Succ Zero` is `false` once `Succ` is a definition into `Bool`, and no literal"
    );
}

#[test]
fn a_literal_reads_as_succ_only_while_succ_is_the_successor() {
    let mut c = context();
    // A second constructor first, so `Nat` is no unit type whose elements are all equal once
    // `Succ` is assumed in place of its constructor.
    c.add_constructor(
        "Other",
        "Nat",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(Term::Global("Nat".into())),
            body_type: Box::new(Term::Global("Nat".into())),
        },
    );
    c.add_declaration(
        "Succ",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(Term::Global("Nat".into())),
            body_type: Box::new(Term::Global("Nat".into())),
        },
    );
    assert!(
        c.refused_declarations().is_empty(),
        "{:?}",
        c.refused_declarations()
    );
    assert!(
        !def_eq(&c, &[], &lit(1), &succ(g("Zero"))),
        "an assumed `Succ` is no successor"
    );
    assert!(
        def_eq(&c, &[], &lit(0), &g("Zero")),
        "`0` is `Zero` while `Zero` is `Nat`'s"
    );
    assert!(
        db_constructor_head(&c, &lit(1)).is_none(),
        "the index unifier reads no constructor into `1`"
    );
    assert!(db_constructor_head(&c, &lit(0)).is_some());
}

#[test]
fn a_literal_match_is_stuck_where_succ_is_not_the_successor() {
    let nat = || Term::Global("Nat".into());
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor("Zero", "Nat", nat());
    c.add_constructor(
        "Succ",
        "Nat",
        Term::Pi {
            param: "a".into(),
            param_type: Box::new(nat()),
            body_type: Box::new(Term::Pi {
                param: "b".into(),
                param_type: Box::new(nat()),
                body_type: Box::new(nat()),
            }),
        },
    );
    c.add_inductive("Bool", Term::Sort(Universe::Type(0)));
    c.add_constructor("false", "Bool", Term::Global("Bool".into()));
    c.add_constructor("true", "Bool", Term::Global("Bool".into()));
    let lam = |p: &str, a: Term, b: Term| Term::Lambda {
        param: p.into(),
        param_type: Box::new(a),
        body: Box::new(b),
    };
    let on = |n: i64| {
        to_db(
            &Term::Match {
                discriminant: Box::new(Term::Lit(Literal::Nat(n.into()))),
                motive: Box::new(lam("n", nat(), Term::Global("Bool".into()))),
                cases: vec![
                    Term::Global("true".into()),
                    lam("a", nat(), lam("b", nat(), Term::Global("false".into()))),
                ],
            },
            &mut Vec::new(),
        )
        .expect("a closed term")
    };
    assert!(
        matches!(whnf_in(&c, &[], &on(1)), Db::Match { .. }),
        "`match 1` is stuck where `Succ` is not `Nat`'s successor"
    );
    assert_eq!(whnf_in(&c, &[], &on(0)), g("true"));
    assert!(
        db_constructor_head(&c, &lit(1)).is_none(),
        "the index unifier reads no one-field `Succ` into `1`"
    );
}

/// The readings are the constructors', not the family's exact shape: with `Other` beside `Zero`
/// and `Succ`, `1` is still `Succ 0`.
#[test]
fn a_literal_reads_as_succ_while_succ_is_a_constructor_of_nat() {
    let mut c = context();
    c.add_constructor(
        "Other",
        "Nat",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(Term::Global("Nat".into())),
            body_type: Box::new(Term::Global("Nat".into())),
        },
    );
    assert!(
        c.refused_declarations().is_empty(),
        "{:?}",
        c.refused_declarations()
    );
    assert!(def_eq(&c, &[], &lit(1), &succ(g("Zero"))));
    assert!(db_constructor_head(&c, &lit(1)).is_some());
}
