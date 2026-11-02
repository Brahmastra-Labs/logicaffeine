//! Storage identity is optional; logical syntax and resource limits are not.
use super::*;

fn setup() {
    WHNF_DEPTH.with(|s| s.set(0));
    WHNF_FUEL.with(|s| s.set(MAX_RECHECK_STEPS));
    WHNF_EXHAUSTED.with(|s| s.set(false));
}
fn live() -> usize {
    FIX_SHARED_LIVE.with(|s| s.get())
}
fn owned(t: Db) -> DbBox {
    DbBox::owned(Box::new(t))
}
fn global(name: &str) -> Db {
    Db::Global(name.into())
}
fn app(a: Db, b: Db) -> Db {
    Db::App(owned(a), owned(b))
}
fn lam(a: Db, b: Db) -> Db {
    Db::Lam(owned(a), owned(b))
}

#[test]
fn open_shared_chunks_shift_and_substitute_without_capturing_binders() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    let syntax = lam(global("Nat"), app(Db::Var(1), Db::Var(0)));
    let edge = DbBox::new(syntax.clone());
    assert!(edge.is_shared());
    assert_eq!(edge.facts().unwrap().required, 1);
    assert_eq!(
        edge.shifted(1, 0).as_ref(),
        &lam(global("Nat"), app(Db::Var(2), Db::Var(0)))
    );
    assert_eq!(
        edge.opened(&Db::Var(3), 0).as_ref(),
        &lam(global("Nat"), app(Db::Var(4), Db::Var(0)))
    );
    let unaffected = edge.shifted(1, 1);
    assert!(std::ptr::eq(edge.as_ref(), unaffected.as_ref()));
    let unopened = edge.opened(&Db::Var(3), 1);
    assert!(std::ptr::eq(edge.as_ref(), unopened.as_ref()));
    assert_eq!(edge.as_ref(), &syntax);
}

#[test]
fn unique_mutable_access_demotes_before_exposing_syntax() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    let baseline = live();
    let mut edge = DbBox::new(global("before"));
    assert!(edge.is_shared());
    assert!(edge.facts().is_some());
    assert_eq!(live(), baseline + edge.retained_bytes());
    *edge = global("after");
    assert!(!edge.is_shared());
    assert!(edge.facts().is_none());
    assert_eq!(edge.retained_bytes(), 0);
    assert_eq!(edge.as_ref(), &global("after"));
    assert_eq!(live(), baseline);
}

#[test]
fn aliased_mutation_preserves_other_syntax_and_its_reservation() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    let baseline = live();
    let mut changed = DbBox::new(app(Db::Var(0), Db::Var(1)));
    let retained = changed.clone();
    let bytes = retained.retained_bytes();
    assert!(bytes > 0);
    *changed = Db::Var(7);
    assert!(changed.facts().is_none());
    assert!(!changed.is_shared());
    assert_eq!(retained.as_ref(), &app(Db::Var(0), Db::Var(1)));
    assert_eq!(retained.facts().unwrap().required, 2);
    assert_eq!(live(), baseline + bytes);
    drop(changed);
    assert_eq!(live(), baseline + bytes);
    drop(retained);
    assert_eq!(live(), baseline);
}

#[test]
fn owned_extraction_releases_only_the_extracted_allocation() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    let baseline = live();
    let child = DbBox::new(global("child"));
    let child_bytes = child.retained_bytes();
    let parent = DbBox::new(Db::App(child.clone(), child.clone()));
    let parent_bytes = parent.retained_bytes();
    assert!(child_bytes > 0 && parent_bytes > 0);
    let alias = parent.clone();
    let copied = parent.into_box();
    assert_eq!(live(), baseline + child_bytes + parent_bytes);
    let moved = alias.into_db();
    assert_eq!(*copied, moved);
    assert_eq!(live(), baseline + child_bytes);
    drop(child);
    drop(copied);
    assert_eq!(live(), baseline + child_bytes);
    drop(moved);
    assert_eq!(live(), baseline);
}

#[test]
fn exclusive_parent_bytes_exclude_shared_children_but_count_owned_children() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    let baseline = live();
    let child = DbBox::new(app(global("left"), global("right")));
    let child_bytes = child.retained_bytes();
    let parent = DbBox::new(Db::App(child.clone(), child.clone()));
    let scalar = DbBox::new(Db::Var(0));
    assert!(parent.is_shared() && scalar.is_shared());
    // Both allocations have only their inline root and Rc header exclusively.
    assert_eq!(parent.retained_bytes(), scalar.retained_bytes());
    let copied = DbBox::new(app(
        app(global("left"), global("right")),
        app(global("left"), global("right")),
    ));
    assert!(copied.is_shared());
    assert!(copied.retained_bytes() > parent.retained_bytes());
    assert_eq!(
        live(),
        baseline
            + child_bytes
            + parent.retained_bytes()
            + scalar.retained_bytes()
            + copied.retained_bytes()
    );
    assert_eq!(parent.as_ref(), copied.as_ref());
}

