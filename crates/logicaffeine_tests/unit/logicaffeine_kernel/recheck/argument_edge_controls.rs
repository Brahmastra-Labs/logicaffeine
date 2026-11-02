//! Suspension must preserve argument ownership until a reduction consumes it.
use super::*;

fn owned(t: Db) -> DbBox {
    DbBox::owned(Box::new(t))
}
fn g(name: &str) -> Db {
    Db::Global(name.into())
}
fn app(a: Db, b: Db) -> Db {
    Db::App(owned(a), owned(b))
}
fn lam(a: Db, b: Db) -> Db {
    Db::Lam(owned(a), owned(b))
}
fn payload() -> Db {
    Db::Fix(FixBody::owned(Box::new(lam(g("A"), Db::Var(0)))))
}
fn live() -> usize {
    FIX_SHARED_LIVE.with(|s| s.get())
}
fn spent() -> usize {
    FIX_FACTORY.with(|s| s.get().spent)
}
fn argument(t: &Db) -> &DbBox {
    let Db::App(_, a) = t else {
        panic!("application")
    };
    a
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
    let copies = OWNED_FIX_CLONES.with(|s| s.replace(None).unwrap());
    (result, copies)
}

#[test]
fn neutral_owned_whnf_retains_active_argument_edge_without_cloning_its_fix() {
    let _reset = reset();
    let ctx = Context::new();
    let baseline = live();
    {
        let _scope = PrefixScope::enter(&ctx);
        assert!(fix_factory_begin());
        let edge = DbBox::new(payload());
        assert!(edge.is_shared());
        let alias = edge.clone();
        let expected = Db::App(owned(g("neutral")), alias.clone());
        let (result, count) =
            copies(|| whnf_in_owned(&ctx, &[], Db::App(owned(g("neutral")), edge)));
        assert_eq!(result, expected);
        assert!(std::ptr::eq(argument(&result).as_ref(), alias.as_ref()));
        assert_eq!(
            count, 0,
            "an unevaluated shared argument needs no owned Fix copy"
        );
        assert!(live() >= baseline + alias.retained_bytes());
        assert!(!take_budget_exhausted());
    }
    assert_eq!(live(), baseline);
}

#[test]
fn neutral_owned_whnf_retains_expired_argument_edge_and_its_persistent_ticket() {
    let _reset = reset();
    let ctx = Context::new();
    let baseline = live();
    let edge = {
        let _scope = PrefixScope::enter(&ctx);
        assert!(fix_factory_begin());
        DbBox::new(payload())
    };
    assert!(edge.is_shared() && edge.facts().is_none());
    let bytes = edge.retained_bytes();
    let alias = edge.clone();
    let expected = payload();
    let (result, count) = copies(|| whnf_in_owned(&ctx, &[], Db::App(owned(g("neutral")), edge)));
    assert_eq!(argument(&result).as_ref(), &expected);
    assert!(std::ptr::eq(argument(&result).as_ref(), alias.as_ref()));
    assert!(argument(&result).is_shared());
    assert_eq!(count, 0);
    drop(alias);
    assert_eq!(
        live(),
        baseline + bytes,
        "the result keeps the original edge's ticket alive"
    );
    drop(result);
    assert_eq!(live(), baseline);
}

#[test]
fn suspended_argument_and_explicit_rebuild_preserve_the_original_edge() {
    let _reset = reset();
    let ctx = Context::new();
    let _scope = PrefixScope::enter(&ctx);
    assert!(fix_factory_begin());
    let edge = DbBox::new(payload());
    assert!(edge.is_shared());
    let alias = edge.clone();
    let before = spent();
    let (step, count) = copies(|| whnf_step(&ctx, &[], Db::App(owned(g("neutral")), edge)));
    let WhnfStep::Demand { term, frame } = step else {
        panic!("head demand")
    };
    assert_eq!(term, g("neutral"));
    assert_eq!(
        spent(),
        before,
        "suspending an argument must not re-admit it"
    );
    assert_eq!(count, 0);
    {
        let mut roots = Vec::new();
        assert_eq!(frame.size_roots(&mut roots), 1);
        assert_eq!(roots.len(), 1);
        assert!(std::ptr::eq(roots[0], alias.as_ref()));
    }
    let rebuilt = frame.rebuild(term);
    assert!(std::ptr::eq(argument(&rebuilt).as_ref(), alias.as_ref()));
}

