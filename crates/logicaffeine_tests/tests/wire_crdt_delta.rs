//! ════════════════════════════════════════════════════════════════════════════════════════════
//! G7 δ-CRDT — `mergeable` ships only what CHANGED. A state-based CRDT sync re-broadcasts the whole
//! set/sequence every time; a delta-state sync ships `delta_since(what the peer already has)` — a
//! handful of bytes for one new element, no matter how large the collection. The delta is idempotent
//! + commutative (redelivery / reordering still converges), and a foreign / garbage delta is refused
//! at the edge, never corrupting the CRDT. This is the CvRDT→δ-CRDT upgrade for the `Send mergeable`
//! wire path.
//! ════════════════════════════════════════════════════════════════════════════════════════════

use logicaffeine_compile::interpreter::RuntimeValue;
use logicaffeine_compile::semantics::crdt::{next_replica_id, CrdtValue};
use logicaffeine_data::crdt::VClock;

#[test]
fn crdt_delta_ships_only_the_change_not_the_whole_state() {
    let r1 = next_replica_id();
    let r2 = next_replica_id();

    // A accumulates 100 elements; remember its version at that point.
    let mut a = CrdtValue::new_set(r1);
    for i in 0..100 {
        a.insert(&RuntimeValue::Int(i)).unwrap();
    }
    let v_after_100 = a.version();

    // B catches up by applying A's FULL delta (everything since nothing).
    let full = a.delta_since_bytes(&VClock::default()).expect("a full delta exists");
    let mut b = CrdtValue::new_set(r2);
    assert!(b.apply_delta_bytes(&full), "B applies the full delta");
    assert_eq!(b.len(), 100, "B caught up to all 100 elements");

    // A adds ONE more element. The delta since B's known version is JUST that one element —
    // orders of magnitude smaller than re-shipping the whole 100-element state.
    a.insert(&RuntimeValue::Int(1000)).unwrap();
    let delta = a.delta_since_bytes(&v_after_100).expect("an incremental delta exists");
    assert!(
        delta.len() * 10 < full.len(),
        "the incremental delta ({} B) must be far smaller than the full state ({} B)",
        delta.len(),
        full.len()
    );

    // B applies the small delta and converges to 101 — without re-receiving the original 100.
    assert!(b.apply_delta_bytes(&delta), "B applies the incremental delta");
    assert_eq!(b.len(), 101, "B converged to 101 from the small delta");
    assert!(b.contains(&RuntimeValue::Int(1000)).unwrap(), "the new element arrived");

    // Idempotent + commutative: redelivering the same delta changes nothing.
    assert!(b.apply_delta_bytes(&delta), "re-applying the delta is accepted");
    assert_eq!(b.len(), 101, "re-applying the delta is idempotent");

    // A foreign / garbage / empty delta is refused, leaving B untouched (edge safety).
    assert!(!b.apply_delta_bytes(&[0xFF, 1, 2, 3]), "a garbage delta is refused");
    assert!(!b.apply_delta_bytes(&[]), "an empty delta is refused");
    assert_eq!(b.len(), 101, "B is unchanged by a refused delta");
}

#[test]
fn crdt_delta_converges_under_concurrent_updates_either_order() {
    // Two replicas each add their own element concurrently; exchanging deltas (in EITHER order)
    // converges both to the union — the commutativity δ-CRDTs guarantee.
    let r1 = next_replica_id();
    let r2 = next_replica_id();
    let mut a = CrdtValue::new_set(r1);
    let mut b = CrdtValue::new_set(r2);

    // Shared base: both start from the same two elements.
    let mut base = CrdtValue::new_set(next_replica_id());
    base.insert(&RuntimeValue::Int(1)).unwrap();
    base.insert(&RuntimeValue::Int(2)).unwrap();
    let base_delta = base.delta_since_bytes(&VClock::default()).unwrap();
    a.apply_delta_bytes(&base_delta);
    b.apply_delta_bytes(&base_delta);
    let base_version_a = a.version();
    let base_version_b = b.version();

    // Concurrent divergence: A adds 10, B adds 20.
    a.insert(&RuntimeValue::Int(10)).unwrap();
    b.insert(&RuntimeValue::Int(20)).unwrap();

    // Each ships only its own change.
    let da = a.delta_since_bytes(&base_version_a).unwrap();
    let db = b.delta_since_bytes(&base_version_b).unwrap();

    // Exchange — A applies B's delta, B applies A's delta.
    a.apply_delta_bytes(&db);
    b.apply_delta_bytes(&da);

    // Both converged to the same union {1, 2, 10, 20}.
    for v in [1, 2, 10, 20] {
        assert!(a.contains(&RuntimeValue::Int(v)).unwrap(), "A has {v}");
        assert!(b.contains(&RuntimeValue::Int(v)).unwrap(), "B has {v}");
    }
    assert_eq!(a.len(), 4);
    assert_eq!(b.len(), 4);
}

#[test]
fn crdt_sequence_delta_ships_only_the_appended_node() {
    // A `SharedSequence` (RGA) grows; the delta for one new append is ONE node, not the whole list.
    let r1 = next_replica_id();
    let r2 = next_replica_id();
    let mut a = CrdtValue::new_seq(r1);
    for i in 0..100 {
        a.append(&RuntimeValue::Int(i)).unwrap();
    }
    let v0 = a.version();

    let full = a.delta_since_bytes(&VClock::default()).expect("a full delta exists");
    let mut b = CrdtValue::new_seq(r2);
    assert!(b.apply_delta_bytes(&full), "B applies the full sequence");
    assert_eq!(b.len(), 100, "B caught up to all 100 nodes");

    a.append(&RuntimeValue::Int(1000)).unwrap();
    let delta = a.delta_since_bytes(&v0).expect("an incremental delta exists");
    assert!(
        delta.len() * 10 < full.len(),
        "the appended-node delta ({} B) must be far smaller than the whole sequence ({} B)",
        delta.len(),
        full.len()
    );
    assert!(b.apply_delta_bytes(&delta), "B applies the appended-node delta");
    assert_eq!(b.len(), 101, "B converged to 101 from the small delta");
}