#[test]
fn logical_metrics_count_every_shared_occurrence_and_binder_depth() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    let child = DbBox::new(app(Db::Var(0), Db::Var(1)));
    let parent = DbBox::new(Db::App(child.clone(), child.clone()));
    let facts = parent.facts().unwrap();
    assert_eq!(facts.metrics.nodes, 7);
    assert_eq!(facts.metrics.depth, 3);
    assert_eq!(facts.required, 2);
    assert!(!facts.metrics.closed);
    let bound = DbBox::new(Db::Lam(owned(global("Nat")), parent));
    let facts = bound.facts().unwrap();
    assert_eq!(facts.metrics.nodes, 9);
    assert_eq!(facts.metrics.depth, 4);
    assert_eq!(facts.required, 1);
}

#[test]
fn escaped_child_keeps_its_ticket_after_parent_and_scope_drop() {
    setup();
    let c = Context::new();
    let baseline = live();
    let escaped = {
        let _scope = PrefixScope::enter(&c);
        assert!(fix_factory_begin());
        let child = DbBox::new(app(Db::Var(0), Db::Var(1)));
        let parent = DbBox::new(Db::App(child.clone(), child));
        let Db::App(a, _) = parent.as_ref() else {
            panic!("application")
        };
        let escaped = a.clone();
        let bytes = escaped.retained_bytes();
        drop(parent);
        assert_eq!(live(), baseline + bytes);
        escaped
    };
    let bytes = escaped.retained_bytes();
    assert!(bytes > 0);
    assert!(escaped.facts().is_none());
    assert_eq!(live(), baseline + bytes);
    {
        let _scope = PrefixScope::enter(&c);
        assert!(fix_factory_begin());
        assert!(escaped.facts().is_none());
        let prefix = PREFIX_MEMORY.with(|s| s.get().live);
        let fill = PrefixTicket::new(PREFIX_LIVE_BYTES - live() - prefix).unwrap();
        assert!(PrefixTicket::new(1).is_none());
        let refused = DbBox::new(Db::Var(0));
        assert!(!refused.is_shared());
        assert_eq!(live(), baseline + bytes);
        drop(fill);
    }
    drop(escaped);
    assert_eq!(live(), baseline);
}

#[test]
fn nested_scope_restores_factory_but_not_escaped_inner_tickets() {
    setup();
    let c = Context::new();
    let baseline = live();
    let _outer = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    let outer = DbBox::new(Db::Var(0));
    let remaining = FIX_FACTORY.with(|s| s.get().remaining);
    let inner = {
        let _inner = PrefixScope::enter(&c);
        assert!(fix_factory_begin());
        assert!(outer.facts().is_none());
        DbBox::new(Db::Var(1))
    };
    assert!(outer.facts().is_some());
    assert!(inner.is_shared());
    assert!(inner.facts().is_none());
    assert_eq!(FIX_FACTORY.with(|s| s.get().remaining), remaining);
    assert_eq!(
        live(),
        baseline + outer.retained_bytes() + inner.retained_bytes()
    );
    drop(inner);
    assert_eq!(live(), baseline + outer.retained_bytes());
    drop(outer);
    assert_eq!(live(), baseline);
}

#[test]
fn unwinding_scope_keeps_escaped_allocation_until_last_owner_drop() {
    setup();
    let c = Context::new();
    let baseline = live();
    let prefix_before = PREFIX_MEMORY.with(|s| s.get().live);
    let mut escaped = None;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _scope = PrefixScope::enter(&c);
        assert!(fix_factory_begin());
        escaped = Some(DbBox::new(app(Db::Var(0), Db::Var(1))));
        panic!("exercise immutable storage scope unwinding");
    }));
    assert!(result.is_err());
    let escaped = escaped.unwrap();
    let bytes = escaped.retained_bytes();
    assert!(bytes > 0);
    assert!(escaped.facts().is_none());
    assert_eq!(PREFIX_MEMORY.with(|s| s.get().live), prefix_before);
    let alias = escaped.clone();
    drop(escaped);
    assert_eq!(live(), baseline + bytes);
    drop(alias);
    assert_eq!(live(), baseline);
}

#[test]
fn excessive_logical_nodes_and_depth_cannot_be_admitted() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    let baseline = live();
    let wide = Db::Match {
        disc: owned(Db::Var(0)),
        motive: owned(Db::Var(0)),
        cases: vec![Db::Var(0); PREFIX_NODES],
    };
    assert!(!DbBox::new(wide).is_shared());
    let mut deep = Db::Var(0);
    for _ in 0..PREFIX_DEPTH {
        deep = lam(Db::Var(0), deep);
    }
    assert!(!DbBox::new(deep).is_shared());
    assert_eq!(live(), baseline);
}

