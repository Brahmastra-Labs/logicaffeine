//! storage_near_compute_lock — near-storage (computational-storage) compute, proven SOUND
//! under the deterministic controller's read reordering. The controller earns its bus
//! utilization by die-interleaving reads (the data arrives in `gather_delivery_order`, not
//! logical order), so any compute fed AT the storage edge must be order-invariant. This locks
//! the DPU primitive — a CRDT join (merge-on-ingest) — as correct there: merging the shipped
//! `logicaffeine_data::crdt::GCounter` in the scrambled storage-delivery order yields EXACTLY
//! the same convergence as merging in logical order. The contrast test shows why the choice
//! matters: an order-DEPENDENT reduction diverges under the same reorder. This is the third
//! proven data edge (memory / network / storage); cited in STORAGE_EDGE_SPEC.md.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::storage_controller_rtl::{
    gather_delivery_order, FlashTimings,
};
use logicaffeine_data::crdt::{GCounter, Merge};

/// One page holds replica `r`'s counter state (value `v`).
fn page_counter(r: u64, v: u64) -> GCounter {
    let mut c = GCounter::with_replica_id(r);
    c.increment(v);
    c
}

/// A gather whose die pattern forces the controller to reorder delivery: the first pages all
/// hammer die 0 (serializing on it), while later pages land on fresh dies and arrive sooner.
fn reordering_gather(t: &FlashTimings) -> (Vec<u64>, Vec<u64>) {
    let n = t.n_dies() as u64;
    let mut pages = Vec::new();
    for k in 0..8 {
        pages.push(k * n); // all map to die 0 → serialize
    }
    for d in 1..9 {
        pages.push(d); // dies 1..8 → arrive early
    }
    // Distinct per-page values (replica r = page index, count = 10*r + 1).
    let values: Vec<u64> = (0..pages.len() as u64).map(|r| 10 * r + 1).collect();
    (pages, values)
}

/// DPU — merge-on-ingest is order-invariant: the CRDT join over the scrambled storage-delivery
/// order equals the join over logical order, and both equal the convergence of all states.
#[test]
fn crdt_merge_on_ingest_is_sound_under_storage_reordering() {
    let t = FlashTimings::zns_tlc();
    let (pages, values) = reordering_gather(&t);
    let delivery = gather_delivery_order(&t, &pages);

    // The controller genuinely reordered the stream (not the identity permutation).
    let identity: Vec<usize> = (0..pages.len()).collect();
    assert_ne!(
        delivery, identity,
        "the die-interleave must actually reorder delivery"
    );

    // Merge in the order the STORAGE EDGE delivers the pages.
    let mut ingest = GCounter::with_replica_id(u64::MAX);
    for &i in &delivery {
        ingest.merge(&page_counter(i as u64, values[i]));
    }
    // Merge in logical order (the reference convergence).
    let mut logical = GCounter::with_replica_id(u64::MAX);
    for i in 0..pages.len() {
        logical.merge(&page_counter(i as u64, values[i]));
    }

    let expected: u64 = values.iter().sum();
    assert_eq!(
        ingest.value(),
        logical.value(),
        "storage-order merge diverged from logical-order merge"
    );
    assert_eq!(
        ingest.value(),
        expected,
        "the join did not converge to the union of all states"
    );
}

/// The contrast that justifies the primitive: an order-DEPENDENT reduction (the raw value
/// sequence) DIVERGES under the same reorder — so a naive append/concat at the storage edge
/// would corrupt, and only an order-invariant join is safe there.
#[test]
fn an_order_dependent_reduction_diverges_under_the_same_reorder() {
    let t = FlashTimings::zns_tlc();
    let (pages, values) = reordering_gather(&t);
    let delivery = gather_delivery_order(&t, &pages);

    let delivered_seq: Vec<u64> = delivery.iter().map(|&i| values[i]).collect();
    let logical_seq: Vec<u64> = values.clone();
    assert_ne!(
        delivered_seq, logical_seq,
        "an order-dependent op must diverge under storage delivery reordering (that's why join wins)"
    );
}

/// A flash read-RETRY re-delivers a page; the CRDT join must be IDEMPOTENT so a retried page
/// never double-counts (a sum would). This is why merge-on-ingest survives the storage layer's
/// duplicate deliveries — the DPU primitive is chosen for exactly this robustness.
#[test]
fn merge_on_ingest_tolerates_duplicate_delivery_from_read_retries() {
    let t = FlashTimings::zns_tlc();
    let (pages, values) = reordering_gather(&t);
    let delivery = gather_delivery_order(&t, &pages);

    let mut once = GCounter::with_replica_id(u64::MAX);
    for &i in &delivery {
        once.merge(&page_counter(i as u64, values[i]));
    }
    // Now ingest the whole delivered stream a SECOND time (every page retried).
    let mut retried = GCounter::with_replica_id(u64::MAX);
    for _ in 0..2 {
        for &i in &delivery {
            retried.merge(&page_counter(i as u64, values[i]));
        }
    }
    assert_eq!(
        once.value(),
        retried.value(),
        "a re-delivered page must not double-count (idempotent join)"
    );
    assert_eq!(retried.value(), values.iter().sum::<u64>());
}

/// XPU — a commutative scan (sum) fed by the storage stream matches the reference reduction,
/// regardless of delivery order (order-invariant, like the CRDT join).
#[test]
fn commutative_scan_over_the_storage_stream_matches_the_reference() {
    let t = FlashTimings::zns_tlc();
    let (pages, values) = reordering_gather(&t);
    let delivery = gather_delivery_order(&t, &pages);

    let edge_sum: u64 = delivery.iter().map(|&i| values[i]).sum();
    let reference_sum: u64 = values.iter().sum();
    assert_eq!(
        edge_sum, reference_sum,
        "a commutative scan at the edge must equal the reference"
    );
}
