//! model_hf_intake_lock — a Hugging Face checkpoint directory is read in place, as llama.cpp's converter reads it.
//!
//! A bring-up test hands over a model as its maintainers publish it: `config.json`, safetensors shards and their index,
//! the tokenizer. `model::hf::HfCheckpoint` reads that directory and presents it as the GGUF the pinned converter
//! (`external/competitors/llama.cpp/convert_hf_to_gguf.py`) writes from it, so every spec arm, the binder and the f64
//! reference read it unchanged. The converter is the oracle: for each family's tiny twin (`scripts/accel/hf-tiny-twin.py`,
//! bf16 weights, sharded, the family's real tokenizer) the directory must give the converted GGUF's architecture keys,
//! spec, tensor directory, dims and values bit for bit, and the reference's logits; and no checkpoint tensor may be left
//! unread except those the converter itself skips.
//!
//! The twins need a Python with torch and transformers (`LOGOS_PYTHON`), the pinned llama.cpp and the tokenizers
//! (`scripts/accel/fetch-hf-tokenizers.sh`); without them each test records an honest skip (`hf-twins`).
//!
//! Run: `LOGOS_PYTHON=<python with torch> cargo nextest run -p logicaffeine-tests --test model_hf_intake_lock`

#![cfg(feature = "codegen")]

mod common;

use logicaffeine_compile::model::gguf::{Gguf, MetaValue};
use logicaffeine_compile::model::hf::HfCheckpoint;
use logicaffeine_compile::model::reference::{Reference, WeightSource};
use logicaffeine_compile::model::{open_checkpoint, Checkpoint};
use logicaffeine_compile::model::spec::{
    Act, Dsa, Ffn, Gating, Layer, Mixer, ModelSpec, Moe, Norm, Part, Recurrent, RopeScaling, Rwkv7, Selective, SpecSource, Ssd, Window,
};
use std::path::{Path, PathBuf};

use logicaffeine_compile::model::hf::{pre_tokenizer_named, tiktoken_split, HfPreTokenizer, HfTokenizer, Recompute, Tiktoken, PRE_TOKENIZER_PROBE};
use logicaffeine_compile::model::quant::{bf16_to_f32, f16_to_f32, f32_to_bf16, f32_to_f16};
use logicaffeine_compile::model::safetensors::{self, Dtype, Shard};
use logicaffeine_compile::model::tokenizer::Vocab;
use logicaffeine_compile::model::triage::ARCHITECTURES;
use std::collections::{BTreeMap, BTreeSet};

const FAMILIES: &[&str] = &[
    "llama", "qwen2", "qwen3", "qwen3moe", "qwen2moe", "qwen35", "qwen35moe", "qwen3next", "gemma3", "gpt-oss", "deepseek2",
    "llama4", "mixtral", "gemma", "gemma2",
];

fn twin(family: &str) -> Option<(PathBuf, PathBuf)> {
    common::require_tool("hf-twins").then(|| common::hf_twins::twin(family))
}

/// A metadata value as a comparable form: integers, floats as their f32 bits where the file stores f32, strings,
/// bools and arrays element by element.
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

/// Every value of `name` from `source`, dequantized to f32.
fn values(source: &mut dyn WeightSource, name: &str) -> Vec<f32> {
    let (ty, dims) = source.describe(name).unwrap_or_else(|e| panic!("{name}: {e}"));
    let inner = dims.first().copied().unwrap_or(1) as usize;
    let rows = dims.iter().product::<u64>() as usize / inner.max(1);
    let bytes = source.row_bytes(name, 0, rows).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut out = Vec::new();
    ty.dequantize(&bytes, &mut out);
    out
}

/// The family's directory reads as its converted GGUF as the converter lays it out: architecture, every architecture
/// key the GGUF holds, tensor directory, dims and values.
fn layout_reads_as_its_converted_gguf(family: &str) {
    let Some((dir, gguf)) = twin(family) else { return };
    let mut hf = HfCheckpoint::open(&dir).unwrap_or_else(|e| panic!("{family}: {e}"));
    let mut g = Gguf::open(&gguf).unwrap();

    assert_eq!(SpecSource::arch(&hf), SpecSource::arch(&g), "{family}: the architecture");
    let prefix = format!("{}.", g.arch.arch);
    for key in g.order.iter().filter(|k| k.starts_with(&prefix)) {
        let suffix = &key[prefix.len()..];
        let want = norm(&g.meta[key]);
        let got = hf.arch_meta(suffix).map(norm);
        assert_eq!(got.as_deref(), Some(want.as_str()), "{family}: `{key}`");
    }

    let mut theirs: Vec<String> = g.tensors().iter().map(|t| t.name.clone()).collect();
    let mut ours: Vec<String> = SpecSource::tensor_names(&hf).iter().map(|s| s.to_string()).collect();
    theirs.sort();
    ours.sort();
    assert_eq!(ours, theirs, "{family}: the tensor directory");
    for name in &theirs {
        assert_eq!(SpecSource::dims(&hf, name), SpecSource::dims(&g, name), "{family}: dims of {name}");
        let a = values(&mut hf, name);
        let b = values(&mut g, name);
        assert_eq!(a.len(), b.len(), "{family}: {name} element count");
        if let Some(i) = (0..a.len()).find(|&i| a[i].to_bits() != b[i].to_bits()) {
            panic!("{family}: {name}[{i}] reads {} where the converter wrote {}", a[i], b[i]);
        }
    }
    assert!(
        hf.unread().is_empty(),
        "{family}: checkpoint tensors nothing reads: {:?}",
        hf.unread()
    );
}

/// The family's directory reads as its converted GGUF: its layout ([`layout_reads_as_its_converted_gguf`]) and the
/// spec both read.
fn reads_as_its_converted_gguf(family: &str) {
    layout_reads_as_its_converted_gguf(family);
    let Some((dir, gguf)) = twin(family) else { return };
    assert_eq!(
        ModelSpec::from_source(&HfCheckpoint::open(&dir).unwrap()).unwrap_or_else(|e| panic!("{family}: {e:?}")),
        ModelSpec::from_gguf(&Gguf::open(&gguf).unwrap()).unwrap(),
        "{family}: the spec"
    );
}

/// A checkpoint `open_checkpoint` opens, owned as the reference holds its source.
struct Opened(Box<dyn Checkpoint>);

impl WeightSource for Opened {
    fn describe(&self, name: &str) -> std::io::Result<(logicaffeine_compile::model::quant::GgmlType, Vec<u64>)> {
        self.0.describe(name)
    }
    fn row_bytes(&mut self, name: &str, first: usize, count: usize) -> std::io::Result<Vec<u8>> {
        self.0.row_bytes(name, first, count)
    }
    fn has(&self, name: &str) -> bool {
        WeightSource::has(&*self.0, name)
    }
}

#[test]
fn a_llama_directory_yields_the_spec_its_gguf_yields() {
    reads_as_its_converted_gguf("llama");
}

#[test]
fn a_qwen2_directory_reads_as_its_converted_gguf() {
    // Qwen 2.5: biased query, key and value projections under a NEOX rotation, its window keys present and off.
    reads_as_its_converted_gguf("qwen2");
}

#[test]
fn mixtral_is_read_as_experts() {
    // Mixtral is a `llama` GGUF that counts experts: every block's FFN is the mixture its experts stack into, never a
    // dense FFN, read as the converter writes it.
    reads_as_its_converted_gguf("mixtral");
    let Some((dir, _)) = twin("mixtral") else { return };
    let spec = ModelSpec::from_source(&HfCheckpoint::open(&dir).unwrap()).unwrap();
    assert_eq!(spec.arch, "llama");
    for (l, layer) in spec.layers.iter().enumerate() {
        match &layer.ffn {
            Ffn::Moe(m) => assert_eq!(
                (m.experts, m.top_k, m.expert_dff, m.gating.clone(), m.norm_top_k, m.shared_experts),
                (4, 2, 32, Gating::Softmax, true, 0),
                "block {l}: four experts of 32, two a token, the kept softmax weights renormalized"
            ),
            other => panic!("block {l} is read as {other:?}, not as its experts"),
        }
    }
}

#[test]
fn a_mistral_window_the_llama_gguf_cannot_carry_is_refused() {
    let Some((dir, _)) = twin("mixtral") else { return };
    let copy = scratch_copy("mixtral-windowed", &dir);
    let path = copy.join("config.json");
    let mut config: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    config["sliding_window"] = serde_json::json!(4096);
    std::fs::write(&path, config.to_string()).unwrap();
    let err = HfCheckpoint::open(&copy).err().expect("a window the GGUF does not carry is refused").to_string();
    assert!(err.contains("sliding_window"), "the refusal names the key: {err}");
}

#[test]
fn a_qwen3_directory_reads_as_its_converted_gguf() {
    reads_as_its_converted_gguf("qwen3");
}

#[test]
fn a_qwen3moe_directory_reads_as_its_converted_gguf() {
    reads_as_its_converted_gguf("qwen3moe");
}

#[test]
fn a_qwen2moe_directory_reads_as_its_converted_gguf() {
    // Qwen1.5/Qwen2-MoE: Qwen2's biased attention and in every block experts by a softmax whose kept weights are not
    // renormalized, beside a shared expert of its own width scaled by its sigmoid gate (`shared_expert_gate`, one
    // output row over the input); no `head_dim` key, so a head is the width over the heads.
    reads_as_its_converted_gguf("qwen2moe");
    let Some((dir, _)) = twin("qwen2moe") else { return };
    let spec = ModelSpec::from_source(&HfCheckpoint::open(&dir).unwrap()).unwrap();
    assert_eq!((spec.arch.as_str(), spec.layers.len(), spec.tied_output), ("qwen2moe", 2, false));
    for (l, layer) in spec.layers.iter().enumerate() {
        let (Mixer::Attention(a), Ffn::Moe(m)) = (&layer.mixer, &layer.ffn) else {
            panic!("block {l} is read as {layer:?}, not attention beside experts")
        };
        assert!(a.bias && a.rope.as_ref().is_some_and(|r| r.neox) && a.head_dim == 16, "block {l}: {a:?}");
        assert_eq!(
            (m.experts, m.top_k, m.expert_dff, m.gating.clone(), m.norm_top_k, m.weights_scale),
            (4, 2, 32, Gating::Softmax, false, 1.0),
            "block {l}: four experts of 32, two a token, the kept softmax weights as they are"
        );
        assert_eq!(
            (m.shared_experts, m.shared_dff, m.shared_gate),
            (1, 64, true),
            "block {l}: one shared expert of 64, scaled by its gate"
        );
    }
}

#[test]
fn a_qwen35moe_directory_reads_as_its_converted_gguf() {
    // Qwen3.5-MoE: the multimodal checkpoint, Qwen3.5's hybrid with Qwen's mixture in every block (the kept softmax
    // weights renormalized, a shared expert scaled by its gate) and a draft block whose experts are stored as one
    // gate-and-up tensor the converter splits.
    reads_as_its_converted_gguf("qwen35moe");
    let Some((dir, _)) = twin("qwen35moe") else { return };
    let hf = HfCheckpoint::open(&dir).unwrap();
    let spec = ModelSpec::from_source(&hf).unwrap();
    assert_eq!((spec.arch.as_str(), spec.mtp_blocks), ("qwen35moe", 1));
    assert!(spec.layers.iter().any(|l| matches!(l.mixer, Mixer::GatedDelta(_))), "the hybrid's recurrent blocks");
    for (l, layer) in spec.layers.iter().enumerate() {
        let Ffn::Moe(m) = &layer.ffn else { panic!("block {l} is read as {:?}, not a mixture", layer.ffn) };
        assert_eq!(
            (m.experts, m.top_k, m.expert_dff, m.gating.clone(), m.norm_top_k, m.shared_experts, m.shared_dff, m.shared_gate),
            (4, 2, 32, Gating::Softmax, true, 1, 64, true),
            "block {l}: four experts of 32, two a token, renormalized, beside a gated shared expert of 64"
        );
    }
    let draft = format!("blk.{}.ffn_gate_exps.weight", spec.layers.len());
    assert!(SpecSource::has(&hf, &draft), "the draft block's split experts: {draft}");
}

#[test]
fn an_fp8_block_scaled_directory_reads_as_its_converted_gguf() {
    // DeepSeek-V3 as its FP8 release stores it: every projection in e4m3 beside a float32 scale per block, blocks that
    // divide few of the twin's axes. Read in place, each weight is the converter's `dequant_simple` of its codes and
    // scale, to the bit, through the latent attention's split and the experts' stack; the scales are read, not left
    // over; and the spec is the GGUF's.
    reads_as_its_converted_gguf("deepseek2-fp8");
    let Some((dir, _)) = twin("deepseek2-fp8") else { return };
    let hf = HfCheckpoint::open(&dir).unwrap();
    let scales = safetensors::shard_paths(&dir)
        .unwrap()
        .iter()
        .map(|p| Shard::open(p).unwrap().tensors.keys().filter(|n| n.ends_with("_scale_inv")).count())
        .sum::<usize>();
    assert!(scales > 0, "the twin stores scales");
    assert_eq!(hf.recomputed().len(), 0, "a scaled weight is read, not recomputed");
}

#[test]
fn a_quantization_the_reader_does_not_read_is_refused_by_name() {
    // A configuration naming a quantized format other than FP8 (GPTQ, AWQ, NVFP4, compressed tensors) is refused
    // naming the format, not read as if its packed integers were weights.
    let Some((dir, _)) = twin("deepseek2-fp8") else { return };
    for method in ["gptq", "awq", "compressed-tensors", "modelopt"] {
        let copy = scratch_copy(&format!("deepseek2-{method}"), &dir);
        let path = copy.join("config.json");
        let mut c: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        c["quantization_config"]["quant_method"] = method.into();
        std::fs::write(&path, c.to_string()).unwrap();
        let refused = HfCheckpoint::open(&copy).err().unwrap_or_else(|| panic!("a {method} checkpoint is refused"));
        let _ = std::fs::remove_dir_all(&copy);
        let said = refused.to_string();
        assert!(said.contains("quantization_config") && said.contains(method), "{method}: the refusal names the format: {said}");
    }
}

