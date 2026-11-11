//! samba_exam_numerics_lock — the rows of the Samba book's exam E2 (section 10.2 of `benchmarks/PREPARE_FOR_SAMBA.md`)
//! on the SN50's datapath: fp32 islands beside bf16 products (NUM-1).
//!
//! Wormhole's float datapath is fp32 in its pointwise and row units and bf16 or fp16 in its products. The Wormhole target
//! lowers no decode step — its token gather and cache writes need the Int32 it does not have — so its datapath is set on
//! the SN50's geometry beside the SN50's own integers, its floats read off `SpatialTarget::wormhole()` unit by unit, and
//! each harness architecture's decode step is lowered for both.
//!
//! A long prompt is prefilled on the SN50 by the authored prefill program a chunk of its rows at a time, each chunk given
//! the state the one before it left, as E4's rows prefill one (`samba_exam_prefill_lock`).
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test samba_exam_numerics_lock`

#![cfg(feature = "codegen")]

#[path = "cli_common/mod.rs"]
#[allow(dead_code)]
mod cli;
#[path = "common/model_harness.rs"]
mod model_harness;
#[path = "common/samba_exam.rs"]
#[allow(dead_code)]
mod samba_exam;
#[path = "common/samba_handoff.rs"]
#[allow(dead_code)]
mod samba_handoff;
#[path = "common/spatial.rs"]
mod spatial_support;
#[path = "common/tiny_models.rs"]
#[allow(dead_code)]
mod tiny_models;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::model::author;
use logicaffeine_compile::model::reference::{argmax, Reference};
use logicaffeine_compile::model::spec::ModelSpec;
use logicaffeine_compile::spatial::eval::{eval_choreography_every_tensor, Value};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::{precision_is_int, ComputeCap, SpatialTarget};
use logicaffeine_compile::model::program::Cut;
use logicaffeine_compile::spatial::handoff::{contract, Handoff};
use model_harness::{carried, checkpoint, deepseek_v3_spec, filled, llama4_spec, qwen35_moe_spec, qwen35_spec, run, CTX, PROMPT};
use samba_exam::{reference_after, within_the_datapath};
use samba_handoff::{authored, decode_inputs, run_at, Model, NEXT};
use spatial_support::inputs_from_fills;
use std::collections::BTreeMap;
use tiny_models::Tiny;

/// The SN50 with Wormhole's float datapath in each of its units: the floats `SpatialTarget::wormhole()` lists for that
/// unit, beside the SN50's integers.
fn wormholes_datapath() -> SpatialTarget {
    let wormhole = SpatialTarget::wormhole();
    let floats = |of: fn(&ComputeCap) -> bool| -> Vec<Precision> {
        let cap = wormhole.compute.iter().find(|c| of(c)).expect("Wormhole has the unit");
        let (ComputeCap::MatmulTile { precisions, .. } | ComputeCap::Elementwise { precisions } | ComputeCap::RowReduce { precisions }) = cap else {
            unreachable!("a unit with precisions")
        };
        precisions.iter().copied().filter(|p| !precision_is_int(*p)).collect()
    };
    let products = floats(|c| matches!(c, ComputeCap::MatmulTile { .. }));
    let pointwise = floats(|c| matches!(c, ComputeCap::Elementwise { .. }));
    let rows = floats(|c| matches!(c, ComputeCap::RowReduce { .. }));
    let mut t = SpatialTarget::sn50_rdu();
    for c in &mut t.compute {
        let (precisions, theirs) = match c {
            ComputeCap::MatmulTile { precisions, .. } => (precisions, &products),
            ComputeCap::Elementwise { precisions } => (precisions, &pointwise),
            ComputeCap::RowReduce { precisions } => (precisions, &rows),
            _ => continue,
        };
        precisions.retain(|p| precision_is_int(*p));
        precisions.extend(theirs.iter().copied());
    }
    t
}

/// `spec`'s decode step after the prompt, every weight and cache written into it, and the VM's logits.
fn step(spec: &ModelSpec) -> (String, Vec<f64>) {
    let ckpt = checkpoint(spec);
    let mut reference = Reference::new(spec.clone(), ckpt.clone()).expect("reference");
    let tok = argmax(&reference.step(PROMPT).expect("the reference prefills"));
    let caches = carried(spec, &reference);
    let prog = author::decode_step(spec, "/w", CTX, tok as usize + 1, PROMPT.len() + 1);
    let src = filled(spec, &prog, &ckpt, &caches);
    let (vm, _) = run(&src, spec.vocab);
    (src, vm)
}

/// The largest distance of `target`'s logits from the VM's, over the largest VM logit.
fn drift(src: &str, vm: &[f64], target: &SpatialTarget) -> f64 {
    let p = compile_program_to_spatial(src, target).expect("a tensor program").into_whole().unwrap_or_else(|why| panic!("{}: {why}", target.name));
    let out = eval_choreography_every_tensor(&p, &inputs_from_fills(&p)).expect("the step evaluates");
    let id = p.tensors.iter().find(|t| t.name == "logits").expect("the logits").id;
    let chip: Vec<f64> = match out.tensors.get(&id).expect("the logits are written") {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    };
    let scale = vm.iter().fold(0.0f64, |m, x| m.max(x.abs())).max(1e-12);
    chip.iter().zip(vm).map(|(c, v)| (c - v).abs()).fold(0.0, f64::max) / scale
}

/// E2.11 — each harness architecture's decode step drifts from the VM's logits no further on the SN50 than an eighth past
/// the same step on Wormhole's datapath.
#[test]
fn fp32_islands_bring_the_sn50_drift_to_wormholes() {
    let wormhole = wormholes_datapath();
    for (what, spec) in [
        ("Qwen3.5", qwen35_spec()),
        ("Qwen3.5-MoE", qwen35_moe_spec()),
        ("DeepSeek-V3", deepseek_v3_spec()),
        ("Llama 4", llama4_spec()),
    ] {
        let (src, vm) = step(&spec);
        let (sn50, floor) = (drift(&src, &vm, &SpatialTarget::sn50_rdu()), drift(&src, &vm, &wormhole));
        assert!(
            sn50 <= floor * 1.125,
            "{what}: the SN50 drifts {sn50:.6} from the VM's logits, past an eighth over Wormhole's datapath ({floor:.6})"
        );
    }
}

/// The token every position of E2.8's prompt holds, and how many positions.
const REPEATED: (u32, usize) = (7, 4096);

/// A cache past the repeated prompt and the token decoded after it.
const LONG: usize = 4104;

/// The rows a prefill chunk of E2.8's prompt takes.
const CHUNK: usize = 1024;

/// E2.8 — 4,096 repetitions of one token: the prompt prefilled on the SN50 a chunk of its rows at a time, each chunk
/// given the state the one before it left, then the next token decoded there over the 4,096-position cache it left. The
/// logits are the f64 reference's within the datapath's bound, its token where it leads past twice that: the scores of
/// one key under 4,096 rotations, a softmax over all of them and a sum of as many values, on every architecture the
/// exam's rows cover.
#[test]
fn a_long_repetition_decodes_as_the_reference() {
    let sn50 = SpatialTarget::sn50_rdu();
    let (token, n) = REPEATED;
    let prompt = vec![token; n];
    for t in [Tiny::Qwen3, Tiny::Qwen35, Tiny::Gemma3, Tiny::GptOss] {
        let m = Model::write(t.spec(LONG), &format!("{t:?}-repeated"));
        let c = contract(&m.spec, LONG).expect("the contract");
        let named = |s: &str| Some(s.to_string());
        let mut state = BTreeMap::new();
        let p = authored(&m, Cut::Prefill { rows: CHUNK, ctx: LONG }, &sn50);
        for first in (0..n).step_by(CHUNK) {
            let mut given = state.clone();
            // The program counts tokens and positions from 1.
            given.insert("toks".to_string(), prompt[first..first + CHUNK].iter().map(|t| f64::from(t + 1)).collect());
            given.insert("pos_s0".to_string(), vec![(first + 1) as f64]);
            let out = run_at(&m, &p, LONG, &named, &given);
            state = c.carried.iter().map(|t| (t.name.clone(), out[&t.name].clone())).collect();
        }
        let mut given = decode_inputs(&Handoff { position: n, state });
        let x = run_at(&m, &authored(&m, Cut::DecodeEmbed, &sn50), LONG, &named, &given)["x"].clone();
        given.insert("x".into(), x);
        for layer in 0..m.spec.layers.len() {
            let out = format!("xo{layer}");
            let v = run_at(&m, &authored(&m, Cut::DecodeLayer { layer, ctx: LONG }, &sn50), LONG, &named, &given)[&out].clone();
            given.insert(out, v);
        }
        let logits = run_at(&m, &authored(&m, Cut::DecodeHead, &sn50), LONG, &named, &given)["logits"].clone();
        within_the_datapath(&m, &logits, &reference_after(&m, &prompt, NEXT), &format!("{t:?}: the token after 4,096 repetitions"));
    }
}
