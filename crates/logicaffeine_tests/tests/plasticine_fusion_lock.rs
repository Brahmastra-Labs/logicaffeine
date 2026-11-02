//! ═══════════════════════════════════════════════════════════════════════
//! PLASTICINE OPERAND-SHARING LOCK — reader fusion on the RDU, decided rather than assumed.
//!
//! When a tiled GEMM is unrolled `par` ways, each instance needs the same operand panels. Either every
//! instance streams its own copy from DRAM (replicated: `par` times the traffic, no fabric cost), or one
//! instance streams it and broadcasts to the rest (fused: one copy, plus a fan-out that occupies the vector
//! network and costs control ports). Which is cheaper depends on the shape, the mesh and the memory — so it
//! is a decision the compiler makes from its cost model, exactly as reader fusion is on Tensix, and never a
//! constant baked into a roofline.
//!
//! A. The decision is a FUNCTION of the costs: broadcast wins when the DRAM traffic it saves exceeds the
//!    fabric time it adds, and loses when it does not. Both outcomes are reachable by moving only the costs.
//! B. There is NO DIAL: no caller may pass the answer in, and the compiler exposes no sharing input. The
//!    only way to change the outcome is to change the machine or the shape.
//! C. It is CONSISTENT with the roofline it feeds, and D. REPORTED per operand with its reason: both read a
//!    GEMM row whose unroll the negotiated router decides, so they are pinned in the private place-and-route
//!    engine's `plasticine_negotiated_lock`.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(plasticine_fusion_lock)'`
//! ═══════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::fusion::{share_operand, OperandStream, Sharing};
use logicaffeine_compile::codegen_sva::plasticine::PlasticineTarget;

/// A stream of `bytes` wanted by `consumers` instances, whose broadcast would cross `hops` and occupy
/// `links` of the vector network.
fn stream(bytes: usize, consumers: usize, hops: usize) -> OperandStream {
    OperandStream {
        name: "B".to_string(),
        bytes,
        consumers,
        broadcast_hops: hops,
    }
}

// ───────────────────────────── A ─────────────────────────────

#[test]
fn sharing_is_decided_by_cost_and_both_outcomes_are_reachable() {
    let t = PlasticineTarget::isca17();
    // One consumer: there is nothing to share, so replication is trivially right.
    assert_eq!(
        share_operand(&stream(1 << 20, 1, 4), &t).sharing,
        Sharing::Replicated
    );
    // A big stream wanted by many instances over a short hop: broadcasting saves far more DRAM time than the
    // fabric costs, so the compiler fuses.
    let big = share_operand(&stream(64 << 20, 16, 2), &t);
    assert_eq!(
        big.sharing,
        Sharing::Broadcast,
        "a large shared panel is worth broadcasting: {big:?}"
    );
    assert!(big.dram_cycles_saved > big.fabric_cycles_added);
    // A small stream shared by FEW consumers over a long hop: the fan-out costs more than the DRAM it saves.
    // (The model is stricter than intuition here — 256 bytes fetched 16 times is 75 DRAM cycles against 72 on
    // the fabric, so even that broadcasts. It takes few consumers AND distance to make replication win.)
    let small = share_operand(&stream(256, 2, 40), &t);
    assert_eq!(
        small.sharing,
        Sharing::Replicated,
        "a small panel shared twice over 40 hops is not worth a fan-out: {small:?}"
    );
    assert!(small.dram_cycles_saved <= small.fabric_cycles_added);
    // And the decision really is marginal where the costs are marginal, not snapped to a heuristic. These
    // numbers moved when the broadcast cost was corrected from store-and-forward (`words * hops`) to the
    // pipelined `words + hops - 1` a registered-link fabric actually pays; the old fixture straddled a
    // boundary an order of magnitude away. The property is unchanged: a case decided by ONE cycle, and its
    // neighbour on the other side, so the decision is provably a comparison rather than a rule of thumb.
    let marginal = share_operand(&stream(64, 16, 18), &t);
    assert_eq!(
        marginal.sharing,
        Sharing::Broadcast,
        "19 DRAM cycles saved beats 18 added: {marginal:?}"
    );
    assert_eq!(
        marginal.dram_cycles_saved - marginal.fabric_cycles_added,
        1,
        "and it is decided on a single cycle: {marginal:?}"
    );
    // One hop further and the same panel is not worth broadcasting — the boundary is real and crossable.
    let over = share_operand(&stream(64, 16, 19), &t);
    assert_eq!(
        over.sharing,
        Sharing::Replicated,
        "one hop more and the fan-out stops paying: {over:?}"
    );
    assert!(over.fabric_cycles_added >= over.dram_cycles_saved);
    // Monotone in each input, holding the others fixed.
    let base = stream(1 << 16, 8, 6);
    assert!(
        share_operand(
            &OperandStream {
                bytes: base.bytes * 64,
                ..base.clone()
            },
            &t
        )
        .dram_cycles_saved
            > share_operand(&base, &t).dram_cycles_saved,
        "more bytes, more to save"
    );
    assert!(
        share_operand(
            &OperandStream {
                broadcast_hops: 40,
                ..base.clone()
            },
            &t
        )
        .fabric_cycles_added
            > share_operand(&base, &t).fabric_cycles_added,
        "a longer fan-out costs more"
    );
    // The SAME stream can flip purely by changing the machine's memory bandwidth.
    let slow = PlasticineTarget::isca17(); // DDR3, 51.2 GB/s
    let fast = PlasticineTarget::tungsten_20x20_ddr4(); // the toolchain's DDR4, faster
    let marginal = stream(1 << 14, 4, 10);
    let a = share_operand(&marginal, &slow);
    let b = share_operand(&marginal, &fast);
    assert!(
        a.dram_cycles_saved >= b.dram_cycles_saved,
        "slower memory makes a shared stream more worth broadcasting: {a:?} vs {b:?}"
    );
}

// ───────────────────────────── B ─────────────────────────────

#[test]
fn there_is_no_dial_anywhere() {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/logicaffeine_compile/src/codegen_sva/fusion.rs"),
    )
    .expect("fusion.rs");
    // No FUNCTION may take the answer as a parameter (a `Sharing` in a return type or an output struct field
    // is the decision, not a dial), and no forcing hook may exist at all.
    for line in src
        .lines()
        .filter(|l| l.trim_start().starts_with("pub fn ") || l.trim_start().starts_with("fn "))
    {
        let params = line
            .split_once('(')
            .map(|(_, r)| r.split(')').next().unwrap_or(""))
            .unwrap_or("");
        assert!(
            !params.contains("Sharing"),
            "a function takes the answer as a parameter: {line}"
        );
        assert!(
            !params.contains("bool"),
            "a boolean parameter is a dial in disguise: {line}"
        );
    }
    for forbidden in [
        "force",
        "prefer_broadcast",
        "always_broadcast",
        "override",
        "_forced",
    ] {
        assert!(
            !src.contains(forbidden),
            "the sharing decision must have no forcing hook: {forbidden}"
        );
    }
    // The public entry takes only a stream and a target — the machine and the shape, nothing else.
    assert!(
        src.contains("pub fn share_operand(stream: &OperandStream, t: &PlasticineTarget)"),
        "share_operand takes exactly the stream and the target"
    );
}
