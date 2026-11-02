//! Regression controls for optional definition-body lowering reuse.
//! Included as a child module of recheck; all cache access goes through the
//! production lookup seams, never through a proposed memo table's layout.
use super::*;

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn sort() -> Term {
    Term::Sort(Universe::Type(0))
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
fn dg(n: &str) -> Db {
    Db::Global(n.into())
}
fn base() -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", sort());
    c.add_constructor_checked("Zero", "Nat", g("Nat")).unwrap();
    c.add_constructor_checked("Succ", "Nat", pi("n", g("Nat"), g("Nat")))
        .unwrap();
    c.declare(
        "cache_id",
        pi("n", g("Nat"), g("Nat")),
        lam("n", g("Nat"), v("n")),
    )
    .unwrap();
    let u = Universe::Var("cache_u".into());
    c.add_universe_poly(
        "cache_sort",
        vec!["cache_u".into()],
        Term::Sort(Universe::Succ(Box::new(u.clone()))),
        Term::Sort(u),
    );
    assert!(
        c.get_universe_poly("cache_sort").is_some(),
        "checked polymorphic fixture"
    );
    c
}
fn identity() -> Db {
    Db::Lam(DbBox::new(dg("Nat")), DbBox::new(Db::Var(0)))
}
struct Meter {
    depth: usize,
    fuel: usize,
    exhausted: bool,
    lowerings: Option<usize>,
}
impl Drop for Meter {
    fn drop(&mut self) {
        WHNF_DEPTH.with(|s| s.set(self.depth));
        WHNF_FUEL.with(|s| s.set(self.fuel));
        WHNF_EXHAUSTED.with(|s| s.set(self.exhausted));
        DEFINITION_LOWERINGS.with(|s| s.set(self.lowerings));
    }
}
fn metered() -> Meter {
    Meter {
        depth: WHNF_DEPTH.with(|s| s.replace(1)),
        fuel: WHNF_FUEL.with(|s| s.replace(MAX_RECHECK_STEPS)),
        exhausted: WHNF_EXHAUSTED.with(|s| s.replace(false)),
        lowerings: DEFINITION_LOWERINGS.with(|s| s.replace(Some(0))),
    }
}
fn lowerings() -> usize {
    DEFINITION_LOWERINGS.with(|s| s.get().unwrap())
}
fn live() -> usize {
    PREFIX_MEMORY.with(|s| s.get().live)
}
fn exhaust_factory_before_first_lookup() {
    FIX_FACTORY.with(|s| {
        let mut f = s.get();
        assert_ne!(f.scope, 0, "test requires an admitted PrefixScope");
        // Mark it started as well: a lazy factory initializer must not replenish it.
        f.started = true;
        f.remaining = 0;
        s.set(f);
    });
}

#[test]
fn repeated_global_lowering_reuses_body_with_factory_empty_before_first_call() {
    let c = base();
    let _meter = metered();
    let _scope = PrefixScope::enter(&c);
    exhaust_factory_before_first_lookup();
    let scoped_live = live();
    for _ in 0..4 {
        assert_eq!(definition_body(&c, "cache_id"), Some(identity()));
    }
    assert_eq!(
        lowerings(),
        1,
        "four identical Global lookups should lower the body once"
    );
    assert!(
        live() > scoped_live,
        "retained body storage must be charged to the prefix ledger"
    );
    assert_eq!(FIX_FACTORY.with(|s| s.get().remaining), 0);
    assert!(!WHNF_EXHAUSTED.with(|s| s.get()));
}

#[test]
fn repeated_const_lowering_reuses_body_with_factory_empty_before_first_call() {
    let c = base();
    let _meter = metered();
    let _scope = PrefixScope::enter(&c);
    exhaust_factory_before_first_lookup();
    let scoped_live = live();
    for _ in 0..4 {
        assert_eq!(
            const_body(&c, "cache_sort", &[Universe::Type(1)]),
            Some(Db::Sort(Universe::Type(1)))
        );
    }
    assert_eq!(
        lowerings(),
        1,
        "four identical Const lookups should lower the body once"
    );
    assert!(
        live() > scoped_live,
        "retained body storage must be charged to the prefix ledger"
    );
    assert_eq!(FIX_FACTORY.with(|s| s.get().remaining), 0);
    assert!(!WHNF_EXHAUSTED.with(|s| s.get()));
}

#[test]
fn explicit_universe_instances_have_distinct_cached_results() {
    let c = base();
    let _meter = metered();
    let _scope = PrefixScope::enter(&c);
    for _ in 0..3 {
        for level in [0, 1] {
            assert_eq!(
                const_body(&c, "cache_sort", &[Universe::Type(level)]),
                Some(Db::Sort(Universe::Type(level)))
            );
        }
    }
    assert_eq!(
        lowerings(),
        2,
        "one lowering per exact explicit universe instance"
    );
}

