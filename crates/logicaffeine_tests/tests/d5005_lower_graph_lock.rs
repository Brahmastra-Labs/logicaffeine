//! The decode engine's program read off a model's step graph (`spatial::decode_engine::lower_graph`): the step
//! `model::author` writes for the model, lifted into the one IR and fused by the shared table under the engine's shim
//! (`spatial::decode_engine::shim::EngineShim`), read block by block into the engine's commands, each merge the engine
//! makes read as the fused symbol the table built for it. It is the hand lowering (`program::hand_lowering`, the oracle) at every knob point, and so is
//! the program the engine is given (`program::lower`, which reads the same step), on the tiny models whose
//! blocks the engine runs, the 27B's header ledgers and the real checkpoints where they are downloaded; and every
//! architecture with a block the engine has no unit for is refused by that block's name.

#![cfg(feature = "codegen")]

mod common;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::collections::{BTreeMap, BTreeSet};

use logicaffeine_compile::model::author;
use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::spec::ModelSpec;
use logicaffeine_compile::spatial::decode::GgufHeader;
use logicaffeine_compile::spatial::decode_engine::capability::spec_refusals;
use logicaffeine_compile::spatial::decode_engine::lower_graph::{lower_graph, LowerError};
use logicaffeine_compile::spatial::decode_engine::model::EngineNumerics;
use logicaffeine_compile::spatial::decode_engine::program::{hand_lowering, lower};
use logicaffeine_compile::spatial::decode_engine::shim::{EngineShim, NO_UNIT};
use logicaffeine_compile::spatial::passes::recognize_stages::{recognize_stages, TensorOpSpec};
use logicaffeine_compile::tensor_graph::fuse::optimize;
use logicaffeine_compile::tensor_graph::graph::{Storage, Sym, TensorGraph};
use logicaffeine_compile::tensor_graph::link::resolve;
use logicaffeine_compile::model::spec::Mixer;
use tiny_models::Tiny;

/// The decode step `spec` authors, as the one IR lifts it.
fn lifted(spec: &ModelSpec) -> TensorGraph {
    author::check(spec).unwrap_or_else(|e| panic!("the author writes {}: {e:?}", spec.arch));
    let (source, _, _) = author::decode_step(spec, "", 64, 1, 1);
    let rp = recognize_stages(&source).unwrap_or_else(|e| panic!("{}: {e:?}", spec.arch));
    TensorGraph::lift(&rp, None, None, &|_| Storage::Values).unwrap_or_else(|e| panic!("{}: {e}", spec.arch))
}

/// The decode step `spec` authors, as the shared table fuses it under the engine's shim.
fn step(spec: &ModelSpec) -> TensorGraph {
    optimize(&lifted(spec), &EngineShim { spec, numerics: EngineNumerics::precise() }, &[]).graph
}

/// The models whose every block the engine runs: the tiny qwen35, qwen3 and llamas, the 27B's header ledgers, and the
/// real checkpoints of those architectures where they are downloaded.
fn engine_models() -> Vec<(String, ModelSpec)> {
    let mut out: Vec<(String, ModelSpec)> = [Tiny::Qwen35, Tiny::Qwen3, Tiny::Llama, Tiny::LlamaTied]
        .into_iter()
        .map(|t| (format!("the tiny {}", t.name()), t.wide(64)))
        .collect();
    for ledger in ["qwen3.8-27b-q4_k_m", "qwen3.8-27b-q8_0"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../benchmarks/silicon/decode/headers/{ledger}.json"));
        out.push((format!("the {ledger} header"), GgufHeader::read_ledger(&path).unwrap().spec().unwrap()));
    }
    for name in ["qwen3.5-0.8b", "qwen3.8-27b/Qwen3.8-27B-Q4_K_M.gguf", "qwen3-8b", "llama3.2-1b"] {
        if common::tool_available(&format!("checkpoint:{name}")) {
            let g = Gguf::open(&common::checkpoint(name).unwrap()).unwrap();
            out.push((name.to_string(), ModelSpec::from_gguf(&g).unwrap()));
        }
    }
    out
}

#[test]
fn the_program_read_off_the_step_graph_is_the_hand_lowering_at_every_knob_point() {
    let points = EngineNumerics::all();
    assert!(points.len() > 1, "every knob point: {}", points.len());
    for (name, spec) in engine_models() {
        let graph = step(&spec);
        for &numerics in &points {
            let read = lower_graph(&graph, &spec, numerics).unwrap_or_else(|e| panic!("{name} at {numerics:?}: {e}"));
            let hand = hand_lowering(&spec, numerics);
            let given = lower(&spec, numerics, 64).unwrap_or_else(|e| panic!("{name} at {numerics:?}: {e}"));
            assert_eq!(given.program, hand, "{name} at {numerics:?}: the program the engine is given");
            assert_eq!(read.commands.len(), hand.commands.len(), "{name} at {numerics:?}");
            for (i, (a, b)) in read.commands.iter().zip(&hand.commands).enumerate() {
                assert_eq!(a, b, "{name} at {numerics:?}: command {i}");
            }
            assert_eq!(read, hand, "{name} at {numerics:?}");
            assert_eq!(read.encode(), hand.encode(), "{name} at {numerics:?}: the sequencer's words");
        }
    }
}

