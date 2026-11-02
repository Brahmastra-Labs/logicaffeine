//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! FABRIC-LANES LOCK — the fabric lane is a CONSUMER of the spatial IR, not a sibling of it.
//!
//! The proven fabric artifacts (`Systolic`, `FusedBlock`, the if-converted decision fabrics) LOWER INTO the
//! spatial IR: a weight-stationary GEMM tile is a `MatmulTiles` compute stage with the weight-stationary
//! policy; a fused transformer block is a fused compute chain (matmul → requant → bias → relu) inside the
//! fusion window; a decision fabric is a `Scalar` stage. This lock proves:
//!   A. `from_systolic` produces a weight-stationary program whose evaluation ≡ `eval_systolic`;
//!   B. `from_fused_block` produces the four-op chain, ≡ `eval_fused_block`, and a target whose fusion window
//!      is too small refuses typed through the existing `Capability::Fusion` gate;
//!   C. `from_scalar_body` lowers a decision fabric to a `Scalar` stage ≡ `eval_decision_fabric`, and its
//!      ALU-only body certifies through the existing placer + SAT;
//!   D. output-stationary and weight-stationary policies give identical results;
//!   E. every shape of `mesh_lock` / `tiled_gemm_reuse_lock` passes through the IR unchanged;
//!   F. a spatial lowering refuses an unverified program (typed), so no backend can skip the passes.
//!
//! Run: `cargo test -p logicaffeine-tests --test spatial_fabric_lanes_lower_into_ir_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::codegen_sva::fabric_rtl::{
    decision_fabric_from_source, eval_decision_fabric,
};
use logicaffeine_compile::codegen_sva::systolic::{
    eval_fused_block, eval_systolic, eval_tiled_gemm, FusedBlock, Systolic,
};
use logicaffeine_compile::spatial::certificate::ComputeStageCert;
use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Value};
use logicaffeine_compile::spatial::ir::{ComputeOp, StageKind, TensorId, TensorRole};
use logicaffeine_compile::spatial::lower::{
    fabric::FabricBackend, SpatialBackend, SpatialLowerError,
};
use logicaffeine_compile::spatial::passes::from_tensor_op::{
    from_fused_block, from_scalar_body, from_systolic,
};
use logicaffeine_compile::spatial::passes::SpatialReject;
use logicaffeine_compile::spatial::pipeline::{run_passes, template_from_source};
use logicaffeine_compile::spatial::target::{ComputeCap, DataflowPolicy, SpatialTarget};
use spatial_support::*;

fn ints(v: &Value) -> Vec<i64> {
    match v {
        Value::Int(x) => x.clone(),
        Value::Float(x) => x.iter().map(|f| *f as i64).collect(),
    }
}

#[test]
fn systolic_is_a_weight_stationary_compute_stage_equal_to_eval_systolic() {
    let sky = SpatialTarget::sky_fabric(64, 64);
    for &(k, n, m) in &[
        (8usize, 8usize, 4usize),
        (16, 12, 6),
        (32, 24, 4),
        (5, 7, 3),
    ] {
        let weights: Vec<i64> = (0..k * n).map(|i| prng(1, i, -128, 127)).collect();
        let acts: Vec<i64> = (0..m * k).map(|i| prng(2, i, -128, 127)).collect();
        let sys = Systolic::new(k, n, weights.clone());
        let p = from_systolic(&sys, m, &sky).unwrap();
        let mm = p
            .stages
            .iter()
            .find(|s| matches!(s.kind, StageKind::Compute(ComputeOp::MatmulTiles { .. })))
            .unwrap();
        match &mm.kind {
            StageKind::Compute(ComputeOp::MatmulTiles { dataflow, .. }) => {
                assert_eq!(*dataflow, DataflowPolicy::WeightStationary)
            }
            _ => unreachable!(),
        }
        let w = p
            .tensors
            .iter()
            .find(|t| t.role == TensorRole::Param)
            .unwrap();
        assert_eq!(
            w.init,
            logicaffeine_compile::spatial::ir::TensorInit::Values(weights.clone()),
            "the stationary weights are baked into the program"
        );
        let p = run_passes(p, &sky).unwrap();
        let mut inputs = Inputs::default();
        inputs.tensors.insert(TensorId(0), Value::Int(acts.clone()));
        let out = eval_choreography(&p, &inputs).unwrap();
        assert_eq!(
            ints(&out.tensors[&output_id(&p)]),
            eval_systolic(&sys, &acts, m),
            "{k}x{n} m={m}"
        );
    }
}

