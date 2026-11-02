//! rdu_bringup_lock — every architecture the author writes brings up on the SN50 from its checkpoint alone (INTAKE-6 of
//! `benchmarks/PREPARE_FOR_SAMBA.md`).
//!
//! Each tiny model of `common/tiny_models.rs` whose layers the spatial lowering takes is written as a GGUF checkpoint (its weights in the formats a converter
//! stores them, its metadata a converter's), handed to `largo emit rdu --chip sn50 --model <checkpoint> --ctx 4 -o DIR`,
//! and the bundle is read back off disk, re-certified, and run for one decode step through the spatial evaluator with
//! every tensor filled as a host fills it from the checkpoint; its logits are the f64 reference's within the bf16
//! datapath's stated bound, and its token the reference's wherever the reference's lead exceeds that bound
//! (`common/samba_bringup.rs`, `brings_up_close`). A row over several architectures names every one that fails, and why.
//!
//! The deployment is one program per layer KIND: layers whose authored programs are one program up to the layer they
//! name (`spatial::streamed::layer_kinds`, by digest) share one lowered program, bound to each layer's tensors.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test rdu_bringup_lock`

#![cfg(feature = "codegen")]

#[path = "cli_common/mod.rs"]
mod cli;
#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "common/samba_bringup.rs"]
mod samba_bringup;

use logicaffeine_compile::model::program::{written, Cut};
use logicaffeine_compile::model::spec::{Act, Attention, Ffn, Mixer, ModelSpec};
use logicaffeine_compile::spatial::streamed::layer_kinds;
use samba_bringup::{brings_up_close, every, CTX};
use tiny_models::{Tiny, ALL};

/// The tiny architectures every layer of which the spatial lowering takes today; the rest are handed REDs in
/// `rdu_bringup_reds_lock`, each moving here as its lowering lands.
const LOWERED: [Tiny; 7] = [Tiny::Qwen3, Tiny::Llama, Tiny::LlamaTied, Tiny::Qwen3Moe, Tiny::Qwen35, Tiny::Gemma3, Tiny::GptOss];

#[test]
fn every_architecture_the_spatial_lowering_takes_brings_up_on_the_sn50() {
    every(LOWERED.iter().map(|&t| (format!("{t:?}"), t)), |t| brings_up_close(t.spec(CTX), &format!("{t:?}")));
}

#[test]
fn every_tiny_architecture_brings_up_on_the_sn50() {
    every(ALL.iter().map(|&t| (format!("{t:?}"), t)), |t| brings_up_close(t.spec(CTX), &format!("{t:?}")));
}

#[test]
fn every_dense_architecture_the_spatial_lowering_takes_brings_up_four_times_as_wide() {
    every(LOWERED.iter().filter(|t| **t != Tiny::Qwen3Moe).map(|&t| (format!("{t:?} four times as wide"), t)), |t| {
        brings_up_close(t.wide(CTX), &format!("{t:?} four times as wide"))
    });
}

/// The kinds `streamed::layer_kinds` finds in `spec`'s decode step, every declared Q8_0 weight stored Q8_0: each kind's
/// name and its layers.
fn kinds(spec: &ModelSpec) -> Vec<(String, Vec<usize>)> {
    layer_kinds(spec, &|l| {
        let ((src, _, q8), _) = written(spec, Cut::DecodeLayer { layer: l, ctx: CTX });
        (src, q8.into_keys().collect())
    })
}