#[test]
fn a_glm4moe_directory_reads_as_its_converted_gguf() {
    // GLM-4.5: biased projections with per-head QK norms under a partial NEOX rotation, heads wider than the model over
    // its width, a dense block then DeepSeek's mixture under sigmoid scores, its FFN norm stored as
    // `post_attention_norm`, and the draft block after the stack presented as the converter writes it
    // (`blk.<n>.nextn.*`). Read raw, the spec refuses it, naming the norm; opened, the norm is `ffn_norm` and the spec
    // reads the stack without the draft block, from the directory as from the GGUF.
    layout_reads_as_its_converted_gguf("glm4moe");
    let Some((dir, gguf)) = twin("glm4moe") else { return };
    let refused = ModelSpec::from_source(&Gguf::open(&gguf).unwrap()).expect_err("a raw glm4moe checkpoint is refused");
    assert!(refused.to_string().contains("post_attention_norm"), "the refusal names the stored norm: {refused}");
    let spec = ModelSpec::from_source(&*open_checkpoint(&dir).unwrap()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(spec, ModelSpec::from_source(&*open_checkpoint(&gguf).unwrap()).unwrap(), "the directory and the GGUF");
    assert_eq!((spec.arch.as_str(), spec.layers.len(), spec.mtp_blocks), ("glm4moe", 3, 1), "the stack and its draft block");
    for (l, layer) in spec.layers.iter().enumerate() {
        let Mixer::Attention(a) = &layer.mixer else { panic!("block {l} is not attention") };
        assert!(a.bias && a.qk_norm && !a.qk_norm_whole && !a.fused_qkv, "block {l}: biased projections, per-head QK norms");
        assert_eq!((a.head_dim, a.heads * a.head_dim), (32, 2 * spec.d), "block {l}: heads wider than the model");
        assert_eq!(a.rope.as_ref().map(|r| (r.dims, r.neox)), Some((a.head_dim / 2, true)), "block {l}: half of each head, NEOX");
        assert!(!layer.post_mixer_norm && !layer.post_ffn_norm, "block {l}: no sandwich norms");
        match (&layer.ffn, l) {
            (Ffn::Dense { .. }, 0) => {}
            (Ffn::Moe(m), 1..) => {
                assert_eq!(m.gating, Gating::Sigmoid { bias: true }, "block {l}: sigmoid scores with a routing bias");
                assert!(m.norm_top_k && m.weights_scale == 2.5, "block {l}: renormalized and scaled by 2.5");
                assert_eq!((m.shared_experts, m.shared_dff, m.shared_gate), (1, m.expert_dff, false), "block {l}: one ungated shared expert");
                assert_eq!(m.groups, None, "block {l}: one group is no group limit");
            }
            (ffn, _) => panic!("block {l}: {ffn:?}"),
        }
    }
}

#[test]
fn a_rotation_share_and_original_context_are_read_from_the_rope_parameters_first() {
    // The converters read `partial_rotary_factor` (and Phi-3's `original_max_position_embeddings`) from the rope
    // parameters, the configuration's own key only where they name none: a configuration naming both reads the rope
    // parameters'. Phi-3's converter fails where neither names the original context, and so does the reader.
    for family in ["phi3", "glm4", "glm4moe"] {
        let Some((dir, _)) = twin(family) else { return };
        let copy = scratch_copy(&format!("{family}-rope-first"), &dir);
        let path = copy.join("config.json");
        let edit = |f: &dyn Fn(&mut serde_json::Value)| {
            let mut c: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            f(&mut c);
            std::fs::write(&path, c.to_string()).unwrap();
            c
        };
        let c = edit(&|c| {
            c["partial_rotary_factor"] = 0.5.into();
            c["rope_parameters"]["partial_rotary_factor"] = 0.25.into();
            if family == "phi3" {
                c["original_max_position_embeddings"] = 16384.into();
                c["rope_parameters"]["original_max_position_embeddings"] = 8192.into();
            }
        });
        let (d, heads) = (c["hidden_size"].as_u64().unwrap(), c["num_attention_heads"].as_u64().unwrap());
        let head_dim = c["head_dim"].as_u64().unwrap_or(d / heads);
        let rotated = if family == "phi3" { (0.25 * d as f64) as u64 / heads } else { (head_dim as f64 * 0.25) as u64 };
        let hf = HfCheckpoint::open(&copy).unwrap_or_else(|e| panic!("{family}: {e}"));
        assert_eq!(hf.arch_meta("rope.dimension_count").and_then(MetaValue::as_u64), Some(rotated), "{family}: the rope parameters' share");
        if family == "phi3" {
            assert_eq!(
                hf.arch_meta("rope.scaling.original_context_length").and_then(MetaValue::as_u64),
                Some(8192),
                "phi3: the rope parameters' original context"
            );
            edit(&|c| {
                c.as_object_mut().unwrap().remove("original_max_position_embeddings");
                c["rope_parameters"].as_object_mut().unwrap().remove("original_max_position_embeddings");
            });
            let refused = HfCheckpoint::open(&copy).err().expect("phi3: a configuration with no original context is refused");
            assert!(refused.to_string().contains("original_max_position_embeddings"), "phi3: the refusal names the key: {refused}");
        }
        let _ = std::fs::remove_dir_all(&copy);
    }
}

#[test]
fn a_glm4_directory_reads_as_its_converted_gguf() {
    // GLM-4: half of each head rotated by interleaved pairs, biased projections, sandwich norms and one gate-and-up
    // projection, as the converter writes them. Read raw, the spec refuses it, naming the fused gate; opened, the gate
    // and up projections are presented apart and the spec reads it, from the directory as from the GGUF.
    layout_reads_as_its_converted_gguf("glm4");
    let Some((dir, gguf)) = twin("glm4") else { return };
    let refused = ModelSpec::from_source(&Gguf::open(&gguf).unwrap()).expect_err("a raw glm4 checkpoint is refused");
    assert!(refused.to_string().contains("ffn_up"), "the refusal names the fused projection: {refused}");
    let spec = ModelSpec::from_source(&*open_checkpoint(&dir).unwrap()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(spec, ModelSpec::from_source(&*open_checkpoint(&gguf).unwrap()).unwrap(), "the directory and the GGUF");
    assert_eq!(spec.arch, "glm4");
    for (l, layer) in spec.layers.iter().enumerate() {
        let Mixer::Attention(a) = &layer.mixer else { panic!("block {l} is not attention") };
        assert!(a.bias && !a.fused_qkv, "block {l}: three biased projections");
        assert_eq!(a.rope.as_ref().map(|r| (r.dims, r.neox)), Some((a.head_dim / 2, false)), "block {l}: half of each head, interleaved");
        assert!(layer.post_mixer_norm && layer.post_ffn_norm, "block {l}: the sandwich norms");
    }
}

#[test]
fn a_phi3_directory_reads_as_its_converted_gguf() {
    // Phi-4: one query-key-value projection and one gate-and-up projection, as the converter writes them (`attn_qkv`,
    // and `ffn_up` twice the FFN's width). Read raw, the spec refuses it, naming the fused gate; opened, the gate and up
    // projections are presented apart and the spec reads it, from the directory as from the GGUF.
    layout_reads_as_its_converted_gguf("phi3");
    let Some((dir, gguf)) = twin("phi3") else { return };
    let refused = ModelSpec::from_source(&Gguf::open(&gguf).unwrap()).expect_err("a raw phi3 checkpoint is refused");
    assert!(refused.to_string().contains("ffn_up"), "the refusal names the fused projection: {refused}");
    let spec = ModelSpec::from_source(&*open_checkpoint(&dir).unwrap()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(spec, ModelSpec::from_source(&*open_checkpoint(&gguf).unwrap()).unwrap(), "the directory and the GGUF");
    assert_eq!(spec.arch, "phi3");
    for (l, layer) in spec.layers.iter().enumerate() {
        let Mixer::Attention(a) = &layer.mixer else { panic!("block {l} is not attention") };
        assert!(a.fused_qkv && a.window.is_none(), "block {l}: one projection, no window");
        assert_eq!(a.rope.as_ref().map(|r| (r.dims, r.neox)), Some((a.head_dim, true)), "block {l}: the whole head rotated");
        assert!(matches!(layer.ffn, Ffn::Dense { dff: 32, .. }), "block {l}: a dense FFN of 32, read as {:?}", layer.ffn);
    }
}

#[test]
fn a_minimax_m2_directory_reads_as_its_converted_gguf() {
    // MiniMax-M2: its query and key each normed across every head at once, half of each head rotated, and in every block
    // experts scored by sigmoids, chosen by score plus a routing bias and renormalized. The directory reads as the
    // converter's GGUF reads, its norms as wide as the projections.
    reads_as_its_converted_gguf("minimax_m2");
    let Some((dir, _)) = twin("minimax_m2") else { return };
    let spec = ModelSpec::from_source(&HfCheckpoint::open(&dir).unwrap()).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(spec.arch, "minimax-m2");
    for (l, layer) in spec.layers.iter().enumerate() {
        let Mixer::Attention(a) = &layer.mixer else { panic!("block {l} is not attention") };
        assert!(a.qk_norm && a.qk_norm_whole, "block {l}: a QK norm across every head");
        assert_eq!(a.rope.as_ref().map(|r| (r.dims, r.neox)), Some((a.head_dim / 2, true)), "block {l}: half of each head rotated");
        let Ffn::Moe(m) = &layer.ffn else { panic!("block {l} is read as {:?}, not a mixture", layer.ffn) };
        assert_eq!((&m.gating, m.norm_top_k, m.shared_experts), (&Gating::Sigmoid { bias: true }, true, 0), "block {l}");
    }
}

#[test]
fn a_qwen3next_directory_reads_as_its_converted_gguf() {
    // Qwen3-Next: Qwen3.5's hybrid with Qwen's mixture in every block, its value heads grouped by key head and its beta
    // and alpha one projection (`ssm_ba`), as the converter writes them. Read raw, the spec refuses it by name; opened,
    // it is presented in Qwen3.5's tiled layout and the spec reads it, from the directory as from the GGUF.
    layout_reads_as_its_converted_gguf("qwen3next");
    let Some((dir, gguf)) = twin("qwen3next") else { return };
    let refused = ModelSpec::from_source(&Gguf::open(&gguf).unwrap()).expect_err("a raw Qwen3-Next checkpoint is refused");
    assert!(refused.to_string().contains("ssm_ba"), "the refusal names the grouped layout's tensor: {refused}");
    let spec = ModelSpec::from_source(&*open_checkpoint(&dir).unwrap()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(spec, ModelSpec::from_source(&*open_checkpoint(&gguf).unwrap()).unwrap(), "the directory and the GGUF");
    assert_eq!((spec.arch.as_str(), spec.mtp_blocks), ("qwen3next", 1));
    assert!(spec.layers.iter().any(|l| matches!(l.mixer, Mixer::GatedDelta(_))), "the hybrid's recurrent blocks");
    for (l, layer) in spec.layers.iter().enumerate() {
        let Ffn::Moe(m) = &layer.ffn else { panic!("block {l} is read as {:?}, not a mixture", layer.ffn) };
        assert_eq!((m.norm_top_k, m.shared_gate, m.shared_dff), (true, true, 64), "block {l}");
    }
}

#[test]
fn the_reference_computes_what_transformers_computes() {
    // The f64 reference over each opened directory, against the model's own forward in transformers at f64 over the
    // same weights: the last position's logits after a prompt, within a millionth of the largest. A layout misread —
    // value heads paired with the wrong key heads, a split projection's halves swapped — moves them far past that.
    let prompt = [1u32, 17, 400, 2024, 9];
    for family in ["qwen3next", "qwen35", "qwen35moe", "qwen3moe", "qwen2moe"] {
        let Some((dir, _)) = twin(family) else { return };
        let opened = open_checkpoint(&dir).unwrap_or_else(|e| panic!("{family}: {e}"));
        let spec = ModelSpec::from_source(&*opened).unwrap_or_else(|e| panic!("{family}: {e}"));
        let mut reference = Reference::new(spec, Opened(opened)).unwrap_or_else(|e| panic!("{family}: {e:?}"));
        let ours = reference.step(&prompt).unwrap();
        let theirs: Vec<f64> = oracle("logits", Some(&dir), &serde_json::json!(prompt))
            .as_array()
            .expect("the logits")
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        assert_eq!(ours.len(), theirs.len(), "{family}: the vocabulary");
        let scale = theirs.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        let worst = ours.iter().zip(&theirs).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(worst <= scale * 1e-6, "{family}: a logit is {worst:e} from transformers', past a millionth of {scale}");
    }
}

#[test]
fn a_qwen3next_directory_computes_what_transformers_computes() {
    // Qwen3-Next alone against its own forward in transformers at f64: the presentation layer's reorder of grouped value
    // heads into tiled ones, and its split of each key head's betas and alphas, are the only reading of its layout,
    // and the converter writes the grouped one, so transformers is the one oracle for them. Six value heads over two key
    // heads: at as many value heads a key head as key heads the reorder is its own inverse.
    let Some((dir, _)) = twin("qwen3next") else { return };
    let prompt = [1u32, 17, 400, 2024, 9];
    let opened = open_checkpoint(&dir).unwrap();
    let spec = ModelSpec::from_source(&*opened).unwrap();
    let mut reference = Reference::new(spec, Opened(opened)).unwrap();
    let ours = reference.step(&prompt).unwrap();
    let theirs: Vec<f64> = oracle("logits", Some(&dir), &serde_json::json!(prompt))
        .as_array()
        .expect("the logits")
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(ours.len(), theirs.len(), "the vocabulary");
    let scale = theirs.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let worst = ours.iter().zip(&theirs).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(worst <= scale * 1e-6, "a logit is {worst:e} from transformers', past a millionth of {scale}");
}

#[test]
fn a_renormalization_the_gguf_cannot_say_is_refused() {
    // llama.cpp fixes it by architecture — Qwen2-MoE's kept weights never renormalized, Qwen3-MoE's always — and the
    // converter writes no key for it: a configuration that says otherwise is a model its GGUF cannot carry.
    for (family, renormalized) in [("qwen2moe", true), ("qwen3moe", false)] {
        let Some((dir, _)) = twin(family) else { return };
        let copy = scratch_copy(&format!("{family}-renormalized"), &dir);
        let path = copy.join("config.json");
        let mut config: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        config["norm_topk_prob"] = serde_json::json!(renormalized);
        std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
        let refused = HfCheckpoint::open(&copy).err().map(|e| e.to_string()).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&copy);
        assert!(
            refused.contains("norm_topk_prob"),
            "{family}: norm_topk_prob {renormalized} is refused naming the key; got {refused:?}"
        );
    }
}

#[test]
fn a_qwen35_directory_reads_as_its_converted_gguf() {
    reads_as_its_converted_gguf("qwen35");
}

#[test]
fn a_qwen35moe_text_model_reads_as_its_converted_gguf() {
    // Qwen3.8-2.4T's layout: Qwen3.5-MoE's text model alone, its decoder under `model.` where the multimodal checkpoint
    // nests it under `model.language_model.`, untied, its rotation without multimodal sections, and every block's experts
    // stored as one gate-and-up tensor and one down tensor.
    reads_as_its_converted_gguf("qwen35moe-causal");
}

#[test]
fn a_gemma3_directory_reads_as_its_converted_gguf() {
    reads_as_its_converted_gguf("gemma3");
}

#[test]
fn a_multimodal_checkpoints_language_model_reads_as_its_converted_gguf() {
    // Gemma 3's and Llama 4's larger releases are published only as their multimodal checkpoints: the text model under
    // the wrapper's prefix, a vision tower and its projector beside it, which the converter skips. Each reads as the
    // text model its converter writes, the tower left unread and named as skipped.
    for (family, text) in [("gemma3-mm", "gemma3"), ("llama4-mm", "llama4")] {
        reads_as_its_converted_gguf(family);
        let Some((dir, _)) = twin(family) else { return };
        let hf = HfCheckpoint::open(&dir).unwrap();
        assert_eq!(SpecSource::arch(&hf), text, "{family}: read as its text model");
        let skipped = hf.skipped();
        assert!(
            skipped.iter().any(|s| s.contains("vision")) && skipped.iter().any(|s| s.contains("multi_modal_projector")),
            "{family}: the tower and the projector are skipped: {skipped:?}"
        );
    }
}

#[test]
fn llama4_experts_stored_one_linear_each_are_refused_by_name() {
    // transformers 4's `for_llm_compressor` stores each expert as its own linear where the releases fuse them.
    let Some(opened) = opened_with("llama4", "compressor", |c| c["for_llm_compressor"] = serde_json::json!(true)) else { return };
    let refused = opened.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(refused.contains("for_llm_compressor"), "experts stored one by one are refused naming the key; got {refused:?}");
    let Some(opened) = opened_with("llama4", "fused", |c| c["for_llm_compressor"] = serde_json::json!(false)) else { return };
    opened.unwrap_or_else(|e| panic!("the releases' fused experts read: {e}"));
}

#[test]
fn a_gemma_directory_reads_as_its_converted_gguf() {
    // Gemma: heads wider than the width over the heads, one rotation, GELU under its legacy name, the √d embedding scale.
    reads_as_its_converted_gguf("gemma");
    let Some((dir, _)) = twin("gemma") else { return };
    let spec = ModelSpec::from_source(&HfCheckpoint::open(&dir).unwrap()).unwrap();
    assert_eq!(
        (spec.arch.as_str(), spec.embedding_scale, spec.final_logit_softcap, spec.tied_output),
        ("gemma", Some(8.0), None, true),
        "the architecture, the embedding's √d, no cap and the tied head"
    );
    for (l, layer) in spec.layers.iter().enumerate() {
        let Mixer::Attention(a) = &layer.mixer else { panic!("block {l} attends") };
        assert_eq!(
            (a.heads, a.kv_heads, a.head_dim, a.window, a.softcap, a.scale),
            (4, 1, 24, None, None, 1.0 / 24f64.sqrt()),
            "block {l}: four query heads 24 wide over one key-value head, no window, no cap, `1/√head_dim`"
        );
        assert!(!layer.post_mixer_norm && !layer.post_ffn_norm, "block {l}: a norm before each sublayer only");
        assert!(matches!(layer.ffn, Ffn::Dense { act: Act::GeluTanh, .. }), "block {l}: a GELU-gated FFN");
    }
    for (tag, key, value) in [("silu", "hidden_act", serde_json::json!("silu")), ("bidirectional", "use_bidirectional_attention", serde_json::json!(true))] {
        let Some(opened) = opened_with("gemma", tag, |c| c[key] = value.clone()) else { return };
        let refused = opened.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(refused.contains(key), "Gemma's `{key}` = {value} is refused naming the key; got {refused:?}");
    }
}

#[test]
fn a_gemma2_directory_reads_as_its_converted_gguf() {
    // Gemma 2: a window on every other block (the first local), soft caps on the scores and the logits, sandwich norms
    // stored as offsets from one, a GELU-gated FFN, the √d embedding scale, one rotation.
    reads_as_its_converted_gguf("gemma2");
    let Some((dir, _)) = twin("gemma2") else { return };
    let spec = ModelSpec::from_source(&HfCheckpoint::open(&dir).unwrap()).unwrap();
    assert_eq!(
        (spec.arch.as_str(), spec.embedding_scale, spec.final_logit_softcap, spec.tied_output),
        ("gemma2", Some(8.0), Some(0.3), true),
        "the architecture, the embedding's √d, the logits' cap and the tied head"
    );
    assert_eq!(spec.layers.len(), 4, "the stack");
    for (l, layer) in spec.layers.iter().enumerate() {
        let Mixer::Attention(a) = &layer.mixer else { panic!("block {l} attends") };
        assert_eq!(
            (a.window, a.softcap, a.scale, a.qk_norm, a.rope.as_ref().map(|r| (r.theta, r.neox))),
            ((l % 2 == 0).then_some(Window::Sliding(4)), Some(0.1), 0.25, false, Some((10000.0, true))),
            "block {l}: its window, its scores' cap, its scale, no QK norm, its rotation"
        );
        assert!(layer.post_mixer_norm && layer.post_ffn_norm, "block {l}: a norm after the mixer and after the FFN");
        assert!(matches!(layer.ffn, Ffn::Dense { act: Act::GeluTanh, .. }), "block {l}: a GELU-gated FFN");
    }
}

/// The f64 reference over the twin `family`'s directory at the last of `prompt`, and transformers' own forward at f64
/// (eager attention, the one that takes a cap) over the same weights: within a millionth of the largest logit — the
/// largest before the logits' cap where the model caps them, as transformers' float32 islands err before it and the cap
/// is 1-Lipschitz. Returns the spec, transformers' logits and that scale, and the reference's logits under any spec.
fn computes_what_transformers_computes(family: &str, prompt: &[u32]) -> Option<(ModelSpec, Vec<f64>, f64, impl Fn(&ModelSpec) -> Vec<f64>)> {
    let (dir, _) = twin(family)?;
    let at = dir.clone();
    let prompt = prompt.to_vec();
    let logits = {
        let prompt = prompt.clone();
        move |spec: &ModelSpec| Reference::new(spec.clone(), Opened(open_checkpoint(&at).unwrap())).unwrap().step(&prompt).unwrap()
    };
    let spec = ModelSpec::from_source(&*open_checkpoint(&dir).unwrap()).unwrap_or_else(|e| panic!("{family}: {e:?}"));
    let ours = logits(&spec);
    let theirs: Vec<f64> = oracle("logits", Some(&dir), &serde_json::json!(prompt))
        .as_array()
        .expect("the logits")
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(ours.len(), theirs.len(), "{family}: the vocabulary");
    let raw = match spec.final_logit_softcap {
        None => ours.clone(),
        Some(_) => logits(&ModelSpec { final_logit_softcap: None, ..spec.clone() }),
    };
    let scale = raw.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let worst = ours.iter().zip(&theirs).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(worst <= scale * 1e-6, "{family}: a logit is {worst:e} from transformers', past a millionth of {scale}");
    Some((spec, theirs, scale, logits))
}

#[test]
fn a_gemma_directory_computes_what_transformers_computes() {
    // Gemma: heads 24 wide over a width of 64 in four heads, so a reference that scaled by `1/√(d/heads)` or read the
    // projections at `d/heads` a head would miss.
    computes_what_transformers_computes("gemma", &[2, 17, 400, 2024, 9]);
}

#[test]
fn a_gemma2_directory_computes_what_transformers_computes() {
    // Gemma 2 at the last of nine positions: every local block's 4-token window drops the first five, and the caps bind
    // on the twin's scores and logits, so a reference that left out the window or either cap lands far past a millionth.
    let Some((spec, theirs, scale, logits)) = computes_what_transformers_computes("gemma2", &[2, 17, 400, 2024, 9, 77, 3, 512, 1000]) else {
        return;
    };
    let attention = |f: &dyn Fn(&mut logicaffeine_compile::model::spec::Attention)| {
        let mut s = spec.clone();
        for layer in &mut s.layers {
            if let Mixer::Attention(a) = &mut layer.mixer {
                f(a);
            }
        }
        s
    };
    let unwindowed = attention(&|a| a.window = None);
    let uncapped = attention(&|a| a.softcap = None);
    let open_head = ModelSpec { final_logit_softcap: None, ..spec.clone() };
    for (what, other) in [("no window", unwindowed), ("no cap on the scores", uncapped), ("no cap on the logits", open_head)] {
        let off = logits(&other).iter().zip(&theirs).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(off > scale * 1e-3, "{what}: the logits move only {off:e}, so the twin does not hold the reference to it");
    }
}

/// The twin `family`'s directory with `edit` applied to its `config.json`, opened.
fn opened_with(family: &str, tag: &str, edit: impl Fn(&mut serde_json::Value)) -> Option<std::io::Result<HfCheckpoint>> {
    let (dir, _) = twin(family)?;
    let copy = scratch_copy(&format!("{family}-{tag}"), &dir);
    let path = copy.join("config.json");
    let mut config: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    edit(&mut config);
    std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
    let opened = HfCheckpoint::open(&copy);
    let _ = std::fs::remove_dir_all(&copy);
    Some(opened)
}

#[test]
fn a_gemma_scale_the_gguf_cannot_carry_is_refused() {
    // llama.cpp fixes Gemma 2's and Gemma 3's score scale by the stack's depth (`1/√(d/heads)` at 27B's, else
    // `1/√head_dim`) and the converter writes no key for it: a `query_pre_attn_scalar` that says otherwise is a model
    // its GGUF cannot carry.
    for family in ["gemma2", "gemma3"] {
        let Some(opened) = opened_with(family, "scaled", |c| c["query_pre_attn_scalar"] = serde_json::json!(24)) else { return };
        let refused = opened.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(refused.contains("query_pre_attn_scalar"), "{family}: a scalar of 24 over heads of 16 is refused naming the key; got {refused:?}");
    }
}

#[test]
fn every_gemma2_setting_the_gguf_cannot_carry_is_refused_by_name() {
    // llama.cpp caps every Gemma 2 by default (50 on the scores, 30 on the logits) and alternates its windows from the
    // first block; the converter writes `sliding_window` alone.
    for (tag, key, value) in [
        ("score-cap-off", "attn_logit_softcapping", serde_json::Value::Null),
        ("logit-cap-off", "final_logit_softcapping", serde_json::Value::Null),
        ("two-windows", "sliding_window_size", serde_json::json!(8)),
        ("global-first", "layer_types", serde_json::json!(["full_attention", "sliding_attention", "full_attention", "sliding_attention"])),
        ("silu", "hidden_activation", serde_json::json!("silu")),
        ("bidirectional", "use_bidirectional_attention", serde_json::json!(true)),
    ] {
        let Some(opened) = opened_with("gemma2", tag, |c| c[key] = value.clone()) else { return };
        let refused = opened.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(refused.contains(key), "`{key}` = {value} is refused naming the key; got {refused:?}");
    }
    // The alternation transformers saves, and a window size that agrees, read as the converter reads them.
    for (tag, key, value) in [
        ("layer-types", "layer_types", serde_json::json!(["sliding_attention", "full_attention", "sliding_attention", "full_attention"])),
        ("window-size", "sliding_window_size", serde_json::json!(4)),
    ] {
        let Some(opened) = opened_with("gemma2", tag, |c| c[key] = value.clone()) else { return };
        opened.unwrap_or_else(|e| panic!("`{key}` = {value} reads: {e}"));
    }
}

#[test]
fn a_gemma2_rotation_base_reads_as_the_configuration_says() {
    // The converter writes no rotation base for Gemma 2, and llama.cpp turns an unwritten one at 10000; the reader writes
    // the configuration's own, so a base other than 10000 is the one the model turns by.
    // transformers saves the base inside `rope_parameters`; a configuration of its earlier releases, as `rope_theta`.
    let Some(opened) = opened_with("gemma2", "theta", |c| match c.get_mut("rope_parameters") {
        Some(r) if r.is_object() => r["rope_theta"] = serde_json::json!(20000.0),
        _ => c["rope_theta"] = serde_json::json!(20000.0),
    }) else {
        return;
    };
    let spec = ModelSpec::from_source(&opened.unwrap()).unwrap();
    for (l, layer) in spec.layers.iter().enumerate() {
        let Mixer::Attention(a) = &layer.mixer else { panic!("block {l} attends") };
        assert_eq!(a.rope.as_ref().map(|r| r.theta), Some(20000.0), "block {l}: the configuration's base");
    }
}

#[test]
fn a_ministral3_directory_reads_as_its_converted_gguf() {
    // Ministral 3's text model (`Ministral3ForCausalLM`): Llama's interleaved query and key pairs, YaRN, and the
    // attention temperature on every layer, written as `attention.temperature_scale` with `mscale_all_dim` as the
    // YaRN log multiplier.
    reads_as_its_converted_gguf("ministral3");
    let Some((dir, _)) = twin("ministral3") else { return };
    let spec = ModelSpec::from_source(&HfCheckpoint::open(&dir).unwrap()).unwrap();
    assert_eq!(spec.arch, "mistral3");
    for (l, layer) in spec.layers.iter().enumerate() {
        let Mixer::Attention(a) = &layer.mixer else { panic!("block {l} attends") };
        let t = a.temperature.unwrap_or_else(|| panic!("block {l}: every layer takes the temperature"));
        assert_eq!((t.floor, t.scale, t.offset), (4.0, 0.5, 0.0), "block {l}: floors of the original context, positions from zero");
        assert!(a.rope.as_ref().is_some_and(|r| !r.neox && r.dims == a.head_dim), "block {l}: interleaved pairs over the whole head");
    }
}

#[test]
fn mistral3s_multimodal_checkpoints_read_as_their_converted_ggufs() {
    // Ministral 3 and Mistral Small 3.1 and 3.2 are published as `Mistral3ForConditionalGeneration`: the text model
    // under the wrapper's prefix beside a Pixtral tower and its projector, which the converter skips. Ministral 3's text
    // model takes YaRN and the temperature; Mistral Small's, Mistral's own, neither.
    for (family, temperature) in [("ministral3-mm", true), ("mistral3-small", false)] {
        reads_as_its_converted_gguf(family);
        let Some((dir, _)) = twin(family) else { return };
        let hf = HfCheckpoint::open(&dir).unwrap();
        assert_eq!(SpecSource::arch(&hf), "mistral3", "{family}: read as its text model");
        let skipped = hf.skipped();
        assert!(
            skipped.iter().any(|s| s.contains("vision_tower")) && skipped.iter().any(|s| s.contains("multi_modal_projector")),
            "{family}: the tower and the projector are skipped: {skipped:?}"
        );
        let spec = ModelSpec::from_source(&hf).unwrap();
        for (l, layer) in spec.layers.iter().enumerate() {
            let Mixer::Attention(a) = &layer.mixer else { panic!("{family}: block {l} attends") };
            assert_eq!(a.temperature.is_some(), temperature, "{family}: block {l}'s temperature");
        }
    }
}

#[test]
fn a_ministral3_directory_computes_what_transformers_computes() {
    // Nine positions over floors of four: the temperature scales positions 4 to 7 by `1 + 0.5·ln 2` and position 8 by
    // `1 + 0.5·ln 3`, and YaRN's ramp over an original context of four turns every frequency the twin holds; a reference
    // without the temperature, counting positions from one as Llama 4's does, or without YaRN lands far past a millionth.
    let Some((spec, theirs, scale, logits)) = computes_what_transformers_computes("ministral3", &[1, 17, 400, 2024, 9, 77, 3, 512, 1000])
    else {
        return;
    };
    let changed = |f: &dyn Fn(&mut logicaffeine_compile::model::spec::Attention)| {
        let mut s = spec.clone();
        for layer in &mut s.layers {
            if let Mixer::Attention(a) = &mut layer.mixer {
                f(a);
            }
        }
        s
    };
    let untempered = changed(&|a| a.temperature = None);
    let from_one = changed(&|a| a.temperature = a.temperature.map(|t| logicaffeine_compile::model::spec::Temperature { offset: 1.0, ..t }));
    let unscaled = changed(&|a| {
        if let Some(r) = a.rope.as_mut() {
            r.scaling = logicaffeine_compile::model::spec::RopeScaling::None;
        }
    });
    for (what, other) in [("no temperature", untempered), ("positions counted from one", from_one), ("no YaRN", unscaled)] {
        let off = logits(&other).iter().zip(&theirs).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(off > scale * 1e-3, "{what}: the logits move only {off:e}, so the twin does not hold the reference to it");
    }
}

#[test]
fn a_ministral3_directorys_tokenizer_reads_as_its_converted_gguf() {
    // Ministral 3 ships tokenizer.json beside tekken.json: the converter reads the byte-pair vocabulary of
    // tokenizer.json (`_set_vocab_gpt2`, the pre-tokenizer `pixtral` by its probe digest), and tekken.json only where
    // tokenizer.json is absent.
    tokenizes_as_its_converted_gguf("ministral3");
    let Some((dir, _)) = twin("ministral3") else { return };
    assert!(dir.join("tekken.json").is_file() && dir.join("tokenizer.json").is_file(), "the twin ships both, as the releases do");
    let meta = HfCheckpoint::read_tokenizer(&dir).unwrap();
    assert_eq!(meta.get("tokenizer.ggml.pre").and_then(MetaValue::as_str), Some("pixtral"));
}

#[test]
fn every_ministral3_setting_the_gguf_cannot_carry_is_refused_by_name() {
    // llama.cpp computes YaRN's magnitude taking `mscale` as 1 and floors and ceils its correction range; the converter
    // asserts YaRN and reads `mscale_all_dim` and `llama_4_scaling_beta` without a default; the mistral3 GGUF carries no
    // window; and a Mistral 4 text model is llama.cpp's mistral4.
    type Edit = Box<dyn Fn(&mut serde_json::Value)>;
    let rope = |key: &'static str, value: serde_json::Value| -> Edit { Box::new(move |c| c["rope_parameters"][key] = value.clone()) };
    let drop = |key: &'static str| -> Edit {
        Box::new(move |c| {
            c["rope_parameters"].as_object_mut().unwrap().remove(key);
        })
    };
    let cases: Vec<(&str, Edit, &str)> = vec![
        ("mscale", rope("mscale", serde_json::json!(2.0)), "mscale"),
        ("attention-factor", rope("attention_factor", serde_json::json!(1.5)), "attention_factor"),
        ("untruncated", rope("truncate", serde_json::json!(false)), "truncate"),
        ("no-beta", drop("llama_4_scaling_beta"), "llama_4_scaling_beta"),
        ("no-log-multiplier", drop("mscale_all_dim"), "mscale_all_dim"),
        ("no-factor", drop("factor"), "factor"),
        (
            "default-rope",
            Box::new(|c| c["rope_parameters"] = serde_json::json!({"rope_type": "default", "rope_theta": 1000000.0, "llama_4_scaling_beta": 0.5})),
            "YaRN",
        ),
        ("windowed", Box::new(|c| c["sliding_window"] = serde_json::json!(4096)), "sliding_window"),
    ];
    for (tag, edit, named) in cases {
        let Some(opened) = opened_with("ministral3", tag, edit) else { return };
        let refused = opened.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(refused.contains(named), "{tag}: refused naming `{named}`; got {refused:?}");
    }
    // `mscale` matters only beside a log multiplier: with `mscale_all_dim` 0 transformers and llama.cpp both take
    // `get_mscale(factor)`, whatever `mscale` says.
    let Some(opened) = opened_with("ministral3", "no-multiplier", |c| {
        c["rope_parameters"]["mscale_all_dim"] = serde_json::json!(0.0);
        c["rope_parameters"]["mscale"] = serde_json::json!(2.0);
    }) else {
        return;
    };
    opened.unwrap_or_else(|e| panic!("an mscale no magnitude reads: {e}"));
    let Some(opened) = opened_with("ministral3-mm", "mistral4", |c| c["text_config"]["model_type"] = serde_json::json!("mistral4")) else { return };
    let refused = opened.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(refused.contains("mistral4"), "a Mistral 4 text model is refused naming it; got {refused:?}");
    // transformers' MistralConfig windows at 4096 where a configuration names no window; Ministral3Config does not.
    let Some(opened) = opened_with("mistral3-small", "unset-window", |c| {
        c["text_config"].as_object_mut().unwrap().remove("sliding_window");
    }) else {
        return;
    };
    let refused = opened.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(refused.contains("sliding_window"), "Mistral Small with no window named reads as transformers' 4096; got {refused:?}");
    let Some(opened) = opened_with("ministral3", "unset-window", |c| {
        c.as_object_mut().unwrap().remove("sliding_window");
    }) else {
        return;
    };
    opened.unwrap_or_else(|e| panic!("Ministral 3 with no window named attends every position: {e}"));
}

#[test]
fn a_phimoe_directory_reads_as_its_converted_gguf() {
    // PhiMoE (`PhiMoEForCausalLM`), read as the model it is: LayerNorms with their biases, biased attention under a NEOX
    // rotation and a window, sparsemixer's two experts at PhiMoE's jitter, a biased head. The converted GGUF holds every
    // tensor and setting of it; llama.cpp's phimoe, phi3's graph, reads them as another model (RMSNorms, a renormalized
    // softmax, no window), which `a_phimoe_directory_computes_what_transformers_computes` holds the spec apart from.
    reads_as_its_converted_gguf("phimoe");
    let Some((dir, _)) = twin("phimoe") else { return };
    let spec = ModelSpec::from_source(&HfCheckpoint::open(&dir).unwrap()).unwrap();
    assert_eq!((spec.arch.as_str(), spec.norm, spec.head_bias), ("phimoe", Norm::Layer { bias: true }, true));
    for (l, layer) in spec.layers.iter().enumerate() {
        let Mixer::Attention(a) = &layer.mixer else { panic!("block {l} attends") };
        assert!(a.bias && a.output_bias && a.window == Some(Window::Sliding(4)), "block {l}: biased and windowed: {a:?}");
        let Ffn::Moe(m) = &layer.ffn else { panic!("block {l} routes") };
        assert_eq!((m.gating.clone(), m.top_k, m.norm_top_k), (Gating::SparseMixer { jitter: 0.01 }, 2, false), "block {l}");
    }
}

#[test]
fn a_phimoe_directory_computes_what_transformers_computes() {
    // Nine positions under a window of four, every norm a LayerNorm with its bias, sparsemixer's weights and the head's
    // bias: llama.cpp's reading of the same tensors (RMSNorms plus their biases, the two kept softmax weights
    // renormalized, no window) lands far past a millionth, and so does each part of it alone.
    let Some((spec, theirs, scale, logits)) = computes_what_transformers_computes("phimoe", &[1, 17, 400, 2024, 9, 77, 3, 512, 1000])
    else {
        return;
    };
    let mut rms = spec.clone();
    rms.norm = Norm::Rms;
    let renormalized = {
        let mut s = spec.clone();
        for layer in &mut s.layers {
            if let Ffn::Moe(m) = &mut layer.ffn {
                (m.gating, m.norm_top_k) = (Gating::Softmax, true);
            }
        }
        s
    };
    let unwindowed = {
        let mut s = spec.clone();
        for layer in &mut s.layers {
            if let Mixer::Attention(a) = &mut layer.mixer {
                a.window = None;
            }
        }
        s
    };
    let llama_cpp = {
        let mut s = renormalized.clone();
        s.norm = Norm::Rms;
        for layer in &mut s.layers {
            if let Mixer::Attention(a) = &mut layer.mixer {
                a.window = None;
            }
        }
        s
    };
    let headless = ModelSpec { head_bias: false, ..spec.clone() };
    for (what, other) in [
        ("RMSNorms", rms),
        ("a renormalized softmax", renormalized),
        ("no window", unwindowed),
        ("no head bias", headless),
        ("llama.cpp's phimoe", llama_cpp),
    ] {
        let off = logits(&other).iter().zip(&theirs).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(off > scale * 1e-3, "{what}: the logits move only {off:e}, so the twin does not hold the reference to it");
    }
}

#[test]
fn a_phimoe_directorys_tokenizer_reads_as_its_converted_gguf() {
    // Phi-3.5-MoE's SentencePiece vocabulary as the converter's Phi-3 class reads it: the pieces by their own kinds, then
    // every added token the tokenizer files list, user-defined or control.
    tokenizes_as_its_converted_gguf("phimoe");
}

#[test]
fn every_phimoe_setting_the_gguf_cannot_carry_is_refused_by_name() {
    // sparsemixer chooses two experts at the 0.01 jitter the arm fixes, which no GGUF key carries; llama.cpp reads heads
    // of `hidden_size / num_attention_heads`; LongRoPE's factor tables are not read; and the converter's Phi-3 class
    // requires the original context.
    for (tag, key, value) in [
        ("three-experts", "num_experts_per_tok", serde_json::json!(3)),
        ("jitter", "router_jitter_noise", serde_json::json!(0.02)),
        ("narrow-heads", "head_dim", serde_json::json!(8)),
    ] {
        let Some(opened) = opened_with("phimoe", tag, |c| c[key] = value.clone()) else { return };
        let refused = opened.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(refused.contains(key), "`{key}` = {value} is refused naming the key; got {refused:?}");
    }
    let Some(opened) = opened_with("phimoe", "longrope", |c| {
        c["rope_scaling"] = serde_json::json!({"type": "longrope", "long_factor": vec![1.0f64; 8], "short_factor": vec![1.0f64; 8], "long_mscale": 1.2, "short_mscale": 1.2, "original_max_position_embeddings": 4096});
        c.as_object_mut().unwrap().remove("rope_parameters");
    }) else {
        return;
    };
    let refused = opened.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(refused.contains("LongRoPE"), "LongRoPE is refused by name; got {refused:?}");
    let Some(opened) = opened_with("phimoe", "no-original", |c| {
        c.as_object_mut().unwrap().remove("original_max_position_embeddings");
    }) else {
        return;
    };
    let refused = opened.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(refused.contains("original_max_position_embeddings"), "the converter's required original context is named; got {refused:?}");
    // The jitter and the experts a token at PhiMoE's own values read.
    let Some(opened) = opened_with("phimoe", "stated", |c| {
        c["router_jitter_noise"] = serde_json::json!(0.01);
        c["num_experts_per_tok"] = serde_json::json!(2);
    }) else {
        return;
    };
    opened.unwrap_or_else(|e| panic!("PhiMoE's own jitter and experts read: {e}"));
}

/// The DSA indexer every latent-attention layer of `spec` carries, by layer.
fn indexers(spec: &ModelSpec) -> Vec<Dsa> {
    spec.layers
        .iter()
        .enumerate()
        .map(|(l, layer)| match &layer.mixer {
            Mixer::Mla(m) => m.indexer.clone().unwrap_or_else(|| panic!("block {l}: latent attention with no indexer")),
            other => panic!("block {l} is {other:?}, not latent attention"),
        })
        .collect()
}

#[test]
fn a_deepseek32_directory_reads_as_its_converted_gguf() {
    // DeepSeek-V3.2 (`DeepseekV32ForCausalLM`): DeepSeek-V3's latent attention and mixture with its lightning indexer on
    // every layer, the indexer's tensors presented as the converter names them (`indexer.attn_q_b`, `indexer.attn_k`,
    // `indexer.k_norm` with its bias, `indexer.proj`).
    reads_as_its_converted_gguf("deepseek32");
    let Some((dir, _)) = twin("deepseek32") else { return };
    let spec = ModelSpec::from_source(&HfCheckpoint::open(&dir).unwrap()).unwrap();
    assert_eq!(spec.arch, "deepseek32");
    for (l, ix) in indexers(&spec).into_iter().enumerate() {
        assert_eq!(ix, Dsa { heads: 8, head_dim: 16, top_k: 4, neox: true, full: true }, "block {l}");
    }
}

#[test]
fn a_glm_dsa_directory_reads_as_its_converted_gguf() {
    // GLM-5.2 (`GlmMoeDsaForCausalLM`): the indexer's rotation interleaved, and its second layer shared, taking the first's
    // choice, with no indexer tensors of its own.
    reads_as_its_converted_gguf("glm-dsa");
    let Some((dir, _)) = twin("glm-dsa") else { return };
    let hf = HfCheckpoint::open(&dir).unwrap();
    let spec = ModelSpec::from_source(&hf).unwrap();
    let full: Vec<bool> = indexers(&spec).iter().map(|ix| ix.full).collect();
    assert_eq!(full, vec![true, false, true], "full, shared, full");
    assert!(indexers(&spec).iter().all(|ix| !ix.neox), "the indexer's rotation interleaved");
    assert!(!SpecSource::has(&hf, "blk.1.indexer.proj.weight"), "the shared layer holds no indexer");
}

/// `spec` with every latent attention's latent norms at the model's epsilon, as llama.cpp's latent-attention graphs norm
/// them.
fn latent_at_the_models_epsilon(spec: &ModelSpec) -> ModelSpec {
    let mut s = spec.clone();
    for layer in &mut s.layers {
        if let Mixer::Mla(m) = &mut layer.mixer {
            m.latent_norm_eps = spec.rms_eps;
        }
    }
    s
}

#[test]
fn a_deepseek32_directory_computes_what_transformers_computes() {
    // Nine positions, each query past the fourth attending the 4 its indexer chooses: attending every position lands far
    // past a millionth.
    let Some((spec, theirs, scale, logits)) = computes_what_transformers_computes("deepseek32", &[1, 17, 400, 2024, 9, 77, 3, 512, 1000])
    else {
        return;
    };
    let mut dense = spec.clone();
    for layer in &mut dense.layers {
        if let Mixer::Mla(m) = &mut layer.mixer {
            m.indexer = None;
        }
    }
    let off = logits(&dense).iter().zip(&theirs).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(off > scale * 1e-3, "every position attended: the logits move only {off:e}, so the twin does not hold the indexer");
}

#[test]
fn a_glm_dsa_directory_computes_what_transformers_computes() {
    // GLM-5.2's shared layer attends the positions the full layer before it chose: attending every position there lands
    // far past a millionth. At GLM-5's epsilon of 10⁻⁵ the latent norms take 10⁻⁶, as GLM-5's code builds them: normed at
    // the model's epsilon, as llama.cpp's `glm-dsa` norms them, the logits land past the millionth.
    let Some((spec, theirs, scale, logits)) = computes_what_transformers_computes("glm-dsa", &[1, 17, 400, 2024, 9, 77, 3, 512, 1000])
    else {
        return;
    };
    assert_eq!(spec.rms_eps, 1e-5, "the twin at GLM-5's epsilon");
    let mut dense_shared = spec.clone();
    if let Mixer::Mla(m) = &mut dense_shared.layers[1].mixer {
        m.indexer = None;
    }
    let off = logits(&dense_shared).iter().zip(&theirs).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(off > scale * 1e-3, "the shared layer's choice moves the logits only {off:e}");
    let off = logits(&latent_at_the_models_epsilon(&spec)).iter().zip(&theirs).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(off > scale * 1e-6, "the latent normed at the model's epsilon lands only {off:e} from transformers, within the millionth");
}

#[test]
fn the_dsa_directories_tokenizers_read_as_their_converted_ggufs() {
    // DeepSeek-V3.2's byte-pair vocabulary, its pre-tokenizer named by a probe that begins with its beginning token (the
    // tokenizer the converter asserts adds one), and GLM-5's, its special tokens named by text as `_set_vocab_glm` names
    // them.
    tokenizes_as_its_converted_gguf("deepseek32");
    tokenizes_as_its_converted_gguf("glm-dsa");
}

#[test]
fn every_dsa_setting_the_gguf_cannot_carry_is_refused_by_name() {
    // GLM-5's per-layer indexer kinds and FFN kinds must be what the GGUF can say, its rotations interleaved and its every
    // block past the dense lead a mixture; the converter's DeepSeek-V3.2 class asserts a tokenizer that adds its
    // beginning token, which the released tokenizer_config.json does not.
    for (tag, key, value) in [
        ("short-types", "indexer_types", serde_json::json!(["full", "shared"])),
        ("bad-type", "indexer_types", serde_json::json!(["full", "sparse", "full"])),
        ("mlp-types", "mlp_layer_types", serde_json::json!(["dense", "dense", "sparse"])),
        ("neox-indexer", "indexer_rope_interleave", serde_json::json!(false)),
        ("neox", "rope_interleave", serde_json::json!(false)),
        ("moe-freq", "moe_layer_freq", serde_json::json!(2)),
    ] {
        let Some(opened) = opened_with("glm-dsa", tag, |c| c[key] = value.clone()) else { return };
        let refused = opened.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(refused.contains(key), "`{key}` = {value} is refused naming the key; got {refused:?}");
    }
    let Some((dir, _)) = twin("deepseek32") else { return };
    let copy = scratch_copy("deepseek32-no-bos", &dir);
    let path = copy.join("tokenizer_config.json");
    let mut tc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    tc["add_bos_token"] = serde_json::json!(false);
    std::fs::write(&path, tc.to_string()).unwrap();
    let refused = HfCheckpoint::read_tokenizer(&copy).err().map(|e| e.to_string()).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&copy);
    assert!(refused.contains("add_bos_token"), "the released tokenizer_config.json is refused naming the key; got {refused:?}");
}

#[test]
fn glm5s_indexer_kinds_read_as_transformers_derives_them() {
    // With no `indexer_types`, transformers derives each layer's kind from `index_topk_pattern`, else from
    // `index_topk_freq` and `index_skip_topk_offset`: full where `max(l − offset + 1, 0)` is a multiple of the frequency.
    let kinds = |edit: &dyn Fn(&mut serde_json::Value)| -> Option<Vec<bool>> {
        let opened = opened_with("glm-dsa", "derived", |c| {
            c.as_object_mut().unwrap().remove("indexer_types");
            edit(c);
        })?;
        Some(indexers(&ModelSpec::from_source(&opened.unwrap()).unwrap()).iter().map(|ix| ix.full).collect())
    };
    let Some(every) = kinds(&|_| {}) else { return };
    assert_eq!(every, vec![true; 3], "transformers' defaults: every layer full");
    let stepped = kinds(&|c| {
        c["index_topk_freq"] = serde_json::json!(2);
        c["index_skip_topk_offset"] = serde_json::json!(2);
    });
    assert_eq!(stepped, Some(vec![true, true, false]), "frequency 2 from offset 2");
    assert_eq!(kinds(&|c| c["index_topk_pattern"] = serde_json::json!("FSF")), Some(vec![true, false, true]), "a pattern");
}

#[test]
fn an_lfm2_directory_reads_as_its_converted_gguf() {
    // LFM2 (`Lfm2ForCausalLM`): gated short convolutions of 3 taps on blocks 0, 1 and 3, attention with per-head QK
    // norms under a NEOX rotation on blocks 2 and 4, a dense SwiGLU after every block, the head tied, and the final norm
    // stored under the converter's name for `embedding_norm`, `token_embd_norm`. Read raw, the spec refuses it, naming
    // the norm; opened (`model::layout` presents it as `output_norm`), the directory reads as the GGUF.
    layout_reads_as_its_converted_gguf("lfm2");
    let Some((dir, gguf)) = twin("lfm2") else { return };
    let refused = ModelSpec::from_source(&Gguf::open(&gguf).unwrap()).expect_err("a raw lfm2 checkpoint is refused");
    assert!(refused.to_string().contains("token_embd_norm"), "the refusal names the stored norm: {refused}");
    let spec = ModelSpec::from_source(&*open_checkpoint(&dir).unwrap()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(spec, ModelSpec::from_source(&*open_checkpoint(&gguf).unwrap()).unwrap(), "the directory and the GGUF");
    assert!(spec.tied_output, "the head is the embedding");
    for (l, layer) in spec.layers.iter().enumerate() {
        match (&layer.mixer, l) {
            (Mixer::Recurrent(Recurrent::ShortConv(c)), 0 | 1 | 3) => assert_eq!(c.taps, 3, "block {l}: `conv_L_cache` taps"),
            (Mixer::Attention(a), 2 | 4) => {
                assert!(a.qk_norm && a.window.is_none(), "block {l}: per-head QK norms, no window");
                assert_eq!((a.heads, a.kv_heads), (4, 2), "block {l}: the heads");
                assert_eq!(a.rope.as_ref().map(|r| (r.neox, r.theta)), Some((true, 1_000_000.0)), "block {l}: NEOX at its base");
            }
            (other, _) => panic!("block {l} reads as {other:?}"),
        }
        assert_eq!(layer.ffn, Ffn::Dense { dff: 128, act: Act::Silu }, "block {l}: two thirds of 192, a multiple of 32");
    }
}

#[test]
fn an_lfm2_directory_computes_what_transformers_computes() {
    // Nine positions through three short convolutions of 3 taps and two attention blocks: each convolution carries the
    // two gated inputs before it from position to position, and the attention's QK norms and NEOX pairing bind.
    let Some((spec, theirs, scale, logits)) = computes_what_transformers_computes("lfm2", &[1, 17, 400, 2024, 9, 77, 3, 512, 1000])
    else {
        return;
    };
    let attention = |f: &dyn Fn(&mut logicaffeine_compile::model::spec::Attention)| {
        let mut s = spec.clone();
        for layer in &mut s.layers {
            if let Mixer::Attention(a) = &mut layer.mixer {
                f(a);
            }
        }
        s
    };
    let unnormed = attention(&|a| a.qk_norm = false);
    let interleaved = attention(&|a| a.rope.as_mut().unwrap().neox = false);
    for (what, other) in [("no QK norms", unnormed), ("an interleaved rotation", interleaved)] {
        let off = logits(&other).iter().zip(&theirs).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(off > scale * 1e-3, "{what}: the logits move only {off:e}, so the twin does not hold the reference to it");
    }
}

#[test]
fn an_lfm2_directorys_tokenizer_reads_as_its_converted_gguf() {
    // LFM2's byte-level BPE as the converter reads it (`pre` lfm2), its chat and tool tokens added.
    tokenizes_as_its_converted_gguf("lfm2");
}

#[test]
fn every_lfm2_setting_the_gguf_cannot_carry_is_refused_by_name() {
    // llama.cpp's lfm2 reads no short-convolution bias; LFM2's configurations restate settings under their training
    // code's names, each of which must say what transformers reads; and with no FFN multiplier transformers keeps two
    // thirds of the width where the converter rounds it up to the multiple.
    type Edit = Box<dyn Fn(&mut serde_json::Value)>;
    let set = |key: &'static str, value: serde_json::Value| -> Edit { Box::new(move |c| c[key] = value.clone()) };
    let cases: Vec<(&str, Edit, &str)> = vec![
        ("conv-bias", set("conv_bias", serde_json::json!(true)), "conv_bias"),
        ("block-dim", set("block_dim", serde_json::json!(96)), "block_dim"),
        ("conv-dim", set("conv_dim", serde_json::json!(96)), "conv_dim"),
        ("num-heads", set("num_heads", serde_json::json!(8)), "num_heads"),
        ("theta", set("theta", serde_json::json!(10000.0)), "theta"),
        ("block-eps", set("block_norm_eps", serde_json::json!(1e-6)), "block_norm_eps"),
        ("no-positions", set("use_pos_enc", serde_json::json!(false)), "use_pos_enc"),
        ("no-swiglu", set("block_use_swiglu", serde_json::json!(false)), "block_use_swiglu"),
        (
            "null-multiplier",
            Box::new(|c| {
                c["block_ffn_dim_multiplier"] = serde_json::Value::Null;
                c["intermediate_size"] = serde_json::json!(200);
            }),
            "block_ffn_dim_multiplier",
        ),
        ("short-types", set("layer_types", serde_json::json!(["conv", "full_attention"])), "layer_types"),
    ];
    for (tag, edit, named) in cases {
        let Some(opened) = opened_with("lfm2", tag, edit) else { return };
        let refused = opened.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(refused.contains(named), "{tag}: refused naming `{named}`; got {refused:?}");
    }
    // A null multiplier where two thirds of the width is already a multiple (128 of 192) is the same FFN to both.
    let Some(opened) = opened_with("lfm2", "null-multiplier-whole", |c| c["block_ffn_dim_multiplier"] = serde_json::Value::Null) else {
        return;
    };
    opened.unwrap_or_else(|e| panic!("two thirds of 192 is a multiple of 32: {e}"));
}

#[test]
fn a_mamba2_directory_reads_as_its_converted_gguf() {
    // Mamba-2 (`Mamba2ForCausalLM`): two blocks of the scan alone — eight heads of 16 over an inner width of twice the
    // model's, one group of B and C of 16 states, a 4-tap convolution with its bias, the gated norm — under the
    // checkpoint's `backbone.` names, its `A` stored as `A_log` and written as `−exp(A_log)`, and GPT-NeoX's vocabulary
    // padded to a multiple of 16.
    reads_as_its_converted_gguf("mamba2");
    let Some((dir, _)) = twin("mamba2") else { return };
    let spec = ModelSpec::from_source(&HfCheckpoint::open(&dir).unwrap()).unwrap();
    assert_eq!((spec.arch.as_str(), spec.vocab, spec.tied_output), ("mamba2", 50288, false));
    for (l, layer) in spec.layers.iter().enumerate() {
        let scan = Ssd { heads: 8, head_dim: 16, state: 16, groups: 1, conv_taps: 4, gated_norm: true };
        assert_eq!(layer.mixer, Mixer::Recurrent(Recurrent::Ssd(scan)), "block {l}: the scan");
        assert_eq!(layer.ffn, Ffn::None, "block {l}: no FFN");
    }
}

#[test]
fn a_mamba2_directory_computes_what_transformers_computes() {
    // Nine positions through two scans, each carrying its convolution's last three inputs and every head's state from
    // position to position. One group: with more, transformers' torch path norms the gated read as one group where its
    // kernels and llama.cpp norm it group by group, and the spec follows llama.cpp.
    let Some((spec, theirs, scale, logits)) = computes_what_transformers_computes("mamba2", &[1, 17, 400, 2024, 9, 77, 3, 512, 1000])
    else {
        return;
    };
    let mut normless = spec.clone();
    for layer in &mut normless.layers {
        if let Mixer::Recurrent(Recurrent::Ssd(s)) = &mut layer.mixer {
            s.gated_norm = false;
        }
    }
    let off = logits(&normless).iter().zip(&theirs).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(off > scale * 1e-3, "no gated norm: the logits move only {off:e}, so the twin does not hold the reference to it");
}

#[test]
fn a_mamba2_directorys_tokenizer_reads_as_its_converted_gguf() {
    // GPT-NeoX's byte-level BPE (`pre` olmo, as the converter's probe names it), its vocabulary padded with the
    // converter's `[PAD<n>]` tokens to a multiple of 16.
    tokenizes_as_its_converted_gguf("mamba2");
}

#[test]
fn every_mamba2_setting_the_gguf_cannot_carry_is_refused_by_name() {
    // llama.cpp's mamba2 builds the block as transformers' defaults do: no projection biases, a biased convolution, an
    // unclamped time step; and the converter asserts an inner width of `expand` times the model's and derives the heads
    // from it. transformers gates and then RMS-normalizes whatever `rms_norm` and `norm_before_gate` say, as llama.cpp
    // does, so those read (Mamba-Codestral states `norm_before_gate`).
    type Edit = Box<dyn Fn(&mut serde_json::Value)>;
    let set = |key: &'static str, value: serde_json::Value| -> Edit { Box::new(move |c| c[key] = value.clone()) };
    let cases: Vec<(&str, Edit, &str)> = vec![
        ("biased", set("use_bias", serde_json::json!(true)), "use_bias"),
        ("unbiased-conv", set("use_conv_bias", serde_json::json!(false)), "use_conv_bias"),
        ("clamped", set("time_step_limit", serde_json::json!([0.001, 0.1])), "time_step_limit"),
        ("heads", set("num_heads", serde_json::json!(4)), "num_heads"),
        ("inner", set("intermediate_size", serde_json::json!(96)), "inner width"),
    ];
    for (tag, edit, named) in cases {
        let Some(opened) = opened_with("mamba2", tag, edit) else { return };
        let refused = opened.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(refused.contains(named), "{tag}: refused naming `{named}`; got {refused:?}");
    }
    // An unbounded limit, as transformers writes its default, clamps nothing.
    let Some(opened) = opened_with("mamba2", "unbounded", |c| c["time_step_limit"] = serde_json::json!([0.0, null])) else { return };
    opened.unwrap_or_else(|e| panic!("an unbounded time step reads: {e}"));
    let inert: Vec<(&str, Edit)> = vec![
        ("no-rms-norm", set("rms_norm", serde_json::json!(false))),
        ("norm-before-gate", set("norm_before_gate", serde_json::json!(true))),
        ("codestral-init", Box::new(|c| {
            c["time_step_init_scheme"] = serde_json::json!("random");
            c["time_step_scale"] = serde_json::json!(1.0);
        })),
    ];
    for (tag, edit) in inert {
        let Some(opened) = opened_with("mamba2", tag, edit) else { return };
        opened.unwrap_or_else(|e| panic!("{tag}: a key no reader of the block reads is refused: {e}"));
    }
}

#[test]
fn an_unbounded_time_step_reads_in_every_spelling_python_writes() {
    // transformers 5 saves the default `time_step_limit` of `(0.0, inf)` as `[0.0, {"__float__": "Infinity"}]`, earlier
    // releases as Python's `json` writes the float, a bare `Infinity` that JSON has no number for (Mamba-Codestral,
    // Falcon-H1, Nemotron-H). Each spelling is the unclamped scan llama.cpp runs; a tagged NaN or a finite bound clamps.
    for family in ["mamba2", "falcon-h1", "nemotron_h"] {
        for (tag, limit, reads) in [
            ("tagged", r#"[0.0, {"__float__": "Infinity"}]"#, true),
            ("bare", "[0.0, Infinity]", true),
            ("null", "[0.0, null]", true),
            ("tagged-nan", r#"[0.0, {"__float__": "NaN"}]"#, false),
            ("bare-nan", "[0.0, NaN]", false),
            ("finite", "[0.0, 100.0]", false),
            ("floored", "[0.5, Infinity]", false),
        ] {
            let Some((dir, _)) = twin(family) else { return };
            let copy = scratch_copy(&format!("{family}-limit-{tag}"), &dir);
            let path = copy.join("config.json");
            let mut config: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            config["time_step_limit"] = serde_json::json!("@limit@");
            let text = serde_json::to_string_pretty(&config).unwrap().replace("\"@limit@\"", limit);
            std::fs::write(&path, text).unwrap();
            let opened = HfCheckpoint::open(&copy).map(|_| ());
            let _ = std::fs::remove_dir_all(&copy);
            match (opened, reads) {
                (Ok(()), true) => {}
                (Err(e), false) => assert!(e.to_string().contains("time_step_limit"), "{family} {limit}: refused naming `time_step_limit`; got {e}"),
                (Ok(()), false) => panic!("{family} {limit}: a clamped time step reads"),
                (Err(e), true) => panic!("{family} {limit}: an unbounded time step is refused: {e}"),
            }
        }
    }
}

#[test]
fn a_falcon_h1_directory_reads_as_its_converted_gguf() {
    // Falcon-H1 (`FalconH1ForCausalLM`): in each of three blocks attention beside Mamba-2's scan on one normed input,
    // then a dense SwiGLU, with every MuP multiplier folded into the weights to the bit as the converter folds it (the
    // key's two one after the other, the scan's input projection by its multiplier and then each of its five parts by
    // its own), and the FFN norm stored as `blk.<n>.ffn_norm`, as llama.cpp loads it. Read raw, the spec refuses that
    // name; opened (`model::layout` presents it with its `.weight`), the directory reads as the GGUF.
    layout_reads_as_its_converted_gguf("falcon-h1");
    let Some((dir, gguf)) = twin("falcon-h1") else { return };
    let refused = ModelSpec::from_source(&Gguf::open(&gguf).unwrap()).expect_err("a raw falcon-h1 checkpoint is refused");
    assert!(refused.to_string().contains("ffn_norm"), "the refusal names the stored norm: {refused}");
    let spec = ModelSpec::from_source(&*open_checkpoint(&dir).unwrap()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(spec, ModelSpec::from_source(&*open_checkpoint(&gguf).unwrap()).unwrap(), "the directory and the GGUF");
    assert!(!spec.tied_output, "an untied head");
    for (l, layer) in spec.layers.iter().enumerate() {
        let Mixer::Recurrent(Recurrent::Parallel(a, scan)) = &layer.mixer else { panic!("block {l} reads as {:?}", layer.mixer) };
        assert_eq!((a.heads, a.kv_heads, a.head_dim), (4, 2, 16), "block {l}: the attention's heads");
        assert_eq!(a.rope.as_ref().map(|r| r.neox), Some(true), "block {l}: a NEOX rotation");
        assert_eq!(*scan, Ssd { heads: 8, head_dim: 16, state: 16, groups: 2, conv_taps: 4, gated_norm: true }, "block {l}: the scan");
        assert_eq!(layer.ffn, Ffn::Dense { dff: 128, act: Act::Silu }, "block {l}: the FFN");
    }
}

#[test]
fn a_falcon_h1_directory_computes_what_transformers_computes() {
    // Nine positions through three blocks of attention beside the scan: the scan's grouped gated norm groups as
    // transformers' `FalconH1RMSNormGated` does, and each half of the block binds.
    let Some((spec, theirs, scale, logits)) = computes_what_transformers_computes("falcon-h1", &[17, 1, 400, 2024, 9, 77, 3, 512, 1000])
    else {
        return;
    };
    let each = |f: &dyn Fn(&mut Mixer)| {
        let mut s = spec.clone();
        for layer in &mut s.layers {
            f(&mut layer.mixer);
        }
        s
    };
    let attention_alone = each(&|m| {
        if let Mixer::Recurrent(Recurrent::Parallel(a, _)) = m {
            *m = Mixer::Attention((**a).clone());
        }
    });
    let scan_alone = each(&|m| {
        if let Mixer::Recurrent(Recurrent::Parallel(_, s)) = m {
            *m = Mixer::Recurrent(Recurrent::Ssd(s.clone()));
        }
    });
    let ungrouped = each(&|m| {
        if let Mixer::Recurrent(Recurrent::Parallel(_, s)) = m {
            s.groups = 1;
        }
    });
    for (what, other) in [("attention alone", attention_alone), ("the scan alone", scan_alone), ("the scan in one group", ungrouped)] {
        let off = logits(&other).iter().zip(&theirs).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(off > scale * 1e-3, "{what}: the logits move only {off:e}, so the twin does not hold the reference to it");
    }
}

#[test]
fn a_falcon_h1_directorys_tokenizer_reads_as_its_converted_gguf() {
    // Falcon-H1's byte-level BPE (`pre` falcon-h1), its vocabulary padded to the configuration's 32,784 entries.
    tokenizes_as_its_converted_gguf("falcon-h1");
}

#[test]
fn every_falcon_h1_setting_the_gguf_cannot_carry_is_refused_by_name() {
    // llama.cpp's scan adds its convolution's bias unconditionally and gates the read before its norm; its falcon-h1
    // reads no projection bias and adds no attention output bias; the converter divides by `mamba_d_head`, asserts SiLU,
    // writes the rotation's base alone, and folds the embedding's multiplier into the one table a tied head reads.
    type Edit = Box<dyn Fn(&mut serde_json::Value)>;
    let set = |key: &'static str, value: serde_json::Value| -> Edit { Box::new(move |c| c[key] = value.clone()) };
    let cases: Vec<(&str, Edit, &str)> = vec![
        ("no-conv-bias", set("mamba_conv_bias", serde_json::json!(false)), "mamba_conv_bias"),
        ("proj-bias", set("mamba_proj_bias", serde_json::json!(true)), "mamba_proj_bias"),
        ("projectors-bias", set("projectors_bias", serde_json::json!(true)), "projectors_bias"),
        ("attention-bias", set("attention_bias", serde_json::json!(true)), "attention_bias"),
        ("mlp-bias", set("mlp_bias", serde_json::json!(true)), "mlp_bias"),
        ("norm-before-gate", set("mamba_norm_before_gate", serde_json::json!(true)), "mamba_norm_before_gate"),
        ("clamped", set("time_step_limit", serde_json::json!([0.001, 0.1])), "time_step_limit"),
        ("gelu", set("hidden_act", serde_json::json!("gelu")), "hidden_act"),
        ("auto-head", set("mamba_d_head", serde_json::json!("auto")), "mamba_d_head"),
        ("heads", set("mamba_n_heads", serde_json::json!(4)), "mamba_n_heads"),
        ("four-ssm-multipliers", set("ssm_multipliers", serde_json::json!([1.0, 1.0, 1.0, 1.0])), "ssm_multipliers"),
        (
            "yarn",
            set("rope_parameters", serde_json::json!({"rope_type": "yarn", "rope_theta": 1e11, "factor": 2.0, "original_max_position_embeddings": 8192})),
            "rope_parameters",
        ),
        (
            "tied",
            Box::new(|c| {
                c["tie_word_embeddings"] = serde_json::json!(true);
            }),
            "lm_head_multiplier",
        ),
    ];
    for (tag, edit, named) in cases {
        let Some(opened) = opened_with("falcon-h1", tag, edit) else { return };
        let refused = opened.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(refused.contains(named), "{tag}: refused naming `{named}`; got {refused:?}");
    }
    // A gated norm after the gate, as every published Falcon-H1 has it, reads.
    let Some(opened) = opened_with("falcon-h1", "gate-then-norm", |c| c["mamba_norm_before_gate"] = serde_json::json!(false)) else { return };
    opened.unwrap_or_else(|e| panic!("the gated norm after the gate reads: {e}"));
}

#[test]
fn a_nemotron_h_directory_reads_as_its_converted_gguf() {
    // Nemotron-H (`NemotronHForCausalLM`, `M-M*-`): blocks of one sublayer each under its one norm — Mamba-2's scan in
    // eight heads of 16 and two groups, an FFN of relu² with no gate, attention with no rotation (its queries and keys
    // permuted as the converter's Llama class permutes them, which leaves every score as it is) — with each block's KV
    // heads and FFN width written per block, the epsilon as a LayerNorm's, and Nemotron-Nano-9B-v2's vocabulary.
    reads_as_its_converted_gguf("nemotron_h");
    let Some((dir, _)) = twin("nemotron_h") else { return };
    let spec = ModelSpec::from_source(&HfCheckpoint::open(&dir).unwrap()).unwrap();
    assert_eq!((spec.arch.as_str(), spec.rms_eps as f32), ("nemotron_h", 1e-5), "the architecture and its epsilon");
    let scan = Ssd { heads: 8, head_dim: 16, state: 16, groups: 2, conv_taps: 4, gated_norm: true };
    for (l, layer) in spec.layers.iter().enumerate() {
        match (l, &layer.mixer, &layer.ffn) {
            (0 | 2, Mixer::Recurrent(Recurrent::Ssd(s)), Ffn::None) => assert_eq!(*s, scan, "block {l}: the scan"),
            (1 | 4, Mixer::None, Ffn::Dense { dff: 128, act: Act::ReluSquared }) => {}
            (3, Mixer::Attention(a), Ffn::None) => {
                assert_eq!((a.heads, a.kv_heads, a.head_dim, a.rope.is_none()), (4, 2, 16, true), "block {l}: unrotated attention")
            }
            (_, mixer, ffn) => panic!("block {l} reads as {mixer:?} and {ffn:?}"),
        }
    }
}

#[test]
fn a_nemotron_h_directory_computes_what_transformers_computes() {
    // Nine positions through two scans, an attention block and two FFN blocks, each block's one sublayer normed by its
    // one norm: an FFN normed by an FFN norm of its own, or attention rotated, lands far past a millionth.
    let Some((spec, theirs, scale, logits)) = computes_what_transformers_computes("nemotron_h", &[1, 17, 400, 2024, 9, 77, 3, 512, 1000])
    else {
        return;
    };
    let mut ungrouped = spec.clone();
    for layer in &mut ungrouped.layers {
        if let Mixer::Recurrent(Recurrent::Ssd(s)) = &mut layer.mixer {
            s.groups = 1;
        }
    }
    let off = logits(&ungrouped).iter().zip(&theirs).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(off > scale * 1e-3, "the scan in one group: the logits move only {off:e}, so the twin does not hold the reference to it");
}

#[test]
fn a_nemotron_h_directorys_tokenizer_reads_as_its_converted_gguf() {
    // Nemotron-Nano-9B-v2's byte-level BPE (`pre` tekken) as `NemotronHModel.set_vocab` reads it through the tokenizer.
    tokenizes_as_its_converted_gguf("nemotron_h");
}

#[test]
fn every_nemotron_h_setting_the_gguf_cannot_carry_is_refused_by_name() {
    // llama.cpp's nemotron_h adds the scan's convolution bias unconditionally and reads no other bias of the scan or the
    // FFN, computes SiLU in the scan and relu² in the FFN, windows no attention and clamps no time step; the converter
    // writes a `layers_block_type` list's FFN blocks with no FFN width; Nemotron 3's mixture blocks are not read yet.
    type Edit = Box<dyn Fn(&mut serde_json::Value)>;
    let set = |key: &'static str, value: serde_json::Value| -> Edit { Box::new(move |c| c[key] = value.clone()) };
    let cases: Vec<(&str, Edit, &str)> = vec![
        ("no-conv-bias", set("use_conv_bias", serde_json::json!(false)), "use_conv_bias"),
        ("scan-bias", set("use_bias", serde_json::json!(true)), "use_bias"),
        ("proj-bias", set("mamba_proj_bias", serde_json::json!(true)), "mamba_proj_bias"),
        ("mlp-bias", set("mlp_bias", serde_json::json!(true)), "mlp_bias"),
        ("attention-bias", set("attention_bias", serde_json::json!(true)), "attention_bias"),
        ("gelu-scan", set("mamba_hidden_act", serde_json::json!("gelu")), "mamba_hidden_act"),
        ("silu-ffn", set("mlp_hidden_act", serde_json::json!("silu")), "mlp_hidden_act"),
        ("windowed", set("sliding_window", serde_json::json!(4)), "sliding_window"),
        ("clamped", set("time_step_limit", serde_json::json!([0.001, 0.1])), "time_step_limit"),
        ("experts", set("hybrid_override_pattern", serde_json::json!("MEM*E")), "hybrid_override_pattern"),
        ("short-pattern", set("hybrid_override_pattern", serde_json::json!("M-M*")), "4 blocks"),
        ("three-groups", set("n_groups", serde_json::json!(3)), "groups"),
        (
            "listed-ffn",
            Box::new(|c| {
                c.as_object_mut().unwrap().remove("hybrid_override_pattern");
                c["layers_block_type"] = serde_json::json!(["mamba", "mlp", "mamba", "attention", "mlp"]);
            }),
            "layers_block_type",
        ),
    ];
    for (tag, edit, named) in cases {
        let Some(opened) = opened_with("nemotron_h", tag, edit) else { return };
        let refused = opened.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(refused.contains(named), "{tag}: refused naming `{named}`; got {refused:?}");
    }
}

#[test]
fn a_jamba_directory_reads_as_its_converted_gguf() {
    // Jamba (`JambaForCausalLM`): Mamba-1's selective scan on blocks 0 and 2 with its time step, B and C each RMS-normed,
    // attention with no rotation on blocks 1 and 3; four experts, two a token, the softmax weights kept as they stand,
    // after each scan block (its experts stacked one tensor a projection), a dense SwiGLU after each attention block; the
    // head tied; Jamba-Reasoning-3B's vocabulary read as `LlamaHfVocab` reads it.
    reads_as_its_converted_gguf("jamba");
    let Some((dir, _)) = twin("jamba") else { return };
    let spec = ModelSpec::from_source(&HfCheckpoint::open(&dir).unwrap()).unwrap();
    assert!(spec.tied_output, "the head is the embedding");
    let scan = Selective { inner: 128, state: 16, dt_rank: 8, conv_taps: 4, inner_norms: true };
    for (l, layer) in spec.layers.iter().enumerate() {
        match (l % 2, &layer.mixer, &layer.ffn) {
            (0, Mixer::Recurrent(Recurrent::Selective(s)), Ffn::Moe(m)) => {
                assert_eq!(*s, scan, "block {l}: the scan");
                assert_eq!((m.experts, m.top_k, m.gating.clone(), m.norm_top_k), (4, 2, Gating::Softmax, false), "block {l}: the mixture");
            }
            (1, Mixer::Attention(a), Ffn::Dense { dff: 128, act: Act::Silu }) => {
                assert_eq!((a.heads, a.kv_heads, a.rope.is_none()), (4, 2, true), "block {l}: unrotated attention")
            }
            (_, mixer, ffn) => panic!("block {l} reads as {mixer:?} and {ffn:?}"),
        }
    }
}

#[test]
fn a_jamba_directory_computes_what_transformers_computes() {
    // Nine positions through two selective scans and two attention blocks: each scan's dt, B and C norms bind, and the
    // mixture's softmax weights are not renormalized.
    let Some((spec, theirs, scale, logits)) = computes_what_transformers_computes("jamba", &[1, 17, 400, 2024, 9, 77, 3, 512, 1000])
    else {
        return;
    };
    let mut normless = spec.clone();
    let mut renormalized = spec.clone();
    for layer in &mut normless.layers {
        if let Mixer::Recurrent(Recurrent::Selective(s)) = &mut layer.mixer {
            s.inner_norms = false;
        }
    }
    for layer in &mut renormalized.layers {
        if let Ffn::Moe(m) = &mut layer.ffn {
            m.norm_top_k = true;
        }
    }
    for (what, other) in [("no dt, B and C norms", normless), ("renormalized experts", renormalized)] {
        let off = logits(&other).iter().zip(&theirs).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(off > scale * 1e-3, "{what}: the logits move only {off:e}, so the twin does not hold the reference to it");
    }
}

#[test]
fn a_jamba_directorys_tokenizer_reads_as_its_converted_gguf() {
    // Jamba-Reasoning-3B ships a byte-fallback tokenizer.json and no SentencePiece model: `LlamaHfVocab`'s entries, every
    // one scored −1000, its byte tokens typed as bytes, its named and additional special tokens as control tokens.
    tokenizes_as_its_converted_gguf("jamba");
}

#[test]
fn every_jamba_setting_the_gguf_cannot_carry_is_refused_by_name() {
    // llama.cpp's jamba reads a convolution bias in every scan block and no projection bias, computes SiLU, windows no
    // attention; the converter writes `mamba_dt_rank` as it stands and counts the layer periods from their offsets.
    type Edit = Box<dyn Fn(&mut serde_json::Value)>;
    let set = |key: &'static str, value: serde_json::Value| -> Edit { Box::new(move |c| c[key] = value.clone()) };
    let cases: Vec<(&str, Edit, &str)> = vec![
        ("no-conv-bias", set("mamba_conv_bias", serde_json::json!(false)), "mamba_conv_bias"),
        ("proj-bias", set("mamba_proj_bias", serde_json::json!(true)), "mamba_proj_bias"),
        ("gelu", set("hidden_act", serde_json::json!("gelu")), "hidden_act"),
        ("windowed", set("sliding_window", serde_json::json!(4)), "sliding_window"),
        ("auto-rank", set("mamba_dt_rank", serde_json::json!("auto")), "mamba_dt_rank"),
        ("no-period", set("attn_layer_period", serde_json::json!(0)), "period"),
        ("too-many-chosen", set("num_experts_per_tok", serde_json::json!(5)), "num_experts_per_tok"),
    ];
    for (tag, edit, named) in cases {
        let Some(opened) = opened_with("jamba", tag, edit) else { return };
        let refused = opened.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(refused.contains(named), "{tag}: refused naming `{named}`; got {refused:?}");
    }
}

#[test]
fn a_gemma3n_directory_reads_as_its_converted_gguf() {
    // Gemma 3n (`Gemma3nForCausalLM`, and as published under `Gemma3nForConditionalGeneration`'s prefix): Gemma 3's
    // blocks with the scores unscaled and the values normed with no weight, four AltUp streams (the converter stacking
    // their three projections each way), LAuReL of 64, per-layer inputs of 256 (the converter padding their table, and
    // the embedding, with zero rows to the whole vocabulary), the first ten blocks' gates sparsified at the 95th
    // percentile (its quantile as torch computes it in f32, −∞ for none), the cache shared from block 20.
    for family in ["gemma3n", "gemma3n-mm"] {
        reads_as_its_converted_gguf(family);
    }
    let Some((dir, _)) = twin("gemma3n") else { return };
    let spec = ModelSpec::from_source(&HfCheckpoint::open(&dir).unwrap()).unwrap();
    assert_eq!((spec.vocab, spec.final_logit_softcap, spec.tied_output), (262400, Some(30.0), true), "the padded vocabulary, the cap, the tied head");
    assert_eq!(
        spec.parts,
        [
            Part::PerLayerInput { dim: 256 },
            Part::AltUp { streams: 4, active: 0 },
            Part::Laurel { rank: 64 },
            Part::ValueNorm,
            Part::GateSparsity { layers: 3, std_mul: f32::from_bits(0x3fd2_8a90) as f64 },
        ],
        "Gemma 3n's parts, none shared under twenty blocks"
    );
    for (l, layer) in spec.layers.iter().enumerate() {
        let Mixer::Attention(a) = &layer.mixer else { panic!("block {l} attends") };
        assert_eq!((a.scale, a.qk_norm, a.window.is_some()), (1.0, true, l < 2), "block {l}: unscaled, QK-normed, windowed but the last");
        assert!(layer.post_mixer_norm && layer.post_ffn_norm, "block {l}: Gemma's sandwich norms");
    }
}

#[test]
fn a_gemma3n_directory_computes_what_transformers_computes() {
    // Nine positions through three blocks of four streams under windows of 4: each part binds — the value norm, LAuReL,
    // the gate's sparsity, the per-layer inputs — and the sparsity's deviation is the population's, as transformers
    // takes it (llama.cpp's `gaussian_topk` divides by `n − 1`).
    let Some((spec, theirs, scale, logits)) = computes_what_transformers_computes("gemma3n", &[2, 17, 400, 2024, 9, 77, 3, 512, 1000])
    else {
        return;
    };
    let without = |part: fn(&Part) -> bool| ModelSpec { parts: spec.parts.iter().filter(|p| !part(p)).cloned().collect(), ..spec.clone() };
    let denser = ModelSpec {
        parts: spec.parts.iter().map(|p| match p { Part::GateSparsity { layers, .. } => Part::GateSparsity { layers: *layers, std_mul: 1.0 }, p => p.clone() }).collect(),
        ..spec.clone()
    };
    for (what, other) in [
        ("no value norm", without(|p| *p == Part::ValueNorm)),
        ("no sparsity", without(|p| matches!(p, Part::GateSparsity { .. }))),
        ("a sparsity of one deviation", denser),
    ] {
        let off = logits(&other).iter().zip(&theirs).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(off > scale * 1e-3, "{what}: the logits move only {off:e}, so the twin does not hold the reference to it");
    }
}

#[test]
fn a_gemma3n_directorys_tokenizer_reads_as_its_converted_gguf() {
    // Gemma 3n's SentencePiece vocabulary, padded to its 262,400 entries, with no space prefix.
    tokenizes_as_its_converted_gguf("gemma3n");
}

#[test]
fn every_gemma3n_setting_the_gguf_cannot_carry_is_refused_by_name() {
    // llama.cpp's gemma3n fixes four AltUp streams with the first active, LAuReL of 64, per-layer inputs of 256, the cache
    // shared from block 20, the first ten blocks' gates sparsified at 0.95, the corrected stream scaled; it reads no
    // attention bias and computes the tanh GELU.
    type Edit = Box<dyn Fn(&mut serde_json::Value)>;
    let set = |key: &'static str, value: serde_json::Value| -> Edit { Box::new(move |c| c[key] = value.clone()) };
    let cases: Vec<(&str, Edit, &str)> = vec![
        ("streams", set("altup_num_inputs", serde_json::json!(2)), "altup_num_inputs"),
        ("active", set("altup_active_idx", serde_json::json!(1)), "altup_active_idx"),
        ("laurel", set("laurel_rank", serde_json::json!(32)), "laurel_rank"),
        ("per-layer", set("hidden_size_per_layer_input", serde_json::json!(128)), "hidden_size_per_layer_input"),
        ("shared", set("num_kv_shared_layers", serde_json::json!(1)), "num_kv_shared_layers"),
        ("sparsity", set("activation_sparsity_pattern", serde_json::json!([0.95, 0.95, 0.0])), "activation_sparsity_pattern"),
        ("unscaled", set("altup_correct_scale", serde_json::json!(false)), "altup_correct_scale"),
        ("bias", set("attention_bias", serde_json::json!(true)), "attention_bias"),
        ("gelu", set("hidden_activation", serde_json::json!("gelu")), "hidden_activation"),
    ];
    for (tag, edit, named) in cases {
        let Some(opened) = opened_with("gemma3n", tag, edit) else { return };
        let refused = opened.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(refused.contains(named), "{tag}: refused naming `{named}`; got {refused:?}");
    }
}

#[test]
fn a_kimi_linear_directory_reads_as_its_converted_gguf() {
    // Kimi-Linear (`KimiLinearForCausalLM`, its configuration as Moonshot publishes it): Kimi delta attention on blocks 0
    // to 2 (each convolution `[taps, 1, channels, 1]`, the decay `−exp(A_log)` a value a head), latent attention with no
    // rotation on block 3 (a full-rank query, the latent's expansion split into its key and value halves); a dense SwiGLU,
    // then four experts, two a token, under sigmoid scores with a selection bias, renormalized and scaled by 2.446, beside
    // a shared expert; Kimi's tiktoken vocabulary.
    reads_as_its_converted_gguf("kimi-linear");
    let Some((dir, _)) = twin("kimi-linear") else { return };
    let spec = ModelSpec::from_source(&HfCheckpoint::open(&dir).unwrap()).unwrap();
    for (l, layer) in spec.layers.iter().enumerate() {
        match (l, &layer.mixer) {
            (0..=2, Mixer::Recurrent(Recurrent::DeltaAttention(d))) => {
                assert_eq!((d.heads, d.head_dim, d.conv_taps), (4, 16, 4), "block {l}: the delta attention")
            }
            (3, Mixer::Mla(m)) => {
                assert_eq!((m.q_lora_rank, m.kv_lora_rank, m.qk_nope, m.qk_rope, m.v_dim, m.rope.is_none()), (None, 32, 16, 8, 16, true), "block {l}")
            }
            (_, mixer) => panic!("block {l} reads as {mixer:?}"),
        }
        match (l, &layer.ffn) {
            (0, Ffn::Dense { dff: 128, act: Act::Silu }) => {}
            (1.., Ffn::Moe(m)) => {
                assert_eq!((m.experts, m.top_k, m.gating.clone(), m.norm_top_k), (4, 2, Gating::Sigmoid { bias: true }, true), "block {l}");
                assert_eq!((m.weights_scale as f32, m.shared_experts, m.shared_dff), (2.446, 1, 32), "block {l}: the scale and the shared expert");
            }
            (_, ffn) => panic!("block {l}'s FFN reads as {ffn:?}"),
        }
    }
}

#[test]
fn a_kimi_linear_directory_computes_what_transformers_computes() {
    // Nine positions through three delta attention blocks, each carrying its three convolutions and every head's memory,
    // and an unrotated latent attention block; the mixture's renormalization and its scores bind. At the releases'
    // epsilon of 10⁻⁵ the latent norm (`kv_a_layernorm`) takes 10⁻⁶, as Kimi's code builds it: normed at the model's
    // epsilon, as llama.cpp's `kimi-linear` norms it, the logits land past the millionth.
    let Some((spec, theirs, scale, logits)) = computes_what_transformers_computes("kimi-linear", &[1, 17, 400, 2024, 9, 77, 3, 512, 1000])
    else {
        return;
    };
    assert_eq!(spec.rms_eps, 1e-5, "the twin at Kimi-Linear's epsilon");
    let off = logits(&latent_at_the_models_epsilon(&spec)).iter().zip(&theirs).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(off > scale * 1e-6, "the latent normed at the model's epsilon lands only {off:e} from transformers, within the millionth");
    let routed = |f: &dyn Fn(&mut Moe)| {
        let mut s = spec.clone();
        for layer in &mut s.layers {
            if let Ffn::Moe(m) = &mut layer.ffn {
                f(m);
            }
        }
        s
    };
    for (what, other) in [("kept weights not renormalized", routed(&|m| m.norm_top_k = false)), ("softmax scores", routed(&|m| m.gating = Gating::Softmax))] {
        let off = logits(&other).iter().zip(&theirs).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(off > scale * 1e-3, "{what}: the logits move only {off:e}, so the twin does not hold the reference to it");
    }
}

#[test]
fn a_kimi_linear_directorys_tokenizer_reads_as_its_converted_gguf() {
    // Kimi's tiktoken vocabulary, as the converter reads Kimi K2's (`pre` kimi-k2).
    tokenizes_as_its_converted_gguf("kimi-linear");
}

#[test]
fn every_kimi_linear_setting_the_gguf_cannot_carry_is_refused_by_name() {
    // llama.cpp's kimi-linear renormalizes the kept weights, rotates no latent key, reads no attention bias, routes every
    // block past the dense ones among every expert and reads no draft block; the converter reads the blocks' kinds from
    // `linear_attn_config`, which transformers 5 saves apart.
    type Edit = Box<dyn Fn(&mut serde_json::Value)>;
    let set = |key: &'static str, value: serde_json::Value| -> Edit { Box::new(move |c| c[key] = value.clone()) };
    let cases: Vec<(&str, Edit, &str)> = vec![
        ("unnormalized", set("moe_renormalize", serde_json::json!(false)), "moe_renormalize"),
        ("rotated", set("mla_use_nope", serde_json::json!(false)), "mla_use_nope"),
        ("biased", set("attention_bias", serde_json::json!(true)), "attention_bias"),
        ("every-other", set("moe_layer_freq", serde_json::json!(2)), "moe_layer_freq"),
        ("grouped", set("num_expert_group", serde_json::json!(2)), "num_expert_group"),
        ("draft", set("num_nextn_predict_layers", serde_json::json!(1)), "num_nextn_predict_layers"),
        ("relu-router", set("moe_router_activation_func", serde_json::json!("relu")), "moe_router_activation_func"),
        ("gelu", set("hidden_act", serde_json::json!("gelu")), "hidden_act"),
        (
            "unlisted",
            Box::new(|c| {
                c.as_object_mut().unwrap().remove("linear_attn_config");
            }),
            "linear_attn_config",
        ),
        (
            "both",
            Box::new(|c| c["linear_attn_config"]["kda_layers"] = serde_json::json!([1, 2, 3, 4])),
            "linear_attn_config",
        ),
    ];
    for (tag, edit, named) in cases {
        let Some(opened) = opened_with("kimi-linear", tag, edit) else { return };
        let refused = opened.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(refused.contains(named), "{tag}: refused naming `{named}`; got {refused:?}");
    }
}

#[test]
fn a_minimax_01_directory_reads_as_its_converted_gguf() {
    // MiniMax-01 as transformers publishes it (`MiniMaxForCausalLM`, MiniMax-Text-01-hf's configuration) and as MiniMax
    // publishes it (`MiniMaxText01ForCausalLM`, the class the converter registers): lightning attention on blocks 0 to 6
    // (four heads of 16 through one fused projection, with a gate and a norm of their own), attention under a NEOX
    // rotation of `rotary_dim` on block 7, every eighth as llama.cpp fixes it; four experts, two a token, the softmax's
    // kept weights renormalized; each sublayer's residual its normed input times 3.5566. The two read alike.
    for family in ["minimax-01", "minimax-text-01"] {
        reads_as_its_converted_gguf(family);
    }
    let (Some((hf, _)), Some((own, _))) = (twin("minimax-01"), twin("minimax-text-01")) else { return };
    let spec = ModelSpec::from_source(&HfCheckpoint::open(&hf).unwrap()).unwrap();
    assert_eq!(ModelSpec::from_source(&HfCheckpoint::open(&own).unwrap()).unwrap(), spec, "MiniMax's configuration reads as transformers'");
    let [Part::NormedResidual { scale }] = &spec.parts[..] else { panic!("MiniMax-01's parts: {:?}", spec.parts) };
    assert_eq!(*scale as f32, 3.5565588200778455f32, "the residual's scale, as the converter writes it");
    for (l, layer) in spec.layers.iter().enumerate() {
        match (l, &layer.mixer) {
            (0..=6, Mixer::Recurrent(Recurrent::Lightning(x))) => assert_eq!((x.heads, x.head_dim), (4, 16), "block {l}: the lightning attention"),
            (7, Mixer::Attention(a)) => {
                let rope = a.rope.as_ref().map(|r| (r.dims, r.neox));
                assert_eq!((a.heads, a.kv_heads, a.head_dim, rope, a.qk_norm, a.fused_qkv), (4, 2, 16, Some((16, true)), false, false), "block {l}")
            }
            (_, mixer) => panic!("block {l} reads as {mixer:?}"),
        }
        let Ffn::Moe(m) = &layer.ffn else { panic!("block {l} routes") };
        assert_eq!((m.experts, m.top_k, m.expert_dff, m.gating.clone(), m.norm_top_k, m.weights_scale), (4, 2, 32, Gating::Softmax, true, 1.0), "block {l}");
    }
    // The rotated width is `rotary_dim`, which the converter writes and MiniMax's code and llama.cpp rotate (transformers'
    // `MiniMaxForCausalLM` rotates the whole head whatever it says); MiniMax-M1's class reads as MiniMax-Text-01's.
    for (family, tag, edit) in [
        ("minimax-01", "half-rotated", Box::new(|c: &mut serde_json::Value| c["rotary_dim"] = serde_json::json!(8)) as Box<dyn Fn(&mut serde_json::Value)>),
        ("minimax-text-01", "m1", Box::new(|c: &mut serde_json::Value| c["architectures"] = serde_json::json!(["MiniMaxM1ForCausalLM"]))),
    ] {
        let Some(opened) = opened_with(family, tag, edit) else { return };
        let read = ModelSpec::from_source(&opened.unwrap_or_else(|e| panic!("{tag}: {e}"))).unwrap();
        let Mixer::Attention(a) = &read.layers[7].mixer else { panic!("{tag}: block 7 attends") };
        assert_eq!(a.rope.as_ref().map(|r| r.dims), Some(if tag == "m1" { 16 } else { 8 }), "{tag}: the rotated width");
    }
}

#[test]
fn a_minimax_01_directory_computes_what_transformers_computes() {
    // Nine positions through seven lightning blocks, each head's memory decaying at its ALiBi slope scaled by its block's
    // depth, and an attention block; the residual from the normed input, its scale and the mixture's renormalization
    // bind. The lightning read is normed at 10⁻⁶, as MiniMax's code and transformers norm it (llama.cpp norms it at the
    // model's 10⁻⁵); transformers takes the depth scale over `L − 1 + 10⁻⁵` blocks, MiniMax's code and the spec over
    // `L − 1`.
    let Some((spec, theirs, scale, logits)) = computes_what_transformers_computes("minimax-01", &[1, 17, 400, 2024, 9, 77, 3, 512, 1000])
    else {
        return;
    };
    let routed = |f: &dyn Fn(&mut Moe)| {
        let mut s = spec.clone();
        for layer in &mut s.layers {
            if let Ffn::Moe(m) = &mut layer.ffn {
                f(m);
            }
        }
        s
    };
    for (what, other) in [
        ("the input's own residual", ModelSpec { parts: Vec::new(), ..spec.clone() }),
        ("a residual scale of 1", ModelSpec { parts: vec![Part::NormedResidual { scale: 1.0 }], ..spec.clone() }),
        ("kept weights not renormalized", routed(&|m| m.norm_top_k = false)),
    ] {
        let off = logits(&other).iter().zip(&theirs).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(off > scale * 1e-3, "{what}: the logits move only {off:e}, so the twin does not hold the reference to it");
    }
}

#[test]
fn a_minimax_01_directorys_tokenizer_reads_as_its_converted_gguf() {
    // MiniMax-Text-01's byte-pair vocabulary, its pre-tokenizer named by its probe digest as the converter's
    // transformers 4 encodes the probe (MiniMax-M2's), padded to 200,064 entries; the pinned llama.cpp's MiniMax-M1 chat
    // template in place of the tokenizer's own; the zero rows of the embedding past the tokenizer's 200,000 suppressed,
    // the zero row below them not.
    for family in ["minimax-01", "minimax-text-01"] {
        tokenizes_as_its_converted_gguf(family);
    }
    let Some((dir, _)) = twin("minimax-01") else { return };
    let hf = HfCheckpoint::open(&dir).unwrap();
    let meta = hf.tokenizer_meta().unwrap();
    let suppressed: Vec<u64> = meta["tokenizer.ggml.suppress_tokens"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(suppressed, [200005].into_iter().chain(200026..200064).collect::<Vec<u64>>(), "the suppressed tokens");
    assert_eq!(meta["tokenizer.ggml.pre"].as_bytes(), Some(&b"minimax-m2"[..]), "MiniMax-Text-01's pre-tokenizer, MiniMax-M2's");
}

#[test]
fn every_minimax_01_setting_the_gguf_cannot_carry_is_refused_by_name() {
    // The converter writes no attention kinds (llama.cpp attends fully on every eighth block alone), one residual scale
    // (its three sublayers' alphas equal, every beta 1), no shared expert, no window and an unscaled rotation's base, and
    // reads `rotary_dim` and the embedding's shard by the index. MiniMax's own code adds the input itself as the residual
    // unless `postnorm` is true, and takes a non-power-of-two head count's slopes from the next power's; each class reads
    // its own configuration's names alone. transformers' class computes the normed-input residual whatever `postnorm`
    // says, so its `postnorm` reads.
    type Edit = Box<dyn Fn(&mut serde_json::Value)>;
    let set = |key: &'static str, value: serde_json::Value| -> Edit { Box::new(move |c| c[key] = value.clone()) };
    let remove = |key: &'static str| -> Edit {
        Box::new(move |c| {
            c.as_object_mut().unwrap().remove(key);
        })
    };
    let mut kinds = vec!["linear_attention"; 8];
    kinds[3] = "full_attention";
    let mut own_kinds = vec![0; 8];
    own_kinds[0] = 1;
    let all_linear = vec!["linear_attention"; 8];
    let cases: Vec<(&str, &str, Edit, &str)> = vec![
        ("minimax-01", "kinds", set("layer_types", serde_json::json!(kinds)), "layer_types"),
        ("minimax-01", "untyped", remove("layer_types"), "layer_types"),
        ("minimax-01", "alphas", set("mlp_alpha_factor", serde_json::json!(2.0)), "mlp_alpha_factor"),
        ("minimax-01", "beta", set("full_attn_beta_factor", serde_json::json!(0.5)), "full_attn_beta_factor"),
        ("minimax-01", "no-alpha", remove("linear_attn_alpha_factor"), "linear_attn_alpha_factor"),
        ("minimax-01", "shared", set("shared_intermediate_size", serde_json::json!(32)), "shared_intermediate_size"),
        ("minimax-01", "windowed", set("sliding_window", serde_json::json!(4)), "sliding_window"),
        (
            "minimax-01",
            "scaled",
            set("rope_parameters", serde_json::json!({"rope_type": "linear", "rope_theta": 10000000.0, "factor": 2.0})),
            "rope_parameters",
        ),
        ("minimax-01", "unrotated", remove("rotary_dim"), "rotary_dim"),
        ("minimax-01", "foreign", set("attn_type_list", serde_json::json!([0, 0, 0, 0, 0, 0, 0, 1])), "attn_type_list"),
        ("minimax-01", "gelu", set("hidden_act", serde_json::json!("gelu")), "hidden_act"),
        ("minimax-text-01", "prenorm", set("postnorm", serde_json::json!(false)), "postnorm"),
        ("minimax-text-01", "no-postnorm", remove("postnorm"), "postnorm"),
        ("minimax-text-01", "six-heads", set("num_attention_heads", serde_json::json!(6)), "num_attention_heads"),
        ("minimax-text-01", "kinds", set("attn_type_list", serde_json::json!(own_kinds)), "attn_type_list"),
        ("minimax-text-01", "foreign", set("layer_types", serde_json::json!(all_linear)), "layer_types"),
        ("minimax-text-01", "no-alpha", remove("layernorm_mlp_alpha"), "layernorm_mlp_alpha"),
    ];
    for (family, tag, edit, named) in cases {
        let Some(opened) = opened_with(family, tag, edit) else { return };
        let refused = opened.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(refused.contains(named), "{family} {tag}: refused naming `{named}`; got {refused:?}");
    }
    let Some(opened) = opened_with("minimax-01", "prenorm", |c| c["postnorm"] = serde_json::json!(false)) else { return };
    assert!(opened.is_ok(), "transformers' class reads its `postnorm` as it computes it: {:?}", opened.err());
    let Some((dir, _)) = twin("minimax-text-01") else { return };
    let copy = scratch_copy("minimax-text-01-unindexed", &dir);
    std::fs::remove_file(copy.join("model.safetensors.index.json")).unwrap();
    let refused = HfCheckpoint::open(&copy).err().map(|e| e.to_string()).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&copy);
    assert!(refused.contains("model.safetensors.index.json"), "a checkpoint with no index: {refused:?}");
}

#[test]
fn an_rwkv7_directory_reads_as_its_converted_gguf() {
    // RWKV-7 as fla-hub publishes it (`RWKV7ForCausalLM`: flash-linear-attention's names, each low-rank pair
    // `<x>_lora.lora.{0,2}` stored output-major, the second's bias the pair's offset) and as the Hugging Face port of
    // BlinkDL's checkpoints holds it (`Rwkv7ForCausalLM`: each pair `<x>1`, `<x>2` stored input-major, which the
    // converter transposes, its unset ranks the converter's defaults): three blocks of two heads of 64, every tensor
    // squeezed, `r_k` flattened, the six lerps stacked `[d, 1, 1, 6]`, block 0's unread value-mix pair the converter's
    // copy of its learning rate's; LayerNorms with their biases; the RWKV world vocabulary.
    for family in ["rwkv7", "rwkv7-hf"] {
        reads_as_its_converted_gguf(family);
    }
    for (family, ranks, dff) in [("rwkv7", (16, 16, 8, 32), 256), ("rwkv7-hf", (32, 32, 32, 16), 512)] {
        let Some((dir, _)) = twin(family) else { return };
        let spec = ModelSpec::from_source(&HfCheckpoint::open(&dir).unwrap()).unwrap();
        assert_eq!((spec.norm, spec.ffn_norm, &spec.parts[..], spec.rms_eps), (Norm::Layer { bias: true }, "attn_norm_2", &[Part::EmbeddingNorm][..], 1e-5), "{family}");
        for (l, layer) in spec.layers.iter().enumerate() {
            let Mixer::Recurrent(Recurrent::Rwkv7(r)) = &layer.mixer else { panic!("{family} block {l}: {:?}", layer.mixer) };
            assert_eq!(
                (r.head_size, r.decay_lora, r.iclr_lora, r.value_res_lora, r.gate_lora, r.first),
                (64, ranks.0, ranks.1, ranks.2, ranks.3, l == 0),
                "{family} block {l}: the time mix"
            );
            assert_eq!(layer.ffn, Ffn::ChannelMix { dff }, "{family} block {l}: the channel mix");
        }
    }
}

#[test]
fn an_rwkv7_directorys_tokenizer_reads_as_its_converted_gguf() {
    // The RWKV world vocabulary: `<s>`, then every line of `rwkv_vocab_v20230424.txt` as Python's `repr` escapes its bytes,
    // padded to 65,536; `\n\n` (261) the end of a turn and 0 the beginning and end; tokenized longest match first.
    for family in ["rwkv7", "rwkv7-hf"] {
        tokenizes_as_its_converted_gguf(family);
    }
}

#[test]
fn an_rwkv7_directory_computes_what_llama_cpp_computes() {
    // No transformers class computes RWKV-7 (fla-hub's code runs flash-linear-attention's kernels), so the arm is held to
    // llama.cpp itself: the pinned llama.cpp's CPU run of the converted twin (`scripts/accel/model-golden.sh`, its weights
    // in f32) and the f64 reference over the directory agree on every one of the twenty top logits after each prompt, to
    // within a ten-thousandth of the largest; the value mix toward the first block's value and the token shift bind. The
    // twin's heads are 64 wide, as every release's are: llama.cpp's vectorized CPU kernel (`rwkv_wkv7`) steps 32 or 64
    // floats a head and reads past a narrower one.
    if !(common::require_tool("hf-twins") && common::require_tool("llama-cpp-build") && common::require_tool("g++")) {
        return;
    }
    let (dir, gguf) = common::hf_twins::twin("rwkv7");
    let scratch = tempfile::tempdir().unwrap();
    let prompts = scratch.path().join("prompts.txt");
    std::fs::write(&prompts, "The quick brown fox jumps over the lazy dog.\n__prompt__\nUser: what is a recurrent network?\n\nAssistant:").unwrap();
    let out = scratch.path().join("golden.json");
    let run = std::process::Command::new(repo_root().join("scripts/accel/model-golden.sh"))
        .args([gguf.to_str().unwrap(), out.to_str().unwrap(), "1", "4", "0", "f32", prompts.to_str().unwrap(), "0", "cpu", "20"])
        .output()
        .expect("model-golden.sh");
    assert!(run.status.success(), "model-golden.sh: {}", String::from_utf8_lossy(&run.stderr));
    let golden: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    let hf = HfCheckpoint::open(&dir).unwrap();
    let vocab = Vocab::from_meta(hf.tokenizer_meta().unwrap()).unwrap();
    let spec = ModelSpec::from_source(&hf).unwrap();
    let logits = |spec: &ModelSpec, tokens: &[u32]| Reference::new(spec.clone(), Opened(open_checkpoint(&dir).unwrap())).unwrap().step(tokens).unwrap();
    let unmixed = ModelSpec {
        layers: spec
            .layers
            .iter()
            .map(|l| match &l.mixer {
                Mixer::Recurrent(Recurrent::Rwkv7(r)) => Layer { mixer: Mixer::Recurrent(Recurrent::Rwkv7(Rwkv7 { first: true, ..r.clone() })), ..l.clone() },
                _ => l.clone(),
            })
            .collect(),
        ..spec.clone()
    };
    for (p, case) in golden["prompts"].as_array().unwrap().iter().enumerate() {
        let tokens: Vec<u32> = case["tokens"].as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as u32).collect();
        assert_eq!(vocab.tokenize(case["text"].as_str().unwrap().as_bytes(), true, false).unwrap(), tokens, "prompt {p}: the tokens llama.cpp reads");
        let ours = logits(&spec, &tokens);
        let top: Vec<(usize, f64)> = case["top"].as_array().unwrap().iter().map(|e| (e[0].as_u64().unwrap() as usize, e[1].as_f64().unwrap())).collect();
        assert_eq!(top.len(), 20, "prompt {p}: llama.cpp's top logits");
        let scale = ours.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        let worst = top.iter().map(|&(id, v)| (ours[id] - v).abs()).fold(0.0, f64::max);
        assert!(worst <= scale * 1e-4, "prompt {p}: a top logit is {worst:e} from llama.cpp's, past a ten-thousandth of {scale}");
        let other = logits(&unmixed, &tokens);
        let off = top.iter().map(|&(id, v)| (other[id] - v).abs()).fold(0.0, f64::max);
        assert!(off > scale * 1e-3, "prompt {p}: with no value mix the logits move only {off:e}, so the twin does not hold the reference to it");
        if p == 1 {
            let mut stepwise = Reference::new(spec.clone(), Opened(open_checkpoint(&dir).unwrap())).unwrap();
            let mut last = Vec::new();
            for &t in &tokens {
                last = stepwise.step(&[t]).unwrap();
            }
            let drift = ours.iter().zip(&last).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
            assert!(drift <= scale * 1e-9, "prompt {p}: one token a step carries the token shifts and states to the same logits ({drift:e})");
        }
    }
}

#[test]
fn every_rwkv7_setting_the_gguf_cannot_carry_is_refused_by_name() {
    // llama.cpp's rwkv7 computes relu² in its channel mix, reads no attention block, norms the embedding first, reads
    // every LayerNorm's bias, six lerps and a gate in every block and a head of its own, and mixes in the model's width
    // over heads that divide it; fla's `num_heads` beside a `head_dim` is not read (fla takes the head width). The converter reads the port's head width, epsilon and ranks only where it states all of
    // them (else fla's names), and reads the channel mix's width (null for four times the model's).
    type Edit = Box<dyn Fn(&mut serde_json::Value)>;
    let set = |key: &'static str, value: serde_json::Value| -> Edit { Box::new(move |c| c[key] = value.clone()) };
    let remove = |key: &'static str| -> Edit {
        Box::new(move |c| {
            c.as_object_mut().unwrap().remove(key);
        })
    };
    let cases: Vec<(&str, &str, Edit, &str)> = vec![
        ("rwkv7", "gelu", set("hidden_act", serde_json::json!("gelu")), "hidden_act"),
        ("rwkv7", "hybrid", set("attn", serde_json::json!({"layers": [1], "num_heads": 4})), "attn"),
        ("rwkv7", "post-norm", set("norm_first", serde_json::json!(false)), "norm_first"),
        ("rwkv7", "unbiased", set("norm_bias", serde_json::json!(false)), "norm_bias"),
        ("rwkv7", "ungated", set("wkv_has_gate", serde_json::json!(false)), "wkv_has_gate"),
        ("rwkv7", "tied", set("tie_word_embeddings", serde_json::json!(true)), "tie_word_embeddings"),
        ("rwkv7", "narrow-values", set("value_dim", serde_json::json!(64)), "value_dim"),
        ("rwkv7", "odd-heads", set("head_dim", serde_json::json!(48)), "head_dim"),
        ("rwkv7", "no-eps", remove("norm_eps"), "norm_eps"),
        ("rwkv7", "no-rank", remove("v_low_rank_dim"), "v_low_rank_dim"),
        ("rwkv7", "no-ffn-width", remove("intermediate_size"), "intermediate_size"),
        ("rwkv7-hf", "narrow-attention", set("attention_hidden_size", serde_json::json!(64)), "attention_hidden_size"),
        ("rwkv7-hf", "three-ranks", remove("lora_rank_gate"), "decay_low_rank_dim"),
        ("rwkv7-hf", "no-port-eps", remove("layer_norm_epsilon"), "head_dim"),
    ];
    for (family, tag, edit, named) in cases {
        let Some(opened) = opened_with(family, tag, edit) else { return };
        let refused = opened.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(refused.contains(named), "{family} {tag}: refused naming `{named}`; got {refused:?}");
    }
    let Some(opened) = opened_with("rwkv7", "stated-heads", |c| c["num_heads"] = serde_json::json!(32)) else { return };
    assert!(opened.is_ok(), "fla's `num_heads` beside its `head_dim` reads: {:?}", opened.err());
}

#[test]
fn a_gemma4_directory_reads_as_its_converted_gguf() {
    // Gemma 4 as published (`Gemma4ForConditionalGeneration`) and as its text model (`Gemma4ForCausalLM`), shaped as
    // 26B-A4B — a window on all but blocks 2 and 5, heads of 16 over two KV heads windowed and of 32 over one globally,
    // the global blocks' values their keys and their rotation proportional (`rope_freqs`: a quarter of each head
    // turning), every block's GELU FFN beside four experts behind Gemma 4's scaled router, each block's output scaled —
    // and as E2B: per-layer inputs of 16, blocks 4 and 5 attending the caches of blocks 2 and 3 with FFNs twice as wide.
    for family in ["gemma4", "gemma4-text", "gemma4-e"] {
        reads_as_its_converted_gguf(family);
    }
    let (Some((moe, _)), Some((e, _))) = (twin("gemma4"), twin("gemma4-e")) else { return };
    let spec = ModelSpec::from_source(&HfCheckpoint::open(&moe).unwrap()).unwrap();
    assert_eq!(spec.parts, [Part::ValueNorm, Part::KeyAsValue { layers: vec![2, 5] }, Part::LayerScale], "26B-A4B's parts");
    assert_eq!((spec.final_logit_softcap, spec.tied_output, spec.embedding_scale), (Some(30.0), true, Some(8.0)), "the cap, the tie, the embedding's scale");
    for (l, layer) in spec.layers.iter().enumerate() {
        let Mixer::Attention(a) = &layer.mixer else { panic!("block {l} attends") };
        let windowed = l % 3 != 2;
        let rope = a.rope.as_ref().map(|r| (r.dims, r.theta, r.scaling.clone()));
        let want = if windowed { (2, 16, Some((16, 10000.0, RopeScaling::None))) } else { (1, 32, Some((32, 1_000_000.0, RopeScaling::Factors))) };
        assert_eq!((a.kv_heads, a.head_dim, rope), want, "block {l}: its heads and rotation");
        assert_eq!((a.window.is_some(), a.qk_norm, a.scale), (windowed, true, 1.0), "block {l}: windowed where marked, QK-normed, unscaled");
        let Ffn::DenseAndMoe { dff: 96, act: Act::GeluTanh, moe } = &layer.ffn else { panic!("block {l}'s FFN: {:?}", layer.ffn) };
        assert_eq!((moe.experts, moe.top_k, moe.expert_dff, moe.gating.clone(), moe.norm_top_k), (4, 2, 32, Gating::Softmax, true), "block {l}");
    }
    let spec = ModelSpec::from_source(&HfCheckpoint::open(&e).unwrap()).unwrap();
    assert_eq!(spec.parts, [Part::PerLayerInput { dim: 16 }, Part::SharedKv { from: 4 }, Part::ValueNorm, Part::LayerScale], "E2B's parts");
    let widths: Vec<Ffn> = spec.layers.iter().map(|l| l.ffn.clone()).collect();
    let dense = |dff| Ffn::Dense { dff, act: Act::GeluTanh };
    assert_eq!(widths, [dense(96), dense(96), dense(96), dense(96), dense(192), dense(192)], "the shared blocks' FFNs twice as wide");
}

#[test]
fn a_gemma4_directory_computes_what_transformers_computes() {
    // Nine positions through six blocks under windows of 4: the value norm, the keys read as values, the mixture beside
    // the dense FFN, the layer scales, the per-layer inputs and the shared caches all bind. Gemma 4's norms take their
    // input `.float()`, which the oracle keeps at float64: rounded to float32 they alone move the logits a millionth.
    let prompt = [2, 17, 400, 2024, 9, 77, 3, 512, 1000];
    for family in ["gemma4", "gemma4-e"] {
        let Some((spec, theirs, scale, logits)) = computes_what_transformers_computes(family, &prompt) else { return };
        let without = |part: fn(&Part) -> bool| ModelSpec { parts: spec.parts.iter().filter(|p| !part(p)).cloned().collect(), ..spec.clone() };
        let mut others = vec![("no layer scale", without(|p| *p == Part::LayerScale)), ("no value norm", without(|p| *p == Part::ValueNorm))];
        if family == "gemma4" {
            let dense = ModelSpec {
                layers: spec.layers.iter().map(|l| Layer { ffn: Ffn::Dense { dff: 96, act: Act::GeluTanh }, ..l.clone() }).collect(),
                ..spec.clone()
            };
            others.push(("the dense FFN alone", dense));
        } else {
            others.push(("no per-layer inputs", without(|p| matches!(p, Part::PerLayerInput { .. }))));
        }
        for (what, other) in others {
            let off = logits(&other).iter().zip(&theirs).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
            assert!(off > scale * 1e-3, "{family} {what}: the logits move only {off:e}, so the twin does not hold the reference to it");
        }
    }
}

#[test]
fn a_gemma4_directorys_tokenizer_reads_as_its_converted_gguf() {
    // Gemma 4's vocabulary as `LlamaHfVocab` reads it under the `gemma4` model, the chat parser's markers kept visible,
    // every merge, the beginning token added.
    tokenizes_as_its_converted_gguf("gemma4");
}

#[test]
fn every_gemma4_setting_the_gguf_cannot_carry_is_refused_by_name() {
    // llama.cpp's gemma4 computes the tanh GELU, reads no attention bias, attends causally, rotates a global block
    // proportionally and a windowed one whole and unscaled, ends the stack in a global block, carries one head width and
    // KV count for the global blocks, shares the caches of a windowed and a global block just before the shared ones,
    // and reads a per-layer row for every token; the converter reads the experts' width. transformers reads a global
    // block's widths from its saved `per_layer_config` override, the windowed blocks' where it is silent, and builds the
    // overrides from `num_global_key_value_heads` only beside `attention_k_eq_v`; the converter reads that key first.
    type Edit = Box<dyn Fn(&mut serde_json::Value)>;
    let set = |key: &'static str, value: serde_json::Value| -> Edit { Box::new(move |c| c[key] = value.clone()) };
    let mut ending_windowed = vec!["sliding_attention"; 6];
    ending_windowed[2] = "full_attention";
    let cases: Vec<(&str, &str, Edit, &str)> = vec![
        ("gemma4-text", "gelu", set("hidden_activation", serde_json::json!("gelu")), "hidden_activation"),
        ("gemma4-text", "biased", set("attention_bias", serde_json::json!(true)), "attention_bias"),
        ("gemma4-text", "bidirectional", set("use_bidirectional_attention", serde_json::json!("all")), "use_bidirectional_attention"),
        (
            "gemma4-text",
            "default-global",
            Box::new(|c| c["rope_parameters"]["full_attention"] = serde_json::json!({"rope_type": "default", "rope_theta": 1000000.0})),
            "rope_parameters",
        ),
        ("gemma4-text", "partial-local", Box::new(|c| c["rope_parameters"]["sliding_attention"]["partial_rotary_factor"] = serde_json::json!(0.5)), "rope_parameters"),
        ("gemma4-text", "ends-windowed", set("layer_types", serde_json::json!(ending_windowed)), "layer_types"),
        ("gemma4-text", "local-override", Box::new(|c| c["per_layer_config"]["0"] = serde_json::json!({"head_dim": 32})), "per_layer_config"),
        ("gemma4-text", "unlike-globals", Box::new(|c| c["per_layer_config"]["5"] = serde_json::json!({"head_dim": 64, "num_key_value_heads": 1})), "per_layer_config"),
        (
            "gemma4-text",
            "ungated-global-kv",
            Box::new(|c| {
                c.as_object_mut().unwrap().remove("per_layer_config");
                c["global_head_dim"] = serde_json::json!(32);
                c["attention_k_eq_v"] = serde_json::json!(false);
                c["num_global_key_value_heads"] = serde_json::json!(1);
            }),
            "num_global_key_value_heads",
        ),
        ("gemma4-text", "saved-global-kv", set("num_global_key_value_heads", serde_json::json!(2)), "per_layer_config"),
        ("gemma4-text", "silent-override", Box::new(|c| c["per_layer_config"]["5"] = serde_json::json!({"head_dim": 32})), "per_layer_config"),
        ("gemma4-text", "unkeyed-without-values", set("attention_k_eq_v", serde_json::json!(false)), "attention_k_eq_v"),
        ("gemma4-e-text", "keyed-with-values", set("attention_k_eq_v", serde_json::json!(true)), "attention_k_eq_v"),
        (
            "gemma4-text",
            "no-expert-width",
            Box::new(|c| {
                c.as_object_mut().unwrap().remove("moe_intermediate_size");
            }),
            "moe_intermediate_size",
        ),
        ("gemma4-e-text", "shared-from-3", set("num_kv_shared_layers", serde_json::json!(3)), "num_kv_shared_layers"),
        ("gemma4-e-text", "short-per-layer", set("vocab_size_per_layer_input", serde_json::json!(1000)), "vocab_size_per_layer_input"),
    ];
    for (family, tag, edit, named) in cases {
        let Some(opened) = opened_with(family, tag, edit) else { return };
        let refused = opened.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(refused.contains(named), "{family} {tag}: refused naming `{named}`; got {refused:?}");
    }
    // The twin's global widths spelled as a published configuration states them read.
    let published = opened_with("gemma4-text", "published", |c| {
        c.as_object_mut().unwrap().remove("per_layer_config");
        c["global_head_dim"] = serde_json::json!(32);
        c["num_global_key_value_heads"] = serde_json::json!(1);
    });
    let Some(published) = published else { return };
    published.unwrap_or_else(|e| panic!("published: {e}"));
    // Saved overrides that agree with an ungated `num_global_key_value_heads` unlike `num_key_value_heads`: transformers
    // reads the overrides and the converter the key, one count; the configuration reads.
    let Some((dir, _)) = twin("gemma4-e-text") else { return };
    let copy = scratch_copy("gemma4-e-text-saved-ungated", &dir);
    let path = copy.join("config.json");
    let mut config: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    config["num_global_key_value_heads"] = serde_json::json!(2);
    for l in ["3", "5"] {
        config["per_layer_config"][l] = serde_json::json!({"head_dim": 32, "num_key_value_heads": 2});
    }
    std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
    let read = HfCheckpoint::read_configuration(&copy);
    let _ = std::fs::remove_dir_all(&copy);
    let (_, meta) = read.unwrap_or_else(|e| panic!("saved-ungated: {e}"));
    let kv = meta.get("gemma4.attention.head_count_kv").and_then(|v| v.as_array().map(|a| a.iter().filter_map(MetaValue::as_u64).collect::<Vec<_>>()));
    assert_eq!(kv, Some(vec![1, 1, 1, 2, 1, 2]), "saved-ungated: the KV heads, block by block");
}

#[test]
fn every_family_the_author_does_not_write_yet_is_refused_by_name() {
    // An arm reads what the author cannot write yet, and the author refuses it naming the first such piece (INTAKE-9):
    // a recurrent mixer by `Recurrent::what`, a part by `Part::what`, a LayerNorm stack by its kind.
    let cases = [
        ("lfm2", "a gated short convolution (LFM2's)"),
        ("mamba2", "Mamba-2's state-space scan (SSD)"),
        ("falcon-h1", "attention beside a state-space scan in one block (Falcon-H1's)"),
        ("nemotron_h", "Mamba-2's state-space scan (SSD)"),
        ("jamba", "Mamba-1's selective scan"),
        ("gemma3n", "per-layer inputs (Gemma 3n's and Gemma 4's)"),
        ("kimi-linear", "Kimi delta attention (KDA)"),
        ("minimax-01", "a residual from the normed input (MiniMax-01's)"),
        ("rwkv7", "block norms that are LayerNorms (PhiMoE's and RWKV-7's)"),
        ("gemma4", "a weightless norm on the values"),
        ("gemma4-e", "per-layer inputs (Gemma 3n's and Gemma 4's)"),
    ];
    for (family, named) in cases {
        let Some((dir, _)) = twin(family) else { return };
        let spec = ModelSpec::from_source(&*open_checkpoint(&dir).unwrap()).unwrap_or_else(|e| panic!("{family}: {e}"));
        let refused = logicaffeine_compile::model::author::check(&spec).expect_err(family).to_string();
        assert!(refused.contains(named), "{family}: the author refuses naming `{named}`; got {refused:?}");
    }
}

#[test]
fn a_gpt_oss_directory_reads_as_its_converted_gguf() {
    reads_as_its_converted_gguf("gpt-oss");
}

#[test]
fn a_deepseek_v3_directory_reads_as_its_converted_gguf() {
    reads_as_its_converted_gguf("deepseek2");
}

#[test]
fn a_llama4_directory_reads_as_its_converted_gguf() {
    // Scout's text model: chunked rotated layers and a rotation-free one, experts every other layer from one stored
    // gate-and-up tensor split in halves, a shared expert, llama3 rotation factors.
    reads_as_its_converted_gguf("llama4");
}

#[test]
fn sharded_safetensors_bind_every_tensor() {
    for family in FAMILIES {
        let Some((dir, _)) = twin(family) else { return };
        let shards = std::fs::read_dir(&dir)
            .unwrap()
            .filter(|e| e.as_ref().unwrap().path().extension().is_some_and(|x| x == "safetensors"))
            .count();
        assert!(shards >= 2, "{family}: the twin is not sharded ({shards} shard)");
        assert!(dir.join("model.safetensors.index.json").is_file(), "{family}: no shard index");
        let hf = HfCheckpoint::open(&dir).unwrap();
        assert!(hf.shard_count() == shards, "{family}: {} shards read of {shards}", hf.shard_count());
        assert!(hf.unread().is_empty(), "{family}: unread {:?}", hf.unread());
    }
}

#[test]
fn a_nested_text_config_is_read() {
    let Some((dir, gguf)) = twin("qwen35") else { return };
    let config: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).unwrap()).unwrap();
    assert!(config.get("text_config").is_some() && config.get("hidden_size").is_none(), "the twin nests its text config");
    let hf = HfCheckpoint::open(&dir).unwrap();
    let g = Gguf::open(&gguf).unwrap();
    assert_eq!(norm(hf.arch_meta("embedding_length").unwrap()), norm(g.arch_meta("embedding_length").unwrap()));
    assert_eq!(ModelSpec::from_source(&hf).unwrap().d, 64, "the text width, not the vision tower's");
}

