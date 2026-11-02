use super::*;
fn g(s: &str) -> Db {
    Db::Global(s.into())
}
fn n(x: i64) -> Db {
    Db::Lit(Literal::Nat(x.into()))
}
fn app(f: Db, a: Db) -> Db {
    Db::App(DbBox::new(f), DbBox::new(a))
}
fn op(s: &str, a: Db, b: Db) -> Db {
    app(app(g(s), a), b)
}
fn beta(x: Db) -> Db {
    app(Db::Lam(DbBox::new(g("Nat")), DbBox::new(Db::Var(0))), x)
}
fn succs(count: usize) -> Db {
    let mut t = g("Zero");
    for _ in 0..count {
        t = app(g("Succ"), t);
    }
    t
}
fn context() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c
}
struct Restore(usize, usize, bool, [usize; 5], usize);
impl Drop for Restore {
    fn drop(&mut self) {
        WHNF_DEPTH.with(|v| v.set(self.0));
        WHNF_FUEL.with(|v| v.set(self.1));
        WHNF_EXHAUSTED.with(|v| v.set(self.2));
        PREFIX_STATS.with(|v| v.set(self.3));
        PREFIX_METRICS_NODES.with(|v| v.set(self.4));
    }
}
fn metered() -> Restore {
    Restore(
        WHNF_DEPTH.with(|v| v.replace(1)),
        WHNF_FUEL.with(|v| v.replace(MAX_RECHECK_STEPS)),
        WHNF_EXHAUSTED.with(|v| v.replace(false)),
        PREFIX_STATS.with(|v| v.replace([0; 5])),
        PREFIX_METRICS_NODES.with(|v| v.replace(0)),
    )
}
fn check(c: &Context, a: i64, b: &Db, yes: bool) {
    let t = op("natLe", n(a), b.clone());
    assert_eq!(
        try_nat_order_db_in(c, &[], &t),
        Some(g(if yes { "true" } else { "false" }))
    );
    assert!(!WHNF_EXHAUSTED.with(|v| v.get()));
}
fn status() -> PrefixStatus {
    PREFIX_TABLE.with(|v| {
        v.borrow()
            .entries
            .iter()
            .flatten()
            .next()
            .unwrap()
            .status
            .get()
    })
}
fn live() -> usize {
    PREFIX_MEMORY.with(|v| v.get().live)
}
fn stats() -> [usize; 5] {
    PREFIX_STATS.with(|v| v.get())
}
fn visits() -> usize {
    PREFIX_METRICS_NODES.with(|v| v.get())
}

#[test]
fn admitted_direct_successor_projections_move_without_copy_or_readmission() {
    let c = context();
    let _meter = metered();
    let _scope = PrefixScope::enter(&c);
    let bound = beta(succs(20));
    check(&c, 1, &bound, true);
    let before = stats();
    let old_visits = visits();
    let old_live = live();
    for i in 2..=12 {
        check(&c, i, &bound, true);
    }
    assert!(stats()[1] >= before[1] + 11);
    assert_eq!(
        stats()[2],
        before[2],
        "hit transfers ownership, never copies the owned tree"
    );
    assert_eq!(
        visits(),
        old_visits,
        "direct nonbinding projections preserve admission bounds"
    );
    assert_eq!(
        live(),
        old_live,
        "conservative certificate retains original reservation"
    );
    PREFIX_TABLE.with(|v| assert_eq!(v.borrow().entries[0].as_ref().unwrap().observed, 12));
    check(&c, 21, &bound, false);
}

#[test]
fn computed_exposure_always_rechecks_tail_even_after_a_cache_hit() {
    let c = context();
    let _meter = metered();
    let _scope = PrefixScope::enter(&c);
    let bound = beta(app(g("Succ"), beta(succs(8))));
    check(&c, 1, &bound, true);
    let before = stats();
    let old_visits = visits();
    check(&c, 2, &bound, true);
    assert!(stats()[1] > before[1]);
    assert!(
        stats()[4] > before[4],
        "computed tail must invoke independent WHNF"
    );
    assert!(
        visits() > old_visits,
        "WHNF dirties even an admitted residual"
    );
    assert_eq!(
        stats()[2],
        before[2],
        "dirty publication moves its freshly checked owned tail"
    );
}

#[test]
fn busy_same_key_falls_back_and_abandonment_leaves_vacant_not_complete() {
    let c = context();
    let _meter = metered();
    let _scope = PrefixScope::enter(&c);
    let bound = beta(succs(8));
    check(&c, 1, &bound, true);
    let (seed, cursor) = prefix_lookup(&c, &bound).unwrap();
    assert!(status() == PrefixStatus::Busy);
    assert!(
        prefix_lookup(&c, &bound).is_none(),
        "recursive demand must not reuse a leased value"
    );
    drop(cursor);
    drop(seed);
    assert!(status() == PrefixStatus::Vacant);
    let hits = stats()[1];
    check(&c, 4, &bound, true);
    assert_eq!(stats()[1], hits, "vacant key requires actual recomputation");
    assert!(status() == PrefixStatus::Available);
}

