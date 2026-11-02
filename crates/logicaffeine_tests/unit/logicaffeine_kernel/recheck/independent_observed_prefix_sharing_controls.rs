use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn n(x: i64) -> Db {
    Db::Lit(Literal::Nat(x.into()))
}
fn app(f: Db, a: Db) -> Db {
    Db::App(DbBox::new(f), DbBox::new(a))
}
fn op(name: &str, a: Db, b: Db) -> Db {
    app(app(g(name), a), b)
}
fn beta(x: Db) -> Db {
    app(Db::Lam(DbBox::new(g("Nat")), DbBox::new(Db::Var(0))), x)
}
fn context() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c.add_declaration("opaque_nat", Term::Global("Nat".into()));
    c
}
struct Restore(usize, usize, bool, [usize; 5]);
impl Drop for Restore {
    fn drop(&mut self) {
        WHNF_DEPTH.with(|v| v.set(self.0));
        WHNF_FUEL.with(|v| v.set(self.1));
        WHNF_EXHAUSTED.with(|v| v.set(self.2));
        PREFIX_STATS.with(|v| v.set(self.3));
    }
}
fn metered() -> Restore {
    Restore(
        WHNF_DEPTH.with(|v| v.replace(1)),
        WHNF_FUEL.with(|v| v.replace(MAX_RECHECK_STEPS)),
        WHNF_EXHAUSTED.with(|v| v.replace(false)),
        PREFIX_STATS.with(|v| v.replace([0; 5])),
    )
}
fn check(c: &Context, name: &str, a: Db, b: Db, expected: bool) {
    let term = op(name, a, b);
    assert_eq!(
        try_nat_order_db_in(c, &[], &term),
        Some(g(if expected { "true" } else { "false" }))
    );
    assert!(
        !WHNF_EXHAUSTED.with(|v| v.get()),
        "a resource refusal is not an order decision"
    );
}
fn slots() -> usize {
    PREFIX_TABLE.with(|v| v.borrow().entries.iter().filter(|e| e.is_some()).count())
}
fn sequence(c: &Context, reuse: bool) -> ([usize; 5], usize, usize) {
    let _meter = metered();
    let _scope = PrefixScope::enter(c);
    if !reuse {
        PREFIX_TABLE.with(|v| v.borrow_mut().context = 0);
    }
    let power = op("Nat_pow", app(g("Succ"), app(g("Succ"), g("Zero"))), n(8));
    // Nat_divMod exposes Succ k and repeatedly compares against k. Reuse
    // that independently produced tail of the actual BitVec bound formula.
    let bound = match whnf_in(c, &[], &power) {
        Db::App(f, k) if matches!(*f,Db::Global(ref name) if name=="Succ") => k.into_db(),
        other => panic!("positive computed power must expose Succ, got {other:?}"),
    };
    for i in 0..64 {
        check(c, "natLt", n(i), bound.clone(), true);
    }
    let stats = PREFIX_STATS.with(|v| v.get());
    let memory = PREFIX_MEMORY.with(|v| v.get());
    (stats, memory.peak, WHNF_FUEL.with(|v| v.get()))
}
#[test]
fn exact_repeated_computed_bound_avoids_replayed_whnf_prefixes() {
    let c = context();
    let old = sequence(&c, false);
    let new = sequence(&c, true);
    assert_eq!(old.0[1], 0);
    assert!(
        new.0[1] >= 63,
        "same exact closed bound must reuse its observed prefix"
    );
    assert!(
        new.0[4] * 2 < old.0[4],
        "actual independent WHNF dispatches must fall by more than half"
    );
    assert!(new.1 <= PREFIX_LIVE_BYTES);
    assert!(
        new.2 > 0,
        "the existing total fuel must suffice without resetting"
    );
}
#[test]
fn published_count_is_total_observed_not_post_cancellation_residual() {
    let c = context();
    let _meter = metered();
    let _scope = PrefixScope::enter(&c);
    let bound = beta(n(3));
    check(&c, "natLt", n(0), bound.clone(), true);
    PREFIX_TABLE.with(|v| {
        let v = v.borrow();
        let e = v
            .entries
            .iter()
            .flatten()
            .next()
            .expect("observed bound published");
        assert_eq!(e.observed, 3);
        assert!(e.tail.is_none());
    });
    for (op, a, yes) in [
        ("natLe", 3, true),
        ("natLt", 3, false),
        ("natLe", 4, false),
        ("natLt", 2, true),
    ] {
        check(&c, op, n(a), bound.clone(), yes);
    }
}
#[test]
fn same_shape_different_closed_inputs_never_share_a_value() {
    let c = context();
    let _meter = metered();
    let _scope = PrefixScope::enter(&c);
    check(&c, "natLt", n(2), beta(n(3)), true);
    check(&c, "natLt", n(2), beta(n(2)), false);
    assert_eq!(slots(), 2);
    check(&c, "natLe", n(3), beta(n(3)), true);
    check(&c, "natLe", n(3), beta(n(2)), false);
}
#[test]
fn lazy_opaque_and_unused_divergent_tails_keep_original_demand_order() {
    let c = context();
    let _meter = metered();
    let _scope = PrefixScope::enter(&c);
    let f = Db::Lam(
        DbBox::new(g("Nat")),
        DbBox::new(app(Db::Var(0), Db::Var(0))),
    );
    let omega = app(f.clone(), f);
    check(&c, "natLe", g("Zero"), omega.clone(), true);
    assert_eq!(slots(), 0);
    check(&c, "natLt", omega, g("Zero"), false);
    check(
        &c,
        "natLe",
        app(g("Succ"), g("opaque_nat")),
        g("Zero"),
        false,
    );
    let t = op("natLe", g("opaque_nat"), g("Zero"));
    assert_eq!(try_nat_order_db_in(&c, &[], &t), None);
    assert!(!WHNF_EXHAUSTED.with(|v| v.get()));
}
#[test]
fn only_closed_keys_and_residuals_are_admitted_with_exact_binder_scope() {
    let c = context();
    let _meter = metered();
    let _scope = PrefixScope::enter(&c);
    assert!(!prefix_metrics(&beta(Db::Var(0))).unwrap().closed);
    let closed = Db::Fix(FixBody::new(Db::Lam(
        DbBox::new(g("Nat")),
        DbBox::new(Db::Var(1)),
    )));
    assert!(prefix_metrics(&closed).unwrap().closed);
    let open = Db::Fix(FixBody::new(Db::Lam(
        DbBox::new(g("Nat")),
        DbBox::new(Db::Var(2)),
    )));
    assert!(!prefix_metrics(&open).unwrap().closed);
    let mutual = Db::MutualFix {
        defs: vec![Db::Var(0), Db::Var(1)],
        index: 0,
    };
    assert!(prefix_metrics(&mutual).unwrap().closed);
    for local in [vec![g("Nat")], vec![g("Nat"), g("Nat")]] {
        let t = op("natLt", n(0), beta(Db::Var(0)));
        assert_eq!(try_nat_order_db_in(&c, &local, &t), None);
        assert!(!WHNF_EXHAUSTED.with(|v| v.get()));
    }
    assert_eq!(slots(), 0);
}
#[test]
fn four_fixed_slots_fall_back_and_keep_existing_keys_unchanged() {
    let c = context();
    let _meter = metered();
    let _scope = PrefixScope::enter(&c);
    for i in 1..=6 {
        check(&c, "natLt", n(0), beta(n(i)), true);
    }
    assert_eq!(slots(), PREFIX_SLOTS);
    check(&c, "natLt", n(1), beta(n(1)), false);
    assert!(PREFIX_STATS.with(|v| v.get()[1]) > 0);
}
#[test]
fn nested_scopes_restore_outer_cache_and_account_combined_ownership() {
    let c = context();
    let clone = c.clone();
    let _meter = metered();
    let outer = PrefixScope::enter(&c);
    check(&c, "natLt", n(0), beta(n(4)), true);
    assert_eq!(slots(), 1);
    let before = PREFIX_MEMORY.with(|v| v.get().live);
    {
        let _inner = PrefixScope::enter(&clone);
        assert_eq!(slots(), 0);
        check(&clone, "natLt", n(0), beta(n(2)), true);
        assert_eq!(slots(), 1);
        assert!(PREFIX_MEMORY.with(|v| v.get().live) > before);
    }
    assert_eq!(slots(), 1);
    assert_eq!(PREFIX_MEMORY.with(|v| v.get().live), before);
    check(&c, "natLt", n(3), beta(n(4)), true);
    drop(outer);
    assert_eq!(PREFIX_MEMORY.with(|v| v.get().live), 0);
}
#[test]
fn rebinding_and_context_clone_cannot_observe_a_previous_check_entry() {
    let c = context();
    {
        let _meter = metered();
        let _scope = PrefixScope::enter(&c);
        check(&c, "natLt", n(0), beta(n(4)), true);
        assert_eq!(slots(), 1);
    }
    assert!(
        c.refused_meaning_change_for_test(|c| {
            c.set_inductive_params("Nat", 1);
            Ok(())
        })
        .is_some(),
        "the gate keeps `Nat`'s parameters under its dependents"
    );
    let mut changed = c.clone();
    changed.permit_meaning_changes_for_cache_tests();
    changed.set_inductive_params("Nat", 1);
    let _meter = metered();
    let _scope = PrefixScope::enter(&changed);
    assert_eq!(slots(), 0);
    let term = op("natLt", n(0), beta(n(4)));
    assert_eq!(try_nat_order_db_in(&changed, &[], &term), None);
    assert_eq!(slots(), 0);
}
#[test]
fn admitted_copy_preserves_every_tag_universe_payload_and_real_capacity() {
    let c = context();
    let _meter = metered();
    let _scope = PrefixScope::enter(&c);
    let mut cases = Vec::with_capacity(16);
    cases.push(Db::Lit(Literal::Text("abc".into())));
    cases.push(Db::Lit(Literal::BigInt(4.into())));
    let term = Db::Match {
        disc: DbBox::new(g("opaque_nat")),
        motive: DbBox::new(Db::Const {
            name: "f".into(),
            levels: vec![Universe::Max(
                Box::new(Universe::Type(0)),
                Box::new(Universe::Var("u".into())),
            )],
        }),
        cases,
    };
    let original = prefix_metrics(&term).unwrap();
    assert!(original.closed);
    let before = PREFIX_MEMORY.with(|v| v.get().live);
    let (copy, memory) = prefix_copy(&term).unwrap();
    assert_eq!(copy, term);
    assert_eq!(memory.bytes, prefix_metrics(&copy).unwrap().bytes);
    assert!(
        memory.bytes < original.bytes,
        "copy does not retain unused source vector capacity"
    );
    assert_eq!(PREFIX_MEMORY.with(|v| v.get().live), before + memory.bytes);
    drop(copy);
    drop(memory);
    assert_eq!(PREFIX_MEMORY.with(|v| v.get().live), before);
    assert!(prefix_metrics(&Db::Lit(Literal::Float(f64::NAN))).is_none());
    assert!(prefix_metrics(&Db::Lit(Literal::Nat((-1).into()))).is_none());
    assert!(prefix_metrics(&Db::Lit(Literal::Nat(
        logicaffeine_base::BigInt::parse_decimal("18446744073709551616").unwrap()
    )))
    .is_none());
}
#[test]
fn hostile_deep_wide_and_payload_misses_stop_before_copying() {
    let c = context();
    let _meter = metered();
    let _scope = PrefixScope::enter(&c);
    check(&c, "natLt", n(0), beta(n(4)), true);
    let before = PREFIX_STATS.with(|v| v.get()[2]);
    let mut deep = n(4);
    for _ in 0..80 {
        deep = Db::Fix(FixBody::new(deep));
    }
    assert!(prefix_lookup(&c, &deep).is_none());
    let wide = Db::MutualFix {
        defs: vec![n(0); PREFIX_NODES + 1],
        index: 0,
    };
    assert!(prefix_lookup(&c, &wide).is_none());
    assert!(prefix_lookup(&c, &g(&"x".repeat(257))).is_none());
    assert_eq!(PREFIX_STATS.with(|v| v.get()[2]), before);
    assert!(!WHNF_EXHAUSTED.with(|v| v.get()));
}
#[test]
fn cached_answers_never_cross_exhaustion_or_copy_capacity_refusal() {
    let c = context();
    let _meter = metered();
    let _scope = PrefixScope::enter(&c);
    let bound = op("Nat_pow", n(2), n(5));
    check(&c, "natLt", n(0), bound.clone(), true);
    assert_eq!(slots(), 1);
    WHNF_EXHAUSTED.with(|v| v.set(true));
    let t = op("natLt", n(0), bound.clone());
    assert_eq!(try_nat_order_db_in(&c, &[], &t), None);
    WHNF_EXHAUSTED.with(|v| v.set(false));
    WHNF_FUEL.with(|v| v.set(1));
    assert_eq!(try_nat_order_db_in(&c, &[], &t), None);
    assert!(WHNF_EXHAUSTED.with(|v| v.get()));
    WHNF_EXHAUSTED.with(|v| v.set(false));
    WHNF_FUEL.with(|v| v.set(MAX_RECHECK_STEPS));
    let available = PREFIX_LIVE_BYTES - PREFIX_MEMORY.with(|v| v.get().live);
    let held = PrefixTicket::new(available).unwrap();
    assert!(
        prefix_lookup(&c, &bound).is_none(),
        "hit tail cannot allocate beyond the common ledger"
    );
    assert!(PREFIX_MEMORY.with(|v| v.get().peak) <= PREFIX_LIVE_BYTES);
    drop(held);
    check(&c, "natLt", n(0), bound, true);
}
#[test]
fn demanded_cycle_cannot_publish_unfinished_progress_and_holes_stay_unsupported() {
    let c = context();
    let _meter = metered();
    let _scope = PrefixScope::enter(&c);
    let f = Db::Lam(
        DbBox::new(g("Nat")),
        DbBox::new(app(Db::Var(0), Db::Var(0))),
    );
    let omega = app(f.clone(), f);
    WHNF_FUEL.with(|v| v.set(1024));
    assert_eq!(
        try_nat_order_db_in(&c, &[], &op("natLt", n(0), omega)),
        None
    );
    assert!(WHNF_EXHAUSTED.with(|v| v.get()));
    assert_eq!(slots(), 0);
    assert!(matches!(
        recheck(&c, &Term::Hole),
        Err(ReCheckError::Unsupported(_))
    ));
}
#[test]
fn exact_depth_boundary_and_compact_million_prefix_do_not_allocate_unary_trees() {
    let c = context();
    let _meter = metered();
    let _scope = PrefixScope::enter(&c);
    let mut at = n(0);
    for _ in 1..PREFIX_DEPTH {
        at = Db::Fix(FixBody::new(at));
    }
    assert_eq!(prefix_metrics(&at).unwrap().depth, PREFIX_DEPTH);
    let past = Db::Fix(FixBody::new(at));
    assert!(prefix_metrics(&past).is_none());
    check(&c, "natLe", n(1_000_000), beta(n(1_000_000)), true);
    assert_eq!(slots(), 1);
    assert!(
        PREFIX_STATS.with(|v| v.get()[2]) < 32,
        "copy work counts syntax nodes, not numeral magnitude"
    );
    assert!(PREFIX_MEMORY.with(|v| v.get().peak) < 16 * 1024);
    check(&c, "natLt", n(1_000_000), beta(n(1_000_000)), false);
    check(&c, "natLe", n(2_000_000), beta(n(2_000_000)), true);
    assert_eq!(
        slots(),
        1,
        "prefix count beyond compact cache cap uses ordinary comparison"
    );
}
#[test]
fn actual_bound_definition_replacement_is_seen_in_a_fresh_scope() {
    let mut c = context();
    c.declare(
        "memo_bound",
        Term::Global("Nat".into()),
        Term::Lit(Literal::Nat(3.into())),
    )
    .unwrap();
    {
        let _meter = metered();
        let _scope = PrefixScope::enter(&c);
        check(&c, "natLt", n(2), g("memo_bound"), true);
        assert_eq!(slots(), 1);
    }
    c.with_equations_suspended(|c| {
        c.declare(
            "memo_bound",
            Term::Global("Nat".into()),
            Term::Lit(Literal::Nat(2.into())),
        )
    })
    .unwrap();
    let _meter = metered();
    let _scope = PrefixScope::enter(&c);
    assert_eq!(slots(), 0);
    check(&c, "natLt", n(2), g("memo_bound"), false);
}
#[test]
fn explicit_levels_partial_calls_and_malformed_carriers_do_not_bypass_original_gates() {
    let c = context();
    let _meter = metered();
    let _scope = PrefixScope::enter(&c);
    let explicit = app(
        app(
            Db::Const {
                name: "natLt".into(),
                levels: vec![],
            },
            n(0),
        ),
        beta(n(4)),
    );
    assert_eq!(try_nat_order_db_in(&c, &[], &explicit), None);
    assert_eq!(try_nat_order_db_in(&c, &[], &app(g("natLt"), n(0))), None);
    let extra = app(op("natLt", n(0), beta(n(4))), n(0));
    assert_eq!(try_nat_order_db_in(&c, &[], &extra), None);
    for bad in [Db::Lit(Literal::Nat((-1).into())), Db::Lit(Literal::Int(0))] {
        assert!(matches!(
            infer(&c, &[], &op("natLt", bad, beta(n(4)))),
            Err(ReCheckError::Ill(_))
        ));
    }
    assert_eq!(slots(), 0);
}

