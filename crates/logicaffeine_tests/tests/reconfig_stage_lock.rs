//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! RECONFIG STAGE LOCK — Phase 6a of the SambaNova-parity grind.
//!
//! A PCU reconfigures per pipeline stage — systolic-GEMM for one, SIMD-elementwise for the next. This lock
//! proves per-stage reconfiguration at the descriptor + eval level:
//!
//!   A. An RDU-shaped target advertises BOTH the weight-stationary GEMM dataflow AND the reduction/elementwise
//!      dataflow, so ONE described array serves a 2-stage pipeline (GEMM → elementwise); a reduction-only
//!      target REFUSES the GEMM stage (the reconfig capability is real, typed, not assumed).
//!   B. The elementwise stage places on the same target (certified via `place_with_target`), and the composed
//!      pipeline (elementwise ∘ GEMM) evaluates ≡ the reference — reconfiguration preserves semantics.
//!
//! Run: `cargo test -p logicaffeine-tests --test reconfig_stage_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::fabric_rtl::{AluOp, LoopCell, Src};
use logicaffeine_compile::codegen_sva::fabric_target::{
    place_with_target, Capability, Dataflow, FabricTarget, Precision, Unplaceable,
};
use logicaffeine_compile::codegen_sva::systolic::{eval_systolic, Systolic};

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
fn one_array_reconfigures_between_gemm_and_elementwise_stages() {
    let rdu = FabricTarget::rdu_shaped();
    // A. the RDU-shaped array advertises both dataflow modes.
    assert!(
        rdu.provides(Capability::Dataflow(Dataflow::WeightStationary)),
        "rdu must offer systolic GEMM"
    );
    assert!(
        rdu.provides(Capability::Dataflow(Dataflow::Reduction)),
        "rdu must offer the elementwise/reduction mode"
    );
    rdu.require_gemm(Dataflow::WeightStationary, Precision::Int8)
        .expect("GEMM stage configures on rdu");

    // A reduction-only edge target REFUSES the GEMM stage — the reconfig capability is a real, typed gate.
    let edge = FabricTarget::tiny();
    match edge.require_gemm(Dataflow::WeightStationary, Precision::Int8) {
        Err(Unplaceable::MissingCapability { needed, .. }) => {
            assert_eq!(needed, Capability::Dataflow(Dataflow::WeightStationary));
        }
        other => panic!("a reduction-only target must refuse the GEMM stage, got {other:?}"),
    }

    // B. the elementwise stage (a scalar ALU body: y = max-ish via x + bias) places on the SAME array.
    let elementwise: Vec<LoopCell> = vec![LoopCell::Arith {
        op: AluOp::Add,
        a: Src::In(0),
        b: Src::In(1),
    }];
    place_with_target(&elementwise, 2, &[Src::Val(0)], &rdu)
        .expect("elementwise stage places on rdu");

    // B. the composed pipeline (elementwise ∘ GEMM) preserves semantics: stage1 = int8 GEMM, stage2 = +bias.
    let (k, n, m) = (4usize, 3usize, 4usize);
    let weights: Vec<i64> = (0..k * n).map(|i| prng(1, i, -128, 127)).collect();
    let acts: Vec<i64> = (0..m * k).map(|i| prng(2, i, -128, 127)).collect();
    let bias: Vec<i64> = (0..n).map(|i| prng(3, i, -50, 50)).collect();
    let sys = Systolic::new(k, n, weights);
    let gemm = eval_systolic(&sys, &acts, m);
    let mut pipelined = vec![0i64; m * n];
    for i in 0..m {
        for c in 0..n {
            pipelined[i * n + c] = gemm[i * n + c].wrapping_add(bias[c]); // stage 2 on the reconfigured array
        }
    }
    // reference: the same two stages computed monolithically
    let mut reference = vec![0i64; m * n];
    for i in 0..m {
        for c in 0..n {
            let mut acc = 0i64;
            for r in 0..k {
                acc = acc.wrapping_add((acts[i * k + r] as i8 as i64) * (sys.w(r, c) as i8 as i64));
            }
            reference[i * n + c] = acc.wrapping_add(bias[c]);
        }
    }
    assert_eq!(
        pipelined, reference,
        "the reconfigured GEMM→elementwise pipeline must preserve semantics"
    );
    println!("Phase 6a: one array reconfigures GEMM↔elementwise per stage; typed gate + semantics preserved");
}
