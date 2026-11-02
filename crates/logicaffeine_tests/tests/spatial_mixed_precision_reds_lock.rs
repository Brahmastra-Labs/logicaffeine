//! spatial_mixed_precision_reds_lock — NUM-1's REDs (fp32 islands in the SN50's bf16 datapath, `benchmarks/PREPARE_FOR_SAMBA.md`),
//! frozen in `fixtures/frozen_reds.tsv` until the islands land.
//!
//! The SN50's products run in bf16, its accumulators in fp32 [SN40L24: the PCU computes in BF16, FP32, INT32 and
//! INT8]. The work a bf16 rounding hurts most — a norm, a softmax, an arg-max over the logits, a gated delta rule's
//! carried state — runs in fp32 beside them, so the step's drift falls to what its bf16 products leave:
//!
//!   • EVERY ISLAND IS FP32: an RMSNorm's output, a softmax's, the logits an arg-max reads and a gated delta rule's
//!     state are fp32 tensors on the SN50, while a product's operands stay bf16;
//!   • THE SN50'S DRIFT FALLS TO WORMHOLE'S: each harness shape's decode step drifts from the VM's logits no further on
//!     the SN50 than an eighth past Wormhole's datapath — fp32 in every unit but the products', the products bf16 —
//!     on the SN50's own geometry. (The Wormhole target itself lowers no decode step: its token gather needs the Int32
//!     it does not have.) The fp32 twin, every unit fp32, drifts by nothing measurable, so the floor is the products'.
//!
//! Run on purpose: `scripts/run-handed-reds-tests.sh`, or
//! `cargo nextest run -p logicaffeine-tests --features handed-reds --cargo-profile test-opt --test spatial_mixed_precision_reds_lock`.

#![cfg(feature = "codegen")]
#![cfg(feature = "handed-reds")]

#[path = "common/model_harness.rs"]
mod model_harness;
#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::model::author;
use logicaffeine_compile::model::reference::{argmax, Reference};
use logicaffeine_compile::model::spec::{Mixer, ModelSpec};
use logicaffeine_compile::spatial::eval::{eval_choreography_every_tensor, Value};
use logicaffeine_compile::spatial::ir::SpatialProgram;
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use model_harness::{carried, checkpoint, deepseek_v3_spec, filled, llama4_spec, qwen35_moe_spec, qwen35_spec, run, CTX, PROMPT};
use spatial_support::inputs_from_fills;

/// The SN50 with Wormhole's datapath: fp32 in every unit but the products', which stay bf16.
fn products_bf16() -> SpatialTarget {
    let mut t = SpatialTarget::sn50_rdu();
    for c in &mut t.compute {
        if let ComputeCap::Elementwise { precisions } | ComputeCap::RowReduce { precisions } = c {
            precisions.insert(Precision::Fp32);
        }
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

/// `src` lowered whole for `target`.
fn lowered(src: &str, target: &SpatialTarget) -> SpatialProgram {
    compile_program_to_spatial(src, target).expect("a tensor program").into_whole().unwrap_or_else(|why| panic!("{why}"))
}

/// The largest distance of `target`'s logits from the VM's, over the largest VM logit.
fn drift(src: &str, vm: &[f64], target: &SpatialTarget) -> f64 {
    let p = lowered(src, target);
    let out = eval_choreography_every_tensor(&p, &inputs_from_fills(&p)).expect("the step evaluates");
    let id = p.tensors.iter().find(|t| t.name == "logits").expect("the logits").id;
    let chip: Vec<f64> = match out.tensors.get(&id).expect("the logits are written") {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    };
    let scale = vm.iter().fold(0.0f64, |m, x| m.max(x.abs())).max(1e-12);
    chip.iter().zip(vm).map(|(c, v)| (c - v).abs()).fold(0.0, f64::max) / scale
}

/// The precision `name` is held in when `src` is lowered for the SN50.
fn sn50_dtype(src: &str, name: &str) -> Precision {
    let p = compile_program_to_spatial(src, &SpatialTarget::sn50_rdu()).expect("a tensor program").program.expect("composes");
    p.tensors.iter().find(|t| t.name == name).unwrap_or_else(|| panic!("the program holds `{name}`")).dtype
}

/// `Let <name> be a R by C tensor of Real.` filled with a spread of values.
fn tensor(name: &str, rows: usize, cols: usize) -> String {
    let mut s = format!("Let {name} be a {rows} by {cols} tensor of Real.\n");
    for i in 0..rows * cols {
        s.push_str(&format!("Set item {} of {name} to {}.\n", i + 1, ((i * 37 % 29) as f64 - 14.0) / 16.0));
    }
    s
}

#[test]
fn every_island_is_fp32() {
    let norm = format!("## Main\n{}{}Let Y be rmsnorm of X with weight W.\nShow item 1 of Y.\n", tensor("X", 2, 64), tensor("W", 1, 64));
    assert_eq!(sn50_dtype(&norm, "Y"), Precision::Fp32, "an RMSNorm's output");
    let softmax = format!("## Main\n{}Let Y be softmax of X.\nShow item 1 of Y.\n", tensor("X", 1, 64));
    assert_eq!(sn50_dtype(&softmax, "Y"), Precision::Fp32, "a softmax's output");
    let pick = format!("## Main\n{}Let t be argmax of L.\nShow t.\n", tensor("L", 1, 256));
    assert_eq!(sn50_dtype(&pick, "L"), Precision::Fp32, "the logits an arg-max reads");
    // A gated delta rule's carried state, in Qwen3.5's decode step.
    let spec = qwen35_spec();
    let (src, _) = step(&spec);
    let p = lowered(&src, &SpatialTarget::sn50_rdu());
    for (l, layer) in spec.layers.iter().enumerate() {
        if matches!(layer.mixer, Mixer::GatedDelta(_)) {
            let state = p.tensors.iter().find(|t| t.name == format!("St{l}")).unwrap_or_else(|| panic!("block {l}'s state"));
            assert_eq!(state.dtype, Precision::Fp32, "block {l}'s gated delta state");
        }
    }
    // Not every tensor: a product's operands stay bf16, its accumulator fp32.
    let product = format!("## Main\n{}{}Let C be A matmul B.\nShow item 1 of C.\n", tensor("A", 32, 64), tensor("B", 64, 32));
    assert_eq!((sn50_dtype(&product, "A"), sn50_dtype(&product, "B")), (Precision::Bf16, Precision::Bf16), "a product's operands");
}

#[test]
fn the_sn50_drift_falls_to_wormholes() {
    for (what, spec) in [
        ("Qwen3.5", qwen35_spec()),
        ("Qwen3.5-MoE", qwen35_moe_spec()),
        ("DeepSeek-V3", deepseek_v3_spec()),
        ("Llama 4", llama4_spec()),
    ] {
        let (src, vm) = step(&spec);
        let (sn50, floor) = (drift(&src, &vm, &SpatialTarget::sn50_rdu()), drift(&src, &vm, &products_bf16()));
        assert!(
            sn50 <= floor * 1.125,
            "{what}: the SN50 drifts {sn50:.6} from the VM's logits, past an eighth over Wormhole's datapath ({floor:.6})"
        );
    }
}
