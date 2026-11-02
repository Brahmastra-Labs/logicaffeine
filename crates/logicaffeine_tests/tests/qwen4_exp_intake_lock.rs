//! qwen4_exp_intake_lock — Qwen3.8-Flash-Next (`qwen4_exp`, llama.cpp's `qwen4exp`; QWEN4-1 of
//! `benchmarks/PREPARE_FOR_SAMBA.md`) read, bound and golden.
//!
//! Its twin (`scripts/accel/hf-tiny-twin.py qwen4exp`: four hyper-connection lanes over a rank of 16, QSA over blocks
//! of four with a budget of eight tokens, the n-gram embedding on layer 2, Gated DeltaNet's sigmoid gate, Qwen's
//! mixture in every block, the vision tower and the draft block) is read from its directory as llama.cpp's converter
//! writes it — every key and every tensor bit for bit, nothing left unread — into a spec that carries every key of its
//! configuration; the f64 reference's step reads every tensor but the draft block's; and the reference computes what
//! transformers computes at positions of a prompt that crosses an end of document and outgrows the budget, run whole
//! and a token at a time. Each of its parts the author does not write yet is refused by name, so no program computes
//! Qwen3.5's model in its place.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test qwen4_exp_intake_lock`

#![cfg(feature = "codegen")]

mod common;
#[path = "cli_common/mod.rs"]
mod cli;

use logicaffeine_compile::model::gguf::{Gguf, MetaValue};
use logicaffeine_compile::model::hf::HfCheckpoint;
use logicaffeine_compile::model::quant::GgmlType;
use logicaffeine_compile::model::reference::{Reference, WeightSource};
use logicaffeine_compile::model::safetensors::{self, Dtype, Shard};
use logicaffeine_compile::model::spec::{
    Ffn, GateActivation, HyperConnections, Indexer, Mixer, ModelSpec, SpecSource,
};
use logicaffeine_compile::model::{author, open_checkpoint, Checkpoint};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// A prompt past the budget with an end of document third, so the n-grams of the next positions reach no further back.
const PROMPT: [u32; 17] = [3, 17, 248044, 5, 9, 11, 13, 2, 7, 19, 23, 29, 31, 37, 41, 43, 47];

/// Before and after the end of document (0 to 3); the last position with every complete block kept (7: two blocks);
/// the first that chooses (11: three blocks, two kept) and those after it, to the last (16: four blocks, two kept, and
/// the position after them).
const AT: [usize; 8] = [0, 1, 2, 3, 7, 11, 12, 16];

fn twin() -> Option<(PathBuf, PathBuf)> {
    common::require_tool("hf-twins").then(|| common::hf_twins::twin("qwen4exp"))
}

fn text_config(dir: &Path) -> serde_json::Value {
    let config: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).unwrap()).unwrap();
    config["text_config"].clone()
}

fn n(c: &serde_json::Value, key: &str) -> usize {
    c[key].as_u64().unwrap_or_else(|| panic!("config.json: `{key}`")) as usize
}

