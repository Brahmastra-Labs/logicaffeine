//! spatial_mixed_precision_lock — two widths of one float in one program (NUM-1 of `benchmarks/PREPARE_FOR_SAMBA.md`).
//!
//! A section may read a tensor wider than its neighbour reads it — Wormhole's fp32 elementwise datapath beside its bf16
//! product operands, or an fp32 island beside the SN50's bf16 datapath. A tensor the program writes holds what its
//! producer wrote and a narrower reader rounds it on the way in; one only readers declare is held at the wider of their
//! widths, the narrower reader rounding it the same way. NUM-1's islands and their drift are
//! `spatial_mixed_precision_reds_lock`'s.
//!
//! A chip whose every unit takes fp32 beside bf16 (the SN50's fp32 twin) computes a model's decode step as the f64
//! reference does, to fp32's rounding: what the SN50 drifts past that is its bf16 datapath's, not a lowering's.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test spatial_mixed_precision_lock`

#![cfg(feature = "codegen")]

#[path = "common/spatial.rs"]
mod spatial_support;
mod common;
#[path = "cli_common/mod.rs"]
mod cli;
#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "common/samba_handoff.rs"]
mod samba_handoff;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography_every_tensor, Value};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::model::reference::Reference;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use samba_handoff::{decode_on_layers, prefill_on, Model, Opened, NEXT, PROMPT};
use spatial_support::inputs_from_fills;

/// The SN50 with fp32 in place of bf16 in every unit: a float matmul takes the narrowest float a unit offers, so fp32
/// beside bf16 would still multiply in bf16.
fn fp32_twin() -> SpatialTarget {
    let mut t = SpatialTarget::sn50_rdu();
    for c in &mut t.compute {
        if let ComputeCap::MatmulTile { precisions, .. } | ComputeCap::Elementwise { precisions } | ComputeCap::RowReduce { precisions } = c {
            if precisions.remove(&Precision::Bf16) {
                precisions.insert(Precision::Fp32);
            }
        }
    }
    t.precisions.remove(&Precision::Bf16);
    t.precisions.insert(Precision::Fp32);
    t
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
fn an_input_two_widths_read_is_held_at_the_wider() {
    // On Wormhole a product reads its operands in bf16 and a sum reads its in fp32. `X` is both, and nothing in the
    // program writes it: it is held at fp32, the wider of its readers', and the product's read rounds it on the way in,
    // as a read below a wider producer does. The program composes and computes what the VM computes.
    let src = format!(
        "## Main\n{}{}Let Y be X matmul W.\nLet Z be X plus X.\nShow item 1 of Y.\nShow item 1 of Z.\n",
        tensor("X", 32, 64),
        tensor("W", 64, 32)
    );
    let p = compile_program_to_spatial(&src, &SpatialTarget::wormhole())
        .expect("a tensor program")
        .into_whole()
        .unwrap_or_else(|why| panic!("{why}"));
    assert_eq!(p.tensors.iter().find(|t| t.name == "X").expect("X").dtype, Precision::Fp32, "the input is held at the wider width");
    let out = eval_choreography_every_tensor(&p, &inputs_from_fills(&p)).expect("the program evaluates");
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "the VM runs it: {:?}", vm.error);
    let vm: Vec<f64> = vm.output.split_whitespace().map(|v| v.parse().expect("a number")).collect();
    for (name, want) in [("Y", vm[0]), ("Z", vm[1])] {
        let id = p.tensors.iter().find(|t| t.name == name).expect("the result").id;
        let got = match out.tensors.get(&id).expect("written") {
            Value::Float(v) => v[0],
            Value::Int(v) => v[0] as f64,
        };
        assert!((got - want).abs() <= want.abs().max(1.0) / 128.0, "{name}: {got} on Wormhole, the VM's {want}");
    }
}

#[test]
fn qwen3next_decodes_on_the_fp32_twin_as_the_reference_computes() {
    // Qwen3-Next's own checkpoint (its tiny twin, value heads grouped by key head as its converter writes them, opened
    // through the presentation layer): the prompt prefilled and the next token decoded with the embedding, every layer
    // and the head lowered for the SN50's fp32 twin. Its logits are the f64 reference's to fp32's rounding. A misread
    // layout, a widened gated delta, the gated shared expert or the partial rotation lowered wrong moves a logit by its
    // own size.
    if !common::require_tool("hf-twins") {
        return;
    }
    let (_, gguf) = common::hf_twins::twin("qwen3next");
    let m = Model::open(&gguf);
    let twin = fp32_twin();
    let logits = decode_on_layers(&m, &twin, &prefill_on(&m, &twin, &PROMPT));
    let source = logicaffeine_compile::model::open_checkpoint(&gguf).expect("the checkpoint");
    let mut reference = Reference::new(m.spec.clone(), Opened(source)).expect("the reference reads the checkpoint");
    reference.step(&PROMPT).expect("the reference prefills");
    let want = reference.step(&[NEXT]).expect("the reference steps");
    let scale = want.iter().fold(0.0f64, |acc, v| acc.max(v.abs()));
    let worst = logits.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(worst <= 1e-4 * scale, "a logit is {worst:e} from the reference's, past fp32's rounding of {scale}");
}

