//! Structural recursion may demand the middle argument without demoting its
//! undemanded neighbors. Tests use the unchanged demand/resume public seam.
use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn owned(t: Db) -> DbBox {
    DbBox::owned(Box::new(t))
}
fn app(f: Db, a: Db) -> Db {
    Db::App(owned(f), owned(a))
}
fn lam(d: Db, b: Db) -> Db {
    Db::Lam(owned(d), owned(b))
}
fn pi(d: Db, b: Db) -> Db {
    Db::Pi(owned(d), owned(b))
}
fn function_type() -> Db {
    pi(g("Nat"), g("Nat"))
}
fn mat(n: Db, zero: Db, succ: Db) -> Db {
    Db::Match {
        disc: owned(n),
        motive: owned(g("Nat")),
        cases: vec![zero, succ],
    }
}
fn payload() -> Db {
    // fix rec. fun n => match n with 0 => 0 | Succ k => Succ (rec k).
    Db::Fix(FixBody::owned(Box::new(lam(
        g("Nat"),
        mat(
            Db::Var(0),
            g("Zero"),
            lam(g("Nat"), app(g("Succ"), app(Db::Var(2), Db::Var(0)))),
        ),
    ))))
}
fn side(which: usize) -> Db {
    if which == 0 {
        payload()
    } else {
        app(g("function_identity"), payload())
    }
}
fn middle() -> Db {
    // fix rec. fun before n after => match n with
    // 0 => before (after 0) | Succ k => rec before k after.
    let step = lam(
        g("Nat"),
        app(app(app(Db::Var(4), Db::Var(3)), Db::Var(0)), Db::Var(1)),
    );
    Db::Fix(FixBody::owned(Box::new(lam(
        function_type(),
        lam(
            g("Nat"),
            lam(
                function_type(),
                mat(
                    Db::Var(1),
                    app(Db::Var(2), app(Db::Var(0), g("Zero"))),
                    step,
                ),
            ),
        ),
    ))))
}
fn full(before: DbBox, selected: Db, after: DbBox) -> Db {
    Db::App(
        owned(Db::App(
            owned(Db::App(owned(middle()), before)),
            owned(selected),
        )),
        after,
    )
}
fn arguments(t: &Db) -> [&DbBox; 3] {
    let Db::App(f, c) = t else {
        panic!("three arguments")
    };
    let Db::App(f, b) = f.as_ref() else {
        panic!("second argument")
    };
    let Db::App(_, a) = f.as_ref() else {
        panic!("first argument")
    };
    [a, b, c]
}
fn context() -> Context {
    let mut ctx = Context::new();
    ctx.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    ctx.add_constructor("Zero", "Nat", Term::Global("Nat".into()));
    ctx.add_constructor(
        "Succ",
        "Nat",
        from_db_with_external(&function_type(), 0, &[]),
    );
    ctx.add_declaration("neutral", Term::Global("Nat".into()));
    ctx.add_declaration(
        "neutral2",
        from_db_with_external(&pi(g("Nat"), pi(g("Nat"), g("Nat"))), 0, &[]),
    );
    let function_id = lam(function_type(), Db::Var(0));
    ctx.declare(
        "function_identity",
        from_db_with_external(&pi(function_type(), function_type()), 0, &[]),
        from_db_with_external(&function_id, 0, &[]),
    )
    .unwrap();
    for (name, ty, body) in [
        ("side_checked", function_type(), payload()),
        (
            "middle_checked",
            pi(function_type(), pi(g("Nat"), pi(function_type(), g("Nat")))),
            middle(),
        ),
    ] {
        let body = from_db_with_external(&body, 0, &[]);
        ctx.declare(name, from_db_with_external(&ty, 0, &[]), body.clone())
            .unwrap();
        assert!(
            recheck(&ctx, &body).is_ok(),
            "fixture must also be accepted independently"
        );
    }
    ctx
}
fn demand(ctx: &Context, term: Db) -> (Db, WhnfFrame) {
    assert_eq!(
        fix_structural_column(ctx, &term),
        Some(1),
        "checked recursion uses the middle column"
    );
    let WhnfStep::Demand { term, frame } = demand_fix_argument(ctx, term) else {
        panic!("structural demand")
    };
    assert!(matches!(&frame, WhnfFrame::FixArgument { .. }));
    (term, frame)
}
fn live() -> usize {
    FIX_SHARED_LIVE.with(|s| s.get())
}
struct Reset {
    depth: usize,
    fuel: usize,
    exhausted: bool,
    local: usize,
    clones: Option<usize>,
}
impl Drop for Reset {
    fn drop(&mut self) {
        WHNF_DEPTH.with(|s| s.set(self.depth));
        WHNF_FUEL.with(|s| s.set(self.fuel));
        WHNF_EXHAUSTED.with(|s| s.set(self.exhausted));
        LOCAL_CONTEXT_BUDGET_DEPTH.with(|s| s.set(self.local));
        OWNED_FIX_CLONES.with(|s| s.set(self.clones));
    }
}
fn reset() -> Reset {
    Reset {
        depth: WHNF_DEPTH.with(|s| s.replace(0)),
        fuel: WHNF_FUEL.with(|s| s.replace(MAX_RECHECK_STEPS)),
        exhausted: WHNF_EXHAUSTED.with(|s| s.replace(false)),
        local: LOCAL_CONTEXT_BUDGET_DEPTH.with(|s| s.replace(0)),
        clones: OWNED_FIX_CLONES.with(|s| s.replace(None)),
    }
}
fn copies<T>(f: impl FnOnce() -> T) -> (T, usize) {
    OWNED_FIX_CLONES.with(|s| s.set(Some(0)));
    let result = f();
    (result, OWNED_FIX_CLONES.with(|s| s.replace(None).unwrap()))
}
fn expired_edges(ctx: &Context) -> (DbBox, DbBox) {
    let _scope = PrefixScope::enter(ctx);
    assert!(fix_factory_begin());
    let a = DbBox::new(side(0));
    let b = DbBox::new(side(1));
    assert!(a.is_shared() && b.is_shared());
    (a, b)
}