#[test]
fn the_reference_reads_a_hugging_face_directory_as_its_gguf() {
    // Each opened as `largo` opens it (`open_checkpoint`), so Qwen3-Next's grouped layout is presented alike from both.
    for family in ["llama", "qwen2", "qwen3", "qwen3moe", "qwen2moe", "qwen35", "qwen35moe", "qwen3next", "gemma3", "gpt-oss", "mixtral", "gemma", "gemma2"] {
        let Some((dir, gguf)) = twin(family) else { return };
        let hf = open_checkpoint(&dir).unwrap();
        let g = open_checkpoint(&gguf).unwrap();
        let mut from_hf = Reference::new(ModelSpec::from_source(&*hf).unwrap(), Opened(hf)).unwrap();
        let mut from_gguf = Reference::new(ModelSpec::from_source(&*g).unwrap(), Opened(g)).unwrap();
        let prompt = [1u32, 17, 400, 2024, 9];
        let a = from_hf.step_every(&prompt).unwrap();
        let b = from_gguf.step_every(&prompt).unwrap();
        assert_eq!(a, b, "{family}: the reference's logits");
    }
}

/// A copy of the family's twin directory to break.
fn scratch_copy(family: &str, dir: &Path) -> PathBuf {
    let to = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/hf-twins/scratch")
        .join(format!("{family}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&to);
    std::fs::create_dir_all(&to).unwrap();
    for e in std::fs::read_dir(dir).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
    }
    to
}

/// gguf's `SpecialVocab` reads a ByteLevel post-processing step as "add neither, unless a step before it said so", so a
/// template earlier in the same sequence keeps its flags. Llama 3's sequence (ByteLevel, then its template) and the same
/// steps in the other order, each held to the converter's own `SpecialVocab` (`scripts/accel/hf-oracle.py special`).
#[test]
fn a_byte_level_step_keeps_the_flags_a_template_before_it_set() {
    let Some((dir, _)) = twin("llama") else { return };
    for (order, template_first) in [("published", false), ("template-first", true)] {
        let copy = scratch_copy(&format!("llama-pp-{order}"), &dir);
        if template_first {
            let path = copy.join("tokenizer.json");
            let mut tj: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let steps = tj["post_processor"]["processors"].as_array_mut().expect("Llama 3's post-processor is a sequence");
            assert_eq!(steps[0]["type"], "ByteLevel", "the published order begins with the ByteLevel step");
            steps.reverse();
            std::fs::write(&path, tj.to_string()).unwrap();
        }
        let ours: BTreeMap<String, Option<bool>> = {
            let hf = HfCheckpoint::open(&copy).unwrap();
            let meta = hf.tokenizer_meta().unwrap();
            ["bos", "eos", "sep"]
                .iter()
                .map(|k| (k.to_string(), meta.get(&format!("tokenizer.ggml.add_{k}_token")).and_then(MetaValue::as_bool)))
                .collect()
        };
        let theirs = oracle("special", Some(&copy), &serde_json::json!(null));
        let _ = std::fs::remove_dir_all(&copy);
        for (kind, flag) in &ours {
            assert_eq!(*flag, theirs["add"][kind].as_bool(), "{order}: add_{kind}_token");
        }
        assert_eq!(ours["bos"], Some(true), "{order}: the template's beginning-of-text token is added");
    }
}

#[test]
fn a_missing_shard_is_refused_by_name() {
    let Some((dir, _)) = twin("qwen3") else { return };
    let copy = scratch_copy("qwen3-missing", &dir);
    let shard = std::fs::read_dir(&copy)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".safetensors"))
        .min()
        .unwrap();
    std::fs::remove_file(copy.join(&shard)).unwrap();
    let refused = HfCheckpoint::open(&copy).err().map(|e| e.to_string()).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&copy);
    assert!(refused.contains(&shard), "a missing shard is refused naming it; got {refused:?}");
}