#[test]
fn exhausted_lease_never_publishes_partial_progress() {
    let c = context();
    let _meter = metered();
    let _scope = PrefixScope::enter(&c);
    let bound = beta(succs(8));
    check(&c, 1, &bound, true);
    let (seed, cursor) = prefix_lookup(&c, &bound).unwrap();
    let mut seed = Some(seed);
    let mut cursor = cursor.unwrap();
    WHNF_FUEL.with(|v| v.set(0));
    assert!(cursor.expose(&c, &[]).is_none());
    prefix_publish(&c, &mut seed, &mut cursor);
    drop(cursor);
    drop(seed);
    assert!(status() == PrefixStatus::Vacant);
    WHNF_FUEL.with(|v| v.set(MAX_RECHECK_STEPS));
    WHNF_EXHAUSTED.with(|v| v.set(false));
    let hits = stats()[1];
    check(&c, 9, &bound, false);
    assert_eq!(stats()[1], hits);
}

#[test]
fn panic_unwinds_cursor_ticket_and_busy_slot_before_scope_cleanup() {
    let c = context();
    let _meter = metered();
    let scope = PrefixScope::enter(&c);
    let bound = beta(succs(8));
    check(&c, 1, &bound, true);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let (seed, cursor) = prefix_lookup(&c, &bound).unwrap();
        assert!(status() == PrefixStatus::Busy);
        let _held = (seed, cursor);
        panic!("intentional lease unwind");
    }));
    assert!(result.is_err());
    assert!(status() == PrefixStatus::Vacant);
    check(&c, 9, &bound, false);
    drop(scope);
    assert_eq!(live(), 0);
}

#[test]
fn abandoning_outer_lease_inside_inner_scope_debits_original_ledger() {
    let c = context();
    let clone = c.clone();
    let _meter = metered();
    let outer = PrefixScope::enter(&c);
    let bound = beta(succs(8));
    check(&c, 1, &bound, true);
    let before = live();
    let (seed, cursor) = prefix_lookup(&c, &bound).unwrap();
    let owned_bytes = cursor
        .as_ref()
        .unwrap()
        .prefix
        .as_ref()
        .unwrap()
        .memory
        .bytes;
    {
        let _inner = PrefixScope::enter(&clone);
        check(&clone, 1, &bound, true);
        drop(cursor);
        drop(seed);
        assert!(
            status() == PrefixStatus::Available,
            "cleanup cannot mutate inner generation"
        );
    }
    assert!(status() == PrefixStatus::Vacant);
    assert_eq!(
        live(),
        before - owned_bytes,
        "inactive outer release is applied on restore"
    );
    check(&c, 8, &bound, true);
    drop(outer);
    assert_eq!(live(), 0);
}

#[test]
fn nested_same_context_keeps_outer_busy_until_its_lease_completes() {
    let c = context();
    let _meter = metered();
    let outer = PrefixScope::enter(&c);
    let bound = beta(succs(8));
    check(&c, 1, &bound, true);
    let (seed, cursor) = prefix_lookup(&c, &bound).unwrap();
    let mut seed = Some(seed);
    let mut cursor = cursor.unwrap();
    let original_scope = seed.as_ref().unwrap().scope;
    {
        let _inner = PrefixScope::enter(&c);
        check(&c, 2, &bound, true);
        assert_ne!(PREFIX_TABLE.with(|v| v.borrow().scope), original_scope);
        prefix_publish(&c, &mut seed, &mut cursor);
        assert!(
            seed.as_ref().unwrap().lease.is_some(),
            "generation mismatch cannot publish"
        );
    }
    assert!(status() == PrefixStatus::Busy);
    prefix_publish(&c, &mut seed, &mut cursor);
    assert!(status() == PrefixStatus::Available);
    drop(cursor);
    drop(seed);
    drop(outer);
    assert_eq!(live(), 0);
}

#[test]
fn generation_exhaustion_suppresses_without_wrapping_or_reusing_outer_slot() {
    let c = context();
    let _meter = metered();
    let outer = PrefixScope::enter(&c);
    let bound = beta(succs(8));
    check(&c, 1, &bound, true);
    let saved = PREFIX_GENERATION.with(|v| v.replace(usize::MAX));
    {
        let _inner = PrefixScope::enter(&c);
        assert_eq!(PREFIX_TABLE.with(|v| v.borrow().scope), 0);
        assert!(prefix_lookup(&c, &bound).is_none());
        assert_eq!(PREFIX_GENERATION.with(|v| v.get()), usize::MAX);
    }
    assert!(status() == PrefixStatus::Available);
    drop(outer);
    assert_eq!(live(), 0);
    PREFIX_GENERATION.with(|v| v.set(saved)); // isolated test-only restoration
}

