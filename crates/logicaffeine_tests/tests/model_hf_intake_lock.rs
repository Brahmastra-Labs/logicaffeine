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
use logicaffeine_compile::model::spec::{Act, Ffn, Gating, Mixer, ModelSpec, SpecSource, Window};
use std::path::{Path, PathBuf};

use logicaffeine_compile::model::hf::{pre_tokenizer_named, tiktoken_split, HfPreTokenizer, HfTokenizer, Recompute, Tiktoken, PRE_TOKENIZER_PROBE};
use logicaffeine_compile::model::quant::{bf16_to_f32, f16_to_f32, f32_to_bf16, f32_to_f16};
use logicaffeine_compile::model::safetensors::{self, Dtype, Shard};
use logicaffeine_compile::model::tokenizer::Vocab;
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
        let mut config: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).unwrap()).unwrap();
        if let Some(text) = config.get("text_config").cloned() {
            config = text;
        }
        let key = |k: &str| meta.get(&format!("{arch}.{k}")).and_then(MetaValue::as_u64);
        assert_eq!(key("embedding_length"), config["hidden_size"].as_u64(), "{repo}: the width");
        assert!(key("block_count") >= config["num_hidden_layers"].as_u64(), "{repo}: the stack");
    }
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