#[test]
fn failed_same_context_scope_suppresses_outer_entries_and_restores_them() {
    let c = context();
    let _meter = metered();
    let outer = PrefixScope::enter(&c);
    check(&c, "natLt", n(0), beta(n(4)), true);
    assert_eq!(slots(), 1);
    let available = PREFIX_LIVE_BYTES - PREFIX_MEMORY.with(|v| v.get().live);
    let held = PrefixTicket::new(available).unwrap();
    let hits = PREFIX_STATS.with(|v| v.get()[1]);
    {
        let _suppressed = PrefixScope::enter(&c);
        assert_eq!(
            slots(),
            0,
            "failed nested admission must not expose outer entries"
        );
        check(&c, "natLt", n(0), beta(n(4)), true);
        assert_eq!(PREFIX_STATS.with(|v| v.get()[1]), hits);
    }
    assert_eq!(slots(), 1);
    drop(held);
    check(&c, "natLt", n(0), beta(n(4)), true);
    assert!(PREFIX_STATS.with(|v| v.get()[1]) > hits);
    drop(outer);
    assert_eq!(PREFIX_MEMORY.with(|v| v.get().live), 0);
}
#[test]
fn nested_order_during_tail_whnf_does_not_hold_a_memo_borrow() {
    let c = context();
    let _meter = metered();
    let scope = PrefixScope::enter(&c);
    let bound = Db::Match {
        disc: DbBox::new(op("natLt", n(1), beta(n(3)))),
        motive: DbBox::new(Db::Lam(DbBox::new(g("Bool")), DbBox::new(g("Nat")))),
        cases: vec![n(4), n(4)],
    };
    assert_eq!(infer(&c, &[], &bound).unwrap(), g("Nat"));
    check(&c, "natLt", n(0), bound.clone(), true);
    check(&c, "natLt", n(4), bound, false);
    assert!(
        slots() >= 2,
        "inner and outer completed observations both publish"
    );
    drop(scope);
    assert_eq!(PREFIX_MEMORY.with(|v| v.get().live), 0);
}
#[test]
fn low_remaining_fuel_skips_optional_cache_work_without_refund_or_exhaustion() {
    let c = context();
    let _meter = metered();
    let _scope = PrefixScope::enter(&c);
    check(&c, "natLt", n(0), beta(n(4)), true);
    assert_eq!(slots(), 1);
    let hits = PREFIX_STATS.with(|v| v.get()[1]);
    WHNF_FUEL.with(|v| v.set(128));
    check(&c, "natLt", n(0), beta(n(4)), true);
    let left = WHNF_FUEL.with(|v| v.get());
    assert!(left < 128 && left > 0);
    assert_eq!(
        PREFIX_STATS.with(|v| v.get()[1]),
        hits,
        "unreserved optional work is skipped"
    );
}