#[test]
fn moved_hit_has_no_capacity_growth_and_dirty_publication_respects_full_ledger() {
    let c = context();
    let _meter = metered();
    let scope = PrefixScope::enter(&c);
    let bound = beta(app(g("Succ"), beta(succs(8))));
    check(&c, 1, &bound, true);
    let before = live();
    let copies = stats()[2];
    let (seed, cursor) = prefix_lookup(&c, &bound).unwrap();
    let mut seed = Some(seed);
    let mut cursor = cursor.unwrap();
    assert_eq!(live(), before);
    assert_eq!(stats()[2], copies);
    cursor.count = 0.into();
    assert!(cursor.expose(&c, &[]).is_some());
    prefix_observe(&mut seed, &cursor.count);
    assert!(!cursor.prefix.as_ref().unwrap().clean);
    let full = PrefixTicket::new(PREFIX_LIVE_BYTES - live()).unwrap();
    prefix_publish(&c, &mut seed, &mut cursor);
    assert!(
        status() == PrefixStatus::Busy,
        "unreserved dirty tail is not published"
    );
    drop(full);
    drop(cursor);
    drop(seed);
    assert!(status() == PrefixStatus::Vacant);
    drop(scope);
    assert_eq!(live(), 0);
}

#[test]
fn fresh_borrowed_tail_abandonment_allocates_no_owned_cache_value() {
    let c = context();
    let _meter = metered();
    let scope = PrefixScope::enter(&c);
    let bound = succs(8);
    let before = live();
    let (seed, cached) = prefix_lookup(&c, &bound).unwrap();
    assert!(cached.is_none());
    let mut seed = Some(seed);
    let mut cursor = NatOrderCursor::new(&bound, false);
    cursor.expose(&c, &[]).unwrap();
    prefix_observe(&mut seed, &cursor.count);
    prefix_publish(&c, &mut seed, &mut cursor);
    assert!(PREFIX_TABLE.with(|v| v.borrow().entries.iter().all(Option::is_none)));
    drop(cursor);
    drop(seed);
    assert_eq!(live(), before);
    drop(scope);
    assert_eq!(live(), 0);
}

#[test]
fn dirty_open_or_oversized_residuals_refuse_actual_publication_admission() {
    let c = context();
    let _meter = metered();
    let scope = PrefixScope::enter(&c);
    let bound = beta(succs(8));
    check(&c, 1, &bound, true);
    for bad in [
        Db::Var(0),
        Db::MutualFix {
            defs: vec![n(0); PREFIX_NODES + 1],
            index: 0,
        },
    ] {
        let (seed, cached) = prefix_lookup(&c, &bound).unwrap();
        let mut seed = Some(seed);
        drop(cached); // actual original owned tail and ticket released
        let mut cursor = NatOrderCursor {
            count: 0.into(),
            tail: Some(NatOrderTail::Owned(bad)),
            prefix: None,
        };
        prefix_publish(&c, &mut seed, &mut cursor);
        assert!(status() == PrefixStatus::Busy);
        drop(cursor);
        drop(seed);
        assert!(status() == PrefixStatus::Vacant);
        check(&c, 1, &bound, true);
    }
    drop(scope);
    assert_eq!(live(), 0);
}

#[test]
fn whole_scope_unwind_destroys_leases_and_all_auxiliary_ownership() {
    let c = context();
    let _meter = metered();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _scope = PrefixScope::enter(&c);
        let bound = beta(succs(8));
        check(&c, 1, &bound, true);
        let _lease = prefix_lookup(&c, &bound).unwrap();
        panic!("intentional scope unwind");
    }));
    assert!(result.is_err());
    assert_eq!(live(), 0);
    assert_eq!(PREFIX_TABLE.with(|v| v.borrow().scope), 0);
}
#[test]
fn suppressed_scope_defers_outer_lease_release_instead_of_restoring_stale_bytes() {
    let c = context();
    let _meter = metered();
    let outer = PrefixScope::enter(&c);
    let bound = beta(succs(8));
    check(&c, 1, &bound, true);
    let (seed, cursor) = prefix_lookup(&c, &bound).unwrap();
    let owned_bytes = cursor
        .as_ref()
        .unwrap()
        .prefix
        .as_ref()
        .unwrap()
        .memory
        .bytes;
    let held = PrefixTicket::new(PREFIX_LIVE_BYTES - live()).unwrap();
    {
        let _suppressed = PrefixScope::enter(&c);
        assert_eq!(PREFIX_TABLE.with(|v| v.borrow().scope), 0);
        drop(cursor);
        drop(seed);
    }
    assert_eq!(live(), PREFIX_LIVE_BYTES - owned_bytes);
    assert!(status() == PrefixStatus::Vacant);
    drop(held);
    check(&c, 8, &bound, true);
    drop(outer);
    assert_eq!(live(), 0);
}