#[test]
fn fuel_and_logical_depth_refusal_rebuild_all_original_argument_storage() {
    let _reset = reset();
    let ctx = Context::new();
    for depth_refusal in [false, true] {
        WHNF_DEPTH.with(|s| s.set(0));
        LOCAL_CONTEXT_BUDGET_DEPTH.with(|s| s.set(0));
        WHNF_FUEL.with(|s| s.set(MAX_RECHECK_STEPS));
        WHNF_EXHAUSTED.with(|s| s.set(false));
        let edge = {
            let _scope = PrefixScope::enter(&ctx);
            assert!(fix_factory_begin());
            DbBox::new(payload())
        };
        assert!(edge.is_shared());
        let alias = edge.clone();
        let expected = Db::App(owned(g("neutral")), alias.clone());
        LOCAL_CONTEXT_BUDGET_DEPTH.with(|s| s.set(1));
        WHNF_DEPTH.with(|s| {
            s.set(if depth_refusal {
                MAX_RECHECK_WHNF_DEPTH - 1
            } else {
                0
            })
        });
        WHNF_FUEL.with(|s| s.set(1));
        WHNF_EXHAUSTED.with(|s| s.set(false));
        let (result, count) =
            copies(|| whnf_in_owned(&ctx, &[], Db::App(owned(g("neutral")), edge)));
        assert_eq!(result, expected);
        assert!(take_budget_exhausted());
        assert_eq!(
            WHNF_FUEL.with(|s| s.get()),
            0,
            "the original one-step allowance is consumed"
        );
        assert!(std::ptr::eq(argument(&result).as_ref(), alias.as_ref()));
        assert_eq!(count, 0);
    }
}

#[test]
fn pending_argument_logical_nodes_are_not_discounted_by_sharing() {
    let _reset = reset();
    let ctx = Context::new();
    let _scope = PrefixScope::enter(&ctx);
    assert!(fix_factory_begin());
    let edge = DbBox::new(app(g("a"), g("b"))); // Three logical nodes.
    assert!(edge.is_shared());
    let alias = edge.clone();
    let WhnfStep::Demand { term, frame } = whnf_step(&ctx, &[], Db::App(owned(g("neutral")), edge))
    else {
        panic!("head demand")
    };
    let pending = [frame];
    assert!(!whnf_size_exceeds(&term, &pending, 5));
    assert!(whnf_size_exceeds(&term, &pending, 4));
    assert_eq!(alias.as_ref(), &app(g("a"), g("b")));
}

#[test]
fn neutral_owned_argument_stays_owned_instead_of_requesting_new_admission() {
    let _reset = reset();
    let ctx = Context::new();
    let _scope = PrefixScope::enter(&ctx);
    assert!(fix_factory_begin());
    let arg = payload();
    let expected = arg.clone();
    let result = whnf_in_owned(&ctx, &[], Db::App(owned(g("neutral")), owned(arg)));
    assert_eq!(argument(&result).as_ref(), &expected);
    assert!(
        matches!(argument(&result), DbBox::Owned(_)),
        "unchanged argument ownership must be retained"
    );
}

