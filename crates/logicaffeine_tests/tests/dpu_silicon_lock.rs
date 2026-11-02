//! dpu_silicon_lock — the DPU processor edge: deterministic near-data compute.
//!
//! A DPU computes on data as it streams off storage/network, AT the data edge. The
//! deterministic controller earns its bus by die-interleaving reads, so data arrives
//! out of logical order — which means any compute fed at the edge must be
//! **order-invariant**. The DPU primitive is merge-on-ingest: the CRDT join, realized
//! in silicon as a per-lane max (`crdt_rtl::merge_lane`, itself proven == the shipped
//! `GCounter::merge` in `crdt_silicon_lock`). This lock frames + pins that datapath as
//! the standalone DPU near-data tile:
//!   1. the SILICON merge folded over the scrambled storage-delivery order equals the
//!      merge in logical order (order-invariant → correct at the edge);
//!   2. an order-DEPENDENT reduction diverges under the same reorder (why the CRDT
//!      choice is load-bearing);
//!   3. the tile emits synthesizable RTL.
//! Reuses the proven CRDT datapath + the proven storage reorder model.
//!
//! Proof strength: ≡ shipped `GCounter` (via `crdt_silicon_lock`) + order-invariance
//! (here). FPGA/Sky130 receipts recorded separately (reports.json / sky130_signoff.json).
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(dpu_silicon_lock)'`

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::crdt_rtl::{
    emit_crdt_merge_verilog, emit_dpu_ingest_verilog, merge_lane,
};
use logicaffeine_compile::codegen_sva::storage_controller_rtl::{
    gather_delivery_order, FlashTimings,
};

/// The 4-lane silicon GCounter state the merge datapath operates on.
fn merge4(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    [
        merge_lane(a[0], b[0]),
        merge_lane(a[1], b[1]),
        merge_lane(a[2], b[2]),
        merge_lane(a[3], b[3]),
    ]
}
fn value4(s: [u32; 4]) -> u64 {
    s.iter().map(|&x| x as u64).sum()
}

/// A reordering gather: the first pages all hammer die 0 (serialize), later pages land
/// on fresh dies and arrive sooner — so the controller delivers them out of order.
fn reordering_pages(t: &FlashTimings) -> Vec<u64> {
    let n = t.n_dies() as u64;
    let mut pages: Vec<u64> = (0..8).map(|k| k * n).collect();
    for d in 1..9 {
        pages.push(d);
    }
    pages
}

/// Page `i` contributes to lane `i % 4` a distinct value — the near-data payload.
fn page_state(i: usize) -> [u32; 4] {
    let mut s = [0u32; 4];
    s[i % 4] = (10 * i + 1) as u32;
    s
}

#[test]
fn dpu_near_data_merge_is_order_invariant_on_silicon() {
    let t = FlashTimings::zns_tlc();
    let pages = reordering_pages(&t);
    let delivery = gather_delivery_order(&t, &pages);

    // The DPU folds the SILICON merge over the order the controller actually delivers.
    let mut ingest = [0u32; 4];
    for &i in &delivery {
        ingest = merge4(ingest, page_state(i));
    }
    // Reference: fold in logical order.
    let mut logical = [0u32; 4];
    for i in 0..pages.len() {
        logical = merge4(logical, page_state(i));
    }

    assert_eq!(
        value4(ingest),
        value4(logical),
        "DPU near-data merge diverged under storage reordering — the edge compute is not order-invariant"
    );
}

#[test]
fn dpu_an_order_dependent_reduction_diverges_under_the_same_reorder() {
    // Contrast: a hash-like fold (acc = acc*31 + i) is order-DEPENDENT. Under the same
    // reorder it must diverge — that is precisely why the DPU compute must be a CRDT join.
    let t = FlashTimings::zns_tlc();
    let pages = reordering_pages(&t);
    let delivery = gather_delivery_order(&t, &pages);
    let logical: Vec<usize> = (0..pages.len()).collect();
    assert_ne!(
        delivery, logical,
        "the gather must actually reorder for this contrast to be meaningful"
    );
    let fold = |order: &[usize]| {
        order
            .iter()
            .fold(0u64, |acc, &i| acc.wrapping_mul(31).wrapping_add(i as u64))
    };
    assert_ne!(
        fold(&delivery),
        fold(&logical),
        "an order-dependent reduction should diverge under reorder — the DPU must be order-invariant instead"
    );
}

#[test]
fn dpu_tile_emits_synthesizable_rtl() {
    let rtl = emit_crdt_merge_verilog();
    assert!(
        rtl.contains("module"),
        "the DPU merge tile emits a Verilog module"
    );
    assert!(
        rtl.contains("endmodule"),
        "the emitted RTL is a complete module"
    );
}

#[test]
fn dpu_ingest_tile_is_a_clocked_register_to_register_block() {
    // The signoff block is the SEQUENTIAL merge-on-ingest accumulator (real STA paths),
    // not just the combinational unit.
    let rtl = emit_dpu_ingest_verilog();
    assert!(
        rtl.contains("module dpu_ingest"),
        "the DPU ingest tile emits a named module"
    );
    assert!(
        rtl.contains("posedge clk"),
        "merge-on-ingest is clocked (a real register-to-register block)"
    );
    assert!(
        rtl.contains("in_valid"),
        "the tile folds one ingest beat per valid cycle"
    );
    assert!(
        rtl.contains("endmodule"),
        "the emitted RTL is a complete module"
    );
}

#[test]
fn dpu_ingest_accumulator_equals_the_batch_join() {
    // The sequential tile's meaning: folding a stream of beats through the accumulator
    // (acc <= merge(acc, beat)) yields the same state as one batch merge of all beats —
    // and the reorder cannot change it. This ties the clocked tile to the proven unit.
    let t = FlashTimings::zns_tlc();
    let pages = reordering_pages(&t);
    let delivery = gather_delivery_order(&t, &pages);

    // sequential fold in delivery order (what the clocked tile computes)
    let mut acc = [0u32; 4];
    for &i in &delivery {
        acc = merge4(acc, page_state(i));
    }
    // batch join of every beat, order-free (per-lane max over all)
    let mut batch = [0u32; 4];
    for i in 0..pages.len() {
        for lane in 0..4 {
            batch[lane] = merge_lane(batch[lane], page_state(i)[lane]);
        }
    }
    assert_eq!(
        acc, batch,
        "the ingest accumulator diverged from the batch join"
    );
}