#[test]
fn an_unknown_key_is_refused_by_name() {
    let Some((dir, _)) = twin("qwen3") else { return };
    let copy = scratch_copy("qwen3-key", &dir);
    let path = copy.join("config.json");
    let mut config: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    config["attention_scaling_invented_by_a_test"] = serde_json::json!(0.5);
    std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
    let refused = HfCheckpoint::open(&copy).err().map(|e| e.to_string()).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&copy);
    assert!(
        refused.contains("attention_scaling_invented_by_a_test"),
        "a configuration key nobody reads is refused naming it; got {refused:?}"
    );
}

#[test]
fn a_gated_deltanets_output_gate_is_read_by_its_activation() {
    // Gated DeltaNet's gated norm multiplies by its gate's activation, which Qwen3.8's configurations state
    // (`output_gate_type`): swish, SiLU by its other name, is the gate the converter's GGUF and the spec hold; the
    // sigmoid Qwen3.8-Flash-Next's states is a model they cannot carry.
    for family in ["qwen35", "qwen3next"] {
        let Some((dir, _)) = twin(family) else { return };
        let spec = |d: &Path| ModelSpec::from_source(&*open_checkpoint(d).unwrap_or_else(|e| panic!("{family}: {e}"))).unwrap();
        let stated = spec(&dir);
        for (gate, read) in [("swish", true), ("silu", true), ("sigmoid", false)] {
            let copy = scratch_copy(&format!("{family}-gate-{gate}"), &dir);
            let path = copy.join("config.json");
            let mut config: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            match config.get_mut("text_config") {
                Some(text) => text["output_gate_type"] = serde_json::json!(gate),
                None => config["output_gate_type"] = serde_json::json!(gate),
            }
            std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
            let opened = open_checkpoint(&copy).map(|c| ModelSpec::from_source(&*c));
            let _ = std::fs::remove_dir_all(&copy);
            match (read, opened) {
                (true, Ok(Ok(got))) => assert_eq!(got, stated, "{family}: a {gate} gate reads as the gate the twin states"),
                (false, Err(e)) => assert!(
                    e.to_string().contains("output_gate_type") && e.to_string().contains("sigmoid"),
                    "{family}: a sigmoid gate is refused naming the key and the activation; got {e}"
                ),
                (true, Err(e)) => panic!("{family}: a {gate} gate is read; got {e}"),
                (true, Ok(Err(e))) => panic!("{family}: a {gate} gate is read; got {e:?}"),
                (false, Ok(_)) => panic!("{family}: a sigmoid gate is refused"),
            }
        }
    }
}