/// An integer tensor of the checkpoint, as stored.
fn stored_i64(dir: &Path, suffix: &str) -> Vec<u64> {
    for p in safetensors::shard_paths(dir).unwrap() {
        let shard = Shard::open(&p).unwrap();
        if let Some((name, e)) = shard.tensors.iter().find(|(n, _)| n.ends_with(suffix)) {
            assert_eq!(e.dtype, Dtype::I64, "{name} is stored as int64");
            let bytes = shard.read(name, 0, (e.end - e.begin) as usize).unwrap();
            return bytes.chunks(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect();
        }
    }
    panic!("no tensor ending `{suffix}`")
}

/// The positions of the stack's QSA layers: `full_attention` as Qwen publishes them, `indexed_attention` as transformers
/// saves them.
fn qsa_layers(c: &serde_json::Value) -> Vec<usize> {
    let types = c["layer_types"].as_array().unwrap();
    (0..types.len()).filter(|&l| matches!(types[l].as_str(), Some("full_attention" | "indexed_attention"))).collect()
}

/// The converter reads the configuration through transformers, which names a QSA layer `indexed_attention`, and matches
/// `full_attention` alone: its GGUF gives the stack's QSA layers a compression ratio of 0, so llama.cpp attends them
/// densely; the draft block keeps its ratio.
const CONVERTER_DROPS_QSA: &str = "the converter's GGUF selects on no layer of the stack";

#[test]
fn the_spec_carries_every_configuration_key() {
    let Some((dir, gguf)) = twin() else { return };
    let c = text_config(&dir);
    let spec = ModelSpec::from_source(&*open_checkpoint(&dir).unwrap()).unwrap_or_else(|e| panic!("{e:?}"));
    let mut converted = ModelSpec::from_gguf(&Gguf::open(&gguf).unwrap()).unwrap();
    assert_eq!(converted.indexer.as_ref().map(|ix| ix.layers.clone()), Some(Vec::new()), "{CONVERTER_DROPS_QSA}");
    converted.indexer = spec.indexer.clone();
    assert_eq!(spec, converted, "the directory reads as its converted GGUF, QSA's layers apart");

    // As transformers saves the configuration, its QSA layers named `indexed_attention`: the same model.
    let saved = tempfile::tempdir().unwrap();
    for e in std::fs::read_dir(&dir).unwrap() {
        let e = e.unwrap();
        if e.file_name() != "config.json" {
            std::os::unix::fs::symlink(e.path(), saved.path().join(e.file_name())).unwrap();
        }
    }
    let mut config: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).unwrap()).unwrap();
    for t in config["text_config"]["layer_types"].as_array_mut().unwrap() {
        if t == "full_attention" {
            *t = serde_json::json!("indexed_attention");
        }
    }
    std::fs::write(saved.path().join("config.json"), config.to_string()).unwrap();
    let resaved = ModelSpec::from_source(&*open_checkpoint(saved.path()).unwrap()).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(resaved, spec, "a configuration as transformers saves it reads as the published one");

    assert_eq!(spec.arch, "qwen4exp");
    assert_eq!((spec.d, spec.vocab), (n(&c, "hidden_size"), n(&c, "vocab_size")), "the width and the vocabulary");
    assert!(!spec.tied_output, "the head is its own");
    assert_eq!(spec.mtp_blocks, n(&c, "mtp_num_hidden_layers"), "the draft blocks");
    assert_eq!(spec.hyper, Some(HyperConnections { lanes: n(&c, "hc_count"), rank: n(&c, "hc_lowrank") }));
    assert_eq!(
        spec.indexer,
        Some(Indexer {
            heads: n(&c, "indexer_n_heads"),
            head_dim: n(&c, "indexer_head_dim"),
            budget: n(&c, "indexer_budget"),
            block: n(&c, "indexer_compress_ratio"),
            layers: qsa_layers(&c),
        })
    );
    assert!(!qsa_layers(&c).is_empty(), "the twin has a QSA layer");

    let ple = spec.ple.clone().expect("the n-gram embedding");
    let ple_layers: Vec<usize> = c["ple_layer_ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as usize - 1).collect();
    assert_eq!(vec![ple.layer], ple_layers, "the layer it joins, counted from 0");
    assert_eq!((ple.ngram, ple.heads_per_ngram, ple.conv_taps), (n(&c, "ngram_size"), n(&c, "heads_per_ngram"), n(&c, "ple_conv_kernel_size")));
    assert_eq!(ple.eos as usize, n(&c, "eos_token_id"), "the end of document");
    assert_eq!(ple.head_dim, n(&c, "ple_embed_dim") / ((ple.ngram - 1) * ple.heads_per_ngram), "a head's row");
    assert_eq!(ple.multipliers, stored_i64(&dir, "ple_embedding.layer_multipliers"), "the multipliers");
    assert_eq!(ple.head_offsets, stored_i64(&dir, "ple_embedding.ngram_heads_offsets"), "the heads' offsets");
    assert_eq!(ple.head_vocab, stored_i64(&dir, "ple_embedding.ngram_heads_vocab_sizes"), "the heads' moduli");

    let types: Vec<&str> = c["layer_types"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(spec.layers.len(), types.len(), "the stack");
    for (l, (layer, ty)) in spec.layers.iter().zip(&types).enumerate() {
        match (&layer.mixer, *ty) {
            (Mixer::GatedDelta(g), "linear_attention") => {
                assert_eq!(g.gate, GateActivation::Sigmoid, "layer {l}: the gate");
                assert_eq!(
                    (g.k_heads, g.v_heads, g.head_dim, g.conv_taps),
                    (n(&c, "linear_num_key_heads"), n(&c, "linear_num_value_heads"), n(&c, "linear_key_head_dim"), n(&c, "linear_conv_kernel_dim")),
                    "layer {l}: the Gated DeltaNet"
                );
            }
            (Mixer::Attention(a), "full_attention") => {
                assert!(a.output_gate && a.qk_norm, "layer {l}: the gated attention and its QK norms");
                assert_eq!((a.heads, a.kv_heads, a.head_dim), (n(&c, "num_attention_heads"), n(&c, "num_key_value_heads"), n(&c, "head_dim")));
                let rotated = (n(&c, "head_dim") as f64 * c["rope_parameters"]["partial_rotary_factor"].as_f64().unwrap()) as usize;
                assert_eq!(a.rope.as_ref().map(|r| r.dims), Some(rotated), "layer {l}: the rotated share");
            }
            (m, ty) => panic!("layer {l} is `{ty}` but reads as {m:?}"),
        }
        match &layer.ffn {
            Ffn::Moe(m) => assert_eq!(
                (m.experts, m.top_k, m.expert_dff, m.shared_dff),
                (n(&c, "num_experts"), n(&c, "num_experts_per_tok"), n(&c, "moe_intermediate_size"), n(&c, "shared_expert_intermediate_size")),
                "layer {l}: the mixture"
            ),
            other => panic!("layer {l}: a mixture, not {other:?}"),
        }
    }
}

/// A metadata value as a comparable form: integers whatever their width, floats as their f32 bits where the file
/// stores f32, strings, bools and arrays element by element.
fn norm(v: &MetaValue) -> String {
    match v {
        MetaValue::F32(f) => format!("f32:{:08x}", f.to_bits()),
        MetaValue::F64(f) => format!("f32:{:08x}", (*f as f32).to_bits()),
        MetaValue::Bool(b) => format!("bool:{b}"),
        MetaValue::Str(s) => format!("str:{}", String::from_utf8_lossy(s)),
        MetaValue::Array { items, .. } => format!("[{}]", items.iter().map(norm).collect::<Vec<_>>().join(",")),
        other => match other.as_u64() {
            Some(u) => format!("int:{u}"),
            None => format!("{other:?}"),
        },
    }
}

fn values(c: &mut dyn WeightSource, name: &str) -> Vec<f32> {
    let (ty, dims) = c.describe(name).unwrap();
    let rows: u64 = dims.iter().skip(1).product();
    let raw = c.row_bytes(name, 0, rows as usize).unwrap();
    let mut out = Vec::new();
    ty.dequantize(&raw, &mut out);
    out
}

/// The directory's checkpoint with every read the reference makes recorded: the tensor and the rows it takes.
struct Recorded {
    inner: Box<dyn Checkpoint>,
    read: Rc<RefCell<Vec<(String, usize)>>>,
}

impl WeightSource for Recorded {
    fn describe(&self, name: &str) -> std::io::Result<(GgmlType, Vec<u64>)> {
        self.inner.describe(name)
    }
    fn row_bytes(&mut self, name: &str, first: usize, count: usize) -> std::io::Result<Vec<u8>> {
        self.read.borrow_mut().push((name.to_string(), count));
        self.inner.row_bytes(name, first, count)
    }
    fn has(&self, name: &str) -> bool {
        WeightSource::has(&*self.inner, name)
    }
}

#[test]
fn every_tensor_of_the_table_is_bound() {
    let Some((dir, gguf)) = twin() else { return };
    let mut hf = HfCheckpoint::open(&dir).unwrap_or_else(|e| panic!("{e}"));
    let mut g = Gguf::open(&gguf).unwrap();
    assert_eq!(SpecSource::arch(&hf), SpecSource::arch(&g), "the architecture");
    let prefix = format!("{}.", g.arch.arch);
    let c = text_config(&dir);
    for key in g.order.iter().filter(|k| k.starts_with(&prefix)) {
        let got = hf.arch_meta(&key[prefix.len()..]).map(norm);
        let mut want = g.meta[key].clone();
        if key.ends_with(".attention.compress_ratios") {
            let MetaValue::Array { items, .. } = &mut want else { panic!("`{key}` is an array") };
            assert!(qsa_layers(&c).iter().all(|&l| items[l].as_u64() == Some(0)), "{CONVERTER_DROPS_QSA}");
            for l in qsa_layers(&c) {
                items[l] = MetaValue::U32(n(&c, "indexer_compress_ratio") as u32);
            }
        }
        assert_eq!(got.as_deref(), Some(norm(&want).as_str()), "`{key}`");
    }
    let mut theirs: Vec<String> = g.tensors().iter().map(|t| t.name.clone()).collect();
    let mut ours: Vec<String> = SpecSource::tensor_names(&hf).iter().map(|s| s.to_string()).collect();
    theirs.sort();
    ours.sort();
    assert_eq!(ours, theirs, "the tensor directory");
    // A tensor the converter reads or moves is its bit for bit. One it computes through an exponential (Gated
    // DeltaNet's `-exp(A_log)`) takes the platform's: torch's f32 exp, MKL's where torch is built with it, is not
    // correctly rounded, so the reader's correctly rounded value is held within one f32 step of it.
    let computed: BTreeSet<String> = hf.recomputed().iter().map(|s| s.to_string()).collect();
    for name in &theirs {
        assert_eq!(SpecSource::dims(&hf, name), SpecSource::dims(&g, name), "dims of {name}");
        let (a, b) = (values(&mut hf, name), values(&mut g, name));
        assert_eq!(a.len(), b.len(), "{name}: element count");
        let steps = |i: usize| (i64::from(a[i].to_bits() as i32) - i64::from(b[i].to_bits() as i32)).abs();
        let allowed = if computed.contains(name) { 1 } else { 0 };
        if let Some(i) = (0..a.len()).find(|&i| a[i].is_sign_negative() != b[i].is_sign_negative() || steps(i) > allowed) {
            panic!("{name}[{i}] reads {} where the converter wrote {}", a[i], b[i]);
        }
    }
    assert!(hf.unread().is_empty(), "checkpoint tensors nothing reads: {:?}", hf.unread());

    let spec = ModelSpec::from_source(&*open_checkpoint(&dir).unwrap()).unwrap();
    let reads = Rc::new(RefCell::new(Vec::new()));
    let source = Recorded { inner: open_checkpoint(&dir).unwrap(), read: reads.clone() };
    let mut reference = Reference::new(spec.clone(), source).unwrap_or_else(|e| panic!("{e:?}"));
    reference.step_every(&PROMPT).unwrap_or_else(|e| panic!("the reference steps: {e:?}"));
    let draft = format!("blk.{}.", spec.layers.len());
    let want: BTreeSet<String> = theirs.iter().filter(|t| !t.starts_with(&draft)).cloned().collect();
    let reads = reads.borrow().clone();
    let read: BTreeSet<String> = reads.iter().map(|(name, _)| name.clone()).collect();
    assert_eq!(read, want, "the step reads every tensor but the draft block's, and no other");
    // The n-gram table is the size of the model: a row a head a token, never more.
    let ple = spec.ple.as_ref().unwrap();
    let table: Vec<usize> = reads.iter().filter(|(name, _)| name == "per_layer_token_embd.weight").map(|&(_, rows)| rows).collect();
    assert!(table.iter().all(|&rows| rows == 1), "the n-gram table read a row at a time: {table:?}");
    assert_eq!(table.len(), PROMPT.len() * (ple.ngram - 1) * ple.heads_per_ngram, "a row each head of each token");
    assert!(theirs.iter().any(|t| t.starts_with(&draft)), "the draft block is read as the converter writes it");
}

fn assert_close(ours: &[f64], theirs: &[f64], what: &str) {
    assert_eq!(ours.len(), theirs.len(), "{what}: the vocabulary");
    let scale = theirs.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let worst = ours.iter().zip(theirs).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(worst <= scale * 1e-6, "{what}: a logit is {worst:e} from transformers', past a millionth of {scale}");
}

#[test]
fn the_reference_equals_transformers_on_the_tiny_checkpoint() {
    let Some((dir, _)) = twin() else { return };
    let theirs = common::hf_twins::transformers_logits_at(&dir, &PROMPT, &AT);
    let spec = ModelSpec::from_source(&*open_checkpoint(&dir).unwrap()).unwrap();
    let source = || Recorded { inner: open_checkpoint(&dir).unwrap(), read: Rc::new(RefCell::new(Vec::new())) };

    let mut whole = Reference::new(spec.clone(), source()).unwrap();
    let every = whole.step_every(&PROMPT).unwrap_or_else(|e| panic!("{e:?}"));
    let mut stepped = Reference::new(spec, source()).unwrap();
    let one: Vec<Vec<f64>> = PROMPT.iter().map(|&t| stepped.step(&[t]).unwrap_or_else(|e| panic!("{e:?}"))).collect();
    for (i, &p) in AT.iter().enumerate() {
        assert_close(&every[p], &theirs[i], &format!("position {p}, the prompt whole"));
        assert_close(&one[p], &theirs[i], &format!("position {p}, a token at a time"));
    }
}

#[test]
fn every_part_the_author_does_not_write_is_refused_by_name() {
    let Some((dir, _)) = twin() else { return };
    let spec = ModelSpec::from_source(&*open_checkpoint(&dir).unwrap()).unwrap();
    let silu = |s: &mut ModelSpec, gate: GateActivation| {
        for layer in &mut s.layers {
            if let Mixer::GatedDelta(g) = &mut layer.mixer {
                g.gate = gate;
            }
        }
    };
    let mut bare = spec.clone();
    (bare.hyper, bare.ple, bare.indexer) = (None, None, None);
    silu(&mut bare, GateActivation::Silu);
    author::check(&bare).unwrap_or_else(|e| panic!("without its own parts the twin is Qwen3.5's mixture, which the author writes: {e}"));
    let parts: [(&str, Box<dyn Fn(&mut ModelSpec)>); 4] = [
        ("hyper-connections", Box::new(|s| s.hyper = spec.hyper)),
        ("n-gram embedding", Box::new(|s| s.ple = spec.ple.clone())),
        ("(QSA)", Box::new(|s| s.indexer = spec.indexer.clone())),
        ("gated other than by SiLU", Box::new(move |s| silu(s, GateActivation::Sigmoid))),
    ];
    for (named, add) in parts {
        let mut one = bare.clone();
        add(&mut one);
        let refused = author::check(&one).expect_err(named).to_string();
        assert!(refused.contains(named), "the author refuses {named} by name; it says: {refused}");
    }

    let out = tempfile::tempdir().unwrap();
    let bundle = out.path().join("bundle");
    let emitted = cli::largo_in(
        out.path(),
        &["emit", "rdu", "--chip", "sn50", "--model", dir.to_str().unwrap(), "--ctx", "4", "-o", bundle.to_str().unwrap()],
    );
    assert_ne!(emitted.status.code(), Some(0), "largo emit rdu writes no bundle for the twin");
    let said = cli::stderr(&emitted);
    assert!(said.contains("hyper-connections"), "the refusal names the hyper-connections: {said}");
    assert!(!bundle.join("manifest.json").exists(), "and leaves no manifest");
}