/// What the engine has no unit for in each architecture, as the refusal names it.
fn refused() -> Vec<(Tiny, &'static str)> {
    vec![
        (Tiny::Qwen3Moe, "a mixture of experts"),
        (Tiny::Gemma3, "a scaled embedding"),
        (Tiny::GptOss, "attention sinks"),
        (Tiny::DeepSeek2, "multi-head latent attention"),
        (Tiny::DeepSeek2Split, "multi-head latent attention"),
    ]
}

#[test]
fn every_architecture_with_a_block_the_engine_has_no_unit_for_is_refused_by_its_name() {
    let numerics = EngineNumerics::all()[0];
    for (tiny, what) in refused() {
        let spec = tiny.wide(64);
        match lower_graph(&step(&spec), &spec, numerics) {
            Err(LowerError::Unsupported { what: said, .. }) => assert_eq!(said, what, "{}", tiny.name()),
            other => panic!("{}: {other:?}", tiny.name()),
        }
    }
    let covered: Vec<Tiny> = refused().into_iter().map(|(t, _)| t).chain([Tiny::Qwen35, Tiny::Qwen3, Tiny::Llama, Tiny::LlamaTied]).collect();
    for t in tiny_models::ALL {
        assert!(covered.contains(t), "{}: every tiny architecture lowers or is refused here", t.name());
    }
    for (name, what) in [("gemma3-1b", "a scaled embedding"), ("gpt-oss-20b", "attention sinks"), ("deepseek-v2-lite", "multi-head latent attention")] {
        if common::tool_available(&format!("checkpoint:{name}")) {
            let g = Gguf::open(&common::checkpoint(name).unwrap()).unwrap();
            let spec = ModelSpec::from_gguf(&g).unwrap();
            match lower_graph(&step(&spec), &spec, numerics) {
                Err(LowerError::Unsupported { what: said, .. }) => assert_eq!(said, what, "{name}"),
                other => panic!("{name}: {other:?}"),
            }
        }
    }
}

#[test]
fn a_step_whose_dataflow_strays_from_its_blocks_is_refused_where_it_strays() {
    let spec = Tiny::Qwen35.wide(64);
    let mut graph = step(&spec);
    let numerics = EngineNumerics::all()[0];
    // The first layer's norm reading something other than the embedding's row.
    graph.ops[1].ins[0] = "elsewhere".into();
    match lower_graph(&graph, &spec, numerics) {
        Err(LowerError::Dataflow { op: 1, found, .. }) => assert_eq!(found, "elsewhere"),
        other => panic!("{other:?}"),
    }
    let mut graph = step(&spec);
    let last = graph.ops.len() - 1;
    let extra = graph.ops[last].clone();
    graph.ops.push(extra);
    assert!(matches!(lower_graph(&graph, &spec, numerics), Err(LowerError::Trailing { .. })), "an op past the head");
    let mut graph = step(&spec);
    graph.ops.truncate(5);
    assert!(matches!(lower_graph(&graph, &spec, numerics), Err(LowerError::Ended { .. })), "a step that ends in a block");
}

#[test]
fn a_block_whose_operands_or_shapes_stray_from_the_spec_is_refused_at_them() {
    let spec = Tiny::Qwen35.wide(64);
    let numerics = EngineNumerics::all()[0];
    let at = |graph: &TensorGraph, kind: &str| graph.ops.iter().position(|op| op.kind() == kind).unwrap();
    // The gated product fed something other than the FFN's up projection.
    let mut graph = step(&spec);
    let swiglu = at(&graph, "SwiGlu");
    graph.ops[swiglu].ins[1] = "elsewhere".into();
    match lower_graph(&graph, &spec, numerics) {
        Err(LowerError::Dataflow { op, found, .. }) => assert_eq!((op, found.as_str()), (swiglu, "elsewhere")),
        other => panic!("{other:?}"),
    }
    // A convolution over other taps than the spec's.
    let mut graph = step(&spec);
    let conv = at(&graph, "conv_bias_silu");
    let Sym::Fused(f) = &mut graph.ops[conv].sym else { unreachable!() };
    let Sym::Prim(TensorOpSpec::CausalConv(c)) = &mut f.parts[0].sym else { unreachable!() };
    c.taps += 1;
    match lower_graph(&graph, &spec, numerics) {
        Err(LowerError::Binding { why, .. }) => assert!(why.contains("taps"), "{why}"),
        other => panic!("{other:?}"),
    }
    // The query and key projections reading each other's rows: one matrix, but its ranges out of order.
    let mut graph = step(&spec);
    let merged = conv - 2;
    assert_eq!(graph.ops[merged].kind(), "merged_qkv", "the block's projections precede its convolutions");
    let Sym::Fused(f) = &mut graph.ops[merged].sym else { unreachable!() };
    let (wq, wk) = (f.parts[0].ins[1].clone(), f.parts[1].ins[1].clone());
    (f.parts[0].ins[1], f.parts[1].ins[1]) = (wk, wq);
    match lower_graph(&graph, &spec, numerics) {
        Err(LowerError::Binding { why, .. }) => assert!(why.starts_with("rows from"), "{why}"),
        other => panic!("{other:?}"),
    }
}

