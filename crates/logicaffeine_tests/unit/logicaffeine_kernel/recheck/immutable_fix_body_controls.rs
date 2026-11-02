use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn nat(n: i64) -> Db {
    Db::Lit(Literal::Nat(logicaffeine_base::BigInt::from_i64(n)))
}
fn app(a: Db, b: Db) -> Db {
    Db::App(DbBox::new(a), DbBox::new(b))
}
fn lam(a: Db, b: Db) -> Db {
    Db::Lam(DbBox::new(a), DbBox::new(b))
}
fn setup() {
    WHNF_DEPTH.with(|s| s.set(0));
    WHNF_FUEL.with(|s| s.set(MAX_RECHECK_STEPS));
    WHNF_EXHAUSTED.with(|s| s.set(false));
    FIX_BODY_STATS.with(|s| s.set([0; 6]));
}
fn same_metrics(a: PrefixMetrics, b: PrefixMetrics) {
    assert_eq!(
        (a.nodes, a.levels, a.depth, a.payload, a.bytes, a.closed),
        (b.nodes, b.levels, b.depth, b.payload, b.bytes, b.closed)
    );
}
fn oracle(t: &Db) -> Option<PrefixMetrics> {
    let old = FIX_FACTORY.with(|s| {
        let old = s.get();
        s.set(FixFactory::default());
        old
    });
    let answer = prefix_metrics(t);
    FIX_FACTORY.with(|s| s.set(old));
    answer
}
fn large_body() -> Db {
    let mut out = Db::Var(0);
    for _ in 0..24 {
        out = app(g("Succ"), out);
    }
    lam(g("Nat"), out)
}
#[test]
fn same_live_fix_identity_avoids_actual_key_and_admission_walks() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    let key = app(Db::Fix(FixBody::new(large_body())), nat(0));
    let same = key.clone();
    let old = FIX_FACTORY.with(|s| {
        let old = s.get();
        s.set(FixFactory::default());
        old
    });
    FIX_BODY_STATS.with(|s| s.set([0; 6]));
    assert_eq!(prefix_exact_equal(&key, &same), Some(true));
    let old_visits = FIX_BODY_STATS.with(|s| s.get()[5]);
    FIX_FACTORY.with(|s| s.set(old));
    FIX_BODY_STATS.with(|s| s.set([0; 6]));
    assert_eq!(prefix_exact_equal(&key, &same), Some(true));
    let stats = FIX_BODY_STATS.with(|s| s.get());
    assert_eq!(stats[3], 1);
    assert!(stats[5] < old_visits / 4);
    PREFIX_METRICS_NODES.with(|s| s.set(0));
    let fast = prefix_metrics(&key).unwrap();
    let fast_visits = PREFIX_METRICS_NODES.with(|s| s.replace(0));
    let original = oracle(&key).unwrap();
    let old_visits = PREFIX_METRICS_NODES.with(|s| s.get());
    same_metrics(fast, original);
    assert!(fast_visits < old_visits / 4);
}
#[test]
fn different_owners_and_different_values_still_use_exact_structure() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    let a = Db::Fix(FixBody::new(large_body()));
    let equal = Db::Fix(FixBody::new(large_body()));
    let different = Db::Fix(FixBody::new(lam(g("Nat"), nat(9))));
    assert_eq!(prefix_exact_equal(&a, &equal), Some(true));
    assert_eq!(prefix_exact_equal(&a, &different), Some(false));
    assert_eq!(FIX_BODY_STATS.with(|s| s.get()[3]), 0);
    assert_eq!(a, equal);
    assert_ne!(a, different);
}
#[test]
fn unchanged_shift_and_beta_preserve_self_scope_but_changed_outer_index_does_not() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    let fixed = Db::Fix(FixBody::new(lam(g("Nat"), Db::Var(1))));
    let shifted = shift(&fixed, 1, 0);
    let opened = beta_open(&fixed, &nat(7));
    let (Db::Fix(a), Db::Fix(b), Db::Fix(d)) = (&fixed, &shifted, &opened) else {
        panic!("Fix");
    };
    assert!(a.same_current(b));
    assert!(a.same_current(d));
    let outer = Db::Fix(FixBody::new(Db::Var(1)));
    let Db::Fix(b) = &outer else { panic!("Fix") };
    assert!(matches!(b, FixBody::Owned(_)));
    assert_eq!(
        shift(&outer, 1, 0),
        Db::Fix(FixBody::owned(Box::new(Db::Var(2))))
    );
    assert_eq!(
        beta_open(&outer, &nat(7)),
        Db::Fix(FixBody::owned(Box::new(nat(7))))
    );
}
#[test]
fn explicit_universe_instantiation_rebuilds_before_metadata_reuse() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    let t = Db::Fix(FixBody::new(Db::Sort(Universe::Var("u".into()))));
    let mut levels = HashMap::new();
    levels.insert("u".into(), Universe::Type(1));
    let result = db_instantiate_universes(&t, &levels);
    let expected = Db::Fix(FixBody::owned(Box::new(Db::Sort(Universe::Type(1)))));
    assert_eq!(result, expected);
    assert_eq!(prefix_exact_equal(&t, &result), Some(false));
}
#[test]
fn one_factory_allowance_cannot_be_recreated_after_optional_exhaustion() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    FIX_FACTORY.with(|s| {
        let mut f = s.get();
        f.remaining = 1;
        s.set(f)
    });
    assert!(matches!(FixBody::new(nat(0)), FixBody::Owned(_)));
    let before = FIX_FACTORY.with(|s| s.get());
    assert_eq!(before.remaining, 0);
    assert!(!fix_factory_begin());
    assert!(matches!(FixBody::new(nat(0)), FixBody::Owned(_)));
    assert_eq!(FIX_FACTORY.with(|s| s.get().spent), before.spent);
}
#[test]
fn outside_epoch_never_reads_or_resets_uninitialized_fuel() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    WHNF_FUEL.with(|s| s.set(0));
    WHNF_EXHAUSTED.with(|s| s.set(true));
    assert!(matches!(FixBody::new(nat(0)), FixBody::Shared(_)));
    assert_eq!(WHNF_FUEL.with(|s| s.get()), 0);
    assert!(WHNF_EXHAUSTED.with(|s| s.get()));
}
#[test]
fn active_epoch_pays_real_summary_work_and_keeps_low_fuel_fallback() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    WHNF_DEPTH.with(|s| s.set(1));
    WHNF_FUEL.with(|s| s.set(10));
    let a = FixBody::new(nat(0));
    assert!(matches!(a, FixBody::Shared(_)));
    assert_eq!(WHNF_FUEL.with(|s| s.get()), 8);
    WHNF_FUEL.with(|s| s.set(0));
    assert!(matches!(FixBody::new(nat(1)), FixBody::Owned(_)));
    assert!(!WHNF_EXHAUSTED.with(|s| s.get()));
    WHNF_DEPTH.with(|s| s.set(0));
}
#[test]
fn escaped_inner_owner_stays_charged_and_cannot_use_a_later_factory_generation() {
    setup();
    let c = Context::new();
    let escaped = {
        let _scope = PrefixScope::enter(&c);
        assert!(fix_factory_begin());
        let b = FixBody::new(large_body());
        assert!(b.facts().is_some());
        b
    };
    let live = FIX_SHARED_LIVE.with(|s| s.get());
    assert!(live > 0);
    assert!(escaped.facts().is_none());
    {
        let _scope = PrefixScope::enter(&c);
        assert!(fix_factory_begin());
        assert!(escaped.facts().is_none());
        let prefix = PREFIX_MEMORY.with(|s| s.get().live);
        let fill = PrefixTicket::new(PREFIX_LIVE_BYTES - live - prefix).unwrap();
        assert!(PrefixTicket::new(1).is_none());
        assert!(matches!(FixBody::new(nat(0)), FixBody::Owned(_)));
        drop(fill);
    }
    assert_eq!(FIX_SHARED_LIVE.with(|s| s.get()), live);
    drop(escaped);
    assert_eq!(FIX_SHARED_LIVE.with(|s| s.get()), 0);
}
#[test]
fn nested_scope_restores_outer_budget_without_losing_persistent_inner_ownership() {
    setup();
    let c = Context::new();
    let _outer = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    let a = FixBody::new(nat(0));
    let left = FIX_FACTORY.with(|s| s.get().remaining);
    let b = {
        let _inner = PrefixScope::enter(&c);
        assert!(fix_factory_begin());
        assert!(a.facts().is_none());
        FixBody::new(nat(1))
    };
    assert!(a.facts().is_some());
    assert!(b.facts().is_none());
    assert_eq!(FIX_FACTORY.with(|s| s.get().remaining), left);
    let live = FIX_SHARED_LIVE.with(|s| s.get());
    drop(b);
    assert!(FIX_SHARED_LIVE.with(|s| s.get()) < live);
    drop(a);
    assert_eq!(FIX_SHARED_LIVE.with(|s| s.get()), 0);
}
#[test]
fn escaped_owner_and_scope_unwind_release_only_at_last_owner_drop() {
    setup();
    let c = Context::new();
    let mut escaped = None;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _scope = PrefixScope::enter(&c);
        assert!(fix_factory_begin());
        escaped = Some(FixBody::new(large_body()));
        panic!("shared owner scope unwind");
    }));
    assert!(result.is_err());
    assert!(FIX_SHARED_LIVE.with(|s| s.get()) > 0);
    assert_eq!(PREFIX_MEMORY.with(|s| s.get().live), 0);
    drop(escaped);
    assert_eq!(FIX_SHARED_LIVE.with(|s| s.get()), 0);
}
#[test]
fn generation_exhaustion_disables_promotion_without_alias_or_budget_reset() {
    setup();
    let c = Context::new();
    let old = PREFIX_GENERATION.with(|s| s.replace(usize::MAX));
    {
        let _scope = PrefixScope::enter(&c);
        assert!(!fix_factory_begin());
        assert!(matches!(FixBody::new(nat(0)), FixBody::Owned(_)));
    }
    PREFIX_GENERATION.with(|s| s.set(old));
}
#[test]
fn malformed_literals_and_open_bodies_cannot_mint_shared_metadata() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    for t in [
        nat(-1),
        Db::Lit(Literal::Float(f64::NAN)),
        Db::Var(usize::MAX),
        Db::Var(2),
        Db::Lit(Literal::Text("x".repeat(257))),
    ] {
        assert!(matches!(FixBody::new(t), FixBody::Owned(_)));
    }
}
#[test]
fn shared_prefix_copy_retains_owned_body_without_deep_copy_or_extra_shared_bytes() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    let t = Db::Fix(FixBody::new(large_body()));
    PREFIX_STATS.with(|s| s.set([0; 5]));
    let live = FIX_SHARED_LIVE.with(|s| s.get());
    let (copied, ticket) = prefix_copy(&t).unwrap();
    assert_eq!(PREFIX_STATS.with(|s| s.get()[2]), 1);
    assert_eq!(FIX_SHARED_LIVE.with(|s| s.get()), live);
    assert_eq!(prefix_exact_equal(&t, &copied), Some(true));
    drop(copied);
    drop(ticket);
    assert_eq!(FIX_SHARED_LIVE.with(|s| s.get()), live);
}
#[test]
fn exhausted_comparison_never_uses_identity_as_a_free_positive_result() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    let a = Db::Fix(FixBody::new(large_body()));
    let b = a.clone();
    WHNF_FUEL.with(|s| s.set(0));
    assert_eq!(prefix_exact_equal(&a, &b), None);
}
#[test]
fn unknown_reference_metadata_does_not_certify_a_fixpoint_type() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    let a = Db::Fix(FixBody::new(lam(g("Missing"), Db::Var(0))));
    let Db::Fix(body) = &a else { panic!("Fix") };
    assert!(body.facts().is_some());
    assert!(matches!(infer(&c, &[], &a), Err(ReCheckError::Ill(_))));
}
#[test]
fn public_independent_check_creates_one_factory_and_drops_all_shared_owners() {
    setup();
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor_checked("Zero", "Nat", Term::Global("Nat".into()))
        .unwrap();
    let term = Term::App(
        Box::new(Term::Fix {
            name: "self".into(),
            body: Box::new(Term::Lambda {
                param: "n".into(),
                param_type: Box::new(Term::Global("Nat".into())),
                body: Box::new(Term::Var("n".into())),
            }),
        }),
        Box::new(Term::Global("Zero".into())),
    );
    assert_eq!(
        crate::infer_type(&c, &term).unwrap(),
        Term::Global("Nat".into())
    );
    assert_eq!(recheck(&c, &term).unwrap(), Term::Global("Nat".into()));
    assert!(FIX_BODY_STATS.with(|s| s.get()[0]) > 0);
    assert_eq!(FIX_SHARED_LIVE.with(|s| s.get()), 0);
    assert!(!FIX_FACTORY.with(|s| s.get().started));
}