#[test]
fn ordinary_global_and_explicit_const_do_not_share_instantiated_results() {
    let mut c = base();
    let u = Universe::Var("ordinary_u".into());
    c.declare(
        "ordinary_sort",
        Term::Sort(Universe::Succ(Box::new(u.clone()))),
        Term::Sort(u.clone()),
    )
    .unwrap();
    let _meter = metered();
    let _scope = PrefixScope::enter(&c);
    for _ in 0..2 {
        assert_eq!(
            definition_body(&c, "ordinary_sort"),
            Some(Db::Sort(u.clone()))
        );
        assert_eq!(
            const_body(&c, "ordinary_sort", &[Universe::Type(2)]),
            Some(Db::Sort(Universe::Type(2)))
        );
    }
}

#[test]
fn different_contexts_and_nested_scopes_preserve_their_own_definition_bodies() {
    let mut outer = base();
    outer.declare("chosen", g("Nat"), g("Zero")).unwrap();
    let mut inner = outer.clone();
    let successor = Term::App(Box::new(g("Succ")), Box::new(g("Zero")));
    inner.declare("chosen", g("Nat"), successor).unwrap();
    let expected_inner = Db::App(DbBox::new(dg("Succ")), DbBox::new(dg("Zero")));
    let _meter = metered();
    let before = live();
    {
        let _outer_scope = PrefixScope::enter(&outer);
        assert_eq!(definition_body(&outer, "chosen"), Some(dg("Zero")));
        let outer_live = live();
        // A mismatched Context must not accidentally consume the active cache entry.
        assert_eq!(
            definition_body(&inner, "chosen"),
            Some(expected_inner.clone())
        );
        assert_eq!(definition_body(&outer, "chosen"), Some(dg("Zero")));
        {
            let _inner_scope = PrefixScope::enter(&inner);
            for _ in 0..2 {
                assert_eq!(
                    definition_body(&inner, "chosen"),
                    Some(expected_inner.clone())
                );
            }
        }
        assert_eq!(
            live(),
            outer_live,
            "inner scope reservations must be released"
        );
        assert_eq!(definition_body(&outer, "chosen"), Some(dg("Zero")));
    }
    assert_eq!(live(), before);
}

#[test]
fn same_context_nested_scope_does_not_steal_or_destroy_outer_reuse() {
    let c = base();
    let _meter = metered();
    let _outer = PrefixScope::enter(&c);
    assert_eq!(definition_body(&c, "cache_id"), Some(identity()));
    let before = live();
    {
        let _inner = PrefixScope::enter(&c);
        assert_eq!(definition_body(&c, "cache_id"), Some(identity()));
        assert_eq!(definition_body(&c, "cache_id"), Some(identity()));
    }
    assert_eq!(live(), before);
    assert_eq!(definition_body(&c, "cache_id"), Some(identity()));
    assert_eq!(
        lowerings(),
        2,
        "one lowering in each scope, then the outer entry is restored"
    );
}

#[test]
fn malformed_constants_pending_definitions_and_open_bodies_are_not_reusable_results() {
    let mut c = base().extend("local", g("Nat"));
    c.declare("open_body", g("Nat"), v("local")).unwrap();
    assert!(
        c.get_definition_body("open_body").is_some(),
        "fixture has an open stored body"
    );
    c.declare("pending_body", g("Nat"), g("not_yet_declared"))
        .unwrap();
    assert!(
        c.get_definition_body("pending_body").is_none(),
        "pending body must stay hidden"
    );
    let _meter = metered();
    let _scope = PrefixScope::enter(&c);
    for _ in 0..2 {
        assert_eq!(definition_body(&c, "open_body"), None);
        assert_eq!(definition_body(&c, "pending_body"), None);
        assert_eq!(definition_body(&c, "missing_body"), None);
        assert_eq!(const_body(&c, "cache_sort", &[]), None);
        assert_eq!(
            const_body(&c, "cache_sort", &[Universe::Type(0), Universe::Type(1)]),
            None
        );
        assert_eq!(
            const_body(&c, "cache_id", &[]),
            None,
            "monomorphic Const remains invalid"
        );
    }
    assert_eq!(
        const_body(&c, "cache_sort", &[Universe::Type(0)]),
        Some(Db::Sort(Universe::Type(0))),
        "bad arity must not poison a valid lookup"
    );
}

#[test]
fn mutating_a_returned_owned_body_cannot_modify_the_next_lookup() {
    let c = base();
    let _meter = metered();
    let _scope = PrefixScope::enter(&c);
    let mut returned = definition_body(&c, "cache_id").unwrap();
    let Db::Lam(_, body) = &mut returned else {
        panic!("identity lambda");
    };
    *body = DbBox::new(dg("Zero"));
    assert_ne!(returned, identity());
    assert_eq!(definition_body(&c, "cache_id"), Some(identity()));
    let mut explicit = const_body(&c, "cache_sort", &[Universe::Type(1)]).unwrap();
    let Db::Sort(level) = &mut explicit else {
        panic!("sort");
    };
    *level = Universe::Prop;
    assert_eq!(
        const_body(&c, "cache_sort", &[Universe::Type(1)]),
        Some(Db::Sort(Universe::Type(1)))
    );
}