#[test]
fn fused_block_is_a_fused_compute_chain_inside_the_fusion_window() {
    let sky = SpatialTarget::sky_fabric(64, 64);
    let (k, n, m) = (6usize, 4usize, 5usize);
    let weights: Vec<i64> = (0..k * n).map(|i| prng(3, i, -110, 110)).collect();
    let acts: Vec<i64> = (0..m * k).map(|i| prng(4, i, -110, 110)).collect();
    let bias: Vec<i64> = (0..n).map(|i| prng(5, i, -400, 400)).collect();
    let fb = FusedBlock {
        sys: Systolic::new(k, n, weights),
        requant_m: 181,
        requant_s: 8,
        bias: bias.clone(),
    };
    let p = from_fused_block(&fb, m, &sky).unwrap();
    let chain: Vec<&str> = p
        .stages
        .iter()
        .filter_map(|s| match &s.kind {
            StageKind::Compute(ComputeOp::MatmulTiles { .. }) => Some("matmul"),
            StageKind::Compute(ComputeOp::Requant { .. }) => Some("requant"),
            StageKind::Compute(ComputeOp::Bias { .. }) => Some("bias"),
            StageKind::Compute(ComputeOp::Relu) => Some("relu"),
            _ => None,
        })
        .collect();
    assert_eq!(
        chain,
        ["matmul", "requant", "bias", "relu"],
        "the four fused ops, in order"
    );
    let p = run_passes(p, &sky).unwrap();
    assert_eq!(p.sites().len(), 1, "the chain is fused onto one site");
    for e in &p.edges {
        assert!(
            e.handshake.is_local(),
            "intermediates stream through local hand-offs, never global memory"
        );
    }
    let mut inputs = Inputs::default();
    inputs.tensors.insert(TensorId(0), Value::Int(acts.clone()));
    let out = eval_choreography(&p, &inputs).unwrap();
    assert_eq!(
        ints(&out.tensors[&output_id(&p)]),
        eval_fused_block(&fb, &acts, m)
    );
    // A target whose fusion window cannot hold the block refuses through the existing capability gate.
    let mut narrow = SpatialTarget::sky_fabric(64, 64);
    for c in narrow.compute.iter_mut() {
        if let ComputeCap::Scalar(ft) = c {
            ft.fusion_window = 1;
        }
    }
    match from_fused_block(&fb, m, &narrow) {
        Err(SpatialReject::MissingCompute { needed, .. }) => {
            assert!(needed.contains("fusion"), "{needed}")
        }
        other => panic!("a fusion window of 1 must refuse a four-op chain: {other:?}"),
    }
}

#[test]
fn a_decision_fabric_is_a_scalar_stage_that_certifies_through_the_existing_sat() {
    let sky = SpatialTarget::sky_fabric(64, 64);
    let cases = [
        ("## To affine (x: Int) -> Int:\n    Return 3 * x + 7.\n", "affine"),
        ("## To absval (a: Int) -> Int:\n    Let result be a.\n    If a is less than 0:\n        Set result to 0 - a.\n    Return result.\n", "absval"),
    ];
    for (src, name) in cases {
        let df = decision_fabric_from_source(src, name);
        let p = from_scalar_body(&df, 64, &sky).unwrap();
        assert!(
            p.stages
                .iter()
                .any(|s| matches!(s.kind, StageKind::Scalar(_))),
            "{name}: a Scalar stage"
        );
        let p = run_passes(p, &sky).unwrap();
        let xs: Vec<i64> = (0..64).map(|i| prng(6, i, -1000, 1000)).collect();
        let mut inputs = Inputs::default();
        inputs.tensors.insert(TensorId(0), Value::Int(xs.clone()));
        let out = eval_choreography(&p, &inputs).unwrap();
        let expected: Vec<i64> = xs
            .iter()
            .map(|x| eval_decision_fabric(&df, &[*x]))
            .collect();
        assert_eq!(ints(&out.tensors[&output_id(&p)]), expected, "{name}");
        let cert = p.certificate.as_ref().unwrap();
        let scalar_cert = cert
            .compute
            .iter()
            .find(|(s, _)| matches!(p.stages[s.idx()].kind, StageKind::Scalar(_)))
            .map(|(_, c)| c)
            .unwrap();
        match scalar_cert {
            ComputeStageCert::Sat(pc) => {
                assert!(pc.recheck(), "{name}: the scalar PE certificate rechecks")
            }
            ComputeStageCert::Uncertified { reason } => {
                panic!("{name}: an ALU/select body must certify, got {reason}")
            }
        }
    }
    let div =
        decision_fabric_from_source("## To half (x: Int) -> Int:\n    Return x / 2.\n", "half");
    let p = run_passes(from_scalar_body(&div, 8, &sky).unwrap(), &sky).unwrap();
    let cert = p.certificate.as_ref().unwrap();
    let scalar_cert = cert
        .compute
        .iter()
        .find(|(s, _)| matches!(p.stages[s.idx()].kind, StageKind::Scalar(_)))
        .map(|(_, c)| c)
        .unwrap();
    assert!(
        matches!(scalar_cert, ComputeStageCert::Uncertified { .. }),
        "a divider body is honestly uncertified by bit-blast"
    );
}