#[test]
fn fix_middle_demand_preserves_active_aliased_edges_and_copies_no_owned_fix() {
    let _reset = reset();
    let ctx = context();
    let baseline = live();
    {
        let _scope = PrefixScope::enter(&ctx);
        assert!(fix_factory_begin());
        let before = DbBox::new(side(0));
        let after = DbBox::new(side(1));
        assert!(before.is_shared() && after.is_shared());
        let (a, b) = (before.clone(), after.clone());
        let input = full(before, g("neutral"), after);
        let ((selected, frame), count) = copies(|| demand(&ctx, input));
        assert_eq!(selected, g("neutral"));
        assert_eq!(
            count, 0,
            "scheduling one argument must not clone either undemanded Owned Fix"
        );
        let mut roots = Vec::new();
        assert_eq!(frame.size_roots(&mut roots), 3);
        assert_eq!(roots.len(), 3);
        assert!(std::ptr::eq(roots[1], a.as_ref()));
        assert!(std::ptr::eq(roots[2], b.as_ref()));
        drop(roots);
        let rebuilt = frame.rebuild(selected);
        let args = arguments(&rebuilt);
        assert!(std::ptr::eq(args[0].as_ref(), a.as_ref()));
        assert!(std::ptr::eq(args[2].as_ref(), b.as_ref()));
        assert_eq!(args[0].as_ref(), &side(0));
        assert_eq!(args[2].as_ref(), &side(1));
        assert!(!take_budget_exhausted());
    }
    assert_eq!(live(), baseline);
}

#[test]
fn fix_neutral_resume_preserves_both_side_edges_in_source_order() {
    let _reset = reset();
    let ctx = context();
    let _scope = PrefixScope::enter(&ctx);
    assert!(fix_factory_begin());
    let before = DbBox::new(side(0));
    let after = DbBox::new(side(1));
    let (a, b) = (before.clone(), after.clone());
    let input = full(before, g("neutral"), after);
    let (result, count) = copies(|| {
        let (selected, frame) = demand(&ctx, input);
        let WhnfStep::Ready(result) = resume_whnf(&ctx, &[], frame, selected) else {
            panic!("neutral argument keeps Fix folded")
        };
        result
    });
    assert_eq!(count, 0);
    assert_eq!(result, full(owned(side(0)), g("neutral"), owned(side(1))));
    let args = arguments(&result);
    assert!(std::ptr::eq(args[0].as_ref(), a.as_ref()));
    assert!(std::ptr::eq(args[2].as_ref(), b.as_ref()));
}

#[test]
fn fix_whnf_keeps_expired_side_tickets_until_the_last_owner_drops() {
    let _reset = reset();
    let ctx = context();
    let baseline = live();
    let (before, after) = expired_edges(&ctx);
    assert!(before.facts().is_none() && after.facts().is_none());
    let bytes = before.retained_bytes() + after.retained_bytes();
    let (a, b) = (before.clone(), after.clone());
    assert_eq!(live(), baseline + bytes);
    let input = full(before, g("neutral"), after);
    let (result, count) = copies(|| whnf_in_owned(&ctx, &[], input));
    assert_eq!(result, full(owned(side(0)), g("neutral"), owned(side(1))));
    assert_eq!(count, 0);
    let args = arguments(&result);
    assert!(std::ptr::eq(args[0].as_ref(), a.as_ref()));
    assert!(std::ptr::eq(args[2].as_ref(), b.as_ref()));
    assert!(args[0].is_shared() && args[2].is_shared());
    drop(a);
    drop(b);
    assert_eq!(
        live(),
        baseline + bytes,
        "returned expression retains both persistent tickets"
    );
    drop(result);
    assert_eq!(live(), baseline);
}