/// A checkpoint tensor's values as stored, read straight from its shard.
fn stored(dir: &Path, name: &str) -> Vec<f32> {
    for p in safetensors::shard_paths(dir).unwrap() {
        let shard = Shard::open(&p).unwrap();
        if let Some(e) = shard.tensors.get(name) {
            let bytes = shard.read(name, 0, (e.end - e.begin) as usize).unwrap();
            let half = |c: &[u8]| u16::from_le_bytes([c[0], c[1]]);
            return match e.dtype {
                Dtype::Bf16 => bytes.chunks(2).map(|c| bf16_to_f32(half(c))).collect(),
                Dtype::F16 => bytes.chunks(2).map(|c| f16_to_f32(half(c))).collect(),
                other => panic!("{name}: the twins store bf16 or f16, not {other:?}"),
            };
        }
    }
    panic!("{}: no tensor {name}", dir.display())
}

/// `x` as the converter's f16 arithmetic leaves it: rounded to binary16, ties to even.
fn f16_round(x: f32) -> f32 {
    f16_to_f32(f32_to_f16(x))
}

/// An f16 directory read as its forward computes it differs from its converted GGUF in exactly the tensors a
/// converter step computes (the converter computes those in f16): each differs somewhere, and every element rounds to
/// the converter's in binary16; every other tensor is the converter's bit for bit.
fn forward_values_round_to_the_converters(family: &str) -> Option<(PathBuf, HfCheckpoint)> {
    let (dir, gguf) = twin(family)?;
    let mut hf = HfCheckpoint::open_with(&dir, Recompute::AsForward).unwrap_or_else(|e| panic!("{family}: {e}"));
    let mut g = Gguf::open(&gguf).unwrap();
    let recomputed: BTreeSet<String> = hf.recomputed().iter().map(|s| s.to_string()).collect();
    let names: Vec<String> = g.tensors().iter().map(|t| t.name.clone()).collect();
    for name in recomputed.iter() {
        assert!(names.contains(name), "{family}: computed {name} is not a tensor of the GGUF");
    }
    for name in &names {
        let (a, b) = (values(&mut hf, name), values(&mut g, name));
        assert_eq!(a.len(), b.len(), "{family}: {name} element count");
        let differs = a.iter().zip(&b).any(|(x, y)| x.to_bits() != y.to_bits());
        if recomputed.contains(name) {
            assert!(differs, "{family}: {name} is computed, yet every element is already the converter's f16 rounding");
            for (i, (x, y)) in a.iter().zip(&b).enumerate() {
                assert_eq!(f16_round(*x).to_bits(), y.to_bits(), "{family}: {name}[{i}] = {x} rounds to the converter's {y}");
            }
        } else {
            assert!(!differs, "{family}: {name} is read, not computed: the converter's bit for bit");
        }
    }
    Some((dir, hf))
}

