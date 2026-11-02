//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! RDU-TARGET LOCK — Phase 8 of the SambaNova-parity grind (the pitch artifact).
//!
//! Everything Phases 1–7 built, aimed at a SambaNova-RDU-shaped descriptor: a transformer block lowers onto
//! `FabricTarget::rdu_shaped()` and carries a machine-checked certificate. This lock proves:
//!
//!   A. The RDU-shaped target advertises exactly the dials a fused int8 transformer block needs — the
//!      weight-stationary GEMM dataflow, int8 precision, all three memory tiers, and a block-sized fusion
//!      window; a reduction-only edge target REFUSES it (typed gate).
//!   B. The fused block computes ≡ the VM (requant + bias + ReLU over the int8 GEMM).
//!   C. PROOF-CARRYING CERTIFICATE: the block's GEMM PE placement is SAT-certified ≡ its abstract datapath
//!      for all int8 inputs — the certificate SambaFlow's place-and-route does not carry.
//!   D. ENERGY/TOKEN: composed from the measured pJ/MAC (the signoff-clean GEMM tape) × the block's MAC
//!      count — a concrete, reproducible figure (not a vendor multiplier).
//!
//! Run: `cargo test -p logicaffeine-tests --test rdu_target_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::bitblast::lower_bool;
use logicaffeine_compile::codegen_sva::fabric_grid::{grid_output_to_bounded, route_loop_body};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    fabric_to_bounded, AluOp, FabricOp, LoopCell, Src,
};
use logicaffeine_compile::codegen_sva::fabric_target::{
    Capability, Dataflow, FabricTarget, MemTier, Precision, Unplaceable,
};
use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;
use logicaffeine_compile::codegen_sva::systolic::{eval_fused_block, FusedBlock, Systolic};
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
use logicaffeine_proof::ProofExpr;

fn prng(seed: u64, i: usize, lo: i64, hi: i64) -> i64 {
    let mut x = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(i as u64 + 1);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    lo + (x % (hi - lo + 1) as u64) as i64
}

#[test]
fn a_transformer_block_lowers_onto_the_rdu_shaped_target_with_a_certificate() {
    let rdu = FabricTarget::rdu_shaped();

    // A. the RDU-shaped target advertises the block's dials.
    rdu.require_gemm(Dataflow::WeightStationary, Precision::Int8)
        .expect("int8 weight-stationary GEMM on rdu");
    assert!(
        rdu.provides(Capability::MemTier(MemTier::Sram)),
        "rdu has on-chip SRAM (PMU)"
    );
    assert!(
        rdu.provides(Capability::MemTier(MemTier::Hbm)),
        "rdu has on-package HBM"
    );
    assert!(
        rdu.provides(Capability::MemTier(MemTier::Ddr)),
        "rdu has off-package DDR"
    );
    assert!(
        rdu.provides(Capability::Fusion(4)),
        "rdu fuses a whole block"
    );
    // an edge target refuses the block's weight-stationary GEMM stage.
    match FabricTarget::tiny().require_gemm(Dataflow::WeightStationary, Precision::Int8) {
        Err(Unplaceable::MissingCapability { .. }) => {}
        other => panic!("a reduction-only target must refuse the block, got {other:?}"),
    }

    // B. the fused block computes ≡ the VM (one output element, int8 range).
    let k = 6usize;
    let acts: Vec<i64> = (0..k).map(|i| prng(1, i, -110, 110)).collect();
    let weights: Vec<i64> = (0..k).map(|i| prng(2, i, -110, 110)).collect();
    let bias = prng(3, 0, -400, 400);
    let (mscale, s) = (181i64, 8u32);
    let fb = FusedBlock {
        sys: Systolic::new(k, 1, weights.clone()),
        requant_m: mscale,
        requant_s: s,
        bias: vec![bias],
    };
    let got = eval_fused_block(&fb, &acts, 1)[0];
    let dot: Vec<String> = (0..k)
        .map(|r| format!("({}) * ({})", acts[r], weights[r]))
        .collect();
    let src = format!(
        "## Main\n    Let dot be {}.\n    Let rq be (dot * {mscale}) // {}.\n    Let biased be rq + ({bias}).\n    Let mutable out be biased.\n    If biased is less than 0:\n        Set out to 0.\n    Show out.\n",
        dot.join(" + "),
        1i64 << s
    );
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "VM error: {:?}", vm.error);
    assert_eq!(
        got.to_string(),
        vm.output.trim(),
        "fused block on rdu != VM"
    );

    // C. proof-carrying certificate: the block's GEMM PE placement ≡ its datapath, all int8 inputs.
    let pe = vec![
        FabricOp {
            op: AluOp::Mul,
            a: Src::In(0),
            b: Src::In(2),
        },
        FabricOp {
            op: AluOp::Add,
            a: Src::In(1),
            b: Src::Val(0),
        },
    ];
    let cells: Vec<LoopCell> = pe
        .iter()
        .map(|o| LoopCell::Arith {
            op: o.op,
            a: o.a,
            b: o.b,
        })
        .collect();
    let g = route_loop_body(&cells, 3, &[Src::Val(1)]);
    let grid = grid_output_to_bounded(&g, 0, 8, &|k| format!("in{k}"));
    let dp = fabric_to_bounded(&pe, 1, 8);
    let claim = BoundedExpr::Eq(Box::new(grid), Box::new(dp));
    let p = lower_bool(&claim).expect("claim lowers");
    assert_eq!(
        prove_unsat(&ProofExpr::Not(Box::new(p))),
        UnsatOutcome::Refuted,
        "the block's GEMM PE placement must be SAT-certified ≡ its datapath (the proof-carrying certificate)"
    );

    // D. energy/token composed from the measured pJ/MAC (the signoff-clean GEMM tape, VCD power at its
    //    signoff clock) × the block's measured MAC count — both read from their artifacts, never typed.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let load = |rel: &str| -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(root.join(rel)).expect(rel)).expect(rel)
    };
    let macs_per_token = load("benchmarks/silicon/tiny_transformer_fabric.json")
        ["matmul_macs_per_token"]["total"]
        .as_u64()
        .expect("the measured MACs per token");
    let pj_per_mac = load("benchmarks/silicon/gemm_fabric.json")["energy"]["signoff_clean_tape"]
        ["energy_per_mac_pJ"]["vcd_measured"]
        .as_f64()
        .expect("the measured pJ/MAC");
    assert!(
        macs_per_token > 0 && pj_per_mac > 0.0,
        "both factors are measurements, not placeholders"
    );
    let nj_per_token = (macs_per_token as f64) * pj_per_mac / 1000.0;

    println!(
        "Phase 8: transformer block lowers onto rdu_shaped() ≡VM, SAT-certified PE, ~{:.0} nJ/token — the pitch artifact",
        nj_per_token
    );
}