/// MIGRATE-4: the engine reads the shared table's fused symbols. On every model whose blocks the engine runs, the step
/// fused under the engine's shim holds exactly the merges the engine makes — a Gated-DeltaNet layer's query, key and
/// value as one `merged_qkv`, each of its three convolutions with its SiLU as a `conv_bias_silu`, its gates as a
/// `gdn_gating`; a gated attention layer's queries and gate as one `merged_qkv` and its output gate as a `glu_act_mul` —
/// and nothing else; and the reader takes each merge by that symbol, with no graph of the lane's.
#[test]
fn the_engine_reads_fused_symbols() {
    let reader = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile/src/spatial/decode_engine/lower_graph.rs")).unwrap();
    assert!(!reader.contains("StepGraph") && !reader.contains("lane::graph"), "the engine reads the one IR, not the lane's graph");
    for class in ["ClassId::MergedQkv", "ClassId::ConvBiasSilu", "ClassId::GdnGating", "ClassId::GluActMul"] {
        assert!(reader.contains(&format!("take_fused({class})")), "the reader takes {class} as the unit that runs it");
    }
    let mut merges = 0usize;
    for (name, spec) in engine_models() {
        let mut want: BTreeMap<String, usize> = BTreeMap::new();
        for layer in &spec.layers {
            match &layer.mixer {
                Mixer::GatedDelta(_) => {
                    for (class, n) in [("merged_qkv", 1), ("conv_bias_silu", 3), ("gdn_gating", 1)] {
                        *want.entry(class.into()).or_default() += n;
                    }
                }
                Mixer::Attention(a) if a.output_gate => {
                    for class in ["merged_qkv", "glu_act_mul"] {
                        *want.entry(class.into()).or_default() += 1;
                    }
                }
                _ => {}
            }
        }
        let mut got: BTreeMap<String, usize> = BTreeMap::new();
        for op in &step(&spec).ops {
            if let Sym::Fused(f) = &op.sym {
                *got.entry(f.class.id().to_string()).or_default() += 1;
            }
        }
        assert_eq!(got, want, "{name}: the fused symbols are the engine's merges, block by block");
        merges += got.values().sum::<usize>();
    }
    assert!(merges > 0, "the engine's models hold merges");
}

/// MIGRATE-4: the capability module is the engine's shim. Every tiny architecture the capability module refuses has a
/// step the engine's shim refuses, and one it admits a step the shim resolves whole; and the shim refuses each op as
/// having no unit for what the capability module names it by — or, for an op no checkpoint feature names, for its kind.
#[test]
fn capability_is_the_engine_shim() {
    let mut refused = 0usize;
    for t in tiny_models::ALL {
        let spec = t.wide(64);
        let said: BTreeSet<String> = spec_refusals(&spec).iter().map(|r| r.gap()).collect();
        let shim = EngineShim { spec: &spec, numerics: EngineNumerics::precise() };
        match resolve(&lifted(&spec), &shim) {
            Ok(_) => assert!(said.is_empty(), "{}: the capability module refuses {said:?}, the shim resolves the step", t.name()),
            Err(unresolved) => {
                assert!(!said.is_empty(), "{}: the shim refuses a step the capability module admits: {unresolved:?}", t.name());
                let named = |why: &str| why.strip_prefix(NO_UNIT).is_some_and(|what| said.contains(what));
                for u in &unresolved {
                    assert!(
                        named(&u.why) || u.why == format!("{NO_UNIT}a {}", u.kind),
                        "{}: op {} ({}) is refused in words the capability module does not say: `{}`; it says {said:?}",
                        t.name(),
                        u.op,
                        u.kind,
                        u.why
                    );
                }
                assert!(unresolved.iter().any(|u| named(&u.why)), "{}: the shim names one of the capability module's refusals", t.name());
                refused += 1;
            }
        }
    }
    assert!(refused >= 4, "the tiny architectures hold refusals: {refused}");
}