#[test]
fn an_f16_gemma3_directory_reads_as_its_converted_gguf() {
    reads_as_its_converted_gguf("gemma3-f16");
}

#[test]
fn an_f16_qwen35_directory_reads_as_its_converted_gguf() {
    reads_as_its_converted_gguf("qwen35-f16");
}

/// The converter widens a bf16 tensor to f32 before it computes, so a bf16 directory's computed values are the
/// forward's: both presentations read every tensor alike.
#[test]
fn a_bf16_directory_computes_as_its_forward_does() {
    for family in ["gemma3", "qwen35"] {
        let Some((dir, _)) = twin(family) else { return };
        let mut forward = HfCheckpoint::open_with(&dir, Recompute::AsForward).unwrap();
        let mut converted = HfCheckpoint::open_with(&dir, Recompute::AsConverted).unwrap();
        assert!(!forward.recomputed().is_empty(), "{family}: the offset norms are computed");
        for name in SpecSource::tensor_names(&forward).iter().map(|s| s.to_string()).collect::<Vec<_>>() {
            let (a, b) = (values(&mut forward, &name), values(&mut converted, &name));
            assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()), "{family}: {name}");
        }
    }
}

#[test]
fn a_directory_without_offset_norms_computes_nothing() {
    for family in ["llama", "qwen3", "qwen3moe", "gpt-oss", "deepseek2"] {
        let Some((_, hf)) = forward_values_round_to_the_converters(family) else { return };
        assert!(hf.recomputed().is_empty(), "{family}: computed {:?}", hf.recomputed());
    }
}

#[test]
fn gemma3s_norms_are_one_plus_their_stored_offset_in_f32() {
    let Some((dir, mut hf)) = forward_values_round_to_the_converters("gemma3-f16") else { return };
    let mut want: Vec<String> = (0..7)
        .flat_map(|l| {
            ["attn_norm", "post_attention_norm", "ffn_norm", "post_ffw_norm", "attn_q_norm", "attn_k_norm"]
                .map(|n| format!("blk.{l}.{n}.weight"))
        })
        .chain(["output_norm.weight".to_string()])
        .collect();
    want.sort();
    let mut got: Vec<String> = hf.recomputed().iter().map(|s| s.to_string()).collect();
    got.sort();
    assert_eq!(got, want, "every norm, and nothing else");
    let offset = stored(&dir, "model.norm.weight");
    let ours = values(&mut hf, "output_norm.weight");
    assert_eq!(ours, offset.iter().map(|w| 1.0f32 + w).collect::<Vec<f32>>(), "1 + w in f32");
}

#[test]
fn qwen35s_norms_and_decay_are_computed_in_f32_as_its_forward_computes_them() {
    let Some((dir, mut hf)) = forward_values_round_to_the_converters("qwen35-f16") else { return };
    let got: BTreeSet<String> = hf.recomputed().iter().map(|s| s.to_string()).collect();
    let mut want = BTreeSet::new();
    for l in 0..5 {
        want.insert(format!("blk.{l}.attn_norm.weight"));
        want.insert(format!("blk.{l}.post_attention_norm.weight"));
    }
    for l in 0..3 {
        want.insert(format!("blk.{l}.ssm_a"));
    }
    for l in 3..5 {
        want.insert(format!("blk.{l}.attn_q_norm.weight"));
        want.insert(format!("blk.{l}.attn_k_norm.weight"));
    }
    for n in ["nextn.shared_head_norm", "nextn.enorm", "nextn.hnorm"] {
        want.insert(format!("blk.4.{n}.weight"));
    }
    want.insert("output_norm.weight".to_string());
    assert_eq!(got, want, "every offset norm (the gated-delta norm excepted, the draft block's included) and every decay");
    let offset = stored(&dir, "model.language_model.norm.weight");
    assert_eq!(values(&mut hf, "output_norm.weight"), offset.iter().map(|w| 1.0f32 + w).collect::<Vec<f32>>());
    let a_log = stored(&dir, "model.language_model.layers.0.linear_attn.A_log");
    let mut want_a: Vec<u32> = a_log.iter().map(|&x| (-((x as f64).exp() as f32)).to_bits()).collect();
    let mut got_a: Vec<u32> = values(&mut hf, "blk.0.ssm_a").iter().map(|x| x.to_bits()).collect();
    want_a.sort();
    got_a.sort();
    assert_eq!(got_a, want_a, "-exp(A_log) in f32, correctly rounded, each value head's (in the tiled order)");
}

#[test]
fn the_reference_reads_an_f16_directory_as_its_gguf() {
    for family in ["gemma3-f16", "qwen35-f16"] {
        let Some((dir, gguf)) = twin(family) else { return };
        let hf = HfCheckpoint::open(&dir).unwrap();
        let g = Gguf::open(&gguf).unwrap();
        let mut from_hf = Reference::new(ModelSpec::from_source(&hf).unwrap(), hf).unwrap();
        let mut from_gguf = Reference::new(ModelSpec::from_gguf(&g).unwrap(), g).unwrap();
        let prompt = [1u32, 17, 400, 2024, 9];
        assert!(from_hf.step_every(&prompt).unwrap() == from_gguf.step_every(&prompt).unwrap(), "{family}: the reference's logits");
    }
}

#[test]
fn the_forwards_values_move_the_logits_by_a_rounding() {
    for family in ["gemma3-f16", "qwen35-f16"] {
        let Some((dir, gguf)) = twin(family) else { return };
        let forward = HfCheckpoint::open_with(&dir, Recompute::AsForward).unwrap();
        let g = Gguf::open(&gguf).unwrap();
        let mut from_forward = Reference::new(ModelSpec::from_source(&forward).unwrap(), forward).unwrap();
        let mut from_gguf = Reference::new(ModelSpec::from_gguf(&g).unwrap(), g).unwrap();
        let prompt = [1u32, 17, 400, 2024, 9];
        let a = from_forward.step_every(&prompt).unwrap();
        let b = from_gguf.step_every(&prompt).unwrap();
        assert!(a != b, "{family}: the f32 values are not the converter's f16 roundings");
        for (pos, (x, y)) in a.iter().zip(&b).enumerate() {
            let top = y.iter().fold(0f64, |m, v| m.max(v.abs()));
            let apart = x.iter().zip(y).map(|(p, q)| (p - q).abs()).fold(0f64, f64::max) / top;
            assert!(x.iter().all(|v| v.is_finite()), "{family}: position {pos} finite");
            assert!(apart < 1e-2, "{family}: position {pos} within an f16 rounding's reach: {apart:.3e}");
        }
    }
}

#[test]
fn hf_checkpoint_open_presents_the_converters_values() {
    assert!(Recompute::default() == Recompute::AsConverted);
    for family in ["gemma3-f16", "qwen35-f16"] {
        let Some((dir, _)) = twin(family) else { return };
        let mut default = HfCheckpoint::open(&dir).unwrap();
        let mut converted = HfCheckpoint::open_with(&dir, Recompute::AsConverted).unwrap();
        let mut forward = HfCheckpoint::open_with(&dir, Recompute::AsForward).unwrap();
        for name in default.recomputed().iter().map(|s| s.to_string()).collect::<Vec<_>>() {
            let (a, b, f) = (values(&mut default, &name), values(&mut converted, &name), values(&mut forward, &name));
            assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()), "{family}: {name}");
            assert!(a.iter().zip(&f).all(|(x, y)| x.to_bits() == f16_round(*y).to_bits()), "{family}: {name} is the f16 rounding");
        }
    }
}

/// `f32_to_bf16` rounds to nearest with ties to even at every boundary bfloat16 has: each value is its own rounding,
/// each midpoint between neighbours goes to the even one, a unit either side of it to the nearer; past the largest
/// finite value is infinite; a NaN stays a NaN.
#[test]
fn bfloat16_rounding_is_to_nearest_with_ties_to_even_at_every_boundary() {
    for h in 0..=u16::MAX {
        let x = bf16_to_f32(h);
        if x.is_nan() {
            assert!(bf16_to_f32(f32_to_bf16(x)).is_nan(), "{h:#06x}: a NaN stays a NaN");
            continue;
        }
        assert_eq!(f32_to_bf16(x), h, "{h:#06x}: {x} is a bfloat16");
        if x.is_infinite() {
            continue;
        }
        let next = h + 1;
        let mid = (u32::from(h) << 16) | 0x8000;
        let even = if h & 1 == 0 { h } else { next };
        assert_eq!(f32_to_bf16(f32::from_bits(mid)), even, "{h:#06x}: the midpoint above {x} goes to the even neighbour");
        assert_eq!(f32_to_bf16(f32::from_bits(mid - 1)), h, "{h:#06x}: below the midpoint");
        assert_eq!(f32_to_bf16(f32::from_bits(mid + 1)), next, "{h:#06x}: above the midpoint");
    }
    assert_eq!(f32_to_bf16(f32::MAX), 0x7f80, "f32::MAX is past bfloat16's largest finite value by more than half a unit");
    assert_eq!(f32_to_bf16(f32::MIN), 0xff80);
}

/// The first element where two metadata values part, or `None` when they agree: arrays element by element, so a
/// vocabulary of a quarter million tokens reports the one token that differs.
fn first_difference(a: &MetaValue, b: &MetaValue) -> Option<String> {
    match (a, b) {
        (MetaValue::Array { ty: ta, items: ia }, MetaValue::Array { ty: tb, items: ib }) => {
            if ta != tb {
                return Some(format!("element type {ta} where the converter wrote {tb}"));
            }
            if let Some(i) = (0..ia.len().min(ib.len())).find(|&i| ia[i] != ib[i]) {
                return Some(format!("[{i}] {:?} where the converter wrote {:?}", ia[i], ib[i]));
            }
            (ia.len() != ib.len()).then(|| format!("{} elements where the converter wrote {}", ia.len(), ib.len()))
        }
        _ => (a != b).then(|| format!("{a:?} where the converter wrote {b:?}")),
    }
}