#[test]
fn kind_programs_are_found_by_digest() {
    let owned = |v: Vec<(&str, Vec<usize>)>| v.into_iter().map(|(n, l)| (n.to_string(), l)).collect::<Vec<_>>();
    assert_eq!(kinds(&Tiny::Qwen35.spec(CTX)), owned(vec![("gated deltanet", vec![0, 1]), ("attention", vec![2])]));
    assert_eq!(kinds(&Tiny::Llama.spec(CTX)), owned(vec![("attention", vec![0, 1, 2])]));
    // Gemma 3: two windowed layers turning by their own base, then a global one.
    let gemma = kinds(&Tiny::Gemma3.spec(CTX));
    assert_eq!(gemma.iter().map(|(_, l)| l.clone()).collect::<Vec<_>>(), vec![vec![0, 1], vec![2]], "{gemma:?}");
    assert!(gemma[0].0.starts_with("attention, window ") && gemma[1].0 == "attention", "{gemma:?}");
    // DeepSeek-V2: a dense first layer, then the expert layers.
    assert_eq!(
        kinds(&Tiny::DeepSeek2.spec(CTX)),
        owned(vec![("latent attention", vec![0]), ("latent attention, experts", vec![1, 2])])
    );
    // A deep stack of one pattern is as many kinds as the pattern, however deep.
    let mut deep = Tiny::Qwen35.spec(CTX);
    let period = deep.layers.len();
    deep.layers = (0..61).map(|l| deep.layers[l % period].clone()).collect();
    let k = kinds(&deep);
    assert_eq!(k.len(), 2, "{k:?}");
    assert_eq!(k[0].1.len() + k[1].1.len(), 61);
}

#[test]
fn two_layers_that_differ_in_one_op_are_two_kinds() {
    let mut spec = Tiny::Llama.spec(CTX);
    spec.layers[1].ffn = match spec.layers[1].ffn.clone() {
        Ffn::Dense { dff, .. } => Ffn::Dense { dff, act: Act::GeluTanh },
        other => panic!("the tiny Llama's FFN is dense: {other:?}"),
    };
    let k = kinds(&spec);
    assert_eq!(k.iter().map(|(_, l)| l.clone()).collect::<Vec<_>>(), vec![vec![0, 2], vec![1]], "{k:?}");
    assert_ne!(k[0].0, k[1].0, "two kinds, two names");
}

#[test]
fn a_model_whose_every_layer_differs_is_one_program_a_layer() {
    // Four layers, each its own kind: a GDN layer, a windowed and a global attention, an attention without a rotation.
    let hybrid = Tiny::Qwen35.spec(CTX);
    let gemma = Tiny::Gemma3.spec(CTX);
    let mut spec = Tiny::Llama.spec(CTX);
    let Mixer::Attention(a) = spec.layers[0].mixer.clone() else { panic!("attention") };
    spec.layers.push(spec.layers[0].clone());
    spec.layers[1].mixer = Mixer::Attention(Attention { window: gemma.layers.iter().find_map(|l| match &l.mixer {
        Mixer::Attention(g) => g.window,
        _ => None,
    }), ..a.clone() });
    spec.layers[2].mixer = Mixer::Attention(Attention { rope: None, ..a.clone() });
    spec.layers[3].mixer = hybrid.layers[0].mixer.clone();
    let k = kinds(&spec);
    assert_eq!(k.len(), 4, "{k:?}");
    let names: std::collections::BTreeSet<&String> = k.iter().map(|(n, _)| n).collect();
    assert_eq!(names.len(), 4, "every kind has its own name: {k:?}");
}

#[test]
fn interleaved_dense_and_expert_layers_are_their_own_kinds() {
    // Llama 4's pattern: dense and expert FFNs interleaved, every fourth layer without a rotation.
    let moe = Tiny::Qwen3Moe.spec(CTX);
    let dense = Tiny::Qwen3.spec(CTX);
    let (Mixer::Attention(a), expert) = (&moe.layers[0].mixer, moe.layers[0].ffn.clone()) else { panic!("attention") };
    let mut spec = moe.clone();
    spec.layers = (0..8)
        .map(|l| {
            let mut layer = moe.layers[0].clone();
            layer.ffn = if l % 2 == 0 { dense.layers[0].ffn.clone() } else { expert.clone() };
            if l % 4 == 3 {
                layer.mixer = Mixer::Attention(Attention { rope: None, ..a.clone() });
            }
            layer
        })
        .collect();
    let k = kinds(&spec);
    let groups: Vec<Vec<usize>> = k.iter().map(|(_, l)| l.clone()).collect();
    assert_eq!(groups, vec![vec![0, 2, 4, 6], vec![1, 5], vec![3, 7]], "{k:?}");
}