#[test]
fn optional_memo_misses_and_hits_preserve_low_fuel_without_exhaustion_poison() {
    let c = base();
    let _meter = metered();
    for warm in [false, true] {
        for allowance in [0, 1] {
            let _scope = PrefixScope::enter(&c);
            WHNF_FUEL.with(|s| s.set(MAX_RECHECK_STEPS));
            WHNF_EXHAUSTED.with(|s| s.set(false));
            if warm {
                assert_eq!(definition_body(&c, "cache_id"), Some(identity()));
                assert_eq!(
                    const_body(&c, "cache_sort", &[Universe::Type(0)]),
                    Some(Db::Sort(Universe::Type(0)))
                );
            }
            WHNF_FUEL.with(|s| s.set(allowance));
            assert_eq!(definition_body(&c, "cache_id"), Some(identity()));
            assert_eq!(
                const_body(&c, "cache_sort", &[Universe::Type(0)]),
                Some(Db::Sort(Universe::Type(0)))
            );
            assert!(
                WHNF_FUEL.with(|s| s.get()) <= allowance,
                "optional work cannot reset fuel"
            );
            assert!(
                !WHNF_EXHAUSTED.with(|s| s.get()),
                "optional admission must decline safely"
            );
        }
    }
}

#[test]
fn capacity_pressure_falls_back_without_leaking_tickets_or_poisoning_fuel() {
    let c = base();
    let _meter = metered();
    let before = live();
    {
        let _scope = PrefixScope::enter(&c);
        let scoped = live();
        let shared = FIX_SHARED_LIVE.with(|s| s.get());
        let pressure = PrefixTicket::new(PREFIX_LIVE_BYTES - scoped - shared).unwrap();
        for _ in 0..3 {
            assert_eq!(definition_body(&c, "cache_id"), Some(identity()));
        }
        assert_eq!(
            live() + FIX_SHARED_LIVE.with(|s| s.get()),
            PREFIX_LIVE_BYTES
        );
        assert!(!WHNF_EXHAUSTED.with(|s| s.get()));
        drop(pressure);
        assert_eq!(live(), scoped);
        assert_eq!(definition_body(&c, "cache_id"), Some(identity()));
        assert_eq!(definition_body(&c, "cache_id"), Some(identity()));
    }
    assert_eq!(live(), before);
}

#[test]
fn many_distinct_bodies_keep_turnover_bounded_and_release_the_scope_ledger() {
    let mut c = base();
    let mut body = v("n");
    for _ in 0..40 {
        body = Term::App(Box::new(g("Succ")), Box::new(body));
    }
    let body = lam("n", g("Nat"), body);
    for i in 0..96 {
        c.declare(
            &format!("turnover_{i}"),
            pi("n", g("Nat"), g("Nat")),
            body.clone(),
        )
        .unwrap();
    }
    let _meter = metered();
    let before = live();
    {
        let _scope = PrefixScope::enter(&c);
        let expected = to_db(&body, &mut Vec::new()).unwrap();
        for i in 0..96 {
            assert_eq!(
                definition_body(&c, &format!("turnover_{i}")),
                Some(expected.clone())
            );
            assert!(live() + FIX_SHARED_LIVE.with(|s| s.get()) <= PREFIX_LIVE_BYTES);
        }
        // May hit or re-lower after eviction/admission refusal; both must be exact.
        assert_eq!(definition_body(&c, "turnover_0"), Some(expected));
        assert!(PREFIX_MEMORY.with(|s| s.get().peak) <= PREFIX_LIVE_BYTES);
        assert!(!WHNF_EXHAUSTED.with(|s| s.get()));
    }
    assert_eq!(
        live(),
        before,
        "scope teardown releases retained and evicted entry tickets"
    );
}

#[test]
fn unwinding_a_scope_releases_definition_body_reservations() {
    let c = base();
    let _meter = metered();
    let before = live();
    let reached = std::cell::Cell::new(false);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _scope = PrefixScope::enter(&c);
        assert_eq!(definition_body(&c, "cache_id"), Some(identity()));
        assert_eq!(
            const_body(&c, "cache_sort", &[Universe::Type(1)]),
            Some(Db::Sort(Universe::Type(1)))
        );
        reached.set(true);
        panic!("intentional definition memo scope unwind");
    }));
    assert!(result.is_err());
    assert!(
        reached.get(),
        "the intended unwind must follow successful lookups"
    );
    assert_eq!(live(), before);
}