/// Text the two vocabularies are held to: every prompt of the golden corpora, and text that exercises what a
/// pre-tokenizer, a merge table and a special-token partition decide.
fn tokenizer_corpus() -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/models");
    let mut out: Vec<String> = ["prompts.txt", "prompts-long.txt"]
        .iter()
        .flat_map(|f| {
            std::fs::read_to_string(root.join(f))
                .unwrap()
                .split("__prompt__\n")
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect();
    out.extend(
        [
            "",
            " ",
            "\n \n\n \t \t\t \t\n  \n   \n    \n     \n",
            "🚀 (normal) 😶\u{200d}🌫️ ✅ 🦙🦙 3 33 333 3333 33333 3.3 3..3 3...3",
            "កាន់តែពិសេសអាច😁 ?我想在apple工作1314151天～ ------======= нещо на Български",
            "''''''```````\"\"\"\"......!!!!!!?????? I've been 'told he's there, 'RE you sure? 'M not sure I'll make it, 'D you like some tea? We'Ve a'lL",
            "<|im_start|>user\nWhat is 2+2?<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n4<|im_end|>",
            "<|begin_of_text|><|start_header_id|>user<|end_header_id|>\n\nHi<|eot_id|>",
            "<bos><start_of_turn>user\nHi<end_of_turn>\n<start_of_turn>model\n",
            "<|start|>assistant<|channel|>final<|message|>Hi<|return|>",
            "<｜begin▁of▁sentence｜>Hi<｜User｜>2+2<｜Assistant｜>",
            "    def f(x):\n\t\treturn x**2  # square\r\n",
            "Ünïcödé çàñ ßé ŝtrange: ﬁ ﬂ Ａｂｃ ①②③ x²",
        ]
        .map(str::to_string),
    );
    out
}

/// The directory's tokenizer files read as the converter's tokenizer keys — every `tokenizer.*` key the GGUF holds,
/// value for value, and no other — and the vocabulary built from them tokenizes the corpus as the GGUF's does.
fn tokenizes_as_its_converted_gguf(family: &str) {
    let Some((dir, gguf)) = twin(family) else { return };
    tokenizes_as_converted(family, &dir, &gguf);
}

fn tokenizes_as_converted(family: &str, dir: &Path, gguf: &Path) {
    let hf = HfCheckpoint::open(dir).unwrap_or_else(|e| panic!("{family}: {e}"));
    let g = Gguf::open(gguf).unwrap();
    let ours = hf.tokenizer_meta().unwrap_or_else(|e| panic!("{family}: the tokenizer files: {e}"));
    let mut theirs: Vec<&String> = g.order.iter().filter(|k| k.starts_with("tokenizer.")).collect();
    let mut mine: Vec<&String> = ours.keys().filter(|k| k.starts_with("tokenizer.")).collect();
    theirs.sort();
    mine.sort();
    assert_eq!(mine, theirs, "{family}: the tokenizer keys");
    for key in theirs {
        let (a, b) = (&ours[key.as_str()], &g.meta[key.as_str()]);
        let differs = if key == "tokenizer.chat_templates" {
            // The converter writes the template names from a Python set, in no fixed order.
            let names = |v: &MetaValue| -> BTreeSet<String> {
                v.as_array().unwrap().iter().map(|n| String::from_utf8_lossy(n.as_bytes().unwrap()).to_string()).collect()
            };
            (names(a) != names(b)).then(|| format!("{:?} where the converter wrote {:?}", names(a), names(b)))
        } else {
            first_difference(a, b)
        };
        if let Some(d) = differs {
            panic!("{family}: `{key}`: {d}");
        }
    }
    assert_eq!(ours.get("general.architecture"), g.meta.get("general.architecture"), "{family}: general.architecture");

    let a = Vocab::from_meta(ours).unwrap_or_else(|e| panic!("{family}: {e:?}"));
    let b = Vocab::from_gguf(&g).unwrap();
    assert_eq!(a.specials, b.specials, "{family}: the special tokens");
    assert_eq!(a.eog().collect::<Vec<_>>(), b.eog().collect::<Vec<_>>(), "{family}: the end-of-generation set");
    assert_eq!((a.add_bos, a.add_eos, a.add_space_prefix), (b.add_bos, b.add_eos, b.add_space_prefix), "{family}: the added tokens");
    for text in tokenizer_corpus() {
        for (add_special, parse_special) in [(true, true), (false, false), (true, false)] {
            let (x, y) = (a.tokenize(text.as_bytes(), add_special, parse_special), b.tokenize(text.as_bytes(), add_special, parse_special));
            assert!(x == y, "{family}: {text:?} (add {add_special}, parse {parse_special}) tokenizes to {x:?}, the GGUF's vocabulary to {y:?}");
        }
    }
}

#[test]
fn a_llama_directorys_tokenizer_reads_as_its_converted_gguf() {
    tokenizes_as_its_converted_gguf("llama");
}

#[test]
fn a_qwen3_directorys_tokenizer_reads_as_its_converted_gguf() {
    tokenizes_as_its_converted_gguf("qwen3");
}

#[test]
fn a_qwen3moe_directorys_tokenizer_reads_as_its_converted_gguf() {
    tokenizes_as_its_converted_gguf("qwen3moe");
}

#[test]
fn a_qwen2moe_directorys_tokenizer_reads_as_its_converted_gguf() {
    tokenizes_as_its_converted_gguf("qwen2moe");
}

#[test]
fn a_directorys_vocabulary_is_read_by_its_converters_rule() {
    // The converter reads Qwen2's and Qwen3's SentencePiece model when the directory ships one, and the tokenizer.json
    // of a family whose class reads no SentencePiece model (Qwen2-MoE, Qwen3-MoE, Qwen3.5) even beside one: Qwen2 with
    // Mistral's SentencePiece files, Qwen3-MoE with a stray SentencePiece model beside its own files.
    tokenizes_as_its_converted_gguf("qwen2-spm");
    tokenizes_as_its_converted_gguf("qwen3moe-spm");
}

#[test]
fn a_qwen35moe_directorys_tokenizer_reads_as_its_converted_gguf() {
    tokenizes_as_its_converted_gguf("qwen35moe");
}

#[test]
fn a_qwen35_directorys_tokenizer_reads_as_its_converted_gguf() {
    tokenizes_as_its_converted_gguf("qwen35");
}

#[test]
fn a_gemma3_directorys_sentencepiece_tokenizer_reads_as_its_converted_gguf() {
    tokenizes_as_its_converted_gguf("gemma3");
}

/// A SentencePiece directory with no `tokenizer_config.json`: nothing lists the special tokens, so each piece keeps the
/// type the model gives it, and the model's control pieces (`<bos>`, `<eos>`, `<pad>`) are control tokens.
#[test]
fn a_bare_sentencepiece_directory_keeps_its_control_pieces() {
    let Some((dir, gguf)) = twin("gemma3-spm") else { return };
    assert!(!dir.join("tokenizer_config.json").exists() && dir.join("tokenizer.model").is_file());
    tokenizes_as_converted("gemma3-spm", &dir, &gguf);
    let hf = HfCheckpoint::open(&dir).unwrap();
    let meta = hf.tokenizer_meta().unwrap();
    let types = meta["tokenizer.ggml.token_type"].as_array().unwrap();
    let tokens = meta["tokenizer.ggml.tokens"].as_array().unwrap();
    let control: Vec<String> = types
        .iter()
        .zip(tokens)
        .filter(|(t, _)| matches!(t, MetaValue::I32(3)))
        .map(|(_, tok)| String::from_utf8_lossy(tok.as_bytes().unwrap()).to_string())
        .collect();
    for piece in ["<bos>", "<eos>", "<pad>"] {
        assert!(control.iter().any(|c| c == piece), "`{piece}` is a control piece of the model: {control:?}");
    }
}

#[test]
fn a_gpt_oss_directorys_tokenizer_reads_as_its_converted_gguf() {
    tokenizes_as_its_converted_gguf("gpt-oss");
}

#[test]
fn a_deepseek_v3_directorys_tokenizer_reads_as_its_converted_gguf() {
    tokenizes_as_its_converted_gguf("deepseek2");
}

#[test]
fn a_llama4_directorys_tokenizer_reads_as_its_converted_gguf() {
    tokenizes_as_its_converted_gguf("llama4");
}

#[test]
fn a_qwen2_directorys_tokenizer_reads_as_its_converted_gguf() {
    tokenizes_as_its_converted_gguf("qwen2");
}

#[test]
fn a_mixtral_directorys_tokenizer_reads_as_its_converted_gguf() {
    tokenizes_as_its_converted_gguf("mixtral");
}

#[test]
fn a_minimax_m2_directorys_tokenizer_reads_as_its_converted_gguf() {
    tokenizes_as_its_converted_gguf("minimax_m2");
}

#[test]
fn a_phi4_directorys_tokenizer_reads_as_its_converted_gguf() {
    tokenizes_as_its_converted_gguf("phi3");
}

#[test]
fn a_glm4_directorys_tokenizer_reads_as_its_converted_gguf() {
    tokenizes_as_its_converted_gguf("glm4");
}

#[test]
fn a_glm45_directorys_tokenizer_reads_as_its_converted_gguf() {
    tokenizes_as_its_converted_gguf("glm4moe");
}

/// The pre-tokenizer names the reader can write are the converter's: its `get_vocab_base_pre` table of tokenization
/// digests, in its order (a digest listed twice names what its last entry says, as the converter's chain of `if`s
/// leaves it), read out of the pinned source.
#[test]
fn the_pre_tokenizer_digests_are_the_converters() {
    if !common::require_tool("competitor-sources") {
        return;
    }
    let src = std::fs::read_to_string(common::llama_cpp_root().join("conversion/base.py")).unwrap();
    let body = &src[src.find("# Marker: Start get_vocab_base_pre").unwrap()..src.find("# Marker: End get_vocab_base_pre").unwrap()];
    let mut theirs: Vec<(String, String)> = Vec::new();
    let mut digest: Option<String> = None;
    for line in body.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("if chkhsh == \"") {
            digest = Some(rest.trim_end_matches("\":").to_string());
        } else if let Some(rest) = line.strip_prefix("res = \"") {
            theirs.push((digest.take().expect("a name follows its digest"), rest.trim_end_matches('"').to_string()));
        }
    }
    assert!(theirs.len() >= 100, "the converter's table: {} entries", theirs.len());
    let ours: Vec<(String, String)> =
        logicaffeine_compile::model::hf::PRE_TOKENIZER_DIGESTS.iter().map(|(d, n)| (d.to_string(), n.to_string())).collect();
    assert_eq!(ours, theirs);
    // The name the reader writes for each digest is the one the converter's chain of `if`s leaves: its last entry.
    let mut listed_twice = 0;
    for (digest, _) in &theirs {
        let last = theirs.iter().rev().find(|(d, _)| d == digest).map(|(_, n)| n.as_str());
        assert_eq!(logicaffeine_compile::model::hf::pre_tokenizer_named(digest), last, "the digest {digest}");
        listed_twice += usize::from(theirs.iter().filter(|(d, _)| d == digest).count() > 1);
    }
    assert!(listed_twice > 0, "the converter's table lists no digest twice, so its last-entry rule is untested here");
    let literal = body.lines().find_map(|l| l.trim().strip_prefix("chktxt = '")).expect("the probe text");
    let literal = literal.strip_suffix('\'').expect("a single-quoted literal");
    let mut probe = String::new();
    let mut chars = literal.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            probe.push(c);
            continue;
        }
        match chars.next().unwrap() {
            'n' => probe.push('\n'),
            't' => probe.push('\t'),
            'u' => probe.push(char::from_u32(u32::from_str_radix(&chars.by_ref().take(4).collect::<String>(), 16).unwrap()).unwrap()),
            other => probe.push(other),
        }
    }
    assert_eq!(probe, logicaffeine_compile::model::hf::PRE_TOKENIZER_PROBE, "the probe text the converter tokenizes");
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `scripts/accel/hf-oracle.py` over `input` (written as JSON beside the twins): the Hugging Face libraries' answer.
fn oracle(command: &str, dir: Option<&Path>, input: &serde_json::Value) -> serde_json::Value {
    let scratch = repo_root().join("target/hf-twins/scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    let path = scratch.join(format!("oracle-{command}-{}-{}.json", std::process::id(), input.to_string().len()));
    std::fs::write(&path, input.to_string()).unwrap();
    let mut cmd = std::process::Command::new(common::python_for_tools());
    cmd.arg(repo_root().join("scripts/accel/hf-oracle.py")).arg(command);
    if let Some(d) = dir {
        cmd.arg(d);
    }
    let out = cmd.arg(&path).output().expect("run the oracle");
    let _ = std::fs::remove_file(&path);
    assert!(out.status.success(), "the oracle failed: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("the oracle's JSON")
}

fn ids(v: &serde_json::Value) -> Vec<u32> {
    v.as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u32).collect()
}

/// The byte-pair tokenizers the intake encodes with, as their maintainers publish them.
const TOKENIZERS: [&str; 6] = ["llama", "qwen3", "qwen35", "gpt-oss", "deepseek2", "gemma3"];

/// The port encodes the corpus and the converter's probe as the `tokenizers` library runs the same `tokenizer.json`,
/// with the post-processor's special tokens and without.
#[test]
fn the_hugging_face_encoder_is_the_tokenizers_librarys() {
    if !common::require_tool("hf-twins") {
        return;
    }
    let mut texts = tokenizer_corpus();
    texts.push(logicaffeine_compile::model::hf::PRE_TOKENIZER_PROBE.to_string());
    for family in TOKENIZERS {
        let dir = repo_root().join("external/hf-tokenizers").join(family);
        let ours = HfTokenizer::from_file(&dir.join("tokenizer.json")).unwrap_or_else(|e| panic!("{family}: {e}"));
        let theirs = oracle("encode", Some(&dir), &serde_json::json!(texts));
        for (i, text) in texts.iter().enumerate() {
            for (special, library) in [(true, "library"), (false, "library_plain")] {
                let got = ours.encode(text, special);
                assert_eq!(ids(&theirs[library][i]), got, "{family}: {text:?} (special tokens {special})");
            }
        }
    }
}

/// `ignore_merges` takes a whole pre-tokenized word straight from the vocabulary, even where the merges would never
/// build it: `abc` is one token with the flag and `ab` + `c` without it, as the `tokenizers` library encodes the same
/// file.
#[test]
fn ignore_merges_takes_a_whole_word_from_the_vocabulary() {
    if !common::require_tool("hf-twins") {
        return;
    }
    let texts = vec!["abc".to_string(), "ab".to_string(), "cab".to_string(), "abc cab".to_string()];
    for ignore in [true, false] {
        let tj = serde_json::json!({
            "version": "1.0", "truncation": null, "padding": null, "added_tokens": [], "normalizer": null,
            "pre_tokenizer": {"type": "WhitespaceSplit"}, "post_processor": null, "decoder": null,
            "model": {
                "type": "BPE", "dropout": null, "unk_token": null, "continuing_subword_prefix": null,
                "end_of_word_suffix": null, "fuse_unk": false, "byte_fallback": false, "ignore_merges": ignore,
                "vocab": {"a": 0, "b": 1, "c": 2, "ab": 3, "abc": 4},
                "merges": [["a", "b"]]
            }
        });
        let dir = repo_root().join("target/hf-twins/scratch").join(format!("ignore-merges-{ignore}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tokenizer.json"), tj.to_string()).unwrap();
        let ours = HfTokenizer::from_json(&tj).unwrap();
        let theirs = oracle("encode", Some(&dir), &serde_json::json!(texts));
        let _ = std::fs::remove_dir_all(&dir);
        for (i, text) in texts.iter().enumerate() {
            assert_eq!(ids(&theirs["library_plain"][i]), ours.encode(text, false), "ignore_merges {ignore}: {text:?}");
        }
        assert_eq!(ours.encode("abc", false), if ignore { vec![4] } else { vec![3, 2] }, "ignore_merges {ignore}");
    }
}

/// A line of every 97th assigned codepoint: each general category, in many scripts.
fn unicode_sweep() -> Vec<String> {
    let cps: Vec<char> = (0x20u32..0x32000).step_by(97).filter_map(char::from_u32).collect();
    cps.chunks(64).map(|c| c.iter().collect()).collect()
}

/// The patterns `tokenizers` compiles split text as Oniguruma splits it: every `Split` pattern of the intake's
/// tokenizers, the byte-level and whitespace pre-tokenizers' own patterns, and patterns that exercise each construct
/// the port reads, over the corpus and a sweep of Unicode; each behavior and inversion on a sample.
#[test]
fn the_tokenizer_patterns_split_as_oniguruma_splits_them() {
    if !common::require_tool("hf-twins") {
        return;
    }
    let mut patterns: Vec<String> = Vec::new();
    for family in TOKENIZERS {
        let tj: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(repo_root().join("external/hf-tokenizers").join(family).join("tokenizer.json")).unwrap(),
        )
        .unwrap();
        let mut stack = vec![tj["pre_tokenizer"].clone()];
        while let Some(p) = stack.pop() {
            if let Some(r) = p["pattern"]["Regex"].as_str() {
                patterns.push(r.to_string());
            }
            stack.extend(p["pretokenizers"].as_array().cloned().unwrap_or_default());
        }
    }
    assert!(patterns.len() >= 6, "the intake's tokenizers' patterns: {patterns:?}");
    patterns.extend(
        [
            r"'s|'t|'re|'ve|'m|'ll|'d| ?\p{L}+| ?\p{N}+| ?[^\s\p{L}\p{N}]+|\s+(?!\S)|\s+",
            r"\w+|[^\w\s]+",
            r"\s+", r"\S+", r"\d+", r"\D+", r"\w+", r"\W+", r".", r"\h+",
            r"(?i:'s|'t|'re|'ve|'m|'ll|'d)", r"(?i)k", r"(?i:[a-c])+", r"x(?i)ab|cd",
            r"a*", r"a+?", r"a{2,3}", r"a{2,3}?", r"a{2}", r"a{2,}", r"a{,2}", r"(?:ab)+", r"a|ab", r"ab|a", r"(a|b)c",
            r"^\w+", r"\w+$", r"\A\w", r"\w\z", r"\w\Z", r"\bthe\b", r"\Bo", r"(?>a+)b", r"a(?=b)", r"a(?!b)",
            r"[-a]", r"[a-]", r"[]a]", r"[^]a]+", r"[\p{Lu}\p{Lt}]+", r"\p{^L}+", r"\P{N}+", r"\p{Zs}+", r"\p{Sm}|\p{So}",
            r"[^\r\n\p{L}\p{N}]?\p{L}+", r"\x{1F600}", r"é", r"[\x{4e00}-\x{9fa5}]+", r"\p{Mn}+", r"[\p{Nd}\p{Nl}]",
            r"\p{Pi}|\p{Pf}|\p{Pc}", r"\.\.\.", r"[\[\]\\]", r"a{3}b{1,2}?c",
        ]
        .map(str::to_string),
    );
    let mut texts = tokenizer_corpus();
    texts.extend(unicode_sweep());
    texts.extend(["aab ab abab aaab", "the other theme", "'S 'T 'Re 'VE 'm 'LL 'd 'ſ ſ K", "a\nb\r\nc\u{2028}d"].map(str::to_string));
    let behaviors = ["removed", "isolated", "merged_with_previous", "merged_with_next", "contiguous"];
    let mut cases: Vec<(String, &str, bool, String)> = Vec::new();
    for p in &patterns {
        for t in &texts {
            cases.push((p.clone(), "isolated", false, t.clone()));
        }
        for (i, b) in behaviors.iter().enumerate() {
            for t in texts.iter().step_by(7) {
                cases.push((p.clone(), b, i % 2 == 0, t.clone()));
            }
        }
    }
    let input = serde_json::json!(cases.iter().map(|(p, b, i, t)| serde_json::json!([p, b, i, t])).collect::<Vec<_>>());
    let theirs = oracle("split", None, &input);
    for (k, (p, b, inv, t)) in cases.iter().enumerate() {
        let name = |b: &str| b.split('_').map(|w| w[..1].to_uppercase() + &w[1..]).collect::<String>();
        let spec = serde_json::json!({"type": "Split", "pattern": {"Regex": p}, "behavior": name(b), "invert": inv});
        let ours = HfPreTokenizer::from_json(&spec).unwrap_or_else(|e| panic!("{p:?}: {e}")).pre_tokenize_str(t);
        let want: Vec<String> = theirs[k].as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect();
        assert_eq!(ours, want, "{p:?} ({b}, invert {inv}) over {t:?}");
    }
}

/// A pattern construct the port does not run is refused by name, never approximated.
#[test]
fn a_pattern_the_port_does_not_run_is_refused() {
    for (p, why) in [
        (r"\p{Han}+", "Han"),
        (r"(?<=a)b", "lookbehind"),
        (r"(a)\1", "backreference"),
        (r"a*+", "possessive"),
        (r"(?:a?)*", "nothing"),
        (r"[a[b]]", "nested"),
    ] {
        let spec = serde_json::json!({"type": "Split", "pattern": {"Regex": p}, "behavior": "Isolated", "invert": false});
        let refused = HfPreTokenizer::from_json(&spec).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(refused.contains(why), "{p:?} is refused naming {why:?}; got {refused:?}");
    }
}

/// Kimi K2's tokenizer files as published (`scripts/accel/fetch-hf-tokenizers.sh`): `tiktoken.model`, its tokenizer code
/// and its configuration.
fn kimi_tokenizer() -> Option<PathBuf> {
    common::require_tool("hf-twins").then(|| repo_root().join("external/hf-tokenizers/kimi_k2"))
}

/// Kimi K2's vocabulary (its `tiktoken.model`, the pattern and reserved count its tokenizer code states, the special
/// tokens its configuration names) encodes as the `tiktoken` library does under the tokenizer's own cuts, its code never
/// run: over the corpus, a sweep of Unicode, special tokens' text inside and between words, Han and contractions, and
/// runs of letters and of spaces past the 25,000-character cut.
#[test]
fn a_tiktoken_vocabulary_encodes_as_tiktoken_encodes() {
    let Some(dir) = kimi_tokenizer() else { return };
    let t = Tiktoken::open(&dir).unwrap_or_else(|e| panic!("{e}"));
    let mut texts = tokenizer_corpus();
    texts.extend(unicode_sweep());
    texts.extend([
        "[BOS]Hello<|im_end|>world [EOS]".to_string(),
        "<|im_user|>user<|im_middle|>hi<|im_end|><|im_assistant|>a<|reserved_token_163700|>b".to_string(),
        "中文汉字测试 Kimi's 'S 'LL 'ſ test 123456789 ½ x²".repeat(3),
        "x".repeat(30_000),
        format!("{}end", " ".repeat(30_000)),
        String::new(),
    ]);
    let theirs = oracle("tiktoken", Some(&dir), &serde_json::json!(texts));
    for (k, text) in texts.iter().enumerate() {
        let (ours, want) = (t.encode(text), ids(&theirs[k]));
        if let Some(i) = (0..ours.len().max(want.len())).find(|&i| ours.get(i) != want.get(i)) {
            panic!("{:?}: token {i} is {:?} here and {:?} from tiktoken", text.chars().take(60).collect::<String>(), ours.get(i), want.get(i));
        }
    }
}

/// Every class Kimi K2's pattern is built of, alone, over every codepoint, matches what `tiktoken`'s fancy-regex matches
/// (the general categories and White_Space of Unicode 16.0, the Han script of its Scripts.txt, `&&` intersections and
/// nested classes), and the case-insensitive contractions fold as it folds.
#[test]
fn every_class_of_kimi_k2s_pattern_matches_what_tiktoken_matches() {
    if !common::require_tool("hf-twins") {
        return;
    }
    let classes = [
        r"\p{Han}",
        r"[\p{Han}]",
        r"[\p{Lu}\p{Lt}\p{Lm}\p{Lo}\p{M}&&[^\p{Han}]]",
        r"[\p{Ll}\p{Lm}\p{Lo}\p{M}&&[^\p{Han}]]",
        r"[^\r\n\p{L}\p{N}]",
        r"\p{N}",
        r"[^\s\p{L}\p{N}]",
        r"\s",
        r"[\r\n]",
        r"(?i:s)",
        r"(?i:t)",
        r"(?i:r)",
        r"(?i:e)",
        r"(?i:v)",
        r"(?i:m)",
        r"(?i:l)",
        r"(?i:d)",
    ];
    let everything: String = (0u32..0x110000).filter_map(char::from_u32).collect();
    let theirs = oracle("tiktoken-classes", None, &serde_json::json!(classes));
    for (k, class) in classes.iter().enumerate() {
        let mut ours: Vec<(u32, u32)> = Vec::new();
        for c in tiktoken_split(class, &everything).unwrap_or_else(|e| panic!("{class}: {e}")).iter().flat_map(|p| p.chars()) {
            match ours.last_mut() {
                Some(run) if run.1 + 1 == c as u32 => run.1 = c as u32,
                _ => ours.push((c as u32, c as u32)),
            }
        }
        let want: Vec<(u32, u32)> =
            theirs[k].as_array().unwrap().iter().map(|r| (r[0].as_u64().unwrap() as u32, r[1].as_u64().unwrap() as u32)).collect();
        if let Some(i) = (0..ours.len().max(want.len())).find(|&i| ours.get(i) != want.get(i)) {
            panic!("{class}: run {i} is {:x?} here and {:x?} in tiktoken", ours.get(i), want.get(i));
        }
    }
}

/// The converter names a tiktoken vocabulary's pre-tokenizer by the digest of its tokenizer's encoding of the probe
/// text: Kimi K2's encoding names `kimi-k2`.
#[test]
fn kimi_k2s_probe_names_its_pre_tokenizer() {
    let Some(dir) = kimi_tokenizer() else { return };
    let ids = Tiktoken::open(&dir).unwrap().encode(PRE_TOKENIZER_PROBE);
    let listed = format!("[{}]", ids.iter().map(u32::to_string).collect::<Vec<_>>().join(", "));
    let digest = logicaffeine_kernel::interface::lean::package::sha256_hex(listed.as_bytes());
    assert_eq!(pre_tokenizer_named(&digest), Some("kimi-k2"), "the probe's digest {digest}");
}