#[test]
fn a_rotary_table_is_every_layers_and_a_weight_its_own() {
    use logicaffeine_compile::spatial::streamed::rebind;
    // A rotary table's suffix is its rotation, which every layer turning by it reads.
    for table in ["Rcos", "Rsin", "Rcos1", "Rsin1", "RC", "RS1"] {
        for (from, to) in [(0, 1), (1, 2), (1, 7)] {
            assert_eq!(rebind(table, from, to), table, "`{table}` from layer {from} to {to}");
        }
    }
    assert_eq!(rebind("Wq1", 1, 2), "Wq2");
    assert_eq!(rebind("Kc1", 1, 7), "Kc7");
    assert_eq!(rebind("xn1__xsq", 1, 3), "xn3__xsq");
    assert_eq!(rebind("x", 0, 2), "xo1", "the hidden state a layer reads is named by position");
}

#[test]
fn layers_that_read_a_weight_differently_are_two_kinds() {
    // One program, but the checkpoint stores layer 1's query projection Q8_0 and layer 0's another way.
    let spec = Tiny::Llama.spec(CTX);
    let k = layer_kinds(&spec, &|l| {
        let ((src, _, q8), _) = written(&spec, Cut::DecodeLayer { layer: l, ctx: CTX });
        (src, q8.into_keys().filter(|v| l != 0 || !v.starts_with("Wq")).collect())
    });
    assert_eq!(k.iter().map(|(_, l)| l.clone()).collect::<Vec<_>>(), vec![vec![0], vec![1, 2]], "{k:?}");
}

#[test]
fn a_renaming_is_one_to_one_and_keeps_every_other_token() {
    use logicaffeine_compile::spatial::streamed::renaming;
    let map = renaming(
        "Let x be a 1 by 4 tensor of Real.\nLet W1 be a 4 by 4 tensor of Real.\nLet y be x matmul W1.\nLet z be y plus 2.",
        "Let u be a 1 by 4 tensor of Real.\nLet W4 be a 4 by 4 tensor of Real.\nLet v be u matmul W4.\nLet w be v plus 2.",
    )
    .expect("one program");
    assert_eq!(map.get("W1").map(String::as_str), Some("W4"));
    assert_eq!(map.get("y").map(String::as_str), Some("v"));
    assert_eq!(map.get("Let"), None, "a keyword is the language's, not a name the program binds");
    // Two names of one program are never one name of the other, nor one name two.
    assert_eq!(renaming("Let a be b plus c.", "Let a be b plus b."), None, "two names merged");
    assert_eq!(renaming("Let a be b plus b.", "Let a be b plus c."), None, "one name split");
    // A literal, a symbol, an operation and a name the program does not bind are kept exactly.
    assert_eq!(renaming("Let a be b plus 2.", "Let a be b plus 3."), None, "a literal differs");
    assert_eq!(renaming("Let a be b plus c.", "Let a be b times c."), None, "an operation differs");
    assert_eq!(renaming("Let a be silu of b.", "Let a be gelu of b."), None, "a function differs");
    assert_eq!(renaming("Let a be b plus c.", "Let d be b plus c."), Some([("a".to_string(), "d".to_string())].into()), "a bound name renames");
    assert_eq!(renaming("Let a be b plus c.", "Let a be e plus c."), None, "a name the program reads but does not bind is kept");
    assert_eq!(renaming("Let a be b.", "Let a be b plus c."), None, "a statement more");
}

#[test]
fn two_bound_names_are_never_one_name_of_the_other_program() {
    use logicaffeine_compile::spatial::streamed::renaming;
    let two = "Let a be 1.\nLet b be 2.\nLet c be a plus b.";
    let one = "Let x be 1.\nLet x be 2.\nLet c be x plus x.";
    // Every name the left binds goes to one name of the right, so only the way back can see two left names meet.
    assert_eq!(renaming(two, one), None, "a and b both renamed x");
    assert_eq!(renaming(one, two), None, "x renamed both a and b");
    assert_eq!(
        renaming(two, "Let x be 1.\nLet y be 2.\nLet c be x plus y."),
        Some([("a", "x"), ("b", "y"), ("c", "c")].map(|(k, v)| (k.to_string(), v.to_string())).into()),
        "two names stay two"
    );
}
