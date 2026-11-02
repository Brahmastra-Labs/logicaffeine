//! Cache plumbing controls. Callbacks are deliberately synthetic storage values,
//! not mathematical certificates; actual independent recurrence tests belong in
//! recheck_equality's integration controls.
use super::*;
use std::cell::Cell;

fn result() -> [String; 2] {
    ["no".into(), "yes".into()]
}
fn counted(counter: &Cell<usize>) -> Option<[String; 2]> {
    counter.set(counter.get() + 1);
    Some(result())
}

#[test]
fn independent_positive_reuses_closed_registry_across_local_scopes() {
    let ctx = Context::new();
    let count = Cell::new(0);
    assert_eq!(
        ctx.cached_independent_equality_recurrence("same", || counted(&count)),
        Some(result())
    );
    let local = ctx.extend("n", Term::Global("Nat".into()));
    assert_eq!(
        local.cached_independent_equality_recurrence("same", || counted(&count)),
        Some(result())
    );
    assert_eq!(
        count.get(),
        1,
        "local bindings are absent from the closed recurrence context"
    );
}

#[test]
fn main_order_positive_cannot_satisfy_independent_equality() {
    let ctx = Context::new();
    let count = Cell::new(0);
    ctx.cached_order_recurrence("same", || Some(result()))
        .unwrap();
    for _ in 0..2 {
        assert!(ctx
            .cached_independent_equality_recurrence("same", || {
                count.set(count.get() + 1);
                None
            })
            .is_none());
    }
    assert_eq!(
        count.get(),
        2,
        "neither a main success nor an independent failure is an independent certificate"
    );
    let reversed = ["yes".into(), "no".into()];
    assert_eq!(
        ctx.cached_independent_equality_recurrence("same", || Some(reversed.clone())),
        Some(reversed.clone())
    );
    assert_eq!(
        ctx.cached_order_recurrence("same", || panic!("main cache lost")),
        Some(result())
    );
    assert_eq!(
        ctx.cached_independent_equality_recurrence("same", || panic!("independent cache lost")),
        Some(reversed)
    );
}

#[test]
fn independent_positive_cannot_satisfy_main_order() {
    let ctx = Context::new();
    ctx.cached_independent_equality_recurrence("same", || Some(result()))
        .unwrap();
    assert!(ctx.cached_order_recurrence("same", || None).is_none());
    assert_eq!(
        ctx.cached_independent_equality_recurrence("same", || panic!(
            "failure contaminated other issuer"
        )),
        Some(result())
    );
}

#[test]
fn failed_independent_attempt_does_not_prevent_later_success() {
    let ctx = Context::new();
    let count = Cell::new(0);
    assert!(ctx
        .cached_independent_equality_recurrence("equal", || None)
        .is_none());
    assert_eq!(
        ctx.cached_independent_equality_recurrence("equal", || counted(&count)),
        Some(result())
    );
    assert_eq!(
        ctx.cached_independent_equality_recurrence("equal", || counted(&count)),
        Some(result())
    );
    assert_eq!(count.get(), 1);
}

#[test]
fn independent_cache_tracks_child_mutation_and_original_generation() {
    let ctx = Context::new();
    let count = Cell::new(0);
    ctx.cached_independent_equality_recurrence("equal", || counted(&count))
        .unwrap();
    let mut changed = ctx.clone();
    changed.add_declaration("new_global", Term::Sort(Universe::Type(0)));
    changed
        .cached_independent_equality_recurrence("equal", || counted(&count))
        .unwrap();
    ctx.cached_independent_equality_recurrence("equal", || counted(&count))
        .unwrap();
    assert_eq!(
        count.get(),
        3,
        "single generation slot must re-attest after either registry change"
    );
    ctx.cached_independent_equality_recurrence("equal", || panic!("same generation missed"))
        .unwrap();
}

#[test]
fn independent_warm_cache_cannot_bypass_recurrence_or_rewrite_gate() {
    let mut ctx = Context::new();
    ctx.cached_independent_equality_recurrence("equal", || {
        assert!(
            ctx.order_attestation.try_lock().is_ok(),
            "no mutex during independent attestation"
        );
        Some(result())
    })
    .unwrap();
    assert!(ctx
        .order_recurrence_context()
        .cached_independent_equality_recurrence("equal", || panic!("recursive attestation"))
        .is_none());
    // Raw rewrite metadata tests only the storage gate, not theorem admission.
    Arc::make_mut(&mut ctx.simp_rules).push(SimpRule {
        name: "gate".into(),
        binders: vec![],
        lhs: Term::Global("a".into()),
        rhs: Term::Global("b".into()),
        head: "a".into(),
    });
    assert!(ctx
        .cached_independent_equality_recurrence("equal", || panic!("rewrite gate bypassed"))
        .is_none());
}

#[test]
fn both_issuers_share_one_entry_bound_and_keep_oversized_results_uncached() {
    let ctx = Context::new();
    let count = Cell::new(0);
    let oversized = ["n".repeat(257), "yes".into()];
    for _ in 0..2 {
        assert_eq!(
            ctx.cached_independent_equality_recurrence("large", || {
                count.set(count.get() + 1);
                Some(oversized.clone())
            }),
            Some(oversized.clone())
        );
    }
    assert_eq!(count.get(), 2);
    assert!(ctx.order_attestation.lock().unwrap().is_none());
    for i in 0..32 {
        let name = format!("same_name_{i}");
        ctx.cached_order_recurrence(&name, || Some(result()))
            .unwrap();
        ctx.cached_independent_equality_recurrence(&name, || Some(result()))
            .unwrap();
    }
    assert_eq!(
        ctx.order_attestation
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .results
            .len(),
        64
    );
    count.set(0);
    for _ in 0..2 {
        ctx.cached_independent_equality_recurrence("overflow", || counted(&count))
            .unwrap();
    }
    assert_eq!(
        count.get(),
        2,
        "combined capacity must not silently become 128 entries"
    );
    assert_eq!(
        ctx.order_attestation
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .results
            .len(),
        64
    );
}