/// Kimi K2's tokenizer, read from the files its published GGUF was converted from (the uploader's copy of the release,
/// `unsloth/Kimi-K2-Instruct`, whose tokenizer configuration names eleven of the reserved tokens and ends text at
/// `<|im_end|>`), is the one llama.cpp's converter wrote into that GGUF (`scripts/accel/fetch-gguf-heads.sh`; the
/// converter ran Kimi's tokenizer code there, the intake never does): every `tokenizer.*` key, value for value.
#[test]
fn kimi_k2s_vocabulary_is_its_published_ggufs() {
    if !(common::require_tool("hf-twins") && common::require_tool("hf-configs") && common::require_tool("gguf-heads")) {
        return;
    }
    let root = repo_root();
    let dir = root.join("target/hf-twins/scratch").join(format!("kimi-k2-vocabulary-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(root.join("external/hf-configs/unsloth/Kimi-K2-Instruct/config.json"), dir.join("config.json")).unwrap();
    for f in ["tiktoken.model", "tokenization_kimi.py", "tokenizer_config.json", "special_tokens_map.json", "chat_template.jinja"] {
        std::fs::copy(root.join("external/hf-tokenizers/kimi_k2_unsloth").join(f), dir.join(f)).unwrap();
    }
    let ours = HfCheckpoint::read_tokenizer(&dir).unwrap_or_else(|e| panic!("{e}"));
    let _ = std::fs::remove_dir_all(&dir);
    let g = Gguf::open(&root.join("external/gguf-heads/unsloth/Kimi-K2-Instruct-GGUF/Kimi-K2-Instruct-BF16-00001-of-00045.gguf")).unwrap();
    let mut theirs: Vec<&String> = g.order.iter().filter(|k| k.starts_with("tokenizer.")).collect();
    let mut mine: Vec<&String> = ours.keys().filter(|k| k.starts_with("tokenizer.")).collect();
    theirs.sort();
    mine.sort();
    assert_eq!(mine, theirs, "the tokenizer keys");
    for key in theirs {
        if let Some(d) = first_difference(&ours[key.as_str()], &g.meta[key.as_str()]) {
            panic!("`{key}`: {d}");
        }
    }
}

/// The published configurations of the releases the Samba book deploys (`scripts/accel/fetch-hf-configs.sh`), each read
/// without its weights as the architecture its family's GGUF names, its width and stack as stated. The configurations a
/// ticket still owes are handed REDs (`model_hf_config_reds_lock`).
const PUBLISHED_CONFIGURATIONS: &[(&str, &str)] = &[
    ("unsloth/Llama-3.2-1B-Instruct", "llama"),
    ("unsloth/Llama-3.2-3B-Instruct", "llama"),
    ("unsloth/Llama-3.1-8B-Instruct", "llama"),
    ("unsloth/Llama-3.3-70B-Instruct", "llama"),
    ("Qwen/Qwen2.5-0.5B-Instruct", "qwen2"),
    ("Qwen/Qwen2.5-72B-Instruct", "qwen2"),
    ("mistralai/Mixtral-8x7B-Instruct-v0.1", "llama"),
    ("deepseek-ai/DeepSeek-V3", "deepseek2"),
    ("deepseek-ai/DeepSeek-V3-0324", "deepseek2"),
    ("deepseek-ai/DeepSeek-R1", "deepseek2"),
    ("MiniMaxAI/MiniMax-M2", "minimax-m2"),
    ("unsloth/gemma-2b-it", "gemma"),
    ("unsloth/gemma-7b-it", "gemma"),
    ("unsloth/gemma-1.1-2b-it", "gemma"),
    ("unsloth/gemma-1.1-7b-it", "gemma"),
    ("unsloth/gemma-2-2b-it", "gemma2"),
    ("unsloth/gemma-2-9b-it", "gemma2"),
    ("unsloth/gemma-2-27b-it", "gemma2"),
    ("unsloth/gemma-3-1b-it", "gemma3"),
    ("unsloth/gemma-3-4b-it", "gemma3"),
    ("unsloth/gemma-3-12b-it", "gemma3"),
    ("unsloth/gemma-3-27b-it", "gemma3"),
    ("unsloth/Llama-4-Scout-17B-16E-Instruct", "llama4"),
    ("unsloth/Llama-4-Maverick-17B-128E-Instruct", "llama4"),
    ("microsoft/phi-4", "phi3"),
    ("zai-org/GLM-4-9B-0414", "glm4"),
    ("zai-org/GLM-4-32B-0414", "glm4"),
    ("zai-org/GLM-4.5", "glm4moe"),
    ("zai-org/GLM-4.5-Air", "glm4moe"),
    ("zai-org/GLM-4.6", "glm4moe"),
    ("zai-org/GLM-4.7", "glm4moe"),
    ("moonshotai/Kimi-K2-Instruct", "deepseek2"),
    ("unsloth/Kimi-K2-Instruct", "deepseek2"),
    ("moonshotai/Kimi-K2-Instruct-0905", "deepseek2"),
    ("Qwen/Qwen3-8B", "qwen3"),
    ("Qwen/Qwen3-30B-A3B", "qwen3moe"),
    ("Qwen/Qwen3-235B-A22B", "qwen3moe"),
    ("Qwen/Qwen3-Next-80B-A3B-Instruct", "qwen3next"),
    ("Qwen/Qwen3.5-0.8B", "qwen35"),
    ("Qwen/Qwen3.5-2B", "qwen35"),
    ("Qwen/Qwen3.5-4B", "qwen35"),
    ("Qwen/Qwen3.5-9B", "qwen35"),
    ("Qwen/Qwen3.5-27B", "qwen35"),
    ("Qwen/Qwen3.5-35B-A3B", "qwen35moe"),
    ("Qwen/Qwen3.5-122B-A10B", "qwen35moe"),
    ("Qwen/Qwen3.5-397B-A17B", "qwen35moe"),
    ("Qwen/Qwen3.8-27B", "qwen35"),
    ("Qwen/Qwen3.8-2.4T-A95B", "qwen35moe"),
    ("Qwen/Qwen3.8-Flash-Next", "qwen4exp"),
    ("allenai/OLMo-2-0425-1B", "olmo2"),
    ("allenai/OLMo-2-1124-7B", "olmo2"),
    ("allenai/OLMo-2-1124-13B", "olmo2"),
    ("allenai/OLMo-2-0325-32B", "olmo2"),
    ("allenai/Olmo-3-7B-Instruct", "olmo2"),
    ("mistralai/Mistral-7B-Instruct-v0.3", "llama"),
    ("mistralai/Ministral-3-3B-Instruct-2512", "mistral3"),
    ("mistralai/Ministral-3-8B-Instruct-2512", "mistral3"),
    ("mistralai/Ministral-3-14B-Instruct-2512", "mistral3"),
    ("mistralai/Mistral-Small-3.1-24B-Instruct-2503", "mistral3"),
    ("mistralai/Mistral-Small-3.2-24B-Instruct-2506", "mistral3"),
    ("deepseek-ai/DeepSeek-V3.2-Exp", "deepseek32"),
    ("deepseek-ai/DeepSeek-V3.2", "deepseek32"),
    ("deepseek-ai/DeepSeek-V3.2-Speciale", "deepseek32"),
    ("zai-org/GLM-5", "glm-dsa"),
    ("zai-org/GLM-5.1", "glm-dsa"),
    ("zai-org/GLM-5.2", "glm-dsa"),
    ("LiquidAI/LFM2-350M", "lfm2"),
    ("LiquidAI/LFM2-700M", "lfm2"),
    ("LiquidAI/LFM2-1.2B", "lfm2"),
    ("LiquidAI/LFM2-2.6B", "lfm2"),
    ("LiquidAI/LFM2.5-1.2B-Instruct", "lfm2"),
    ("mistralai/Mamba-Codestral-7B-v0.1", "mamba2"),
    ("tiiuae/Falcon-H1-0.5B-Instruct", "falcon-h1"),
    ("tiiuae/Falcon-H1-1.5B-Instruct", "falcon-h1"),
    ("tiiuae/Falcon-H1-1.5B-Deep-Instruct", "falcon-h1"),
    ("tiiuae/Falcon-H1-3B-Instruct", "falcon-h1"),
    ("tiiuae/Falcon-H1-7B-Instruct", "falcon-h1"),
    ("tiiuae/Falcon-H1-34B-Instruct", "falcon-h1"),
    ("nvidia/Nemotron-H-8B-Base-8K", "nemotron_h"),
    ("nvidia/Nemotron-H-47B-Base-8K", "nemotron_h"),
    ("nvidia/Nemotron-H-56B-Base-8K", "nemotron_h"),
    ("nvidia/NVIDIA-Nemotron-Nano-9B-v2", "nemotron_h"),
    ("nvidia/NVIDIA-Nemotron-Nano-12B-v2", "nemotron_h"),
    ("ai21labs/AI21-Jamba-Reasoning-3B", "jamba"),
    ("ai21labs/Jamba-v0.1", "jamba"),
    ("unsloth/gemma-3n-E2B-it", "gemma3n"),
    ("unsloth/gemma-3n-E4B-it", "gemma3n"),
    ("moonshotai/Kimi-Linear-48B-A3B-Instruct", "kimi-linear"),
    ("moonshotai/Kimi-Linear-48B-A3B-Base", "kimi-linear"),
    ("MiniMaxAI/MiniMax-Text-01", "minimax-01"),
    ("MiniMaxAI/MiniMax-Text-01-hf", "minimax-01"),
    ("MiniMaxAI/MiniMax-M1-40k", "minimax-01"),
    ("MiniMaxAI/MiniMax-M1-80k", "minimax-01"),
    ("fla-hub/rwkv7-0.1B-g1", "rwkv7"),
    ("fla-hub/rwkv7-191M-world", "rwkv7"),
    ("fla-hub/rwkv7-1.5B-world", "rwkv7"),
    ("fla-hub/rwkv7-2.9B-world", "rwkv7"),
    ("SmerkyG/RWKV7-Goose-0.1B-World2.8-HF", "rwkv7"),
    ("mollysama/rwkv-7-world-0b4-hf", "rwkv7"),
    ("mollysama/rwkv-7-world-1b5-hf", "rwkv7"),
    ("google/gemma-4-E2B-it", "gemma4"),
    ("google/gemma-4-E4B-it", "gemma4"),
    ("google/gemma-4-26B-A4B-it", "gemma4"),
    ("google/gemma-4-31B-it", "gemma4"),
];

/// The published configurations a ticket still owes, each a handed RED of `model_hf_config_reds_lock`.
const OWED_CONFIGURATIONS: &[&str] = &[
    "openai/gpt-oss-20b",
    "openai/gpt-oss-120b",
    "moonshotai/Kimi-K2-Thinking",
];

#[test]
fn every_published_configuration_reads() {
    if !common::require_tool("hf-configs") {
        return;
    }
    let script = std::fs::read_to_string(repo_root().join("scripts/accel/fetch-hf-configs.sh")).unwrap();
    let listed: Vec<&str> = script
        .split_once("REPOS=(")
        .and_then(|(_, rest)| rest.split_once("\n)"))
        .map(|(body, _)| body.lines().map(str::trim).filter(|l| !l.is_empty()).collect())
        .expect("the fetch script's REPOS list");
    for repo in &listed {
        let read = PUBLISHED_CONFIGURATIONS.iter().any(|(r, _)| r == repo);
        assert!(read != OWED_CONFIGURATIONS.contains(repo), "{repo}: fetched, and either read here or owed by a handed RED, never both");
    }
    for repo in PUBLISHED_CONFIGURATIONS.iter().map(|(r, _)| *r).chain(OWED_CONFIGURATIONS.iter().copied()) {
        assert!(listed.contains(&repo), "{repo}: named here, not fetched by scripts/accel/fetch-hf-configs.sh");
    }
    let root = repo_root().join("external/hf-configs");
    for (repo, arch) in PUBLISHED_CONFIGURATIONS {
        let dir = root.join(repo);
        let (got, meta) = HfCheckpoint::read_configuration(&dir).unwrap_or_else(|e| panic!("{repo}: {e}"));
        assert_eq!(got, *arch, "{repo}: the architecture");
        let mut config = python_config(&dir.join("config.json"));
        if let Some(text) = config.get("text_config").cloned() {
            config = text;
        }
        let key = |k: &str| meta.get(&format!("{arch}.{k}")).and_then(MetaValue::as_u64);
        assert_eq!(key("embedding_length"), config["hidden_size"].as_u64(), "{repo}: the width");
        assert!(key("block_count") >= config["num_hidden_layers"].as_u64(), "{repo}: the stack");
    }
}

/// A configuration as Python's `json` writes it, its bare `NaN`, `Infinity` and `-Infinity` (Falcon-H1's and Nemotron-H's
/// unbounded time step) read as nulls, since the widths and depths compared against it are whole numbers.
fn python_config(path: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let (mut out, mut rest, mut in_string, mut escaped) = (String::with_capacity(text.len()), text.as_str(), false, false);
    while let Some(c) = rest.chars().next() {
        if let Some(word) = ["-Infinity", "Infinity", "NaN"].into_iter().find(|w| !in_string && rest.starts_with(w)) {
            out.push_str("null");
            rest = &rest[word.len()..];
            continue;
        }
        (in_string, escaped) = if in_string { (escaped || c != '"', !escaped && c == '\\') } else { (c == '"', false) };
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// NFC as the `tokenizers` library's normalizer computes it, over every codepoint in runs of its neighbours, the runs
/// reversed, and marks written out of canonical order.
#[test]
fn nfc_is_unicodedatas() {
    if !common::require_tool("hf-twins") {
        return;
    }
    let all: Vec<char> = (0u32..0x110000).filter_map(char::from_u32).collect();
    let mut texts: Vec<String> = all.chunks(512).map(|c| c.iter().collect()).collect();
    texts.extend(all.chunks(509).map(|c| c.iter().rev().collect::<String>()));
    for base in ['a', 'e', 'A', 'o', 'u', '\u{1100}', '\u{AC00}', 'ᄀ'] {
        for marks in ["\u{0301}\u{0323}\u{0302}", "\u{0323}\u{0301}", "\u{0308}\u{0301}\u{0327}", "\u{1161}\u{11A8}", "\u{0F71}\u{0F72}\u{0F74}"] {
            texts.push(format!("{base}{marks}"));
        }
    }
    texts.extend(tokenizer_corpus());
    let theirs = oracle("nfc", None, &serde_json::json!(texts));
    for (i, t) in texts.iter().enumerate() {
        let want = theirs[i].as_str().unwrap();
        let got = logicaffeine_compile::model::hf::nfc(t);
        if got != want {
            let at = got.chars().zip(want.chars()).position(|(a, b)| a != b).unwrap_or(0);
            panic!("NFC of text {i} parts at codepoint {at}: {:?} where the library gives {:?}", got.chars().skip(at).take(4).collect::<String>(), want.chars().skip(at).take(4).collect::<String>());
        }
    }
}

/// The checked-in Unicode data is what its generator writes from the database files of the two versions `tokenizers`
/// reads (Oniguruma's for the patterns, unicode-normalization-alignments' for the normal forms).
#[test]
fn the_hugging_face_unicode_data_is_its_generators() {
    use logicaffeine_compile::model::hf::{NORMAL_FORM_VERSION, PATTERN_VERSION};
    if !common::require_tool("unicode-data") {
        return;
    }
    assert_eq!((PATTERN_VERSION, NORMAL_FORM_VERSION), ("16.0.0", "9.0.0"));
    let out = std::process::Command::new(common::python_for_tools())
        .arg(repo_root().join("scripts/accel/gen-hf-unicode-data.py"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let checked_in = std::fs::read(repo_root().join("crates/logicaffeine_compile/src/model/hf/unicode_data.rs")).unwrap();
    assert!(out.stdout == checked_in, "unicode_data.rs is not what scripts/accel/gen-hf-unicode-data.py writes; regenerate it");
}

/// The golden models' published checkpoints, as their maintainers publish them (`models/<model>/hf`).
const PUBLISHED: [&str; 3] = ["qwen3.5-0.8b", "gemma3-1b", "llama3.2-1b"];

fn published(model: &str) -> Option<(PathBuf, PathBuf)> {
    common::require_tool("hf-published").then(|| common::hf_twins::published(model))
}

/// Units in the last place between two f32 values of one sign.
fn ulps(a: f32, b: f32) -> u32 {
    if a.is_sign_negative() != b.is_sign_negative() {
        return if a == b { 0 } else { u32::MAX };
    }
    a.to_bits().abs_diff(b.to_bits())
}

/// A published checkpoint reads as the GGUF the converter writes from it: every architecture key, the spec, the
/// tensor directory, dims, and every value (compared a block of rows at a time), with nothing left unread. A value a
/// converter step computes is held within one unit in the last place: the reader's exponential is correctly rounded,
/// the converter's is torch's CPU exponential, which is not always.
fn published_reads_as_its_converted_gguf(model: &str) {
    let Some((dir, gguf)) = published(model) else { return };
    let mut hf = HfCheckpoint::open(&dir).unwrap_or_else(|e| panic!("{model}: {e}"));
    let mut g = Gguf::open(&gguf).unwrap();
    assert_eq!(SpecSource::arch(&hf), SpecSource::arch(&g), "{model}: the architecture");
    let prefix = format!("{}.", g.arch.arch);
    for key in g.order.iter().filter(|k| k.starts_with(&prefix)) {
        let want = norm(&g.meta[key]);
        assert_eq!(hf.arch_meta(&key[prefix.len()..]).map(norm).as_deref(), Some(want.as_str()), "{model}: `{key}`");
    }
    assert_eq!(ModelSpec::from_source(&hf).unwrap_or_else(|e| panic!("{model}: {e:?}")), ModelSpec::from_gguf(&g).unwrap(), "{model}: the spec");
    let mut theirs: Vec<String> = g.tensors().iter().map(|t| t.name.clone()).collect();
    let mut ours: Vec<String> = SpecSource::tensor_names(&hf).iter().map(|s| s.to_string()).collect();
    theirs.sort();
    ours.sort();
    assert_eq!(ours, theirs, "{model}: the tensor directory");
    let recomputed: BTreeSet<String> = hf.recomputed().iter().map(|s| s.to_string()).collect();
    for name in &theirs {
        assert_eq!(SpecSource::dims(&hf, name), SpecSource::dims(&g, name), "{model}: dims of {name}");
        let within = if recomputed.contains(name) { 1 } else { 0 };
        let (ta, dims) = hf.describe(name).unwrap();
        let (tb, _) = g.describe(name).unwrap();
        let rows = dims.iter().product::<u64>() as usize / dims[0] as usize;
        for first in (0..rows).step_by(4096) {
            let count = 4096.min(rows - first);
            let (mut a, mut b) = (Vec::new(), Vec::new());
            ta.dequantize(&hf.row_bytes(name, first, count).unwrap(), &mut a);
            tb.dequantize(&g.row_bytes(name, first, count).unwrap(), &mut b);
            if let Some(i) = (0..a.len()).find(|&i| ulps(a[i], b[i]) > within) {
                panic!("{model}: {name}[{}] reads {} where the converter wrote {}", first * dims[0] as usize + i, a[i], b[i]);
            }
        }
    }
    assert!(hf.unread().is_empty(), "{model}: checkpoint tensors nothing reads: {:?}", hf.unread());
}

#[test]
fn the_published_qwen3_5_0_8b_checkpoint_reads_as_its_converted_gguf() {
    published_reads_as_its_converted_gguf("qwen3.5-0.8b");
}

#[test]
fn the_published_gemma3_1b_checkpoint_reads_as_its_converted_gguf() {
    published_reads_as_its_converted_gguf("gemma3-1b");
}

#[test]
fn the_published_llama3_2_1b_checkpoint_reads_as_its_converted_gguf() {
    published_reads_as_its_converted_gguf("llama3.2-1b");
}

#[test]
fn the_published_checkpoints_tokenizers_read_as_their_converted_ggufs() {
    for model in PUBLISHED {
        let Some((dir, gguf)) = published(model) else { return };
        tokenizes_as_converted(model, &dir, &gguf);
    }
}

/// The reference computes the same logits from a published directory as from its converted GGUF, each computed in
/// turn so one model's weights are held at a time: the same to the bit where the two hold the same values, and where a
/// decay's exponential parts them by a unit in the last place, the same greedy token at every position and logits
/// within that unit's reach.
#[test]
fn the_reference_reads_the_published_checkpoints_as_their_ggufs() {
    for model in PUBLISHED {
        let Some((dir, gguf)) = published(model) else { return };
        let prompt = [1u32, 17, 400, 2024, 9];
        let (from_hf, decays) = {
            let hf = HfCheckpoint::open(&dir).unwrap();
            let decays = hf.recomputed().iter().any(|n| n.ends_with("ssm_a"));
            (Reference::new(ModelSpec::from_source(&hf).unwrap(), hf).unwrap().step_every(&prompt).unwrap(), decays)
        };
        let from_gguf = {
            let g = Gguf::open(&gguf).unwrap();
            Reference::new(ModelSpec::from_gguf(&g).unwrap(), g).unwrap().step_every(&prompt).unwrap()
        };
        if !decays {
            assert!(from_hf == from_gguf, "{model}: the reference's logits");
            continue;
        }
        for (pos, (x, y)) in from_hf.iter().zip(&from_gguf).enumerate() {
            let argmax = |v: &[f64]| (0..v.len()).max_by(|&i, &j| v[i].total_cmp(&v[j])).unwrap();
            assert_eq!(argmax(x), argmax(y), "{model}: the greedy token at position {pos}");
            let top = y.iter().fold(0f64, |m, v| m.max(v.abs()));
            let apart = x.iter().zip(y).map(|(p, q)| (p - q).abs()).fold(0f64, f64::max) / top;
            assert!(apart < 1e-4, "{model}: position {pos}'s logits {apart:.3e} apart");
        }
    }
}


/// A checkpoint whose tensors are stored in five dtypes at once (f32, f16, bf16 and both 8-bit floats, every 8-bit
/// code among them) reads as its converted GGUF: the 8-bit floats and every other dtype the converter widens are
/// presented as the f32 values it widens them to.
#[test]
fn a_checkpoint_of_mixed_dtypes_reads_as_its_converted_gguf() {
    let Some((dir, _)) = twin("qwen3-mixed") else { return };
    let mut dtypes = BTreeSet::new();
    for p in safetensors::shard_paths(&dir).unwrap() {
        for e in Shard::open(&p).unwrap().tensors.values() {
            dtypes.insert(format!("{:?}", e.dtype));
        }
    }
    for d in ["F32", "F16", "Bf16", "F8E4m3", "F8E5m2"] {
        assert!(dtypes.contains(d), "the twin stores {d}: {dtypes:?}");
    }
    reads_as_its_converted_gguf("qwen3-mixed");
}

/// The bytes this process has read through `read`-family calls (`rchar` of `/proc/self/io`).
fn bytes_read() -> u64 {
    let io = std::fs::read_to_string("/proc/self/io").unwrap();
    io.lines().find_map(|l| l.strip_prefix("rchar: ")).unwrap().trim().parse().unwrap()
}

/// A checkpoint directory removed when the test ends, however it ends.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A checkpoint is opened and read in place, never whole: a directory shaped as Llama 3.1 405B (126 layers of width
/// 16384, 812 GB of bf16 in eight sparse shards and an index) opens, yields its spec, and serves a row of its first
/// and last layers, its embedding and its head, all while the process reads a few megabytes.
#[test]
fn a_checkpoint_of_hundreds_of_gigabytes_is_never_read_whole() {
    let (d, ff, heads, kv, hd, layers, vocab) = (16384u64, 53248u64, 128u64, 8u64, 128u64, 126usize, 128256u64);
    let dir = repo_root().join("target/hf-twins/scratch").join(format!("llama-405b-{}", std::process::id()));
    let _scratch = Scratch(dir.clone());
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let config = serde_json::json!({
        "architectures": ["LlamaForCausalLM"], "model_type": "llama", "hidden_size": d, "intermediate_size": ff,
        "num_hidden_layers": layers, "num_attention_heads": heads, "num_key_value_heads": kv, "head_dim": hd,
        "rms_norm_eps": 1e-5, "vocab_size": vocab, "max_position_embeddings": 131072, "rope_theta": 500000.0,
        "rope_scaling": {"factor": 8.0, "low_freq_factor": 1.0, "high_freq_factor": 4.0,
                         "original_max_position_embeddings": 8192, "rope_type": "llama3"},
        "tie_word_embeddings": false, "hidden_act": "silu", "torch_dtype": "bfloat16", "bos_token_id": 128000,
        "eos_token_id": 128001, "attention_bias": false, "mlp_bias": false, "pretraining_tp": 1, "use_cache": true,
        "attention_dropout": 0.0, "initializer_range": 0.02
    });
    std::fs::write(dir.join("config.json"), config.to_string()).unwrap();
    let mut tensors: Vec<(String, Vec<u64>)> = vec![("model.embed_tokens.weight".into(), vec![vocab, d])];
    for l in 0..layers {
        let p = format!("model.layers.{l}.");
        for (name, shape) in [
            ("input_layernorm.weight", vec![d]),
            ("self_attn.q_proj.weight", vec![heads * hd, d]),
            ("self_attn.k_proj.weight", vec![kv * hd, d]),
            ("self_attn.v_proj.weight", vec![kv * hd, d]),
            ("self_attn.o_proj.weight", vec![d, heads * hd]),
            ("post_attention_layernorm.weight", vec![d]),
            ("mlp.gate_proj.weight", vec![ff, d]),
            ("mlp.up_proj.weight", vec![ff, d]),
            ("mlp.down_proj.weight", vec![d, ff]),
        ] {
            tensors.push((format!("{p}{name}"), shape));
        }
    }
    tensors.push(("model.norm.weight".into(), vec![d]));
    tensors.push(("lm_head.weight".into(), vec![vocab, d]));
    let total: u64 = tensors.iter().map(|(_, s)| s.iter().product::<u64>() * 2).sum();
    assert!(total > 800_000_000_000, "the checkpoint holds {total} bytes");
    // Each tensor goes to shard ⌊8 · (bytes before it) / total⌋: eight shards, each a contiguous run of tensors.
    let mut shards: Vec<Vec<(String, Vec<u64>)>> = vec![Vec::new(); 8];
    let mut before = 0u64;
    for (name, shape) in tensors {
        let len = shape.iter().product::<u64>() * 2;
        shards[(before * 8 / total) as usize].push((name, shape));
        before += len;
    }
    let mut weight_map = serde_json::Map::new();
    for (index, shard) in shards.into_iter().enumerate() {
        assert!(!shard.is_empty(), "shard {index} holds a tensor");
        let file = format!("model-{:05}-of-00008.safetensors", index + 1);
        let mut header = serde_json::Map::new();
        let mut at = 0u64;
        for (name, shape) in shard {
            let len = shape.iter().product::<u64>() * 2;
            header.insert(name.clone(), serde_json::json!({"dtype": "BF16", "shape": shape, "data_offsets": [at, at + len]}));
            weight_map.insert(name, serde_json::json!(file));
            at += len;
        }
        let header = serde_json::Value::Object(header).to_string().into_bytes();
        let mut f = std::fs::File::create(dir.join(&file)).unwrap();
        use std::io::Write;
        f.write_all(&(header.len() as u64).to_le_bytes()).unwrap();
        f.write_all(&header).unwrap();
        f.set_len(8 + header.len() as u64 + at).unwrap();
    }
    std::fs::write(dir.join("model.safetensors.index.json"), serde_json::json!({"metadata": {"total_size": total}, "weight_map": weight_map}).to_string()).unwrap();

    let before = bytes_read();
    let mut hf = HfCheckpoint::open(&dir).unwrap();
    let spec = ModelSpec::from_source(&hf).unwrap();
    assert_eq!(spec.d, d as usize, "the width");
    assert_eq!(hf.shard_count(), 8);
    assert_eq!(SpecSource::dims(&hf, "blk.125.ffn_down.weight"), Some(&[ff, d][..]));
    assert!(hf.unread().is_empty(), "{:?}", hf.unread());
    for (name, row) in [
        ("token_embd.weight", vocab as usize - 1),
        ("output.weight", 0),
        ("blk.0.attn_q.weight", (heads * hd) as usize - 1),
        ("blk.125.attn_k.weight", 7),
        ("blk.125.ffn_down.weight", d as usize - 1),
    ] {
        let (ty, dims) = hf.describe(name).unwrap();
        let bytes = hf.row_bytes(name, row, 1).unwrap();
        let mut values = Vec::new();
        ty.dequantize(&bytes, &mut values);
        assert_eq!(values.len(), dims[0] as usize, "{name}: one row");
        assert!(values.iter().all(|&v| v == 0.0), "{name}: an unwritten row reads as zeros");
    }
    let (_, rope) = hf.describe("rope_freqs.weight").unwrap();
    assert_eq!(rope, vec![hd / 2], "the rotary factors the converter computes");
    let read = bytes_read() - before;
    assert!(read < 16 << 20, "opening and reading five rows of an 812 GB checkpoint read {read} bytes");
}

/// Every architecture section 6 of `benchmarks/PREPARE_FOR_SAMBA.md` lists (its tiers A to C, the vision towers, the
/// encoders and the pooling heads aside): the `model_type` its configurations name, the class its checkpoints name, the
/// GGUF architecture the pinned converter writes for it, and the twin (`scripts/accel/hf-tiny-twin.py`) that holds the
/// arm to that converter.
const SECTION_6: &[(&str, &str, &str, &str)] = &[
    ("llama", "LlamaForCausalLM", "llama", "llama"),
    ("mistral", "MistralForCausalLM", "llama", "mistral"),
    ("mixtral", "MixtralForCausalLM", "llama", "mixtral"),
    ("ministral3", "Ministral3ForCausalLM", "mistral3", "ministral3"),
    ("mistral3", "Mistral3ForConditionalGeneration", "mistral3", "ministral3-mm"),
    ("qwen2", "Qwen2ForCausalLM", "qwen2", "qwen2"),
    ("qwen2_moe", "Qwen2MoeForCausalLM", "qwen2moe", "qwen2moe"),
    ("qwen3_next", "Qwen3NextForCausalLM", "qwen3next", "qwen3next"),
    ("qwen3_5", "Qwen3_5ForConditionalGeneration", "qwen35", "qwen35"),
    ("qwen3_5_moe", "Qwen3_5MoeForConditionalGeneration", "qwen35moe", "qwen35moe"),
    ("qwen4_exp", "Qwen4ExpForConditionalGeneration", "qwen4exp", "qwen4exp"),
    ("llama4", "Llama4ForConditionalGeneration", "llama4", "llama4-mm"),
    ("gpt_oss", "GptOssForCausalLM", "gpt-oss", "gpt-oss"),
    ("phi3", "Phi3ForCausalLM", "phi3", "phi3"),
    ("phimoe", "PhiMoEForCausalLM", "phimoe", "phimoe"),
    ("gemma", "GemmaForCausalLM", "gemma", "gemma"),
    ("gemma2", "Gemma2ForCausalLM", "gemma2", "gemma2"),
    ("gemma3_text", "Gemma3ForCausalLM", "gemma3", "gemma3"),
    ("gemma3n", "Gemma3nForConditionalGeneration", "gemma3n", "gemma3n-mm"),
    ("gemma4", "Gemma4ForConditionalGeneration", "gemma4", "gemma4"),
    ("glm4", "Glm4ForCausalLM", "glm4", "glm4"),
    ("glm4_moe", "Glm4MoeForCausalLM", "glm4moe", "glm4moe"),
    ("glm_moe_dsa", "GlmMoeDsaForCausalLM", "glm-dsa", "glm-dsa"),
    ("deepseek_v3", "DeepseekV3ForCausalLM", "deepseek2", "deepseek2"),
    ("deepseek_v3", "DeepseekV3ForCausalLM", "deepseek2", "kimi_k2"),
    ("deepseek_v32", "DeepseekV32ForCausalLM", "deepseek32", "deepseek32"),
    ("kimi_linear", "KimiLinearForCausalLM", "kimi-linear", "kimi-linear"),
    ("minimax_m2", "MiniMaxM2ForCausalLM", "minimax-m2", "minimax_m2"),
    ("minimax", "MiniMaxForCausalLM", "minimax-01", "minimax-01"),
    ("minimax_text_01", "MiniMaxText01ForCausalLM", "minimax-01", "minimax-text-01"),
    ("jamba", "JambaForCausalLM", "jamba", "jamba"),
    ("falcon_h1", "FalconH1ForCausalLM", "falcon-h1", "falcon-h1"),
    ("nemotron_h", "NemotronHForCausalLM", "nemotron_h", "nemotron_h"),
    ("mamba2", "Mamba2ForCausalLM", "mamba2", "mamba2"),
    ("rwkv7", "RWKV7ForCausalLM", "rwkv7", "rwkv7"),
    ("rwkv7", "Rwkv7ForCausalLM", "rwkv7", "rwkv7-hf"),
    ("lfm2", "Lfm2ForCausalLM", "lfm2", "lfm2"),
];

/// The tokenizer the twin script's family `family` is built with: the entry's own `tokenizer=`, or that of the family
/// it is made from (`dict(FAMILIES["<base>"], …)`).
fn twin_tokenizer(script: &str, family: &str) -> Option<String> {
    let starts = [format!("\n    \"{family}\": dict("), format!("\nFAMILIES[\"{family}\"] = dict(")];
    let at = starts.iter().find_map(|s| script.find(s.as_str()).map(|i| i + s.len()))?;
    let rest = &script[at..];
    let end = ["\n    ),", "\n)\n"].iter().filter_map(|e| rest.find(e)).min().unwrap_or(rest.len());
    let entry = &rest[..end];
    if let Some(own) = entry.split("tokenizer=\"").nth(1).and_then(|t| t.split('"').next()) {
        return Some(own.to_string());
    }
    let base = entry.trim_start().strip_prefix("FAMILIES[\"")?.split('"').next()?;
    twin_tokenizer(script, base)
}

#[test]
fn every_section_6_architecture_has_an_arm() {
    // Each architecture reaches an arm of `ModelSpec::from_source` (`model::triage::ARCHITECTURES` lists them, and
    // `model_triage_lock` holds that list to the arms), its twin is a family of the twin script and its tokenizer one
    // the fetch script fetches; and the twin's directory reads as its class's GGUF architecture, its configuration and
    // its converted GGUF alike.
    let unread: Vec<String> = SECTION_6
        .iter()
        .filter(|(_, _, arch, _)| !ARCHITECTURES.contains(arch))
        .map(|(model_type, class, arch, _)| format!("{model_type} ({class}, `{arch}`)"))
        .collect();
    assert!(unread.is_empty(), "section 6's architectures no arm reads: {unread:?}");
    let twins = std::fs::read_to_string(repo_root().join("scripts/accel/hf-tiny-twin.py")).unwrap();
    let fetched = std::fs::read_to_string(repo_root().join("scripts/accel/fetch-hf-tokenizers.sh")).unwrap();
    for (model_type, _, _, family) in SECTION_6 {
        let tokenizer = twin_tokenizer(&twins, family).unwrap_or_else(|| panic!("{model_type}: no twin `{family}` naming its tokenizer"));
        assert!(fetched.contains(&format!("\n    \"{tokenizer} ")), "{model_type}: the tokenizer `{tokenizer}` is not one the fetch script fetches");
    }
    if !common::require_tool("hf-twins") {
        return;
    }
    for (model_type, class, arch, family) in SECTION_6 {
        let (dir, gguf) = twin(family).expect("the twins");
        let config: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).unwrap()).unwrap();
        assert_eq!(config["architectures"][0].as_str(), Some(*class), "{model_type}: the twin's class");
        let (read, _) = HfCheckpoint::read_configuration(&dir).unwrap_or_else(|e| panic!("{model_type}: {e}"));
        assert_eq!(read, *arch, "{model_type}: the architecture its configuration reads as");
        let spec = ModelSpec::from_gguf(&Gguf::open(&gguf).unwrap()).unwrap_or_else(|e| panic!("{model_type}: its converted GGUF: {e}"));
        assert_eq!(spec.arch, *arch, "{model_type}: the converted GGUF's architecture");
    }
}
