use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn lit(n: i64) -> Term {
    Term::Lit(Literal::Nat(crate::BigInt::from_i64(n)))
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn apps(f: Term, args: &[Term]) -> Term {
    args.iter().cloned().fold(f, app)
}
fn lam(n: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn peano(n: usize, tail: Term) -> Term {
    (0..n).fold(tail, |t, _| app(g("Succ"), t))
}
fn context() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c
}
fn compare(c: &Context, op: &str, a: Term, b: Term, fuel: u64) -> (Option<bool>, u64) {
    let mut r = HeadReducer::for_nat_readback(c, fuel);
    let root = r.store.intern(&apps(g(op), &[a, b]));
    let answer = match r.whnf(root) {
        Some(Value::Constructor { head, args }) if args.is_empty() => match r.store.extract(head) {
            Term::Global(n) if n == "true" => Some(true),
            Term::Global(n) if n == "false" => Some(false),
            _ => None,
        },
        _ => None,
    };
    (answer, r.fuel)
}
#[test]
fn literal_comparisons_use_bounded_work_in_both_order_directions() {
    let c = context();
    for (a, b) in [
        (1_000_000, 1_000_001),
        (1_000_001, 1_000_000),
        (1_000_000, 1_000_000),
    ] {
        for op in ["natLe", "natLt"] {
            let expected = if op == "natLe" { a <= b } else { a < b };
            assert_eq!(compare(&c, op, lit(a), lit(b), 1000).0, Some(expected));
        }
    }
}
#[test]
fn mixed_chunks_match_both_kernel_types_and_small_ordinary_results() {
    let c = context();
    for (a, b) in [(0, 0), (0, 3), (3, 0), (2, 3), (3, 2), (3, 3)] {
        for op in ["natLe", "natLt"] {
            let lhs = peano(a as usize, g("Zero"));
            let rhs = peano((b / 2) as usize, lit(b - b / 2));
            let term = apps(g(op), &[lhs.clone(), rhs.clone()]);
            assert_eq!(crate::infer_type(&c, &term).unwrap(), g("Bool"));
            assert_eq!(crate::recheck(&c, &term).unwrap(), g("Bool"));
            let expected = if op == "natLe" { a <= b } else { a < b };
            assert_eq!(compare(&c, op, lhs, rhs, 10000).0, Some(expected));
            assert_eq!(
                super::super::normalize_tree_with_spent(
                    &c,
                    &term,
                    super::super::Transparency::All,
                    0
                )
                .unwrap(),
                g(if expected { "true" } else { "false" })
            );
        }
    }
}
#[test]
fn left_first_zero_and_successor_short_circuits_do_not_force_opaque_tails() {
    let mut c = context();
    c.add_declaration("opaque_nat", g("Nat"));
    assert_eq!(
        compare(&c, "natLe", g("Zero"), g("opaque_nat"), 1000).0,
        Some(true)
    );
    assert_eq!(
        compare(
            &c,
            "natLe",
            app(g("Succ"), g("opaque_nat")),
            g("Zero"),
            1000
        )
        .0,
        Some(false)
    );
    assert_eq!(
        compare(&c, "natLe", g("opaque_nat"), g("Zero"), 1000).0,
        None
    );
    assert_eq!(
        compare(&c, "natLt", g("opaque_nat"), g("Zero"), 1000).0,
        Some(false)
    );
    assert_eq!(
        compare(&c, "natLt", g("Zero"), g("opaque_nat"), 1000).0,
        None
    );
}
#[test]
fn equal_prefixes_demand_the_original_left_tail_before_the_right_tail() {
    let mut c = context();
    c.add_declaration("opaque_nat", g("Nat"));
    assert_eq!(
        compare(
            &c,
            "natLe",
            peano(8, g("opaque_nat")),
            peano(8, g("Zero")),
            2000
        )
        .0,
        None
    );
    assert_eq!(
        compare(
            &c,
            "natLe",
            peano(8, g("Zero")),
            peano(8, g("opaque_nat")),
            2000
        )
        .0,
        Some(true)
    );
    assert_eq!(
        compare(
            &c,
            "natLe",
            peano(9, g("opaque_nat")),
            peano(8, g("Zero")),
            2000
        )
        .0,
        Some(false)
    );
    assert_eq!(
        compare(
            &c,
            "natLt",
            peano(8, g("opaque_nat")),
            peano(8, g("Zero")),
            2000
        )
        .0,
        Some(false)
    );
}
#[test]
fn checked_rebinding_and_a_new_context_do_not_reuse_attestation() {
    let mut c = context();
    assert_eq!(
        compare(&c, "natLe", lit(100000), lit(100001), 1000).0,
        Some(true)
    );
    let ty = c.get_definition_type("natLe").unwrap().clone();
    let kept = c
        .refused_meaning_change_for_test(|c| {
            c.declare(
                "natLe",
                ty.clone(),
                lam("a", g("Nat"), lam("b", g("Nat"), g("false"))),
            )
        })
        .expect("the gate keeps `natLe` under its dependents");
    assert_eq!(
        compare(&kept, "natLe", lit(100000), lit(100001), 1000).0,
        Some(true)
    );
    c.permit_meaning_changes_for_cache_tests();
    c.declare(
        "natLe",
        ty,
        lam("a", g("Nat"), lam("b", g("Nat"), g("false"))),
    )
    .unwrap();
    assert_eq!(compare(&c, "natLe", lit(0), lit(0), 1000).0, Some(false));
    assert_eq!(compare(&c, "natLt", lit(0), lit(1), 1000).0, Some(false));
}
#[test]
fn constructor_header_collision_does_not_license_the_shortcut() {
    let mut c = context();
    assert!(
        c.refused_meaning_change_for_test(|c| {
            c.add_constructor("natLe", "Nat", g("Nat"));
            Ok(())
        })
        .is_some(),
        "the stable-meaning gate refuses this change; the cache is tested with it permitted"
    );
    c.permit_meaning_changes_for_cache_tests();
    c.add_constructor("natLe", "Nat", g("Nat"));
    assert_eq!(c.get_global("natLe"), Some(&g("Nat")));
    assert_eq!(compare(&c, "natLe", lit(100000), lit(100001), 1000).0, None);
}
#[test]
fn rebound_boolean_constructor_uses_the_actual_definition() {
    let mut c = context();
    assert!(
        c.refused_meaning_change_for_test(|c| {
            c.declare("true", g("Bool"), g("false"))?;
            Ok(())
        })
        .is_some(),
        "the stable-meaning gate refuses this change; the cache is tested with it permitted"
    );
    c.permit_meaning_changes_for_cache_tests();
    c.declare("true", g("Bool"), g("false")).unwrap();
    assert_eq!(
        compare(&c, "natLe", g("Zero"), g("Zero"), 1000).0,
        Some(false)
    );
}
#[test]
fn explicit_operator_levels_and_ill_typed_extra_application_stay_unsupported() {
    let c = context();
    for term in [
        apps(
            Term::Const {
                name: "natLe".into(),
                levels: vec![],
            },
            &[lit(2), lit(3)],
        ),
        app(apps(g("natLe"), &[lit(2), lit(3)]), lit(0)),
    ] {
        assert!(crate::infer_type(&c, &term).is_err());
        assert!(crate::recheck(&c, &term).is_err());
        let mut r = HeadReducer::for_nat_readback(&c, 1000);
        let id = r.store.intern(&term);
        assert!(r.whnf(id).is_none());
    }
    let mut r = HeadReducer::for_nat_readback(&c, 1000);
    let id = r.store.intern(&app(g("natLe"), lit(2)));
    assert!(matches!(r.whnf(id), Some(Value::Lambda { .. })));
}
#[test]
fn negative_literals_remain_refused_when_demanded_and_skipped_when_not_demanded() {
    let c = context();
    assert_eq!(compare(&c, "natLe", lit(-1), g("Zero"), 1000).0, None);
    // The existing delay operation refuses a malformed literal argument
    // before evaluation; no new clamp is introduced by the chunk view.
    assert_eq!(compare(&c, "natLt", lit(-1), g("Zero"), 1000).0, None);
}
#[test]
fn zero_and_tiny_fuel_never_certify_an_unfinished_comparison() {
    let c = context();
    for fuel in [0, 1, 2] {
        assert_eq!(
            compare(&c, "natLe", lit(100000), lit(100001), fuel),
            (None, 0)
        );
    }
    let (answer, left) = compare(&c, "natLe", lit(100000), lit(100001), 1000);
    assert_eq!(answer, Some(true));
    assert!(left > 0 && left < 1000);
}
#[test]
fn equality_observer_mode_reuses_attested_chunks_with_actual_fuel() {
    let c = context();
    let mut r = HeadReducer::new(&c, 1000);
    assert!(!r.preserve_nat_syntax);
    let term = apps(g("natLe"), &[lit(100000), lit(100001)]);
    let id = r.store.intern(&term);
    let Some(Value::Constructor { head, args }) = r.whnf(id) else {
        panic!("the checked comparison must produce a Boolean constructor");
    };
    assert!(args.is_empty());
    assert_eq!(r.store.extract(head), g("true"));
    assert!(r.fuel > 0 && r.fuel < 1000);
    assert_eq!(r.store.extract(id), term);
}
#[test]
fn canonical_remainder_uses_less_than_one_hundred_thousand_transitions() {
    let c = context();
    let term = apps(g("Nat_mod"), &[lit(255), lit(256)]);
    let (answer, left) = natural_normal_form(&c, &term, super::super::Transparency::All, 100000);
    assert_eq!(answer, NatReadbackOutcome::Normal(peano(255, g("Zero"))));
    assert!(left > 0);
}
#[test]
fn comparison_selection_preserves_exact_natural_representation() {
    let c = context();
    let choose = lam(
        "x",
        g("Nat"),
        Term::Match {
            discriminant: Box::new(apps(g("natLe"), &[v("x"), lit(10)])),
            motive: Box::new(lam("b", g("Bool"), g("Nat"))),
            cases: vec![v("x"), v("x")],
        },
    );
    for value in [lit(3), peano(3, g("Zero")), peano(2, lit(1))] {
        let term = app(choose.clone(), value.clone());
        assert_eq!(
            natural_normal_form(&c, &term, super::super::Transparency::All, 10000).0,
            NatReadbackOutcome::Normal(value)
        );
    }
}
#[test]
fn rebound_natural_successor_does_not_license_numeric_chunks() {
    let mut c = context();
    let ty = c.get_global("Succ").unwrap().clone();
    assert!(
        c.refused_meaning_change_for_test(|c| {
            c.declare("Succ", ty.clone(), lam("n", g("Nat"), v("n")))?;
            Ok(())
        })
        .is_some(),
        "the stable-meaning gate refuses this change; the cache is tested with it permitted"
    );
    c.permit_meaning_changes_for_cache_tests();
    c.declare("Succ", ty, lam("n", g("Nat"), v("n"))).unwrap();
    assert!(!crate::int_inductive::canonical_nat_environment(&c));
    assert_eq!(compare(&c, "natLe", lit(100000), lit(100001), 1000).0, None);
}
#[test]
fn registered_rewrites_disable_comparison_acceleration_and_public_readback() {
    let mut c = context();
    let statement = apps(
        g("Eq"),
        &[
            g("Nat"),
            apps(g("Nat_add"), &[g("Zero"), g("Zero")]),
            g("Zero"),
        ],
    );
    c.declare(
        "chunk_readback_refl",
        statement,
        apps(g("refl"), &[g("Nat"), g("Zero")]),
    )
    .unwrap();
    c.add_simp_lemma("chunk_readback_refl").unwrap();
    assert!(!c.simp_rules().is_empty());
    assert_eq!(compare(&c, "natLe", lit(100000), lit(100001), 1000).0, None);
    assert_eq!(
        natural_normal_form(
            &c,
            &apps(g("Nat_mod"), &[lit(2), lit(3)]),
            super::super::Transparency::All,
            1000
        ),
        (NatReadbackOutcome::Unsupported, 1000)
    );
}