#[test]
fn output_stationary_and_weight_stationary_policies_give_identical_results() {
    let sky = SpatialTarget::sky_fabric(64, 64);
    let (m, n, k) = (12usize, 10usize, 9usize);
    let weights: Vec<i64> = (0..k * n).map(|i| prng(7, i, -128, 127)).collect();
    let acts: Vec<i64> = (0..m * k).map(|i| prng(8, i, -128, 127)).collect();
    let sys = Systolic::new(k, n, weights);
    let mut results = Vec::new();
    for policy in [
        DataflowPolicy::WeightStationary,
        DataflowPolicy::OutputStationary,
    ] {
        let mut t = sky.clone();
        for c in t.compute.iter_mut() {
            if let ComputeCap::MatmulTile { dataflows, .. } = c {
                *dataflows = [policy].into_iter().collect();
            }
        }
        let p = run_passes(from_systolic(&sys, m, &t).unwrap(), &t).unwrap();
        let mm = p
            .stages
            .iter()
            .find(|s| matches!(s.kind, StageKind::Compute(ComputeOp::MatmulTiles { .. })))
            .unwrap();
        assert!(
            matches!(&mm.kind, StageKind::Compute(ComputeOp::MatmulTiles { dataflow, .. }) if *dataflow == policy)
        );
        let mut inputs = Inputs::default();
        inputs.tensors.insert(TensorId(0), Value::Int(acts.clone()));
        results.push(ints(
            &eval_choreography(&p, &inputs).unwrap().tensors[&output_id(&p)],
        ));
    }
    assert_eq!(results[0], results[1]);
    assert_eq!(results[0], eval_systolic(&sys, &acts, m));
}

#[test]
fn the_mesh_and_tiled_reuse_shapes_pass_through_the_ir_unchanged() {
    let sky = SpatialTarget::sky_fabric(64, 64);
    for &(k, n, m, tk, tn) in &[
        (16usize, 16usize, 8usize, 4usize, 4usize),
        (32, 24, 4, 8, 8),
        (12, 12, 6, 3, 3),
    ] {
        let weights: Vec<i64> = (0..k * n).map(|i| prng(1, i, -128, 127)).collect();
        let acts: Vec<i64> = (0..m * k).map(|i| prng(2, i, -128, 127)).collect();
        let sys = Systolic::new(k, n, weights);
        let p = run_passes(from_systolic(&sys, m, &sky).unwrap(), &sky).unwrap();
        let mut inputs = Inputs::default();
        inputs.tensors.insert(TensorId(0), Value::Int(acts.clone()));
        let got = ints(&eval_choreography(&p, &inputs).unwrap().tensors[&output_id(&p)]);
        assert_eq!(got, eval_tiled_gemm(&sys, &acts, m, tk, tn));
        assert_eq!(got, eval_systolic(&sys, &acts, m));
    }
}

#[test]
fn a_lowering_refuses_an_unverified_program() {
    let sky = SpatialTarget::sky_fabric(64, 64);
    let template = template_from_source(&gemm_program(16, 16, 16), &sky).unwrap();
    let backend = FabricBackend::new(sky.clone());
    match backend.lower(&template) {
        Err(SpatialLowerError::NotVerified { .. }) => {}
        other => panic!("a template must not lower — the passes are the contract: {other:?}"),
    }
    let verified = run_passes(template, &sky).unwrap();
    assert!(
        backend.lower(&verified).is_ok(),
        "a verified program lowers"
    );
    assert_eq!(backend.name(), "sky130-fabric");
}
