use super::*;
use std::cell::Cell;
fn outcomes() -> [String; 2] {
    ["no".into(), "yes".into()]
}
fn attest(n: &Cell<usize>) -> Option<[String; 2]> {
    n.set(n.get() + 1);
    Some(outcomes())
}

#[test]
fn local_contexts_reuse_closed_certificates_without_retaining_registry_payloads() {
    let c = Context::new();
    let definitions = Arc::strong_count(&c.definitions);
    let structures = Arc::strong_count(&c.structures);
    let n = Cell::new(0);
    assert_eq!(
        c.cached_order_recurrence("order", || attest(&n)),
        Some(outcomes())
    );
    let local = c.extend("order.certificate.m", Term::Sort(Universe::Type(0)));
    assert_eq!(
        local.cached_order_recurrence("order", || attest(&n)),
        Some(outcomes())
    );
    assert_eq!(n.get(), 1);
    drop(local);
    assert_eq!(Arc::strong_count(&c.definitions), definitions);
    assert_eq!(Arc::strong_count(&c.structures), structures);
    assert!(Arc::weak_count(&c.definitions) > 0);
    assert!(Arc::weak_count(&c.structures) > 0);
}

#[test]
fn both_unique_owner_mutation_and_cloned_registry_generations_revalidate() {
    let mut c = Context::new();
    let n = Cell::new(0);
    c.cached_order_recurrence("order", || attest(&n)).unwrap();
    let old = Arc::downgrade(&c.definitions);
    Arc::make_mut(&mut c.definitions).clear();
    assert!(old.upgrade().is_none());
    c.cached_order_recurrence("order", || attest(&n)).unwrap();
    let mut child = c.clone();
    Arc::make_mut(&mut child.structures).clear();
    child
        .cached_order_recurrence("order", || attest(&n))
        .unwrap();
    c.cached_order_recurrence("order", || attest(&n)).unwrap();
    assert_eq!(n.get(), 4);
}

#[test]
fn each_extra_semantic_registry_has_an_independent_generation_token() {
    let mut c = Context::new();
    let n = Cell::new(0);
    c.cached_order_recurrence("order", || attest(&n)).unwrap();
    macro_rules! changed {
        ($field:ident) => {{
            let before = n.get();
            c.$field = Arc::new(Default::default());
            c.cached_order_recurrence("order", || attest(&n)).unwrap();
            assert_eq!(n.get(), before + 1, stringify!($field));
        }};
    }
    changed!(structures);
    changed!(quotient_primitives);
    changed!(definition_deps);
    changed!(hints);
    changed!(mutual_blocks);
    changed!(reducibility);
    changed!(simp_rules);
    changed!(simp_by_head);
    changed!(checked_opaque);
    changed!(checked_opaque_graph);
    changed!(checked_opaque_dependents);
}

#[test]
fn gates_precede_hits_and_attestation_does_not_hold_the_mutex() {
    let mut c = Context::new();
    assert_eq!(
        c.cached_order_recurrence("order", || {
            assert!(c.order_attestation.try_lock().is_ok());
            Some(outcomes())
        }),
        Some(outcomes())
    );
    let certifying = c.order_recurrence_context();
    assert!(certifying
        .cached_order_recurrence("order", || panic!("recursive attestation"))
        .is_none());
    Arc::make_mut(&mut c.simp_rules).push(SimpRule {
        name: "diagnostic".into(),
        binders: vec![],
        lhs: Term::Global("a".into()),
        rhs: Term::Global("b".into()),
        head: "a".into(),
    });
    assert!(c
        .cached_order_recurrence("order", || panic!("rewrite-context attestation"))
        .is_none());
}

#[test]
fn failures_are_not_cached_and_operation_names_keep_outcome_order() {
    let c = Context::new();
    let n = Cell::new(0);
    for _ in 0..2 {
        assert!(c
            .cached_order_recurrence("order", || {
                n.set(n.get() + 1);
                None
            })
            .is_none());
    }
    assert_eq!(n.get(), 2);
    assert_eq!(
        c.cached_order_recurrence("order", || attest(&n)),
        Some(outcomes())
    );
    let reversed = ["yes".into(), "no".into()];
    assert_eq!(
        c.cached_order_recurrence("other", || Some(reversed.clone())),
        Some(reversed)
    );
    assert_eq!(
        c.cached_order_recurrence("order", || panic!("positive cache miss")),
        Some(outcomes())
    );
}

#[test]
fn name_bounds_and_entry_capacity_bound_retained_certificates() {
    let c = Context::new();
    assert_eq!(
        c.cached_order_recurrence(&"x".repeat(257), || Some(outcomes())),
        Some(outcomes())
    );
    let oversized = ["x".repeat(257), "yes".into()];
    assert_eq!(
        c.cached_order_recurrence("large", || Some(oversized.clone())),
        Some(oversized)
    );
    assert!(
        c.order_attestation.lock().unwrap().is_none(),
        "oversized results must not be retained"
    );
    let boundary = "x".repeat(256);
    assert!(c
        .cached_order_recurrence(&boundary, || Some([boundary.clone(), "yes".into()]))
        .is_some());
    for i in 0..63 {
        c.cached_order_recurrence(&format!("order{i}"), || Some(outcomes()))
            .unwrap();
    }
    assert_eq!(
        c.order_attestation
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .results
            .len(),
        64
    );
    let n = Cell::new(0);
    for _ in 0..2 {
        assert_eq!(
            c.cached_order_recurrence("overflow", || attest(&n)),
            Some(outcomes())
        );
    }
    assert_eq!(
        n.get(),
        2,
        "a full cache must not retain the additional result"
    );
    assert_eq!(
        c.order_attestation
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .results
            .len(),
        64
    );
}