#[test]
fn oversized_payload_capacity_and_universe_depth_are_refused() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    let baseline = live();
    assert!(!DbBox::new(Db::Lit(Literal::Text("x".repeat(257)))).is_shared());
    let mut capacity = String::with_capacity(PREFIX_TREE_BYTES + 1);
    capacity.push('x');
    assert!(!DbBox::new(Db::Global(capacity)).is_shared());
    let mut level = Universe::Type(0);
    for _ in 0..PREFIX_DEPTH {
        level = Universe::Succ(Box::new(level));
    }
    assert!(!DbBox::new(Db::Sort(level)).is_shared());
    assert!(!DbBox::new(Db::Const {
        name: "U".into(),
        levels: vec![Universe::Type(0); PREFIX_NODES + 1],
    })
    .is_shared());
    assert_eq!(live(), baseline);
}

#[test]
fn absent_factory_and_exhausted_budgets_preserve_owned_fallback() {
    setup();
    let baseline = live();
    assert!(!DbBox::new(Db::Var(0)).is_shared());
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(!DbBox::new(Db::Var(0)).is_shared());
    assert!(fix_factory_begin());
    WHNF_DEPTH.with(|s| s.set(1));
    WHNF_FUEL.with(|s| s.set(0));
    let no_fuel = DbBox::new(Db::Var(0));
    let exhausted = WHNF_EXHAUSTED.with(|s| s.get());
    WHNF_DEPTH.with(|s| s.set(0));
    assert!(!no_fuel.is_shared());
    assert!(
        !exhausted,
        "optional admission must not poison ordinary fallback"
    );
    FIX_FACTORY.with(|s| {
        let mut f = s.get();
        f.remaining = 0;
        s.set(f);
    });
    let spent = FIX_FACTORY.with(|s| s.get().spent);
    assert!(!DbBox::new(Db::Var(0)).is_shared());
    assert!(!fix_factory_begin());
    assert_eq!(FIX_FACTORY.with(|s| s.get().spent), spent);
    assert_eq!(live(), baseline);
}

#[test]
fn float_storage_keeps_underlying_structural_equality_semantics() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    let baseline = live();
    for value in [f64::NAN, 0.0, -0.0, f64::INFINITY] {
        let syntax = Db::Lit(Literal::Float(value));
        let expected = syntax == syntax.clone();
        let edge = DbBox::new(syntax);
        assert!(!edge.is_shared());
        assert!(edge.facts().is_none());
        assert_eq!(edge == edge.clone(), expected);
    }
    let ordinary = DbBox::new(app(Db::Var(0), Db::Var(1)));
    let separate = DbBox::new(app(Db::Var(0), Db::Var(1)));
    let other = DbBox::new(app(Db::Var(0), Db::Var(2)));
    assert_eq!(ordinary == separate, ordinary.as_ref() == separate.as_ref());
    assert_eq!(ordinary == other, ordinary.as_ref() == other.as_ref());
    drop(ordinary);
    drop(separate);
    drop(other);
    assert_eq!(live(), baseline);
}

#[test]
fn prefix_copy_promotion_keeps_new_ticket_after_copy_reservation_drops() {
    setup();
    let c = Context::new();
    let baseline = live();
    let escaped = {
        let _scope = PrefixScope::enter(&c);
        assert!(fix_factory_begin());
        let child = DbBox::new(app(Db::Var(0), Db::Var(1)));
        assert!(child.is_shared());
        let source = Db::App(child.clone(), child);
        let source_bytes = live() - baseline;
        let prefix_before = PREFIX_MEMORY.with(|s| s.get().live);
        let (copy, reservation) = prefix_copy(&source).unwrap();
        let Db::App(a, b) = &copy else {
            panic!("application")
        };
        assert!(!a.is_shared() && !b.is_shared());
        assert_eq!(copy, source);
        assert_eq!(live(), baseline + source_bytes);
        assert!(PREFIX_MEMORY.with(|s| s.get().live) > prefix_before);
        let promoted = DbBox::new(copy);
        assert!(promoted.is_shared());
        let promoted_bytes = promoted.retained_bytes();
        assert_eq!(live(), baseline + source_bytes + promoted_bytes);
        drop(reservation);
        assert_eq!(PREFIX_MEMORY.with(|s| s.get().live), prefix_before);
        assert_eq!(live(), baseline + source_bytes + promoted_bytes);
        drop(source);
        assert_eq!(live(), baseline + promoted_bytes);
        assert_eq!(
            promoted.as_ref(),
            &app(app(Db::Var(0), Db::Var(1)), app(Db::Var(0), Db::Var(1)))
        );
        promoted
    };
    assert!(escaped.facts().is_none());
    assert_eq!(live(), baseline + escaped.retained_bytes());
    drop(escaped);
    assert_eq!(live(), baseline);
}

#[test]
fn recursive_body_reservation_excludes_independently_ticketed_children() {
    setup();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    let child = DbBox::new(app(Db::Var(0), Db::Var(1)));
    let with_shared = FixBody::new(Db::Lam(owned(global("Nat")), child));
    let with_owned = FixBody::new(lam(global("Nat"), Db::Var(0)));
    let (FixBody::Shared(a), FixBody::Shared(b)) = (&with_shared, &with_owned) else {
        panic!("both bounded recursive bodies must be admitted")
    };
    assert!(
        a._memory.bytes < b._memory.bytes,
        "independently ticketed child storage must not be reserved twice"
    );
}
