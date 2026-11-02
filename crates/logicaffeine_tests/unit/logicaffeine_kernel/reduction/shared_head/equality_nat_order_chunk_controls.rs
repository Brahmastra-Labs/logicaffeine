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
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, a: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(a),
        body: Box::new(b),
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
fn order(op: &str, a: Term, b: Term) -> Term {
    apps(g(op), &[a, b])
}
fn observe(c: &Context, t: &Term, fuel: u64, full_data: bool) -> (Option<bool>, u64) {
    let mut r = HeadReducer::new(c, fuel);
    assert!(!r.preserve_nat_syntax, "this is equality observation mode");
    r.normalize_unary_spines = full_data;
    let root = r.store.intern(t);
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
fn no_shortcut(c: &Context, op: &str) {
    let mut r = HeadReducer::new(c, 1000);
    let head = r.store.intern(&g(op));
    assert_eq!(r.readback_nat_order(head), None);
}

#[test]
fn equality_literals_use_real_bounded_machine_work_for_both_order_relations() {
    let c = context();
    for (a, b) in [
        (1_000_000, 1_000_001),
        (1_000_001, 1_000_000),
        (1_000_000, 1_000_000),
    ] {
        for op in ["natLe", "natLt"] {
            let t = order(op, lit(a), lit(b));
            let expected = if op == "natLe" { a <= b } else { a < b };
            assert_eq!(crate::infer_type(&c, &t).unwrap(), g("Bool"));
            assert_eq!(crate::recheck(&c, &t).unwrap(), g("Bool"));
            for full_data in [false, true] {
                let (answer, left) = observe(&c, &t, 1000, full_data);
                assert_eq!(answer, Some(expected));
                assert!(
                    left > 0 && left < 1000,
                    "actual shared-machine fuel must be spent without exhaustion"
                );
            }
        }
    }
}

#[test]
fn public_constructor_observation_accepts_only_the_correct_boolean_in_both_directions() {
    let c = context();
    for (op, a, b, want) in [
        ("natLe", 100000, 100001, true),
        ("natLe", 100001, 100000, false),
        ("natLt", 100000, 100000, false),
        ("natLt", 100000, 100001, true),
    ] {
        let t = order(op, lit(a), lit(b));
        let yes = g(if want { "true" } else { "false" });
        let no = g(if want { "false" } else { "true" });
        assert!(constructor_equivalent(&c, &t, &yes, 1000));
        assert!(constructor_equivalent(&c, &yes, &t, 1000));
        assert!(!constructor_equivalent(&c, &t, &no, 1000));
        assert!(!constructor_equivalent(&c, &no, &t, 1000));
    }
}

#[test]
fn compact_and_constructor_naturals_agree_without_changing_stored_syntax() {
    let c = context();
    for (a, b) in [(0, 0), (0, 3), (3, 0), (2, 3), (3, 2), (3, 3)] {
        for op in ["natLe", "natLt"] {
            let t = order(
                op,
                peano(a, g("Zero")),
                peano(b / 2, lit((b - b / 2) as i64)),
            );
            assert_eq!(crate::infer_type(&c, &t).unwrap(), g("Bool"));
            assert_eq!(crate::recheck(&c, &t).unwrap(), g("Bool"));
            let want = if op == "natLe" { a <= b } else { a < b };
            for full in [false, true] {
                assert_eq!(observe(&c, &t, 10000, full).0, Some(want));
            }
            let mut r = HeadReducer::new(&c, 10000);
            let id = r.store.intern(&t);
            assert!(r.whnf(id).is_some());
            assert_eq!(
                r.store.extract(id),
                t,
                "observer never rewrites the immutable source syntax"
            );
        }
    }
}

#[test]
fn lazy_zero_and_prefix_decisions_preserve_left_first_opaque_tail_boundaries() {
    let mut c = context();
    c.add_declaration("order.opaque", g("Nat"));
    let opaque = || g("order.opaque");
    for full in [false, true] {
        for (op, a, b, want) in [
            ("natLe", g("Zero"), opaque(), Some(true)),
            ("natLe", app(g("Succ"), opaque()), g("Zero"), Some(false)),
            ("natLe", opaque(), g("Zero"), None),
            ("natLt", opaque(), g("Zero"), Some(false)),
            ("natLt", g("Zero"), opaque(), None),
            ("natLe", peano(8, opaque()), peano(8, g("Zero")), None),
            ("natLe", peano(8, g("Zero")), peano(8, opaque()), Some(true)),
            (
                "natLe",
                peano(9, opaque()),
                peano(8, g("Zero")),
                Some(false),
            ),
        ] {
            assert_eq!(observe(&c, &order(op, a, b), 10000, full).0, want);
        }
    }
}

#[test]
fn cursor_views_keep_compact_values_and_unary_tail_identity_distinct() {
    let c = context();
    let mut r = HeadReducer::new(&c, 1000);
    let head = r.store.intern(&g("Succ"));
    let tail = r
        .thunk(Thunk::Done(Value::Nat(crate::BigInt::from_i64(3))))
        .unwrap();
    let id = tail.id;
    let key = tail.key.get();
    assert!(
        matches!(r.nat_cursor(Value::Nat(crate::BigInt::from_i64(10))),
        Some(NatCursor::Prefix{count,tail:None}) if count==crate::BigInt::from_i64(10))
    );
    let view = r
        .nat_cursor(Value::UnaryRun {
            head,
            count: 7,
            tail: tail.clone(),
        })
        .unwrap();
    let NatCursor::Prefix {
        count,
        tail: Some(same),
    } = view
    else {
        panic!("exact unary cursor");
    };
    assert_eq!(count, crate::BigInt::from_i64(7));
    assert!(Rc::ptr_eq(&same, &tail));
    assert_eq!(tail.id, id);
    assert_eq!(tail.key.get(), key);
    assert!(
        matches!(&*tail.state.borrow(),Thunk::Done(Value::Nat(n)) if *n==crate::BigInt::from_i64(3))
    );
}

#[test]
fn checked_definition_rebinding_and_clone_isolation_cannot_reuse_old_attestation() {
    let original = context();
    let mut changed = original.clone();
    assert_eq!(
        observe(
            &original,
            &order("natLe", lit(100000), lit(100001)),
            1000,
            false
        )
        .0,
        Some(true)
    );
    let ty = changed.get_definition_type("natLe").unwrap().clone();
    let kept = changed
        .refused_meaning_change_for_test(|c| {
            c.declare(
                "natLe",
                ty.clone(),
                lam("a", g("Nat"), lam("b", g("Nat"), g("false"))),
            )
        })
        .expect("the gate keeps `natLe` under its dependents");
    assert_eq!(
        observe(
            &kept,
            &order("natLe", lit(100000), lit(100001)),
            1000,
            false
        )
        .0,
        Some(true)
    );
    changed.permit_meaning_changes_for_cache_tests();
    changed
        .declare(
            "natLe",
            ty,
            lam("a", g("Nat"), lam("b", g("Nat"), g("false"))),
        )
        .unwrap();
    no_shortcut(&changed, "natLe");
    no_shortcut(&changed, "natLt");
    assert_eq!(
        observe(&changed, &order("natLe", lit(0), lit(0)), 1000, false).0,
        Some(false)
    );
    assert_eq!(
        observe(&changed, &order("natLt", lit(0), lit(1)), 1000, false).0,
        Some(false)
    );
    assert_eq!(
        observe(
            &original,
            &order("natLe", lit(100000), lit(100001)),
            1000,
            false
        )
        .0,
        Some(true)
    );
}

#[test]
fn constructor_collisions_and_changed_carrier_metadata_refuse_chunk_admission() {
    let mut c = context();
    assert!(
        c.refused_meaning_change_for_test(|c| {
            c.add_constructor("natLe", "Nat", g("Nat"));
            Ok(())
        })
        .is_some(),
        "the stable-meaning gate refuses this change; the cache is tested with it permitted"
    );
    assert!(
        c.refused_meaning_change_for_test(|c| {
            c.set_inductive_params("Nat", 1);
            Ok(())
        })
        .is_some(),
        "the stable-meaning gate refuses this change; the cache is tested with it permitted"
    );
    assert!(
        c.refused_meaning_change_for_test(|c| {
            c.add_constructor("OtherNatural", "Nat", g("Nat"));
            Ok(())
        })
        .is_some(),
        "the stable-meaning gate refuses this change; the cache is tested with it permitted"
    );
    c.permit_meaning_changes_for_cache_tests();
    let mut collision = c.clone();
    collision.add_constructor("natLe", "Nat", g("Nat"));
    no_shortcut(&collision, "natLe");
    assert!(crate::infer_type(&collision, &order("natLe", lit(1), lit(2))).is_err());
    assert!(crate::recheck(&collision, &order("natLe", lit(1), lit(2))).is_err());
    let mut parameters = c.clone();
    parameters.set_inductive_params("Nat", 1);
    no_shortcut(&parameters, "natLe");
    no_shortcut(&parameters, "natLt");
    let mut extra = c.clone();
    extra.add_constructor("OtherNatural", "Nat", g("Nat"));
    no_shortcut(&extra, "natLe");
    no_shortcut(&extra, "natLt");
}

#[test]
fn universe_arity_hole_and_malformed_literal_boundaries_are_unchanged() {
    let c = context();
    for t in [
        apps(
            Term::Const {
                name: "natLe".into(),
                levels: vec![],
            },
            &[lit(2), lit(3)],
        ),
        apps(
            Term::Const {
                name: "natLe".into(),
                levels: vec![crate::Universe::Type(0)],
            },
            &[lit(2), lit(3)],
        ),
        app(order("natLe", lit(2), lit(3)), lit(0)),
        order("natLe", Term::Hole, lit(0)),
        order("natLe", lit(-1), lit(0)),
    ] {
        assert!(crate::infer_type(&c, &t).is_err());
        assert!(crate::recheck(&c, &t).is_err());
        assert!(!constructor_equivalent(&c, &t, &g("true"), 1000));
        assert!(!constructor_equivalent(&c, &t, &g("false"), 1000));
    }
    let mut r = HeadReducer::new(&c, 1000);
    let t = app(g("natLe"), lit(2));
    let id = r.store.intern(&t);
    assert!(matches!(r.whnf(id), Some(Value::Lambda { .. })));
    assert!(!constructor_equivalent(&c, &t, &g("true"), 1000));
}

#[test]
fn exhausted_equality_observation_cannot_certify_a_decision_or_refund_work() {
    let c = context();
    let t = order("natLe", lit(100000), lit(100001));
    for fuel in [0, 1, 2] {
        assert_eq!(observe(&c, &t, fuel, false), (None, 0));
        assert!(!constructor_equivalent(&c, &t, &g("true"), fuel));
    }
    let mut r = HeadReducer::new(&c, 1000);
    let id = r.store.intern(&t);
    assert!(r.whnf(id).is_some());
    let remaining = r.fuel;
    assert!(remaining > 0 && remaining < 1000);
    r.fuel = 0;
    assert!(
        r.whnf(id).is_none(),
        "a completed cache cannot bypass the outer machine fuel door"
    );
    assert_eq!(r.fuel, 0);
}

#[test]
fn changed_boolean_constructor_falls_back_to_its_actual_meaning() {
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
    no_shortcut(&c, "natLe");
    assert_eq!(
        observe(&c, &order("natLe", g("Zero"), g("Zero")), 1000, false).0,
        Some(false)
    );
}

#[test]
fn equality_observation_still_requires_every_data_and_proof_field() {
    let mut c = context();
    c.add_inductive("OrderWitness", Term::Sort(crate::Universe::Prop));
    c.add_constructor("OrderWitness.mk", "OrderWitness", g("OrderWitness"));
    c.add_inductive("OrderRecord", Term::Sort(crate::Universe::Type(0)));
    c.add_constructor(
        "OrderRecord.mk",
        "OrderRecord",
        pi(
            "decision",
            g("Bool"),
            pi("evidence", g("OrderWitness"), g("OrderRecord")),
        ),
    );
    let record = apps(
        g("OrderRecord.mk"),
        &[
            order("natLe", lit(100000), lit(100001)),
            g("OrderWitness.mk"),
        ],
    );
    assert_eq!(crate::infer_type(&c, &record).unwrap(), g("OrderRecord"));
    assert_eq!(crate::recheck(&c, &record).unwrap(), g("OrderRecord"));
    assert!(
        !constructor_equivalent(&c, &record, &record, 10000),
        "chunk computation does not authorize ignoring an Unknown proof field"
    );
    let different = apps(g("OrderRecord.mk"), &[g("false"), g("OrderWitness.mk")]);
    assert!(!constructor_equivalent(&c, &record, &different, 10000));
}

#[test]
fn active_simp_rules_keep_the_existing_observer_refusal() {
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
        "equality_order_rewrite",
        statement,
        apps(g("refl"), &[g("Nat"), g("Zero")]),
    )
    .unwrap();
    c.add_simp_lemma("equality_order_rewrite").unwrap();
    assert!(!c.simp_rules().is_empty());
    no_shortcut(&c, "natLe");
    let t = order("natLe", lit(100000), lit(100001));
    assert!(!constructor_equivalent(&c, &t, &g("true"), 1000));
}