#[test]
fn actual_lambda_consumption_moves_owned_storage_and_rebases_captured_indices() {
    let _reset = reset();
    let ctx = Context::new();
    for nested in [false, true] {
        let arg = app(g(&"owned_payload".repeat(30)), Db::Var(3));
        let Db::App(a, b) = &arg else { unreachable!() };
        let ptr = (a.as_ref() as *const Db, b.as_ref() as *const Db);
        let body = if nested {
            lam(g("A"), Db::Var(1))
        } else {
            Db::Var(0)
        };
        let expected = beta_open(&body, &arg);
        let term = Db::App(owned(lam(g("A"), body)), owned(arg));
        let WhnfStep::Demand { term, frame } = whnf_step(&ctx, &[], term) else {
            panic!("head demand")
        };
        let (step, count) = copies(|| resume_whnf(&ctx, &[], frame, term));
        let WhnfStep::Reduce(result) = step else {
            panic!("actual beta reduction")
        };
        assert_eq!(result, expected);
        let inner = if nested {
            let Db::Lam(_, b) = &result else {
                panic!("lambda")
            };
            b.as_ref()
        } else {
            &result
        };
        let Db::App(a, b) = inner else {
            panic!("substituted argument")
        };
        assert_eq!((a.as_ref() as *const Db, b.as_ref() as *const Db), ptr);
        assert_eq!(b.as_ref(), &Db::Var(if nested { 4 } else { 3 }));
        assert_eq!(count, 0);
    }
}

#[test]
fn mutual_fix_ready_and_stuck_guards_retain_the_unevaluated_argument_edge() {
    let _reset = reset();
    let mut ctx = Context::new();
    crate::prelude::StandardLibrary::register(&mut ctx);
    for ready in [false, true] {
        let _scope = PrefixScope::enter(&ctx);
        assert!(fix_factory_begin());
        let edge = DbBox::new(if ready { g("Zero") } else { Db::Var(0) });
        assert!(edge.is_shared());
        let alias = edge.clone();
        let head = Db::MutualFix {
            defs: vec![lam(g("Nat"), Db::Var(0))],
            index: 0,
        };
        let WhnfStep::Demand { term, frame } = whnf_step(&ctx, &[], Db::App(owned(head), edge))
        else {
            panic!("head demand")
        };
        let step = resume_whnf(&ctx, &[], frame, term);
        let result = match step {
            WhnfStep::Reduce(result) if ready => result,
            WhnfStep::Ready(result) if !ready => result,
            _ => panic!("mutual-fix constructor guard changed"),
        };
        assert_eq!(argument(&result).as_ref(), alias.as_ref());
        assert!(std::ptr::eq(argument(&result).as_ref(), alias.as_ref()));
    }
}

#[test]
fn expired_aliased_open_argument_is_copied_only_when_nested_beta_consumes_it() {
    let _reset = reset();
    let ctx = Context::new();
    let baseline = live();
    let edge = {
        let _scope = PrefixScope::enter(&ctx);
        assert!(fix_factory_begin());
        DbBox::new(app(g("captured_payload"), Db::Var(3)))
    };
    assert!(edge.is_shared() && edge.facts().is_none());
    let bytes = edge.retained_bytes();
    let alias = edge.clone();
    let original = app(g("captured_payload"), Db::Var(3));
    let expected = lam(g("A"), app(g("captured_payload"), Db::Var(4)));
    assert_eq!(live(), baseline + bytes);
    let function = lam(g("A"), lam(g("A"), Db::Var(1)));
    let WhnfStep::Demand { term, frame } = whnf_step(&ctx, &[], Db::App(owned(function), edge))
    else {
        panic!("head demand")
    };
    {
        let mut roots = Vec::new();
        assert_eq!(frame.size_roots(&mut roots), 1);
        assert!(
            std::ptr::eq(roots[0], alias.as_ref()),
            "the expired open edge stays intact until beta actually needs it"
        );
    }
    let WhnfStep::Reduce(result) = resume_whnf(&ctx, &[], frame, term) else {
        panic!("nested beta reduction")
    };
    assert_eq!(
        result, expected,
        "the outer capture must shift under the retained lambda"
    );
    assert_eq!(
        alias.as_ref(),
        &original,
        "consumption must not mutate the other owner"
    );
    assert_eq!(
        live(),
        baseline + bytes,
        "the alias still owns its persistent ticket"
    );
    drop(alias);
    assert_eq!(
        live(),
        baseline,
        "the shifted owned result does not retain the old ticket"
    );
    assert_eq!(
        result, expected,
        "the shifted result survives the original owner's release"
    );
}