#[test]
fn open_telescope_owner_pays_metadata_work_at_zero_whnf_depth() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    let _budget = LocalContextBudget::enter();
    assert!(fix_factory_begin());
    assert_eq!(WHNF_DEPTH.with(|d| d.get()), 0);
    WHNF_FUEL.with(|s| s.set(10));
    let b = FixBody::new(nat(0));
    assert!(matches!(b, FixBody::Shared(_)));
    assert_eq!(WHNF_FUEL.with(|s| s.get()), 8);
    WHNF_FUEL.with(|s| s.set(0));
    assert!(matches!(FixBody::new(nat(1)), FixBody::Owned(_)));
    assert!(!WHNF_EXHAUSTED.with(|s| s.get()));
}
#[test]
fn owned_opening_shares_only_unaffected_recursive_scope() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    for body in [
        Db::Var(0),
        Db::Var(1),
        lam(g("Nat"), Db::Var(1)),
        lam(g("Nat"), Db::Var(2)),
        app(Db::Var(0), Db::Var(3)),
    ] {
        let fix = Db::Fix(FixBody::new(body));
        for arg in [nat(7), Db::Var(0), Db::Var(5)] {
            assert_eq!(beta_open_owned(fix.clone(), &arg), beta_open(&fix, &arg));
        }
    }
    let fixed = Db::Fix(FixBody::new(large_body()));
    let Db::Fix(a) = &fixed else { panic!("Fix") };
    let result = beta_open_owned(fixed.clone(), &Db::Var(7));
    let Db::Fix(b) = &result else { panic!("Fix") };
    assert!(a.same_current(b));
}