#[test]
fn fix_undemanded_owned_edges_stay_owned_at_their_original_allocations() {
    let _reset = reset();
    let ctx = context();
    let _scope = PrefixScope::enter(&ctx);
    assert!(fix_factory_begin());
    let before = owned(side(0));
    let after = owned(side(1));
    let pointers = (before.as_ref() as *const Db, after.as_ref() as *const Db);
    let (selected, frame) = demand(&ctx, full(before, g("neutral"), after));
    let WhnfStep::Ready(result) = resume_whnf(&ctx, &[], frame, selected) else {
        panic!("neutral")
    };
    let args = arguments(&result);
    assert!(
        matches!(args[0], DbBox::Owned(_)) && matches!(args[2], DbBox::Owned(_)),
        "unchanged side edges must not request fresh admission"
    );
    assert_eq!(
        (args[0].as_ref() as *const Db, args[2].as_ref() as *const Db),
        pointers
    );
}

#[test]
fn fix_constructor_resume_preserves_side_edges_before_beta_consumption() {
    let _reset = reset();
    let ctx = context();
    let (before, after) = expired_edges(&ctx);
    let (a, b) = (before.clone(), after.clone());
    let selected = app(lam(g("Nat"), Db::Var(0)), g("Zero"));
    let input = full(before, selected, after);
    let ((selected, frame), count) = copies(|| demand(&ctx, input));
    assert_eq!(count, 0);
    let constructor = whnf_in_owned(&ctx, &[], selected);
    assert_eq!(constructor, g("Zero"));
    let WhnfStep::Reduce(result) = resume_whnf(&ctx, &[], frame, constructor) else {
        panic!("constructor permits unfolding")
    };
    let args = arguments(&result);
    assert!(std::ptr::eq(args[0].as_ref(), a.as_ref()));
    assert!(std::ptr::eq(args[2].as_ref(), b.as_ref()));
    assert_eq!(args[1].as_ref(), &g("Zero"));
    assert_eq!(
        whnf_in_owned(&ctx, &[], result),
        g("Zero"),
        "checked functions and selected branch still compute correctly"
    );
    assert_eq!(a.as_ref(), &side(0));
    assert_eq!(b.as_ref(), &side(1));
    assert!(!take_budget_exhausted());
}

#[test]
fn fix_fuel_and_depth_refusal_reconstruct_exact_undemanded_edges() {
    let _reset = reset();
    let ctx = context();
    for depth_refusal in [false, true] {
        WHNF_DEPTH.with(|s| s.set(0));
        LOCAL_CONTEXT_BUDGET_DEPTH.with(|s| s.set(0));
        WHNF_FUEL.with(|s| s.set(MAX_RECHECK_STEPS));
        WHNF_EXHAUSTED.with(|s| s.set(false));
        let (before, after) = expired_edges(&ctx);
        let (a, b) = (before.clone(), after.clone());
        let selected = if depth_refusal {
            app(app(g("neutral2"), g("Zero")), g("Zero"))
        } else {
            g("neutral")
        };
        let expected = full(owned(side(0)), selected.clone(), owned(side(1)));
        let input = full(before, selected, after);
        LOCAL_CONTEXT_BUDGET_DEPTH.with(|s| s.set(1));
        let allowance = if depth_refusal { 100 } else { 5 };
        WHNF_DEPTH.with(|s| {
            s.set(if depth_refusal {
                MAX_RECHECK_WHNF_DEPTH - 4
            } else {
                0
            })
        });
        WHNF_FUEL.with(|s| s.set(allowance));
        let (result, count) = copies(|| whnf_in_owned(&ctx, &[], input));
        assert!(take_budget_exhausted());
        assert!(WHNF_FUEL.with(|s| s.get()) < allowance);
        assert_eq!(
            result, expected,
            "resource refusal returns every argument in its original order"
        );
        assert_eq!(count, 0);
        let args = arguments(&result);
        assert!(std::ptr::eq(args[0].as_ref(), a.as_ref()));
        assert!(std::ptr::eq(args[2].as_ref(), b.as_ref()));
    }
}

#[test]
fn fix_pending_logical_size_counts_repeated_shared_side_occurrences() {
    let _reset = reset();
    let ctx = context();
    let _scope = PrefixScope::enter(&ctx);
    assert!(fix_factory_begin());
    let edge = DbBox::new(payload());
    assert!(edge.is_shared());
    let alias = edge.clone();
    let expected = full(owned(payload()), g("neutral"), owned(payload()));
    let logical = db_node_count(&expected);
    let (selected, frame) = demand(&ctx, full(edge, g("neutral"), alias));
    let pending = [frame];
    assert!(!whnf_size_exceeds(&selected, &pending, logical));
    assert!(
        whnf_size_exceeds(&selected, &pending, logical - 1),
        "sharing cannot discount a logical occurrence or application node"
    );
}
