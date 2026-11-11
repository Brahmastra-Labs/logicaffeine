//! model_tiny_checkpoint_lock — a tiny model `common/tiny_models.rs` writes is a whole checkpoint: its GGUF carries the
//! metadata a converter writes (`tiny_models::checkpoint_meta`), so `ModelSpec::from_gguf` reads it back as the spec it
//! was written from, and a tool that takes a checkpoint (the RDU emitter, the reference) can be handed the file alone.
//!
//! Every tiny architecture at its two widths reads back, and so does every shape the Samba exam's bring-up rows vary
//! (section 10.1 of `benchmarks/PREPARE_FOR_SAMBA.md`): a width, a head dimension, a vocabulary, a KV-head count, a
//! head count, a layer count.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test model_tiny_checkpoint_lock`

#![cfg(feature = "codegen")]

#[path = "common/tiny_models.rs"]
mod tiny_models;

use logicaffeine_compile::model::gguf::{self, Gguf, MetaValue, TensorOut};
use logicaffeine_compile::model::quant::GgmlType;
use logicaffeine_compile::model::spec::{
    Act, Attention, DeltaAttention, Dsa, Ffn, Gating, Layer, Lightning, LogitScale, Mixer, ModelSpec, Moe, Norm, Part, Recurrent, Rope,
    RopeScaling, Rwkv7, Selective, ShortConv, SpecError, Ssd, Temperature, Window,
};
use tiny_models::{checkpoint_meta, tensors, write_checkpoint, Tiny, ALL};

const CTX: usize = 8;

/// `spec` written as a GGUF and read back.
fn round_trip(spec: &ModelSpec, what: &str) -> ModelSpec {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model.gguf");
    write_checkpoint(spec, &path);
    let g = Gguf::open(&path).unwrap_or_else(|e| panic!("{what}: the checkpoint opens: {e}"));
    ModelSpec::from_gguf(&g).unwrap_or_else(|e| panic!("{what}: the checkpoint reads: {e}"))
}

#[test]
fn every_tiny_model_reads_back_as_its_spec() {
    for &t in ALL {
        for (spec, width) in [(t.spec(CTX), "at D"), (t.wide(CTX), "four times as wide")] {
            let what = format!("{t:?} {width}");
            assert_eq!(round_trip(&spec, &what), spec, "{what}");
        }
    }
}

#[test]
fn every_shape_the_bring_up_exam_varies_reads_back() {
    let mut shapes: Vec<(String, ModelSpec)> = Vec::new();
    for d in [2880, 7168] {
        let mut s = Tiny::Qwen35.spec(CTX);
        s.d = d;
        shapes.push((format!("{d} wide"), s));
    }
    for head_dim in [64, 80, 96, 112, 128, 256] {
        let mut s = Tiny::Qwen35.spec(CTX);
        for l in &mut s.layers {
            if let Mixer::Attention(a) = &mut l.mixer {
                a.head_dim = head_dim;
                a.value_dim = head_dim;
                a.scale = 1.0 / (head_dim as f64).sqrt();
                if let Some(r) = a.rope.as_mut() {
                    r.dims = r.dims.min(head_dim);
                }
            }
        }
        shapes.push((format!("heads of {head_dim}"), s));
    }
    for vocab in [151_936, 163_840, 201_088, 248_320] {
        let mut s = Tiny::Qwen35.spec(CTX);
        s.vocab = vocab;
        shapes.push((format!("a vocabulary of {vocab}"), s));
    }
    for (heads, kv) in [(4, 1), (4, 4), (24, 8), (40, 8), (56, 8)] {
        let mut s = Tiny::Qwen35.spec(CTX);
        for l in &mut s.layers {
            if let Mixer::Attention(a) = &mut l.mixer {
                (a.heads, a.kv_heads) = (heads, kv);
            }
        }
        shapes.push((format!("{heads} heads over {kv} KV heads"), s));
    }
    let mut s = Tiny::Qwen35.spec(CTX);
    let period = s.layers.len();
    s.layers = (0..61).map(|l| s.layers[l % period].clone()).collect();
    shapes.push(("61 layers".to_string(), s));
    let mut s = Tiny::Llama.spec(CTX);
    s.layers.truncate(1);
    shapes.push(("one layer".to_string(), s));
    for (what, spec) in shapes {
        assert_eq!(round_trip(&spec, &what), spec, "{what}");
    }
}

/// `models/qwen2moe.cpp`'s spec: Qwen2's biased attention and experts by a softmax whose kept weights are not
/// renormalized, beside a shared expert twice a routed one's width scaled by its sigmoid gate.
fn qwen2moe() -> ModelSpec {
    let mut s = Tiny::Qwen3Moe.spec(CTX);
    s.arch = "qwen2moe".to_string();
    for l in &mut s.layers {
        if let Mixer::Attention(a) = &mut l.mixer {
            (a.bias, a.qk_norm) = (true, false);
        }
        if let Ffn::Moe(m) = &mut l.ffn {
            m.norm_top_k = false;
            (m.shared_experts, m.shared_dff, m.shared_gate) = (1, 2 * m.expert_dff, true);
        }
    }
    s
}

/// A mixture a `llama` checkpoint carries (Mixtral's): interleaved rotation, the kept weights renormalized.
fn llama_mixture() -> ModelSpec {
    let mut s = Tiny::Qwen3Moe.spec(CTX);
    s.arch = "llama".to_string();
    for l in &mut s.layers {
        if let Mixer::Attention(a) = &mut l.mixer {
            a.qk_norm = false;
            if let Some(r) = a.rope.as_mut() {
                r.neox = false;
            }
        }
    }
    s
}

#[test]
fn a_qwen35moe_checkpoint_reads_back_as_its_spec() {
    // `models/qwen35moe.cpp`: Qwen3.5's hybrid with Qwen's mixture in every block, the kept weights renormalized.
    let mut s = Tiny::Qwen35.spec(CTX);
    s.arch = "qwen35moe".to_string();
    let Ffn::Moe(moe) = qwen2moe().layers[0].ffn.clone() else { unreachable!("a mixture") };
    for l in &mut s.layers {
        l.ffn = Ffn::Moe(logicaffeine_compile::model::spec::Moe { norm_top_k: true, ..moe.clone() });
    }
    assert_eq!(round_trip(&s, "Qwen3.5-MoE"), s, "Qwen3.5-MoE");
}

#[test]
fn a_qwen2moe_checkpoint_reads_back_as_its_spec() {
    let s = qwen2moe();
    assert_eq!(round_trip(&s, "Qwen2-MoE"), s, "Qwen2-MoE");
}

/// MiniMax-M2's shape: Qwen3-MoE's with its query and key normed across every head at once, half of each head rotated,
/// and the experts scored by sigmoids beside a routing bias.
fn minimax_m2() -> ModelSpec {
    let mut s = Tiny::Qwen3Moe.spec(CTX);
    s.arch = "minimax-m2".to_string();
    for l in &mut s.layers {
        if let Mixer::Attention(a) = &mut l.mixer {
            (a.qk_norm, a.qk_norm_whole) = (true, true);
            if let Some(r) = a.rope.as_mut() {
                r.dims = a.head_dim / 2;
            }
        }
        if let Ffn::Moe(m) = &mut l.ffn {
            m.gating = Gating::Sigmoid { bias: true };
        }
    }
    s
}

/// Phi-4's shape: one query-key-value projection, NEOX rotation, a dense SwiGLU, no window.
fn phi3() -> ModelSpec {
    let mut s = Tiny::Qwen3.spec(CTX);
    s.arch = "phi3".to_string();
    for l in &mut s.layers {
        if let Mixer::Attention(a) = &mut l.mixer {
            (a.qk_norm, a.fused_qkv) = (false, true);
        }
    }
    s
}

/// GLM-4's shape: Llama's interleaved rotation over half of each head, biased projections, sandwich norms.
fn glm4() -> ModelSpec {
    let mut s = Tiny::Llama.spec(CTX);
    s.arch = "glm4".to_string();
    for l in &mut s.layers {
        if let Mixer::Attention(a) = &mut l.mixer {
            a.bias = true;
            if let Some(r) = a.rope.as_mut() {
                r.dims = a.head_dim / 2;
            }
        }
        (l.post_mixer_norm, l.post_ffn_norm) = (true, true);
    }
    s
}

/// GLM-4.5's shape: biased projections with per-head QK norms under NEOX rotation over half of each head; a dense first
/// block, then DeepSeek's mixture (sigmoid scores chosen with a routing bias, renormalized and scaled, one shared
/// expert); the draft block after the stack.
fn glm4moe() -> ModelSpec {
    let mut s = Tiny::Qwen3Moe.spec(CTX);
    s.arch = "glm4moe".to_string();
    s.mtp_blocks = 1;
    let dense = Ffn::Dense { dff: 2 * s.d, act: Act::Silu };
    for (i, l) in s.layers.iter_mut().enumerate() {
        if let Mixer::Attention(a) = &mut l.mixer {
            (a.bias, a.qk_norm) = (true, true);
            if let Some(r) = a.rope.as_mut() {
                r.dims = a.head_dim / 2;
            }
        }
        match &mut l.ffn {
            _ if i == 0 => l.ffn = dense.clone(),
            Ffn::Moe(m) => {
                m.gating = Gating::Sigmoid { bias: true };
                (m.norm_top_k, m.weights_scale, m.shared_experts, m.shared_dff) = (true, 2.5, 1, m.expert_dff);
            }
            other => panic!("Qwen3-MoE's block {i} is {other:?}, not a mixture"),
        }
    }
    s
}

#[test]
fn a_glm4moe_checkpoint_reads_back_as_its_spec() {
    let s = glm4moe();
    assert!(s.layers.len() >= 2, "a dense block and a mixture");
    assert_eq!(round_trip(&s, "GLM-4.5"), s, "GLM-4.5");
}

#[test]
fn a_glm4_checkpoint_reads_back_as_its_spec() {
    let s = glm4();
    assert_eq!(round_trip(&s, "GLM-4"), s, "GLM-4");
}

#[test]
fn a_phi3_checkpoint_reads_back_as_its_spec() {
    let s = phi3();
    assert_eq!(round_trip(&s, "Phi-4"), s, "Phi-4");
}

#[test]
fn a_minimax_m2_checkpoint_reads_back_as_its_spec() {
    let s = minimax_m2();
    assert_eq!(round_trip(&s, "MiniMax-M2"), s, "MiniMax-M2");
}

/// What `from_gguf` reads of `spec`'s checkpoint with its metadata as `edit` leaves it (every value zero: the spec
/// reads keys, names and shapes).
fn read_with(spec: &ModelSpec, edit: impl Fn(&mut Vec<(String, MetaValue)>)) -> ModelSpec {
    let mut meta = checkpoint_meta(spec);
    edit(&mut meta);
    let list = tensors(spec);
    let outs: Vec<TensorOut> = list.iter().map(|(n, d)| TensorOut { name: n.clone(), dims: d.clone(), ty: GgmlType::F32 }).collect();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model.gguf");
    gguf::write(&path, &meta, &outs, |k| Ok(vec![0u8; 4 * list[k].1.iter().product::<u64>() as usize])).expect("the checkpoint writes");
    ModelSpec::from_gguf(&Gguf::open(&path).expect("the checkpoint opens")).expect("the checkpoint reads")
}

#[test]
fn a_qwen2moe_checkpoint_reads_llama_cpps_widths() {
    // llama.cpp's qwen2moe: unnamed or 0, an expert is `feed_forward_length` over the experts a token takes and the
    // shared expert `feed_forward_length`; a per-block array of expert widths is read at block 0 for every block.
    let s = qwen2moe();
    let Ffn::Moe(m) = &s.layers[0].ffn else { unreachable!("a mixture") };
    let ff = 48 * m.top_k;
    let widths = |spec: &ModelSpec| -> Vec<(usize, usize)> {
        spec.layers.iter().map(|l| match &l.ffn {
            Ffn::Moe(m) => (m.expert_dff, m.shared_dff),
            other => panic!("a mixture, not {other:?}"),
        }).collect()
    };
    let unnamed = read_with(&s, |meta| {
        meta.retain(|(k, _)| !k.ends_with(".expert_feed_forward_length") && !k.ends_with(".expert_shared_feed_forward_length"));
        meta.push(("qwen2moe.feed_forward_length".into(), MetaValue::U32(ff as u32)));
    });
    assert_eq!(widths(&unnamed), vec![(ff / m.top_k, ff); s.layers.len()], "unnamed widths");
    let zero = read_with(&s, |meta| {
        for (k, v) in meta.iter_mut() {
            if k.ends_with(".expert_feed_forward_length") || k.ends_with(".expert_shared_feed_forward_length") {
                *v = MetaValue::U32(0);
            }
        }
        meta.push(("qwen2moe.feed_forward_length".into(), MetaValue::U32(ff as u32)));
    });
    assert_eq!(widths(&zero), vec![(ff / m.top_k, ff); s.layers.len()], "zero widths");
    let per_block = read_with(&s, |meta| {
        for (k, v) in meta.iter_mut() {
            if k.ends_with(".expert_feed_forward_length") {
                let items = (0..s.layers.len()).map(|l| MetaValue::U32(((l + 1) * m.expert_dff) as u32)).collect();
                *v = MetaValue::Array { ty: 4, items };
            }
        }
    });
    assert_eq!(widths(&per_block), vec![(m.expert_dff, m.shared_dff); s.layers.len()], "block 0's width for every block");
}

#[test]
fn a_routed_scale_llama_cpp_never_reads_is_not_read() {
    // llama.cpp scales the kept expert weights only on the graphs that read `expert_weights_scale` (DeepSeek's among
    // them); Qwen3-MoE's, Qwen2-MoE's, gpt-oss's and a Llama-named mixture's never do, so a scale such a checkpoint
    // carries is not the model's.
    let models = [
        ("Qwen3-MoE", Tiny::Qwen3Moe.spec(CTX)),
        ("Qwen2-MoE", qwen2moe()),
        ("gpt-oss", Tiny::GptOss.spec(CTX)),
        ("a llama mixture", llama_mixture()),
    ];
    for (what, plain) in models {
        assert_eq!(round_trip(&plain, what), plain, "{what}");
        let mut scaled = plain.clone();
        for l in &mut scaled.layers {
            if let Ffn::Moe(m) = &mut l.ffn {
                m.weights_scale = 2.0;
            }
        }
        assert_eq!(round_trip(&scaled, what), plain, "{what}: the routed scale is not read");
    }
}

#[test]
fn an_unknown_architecture_name_is_written_as_given() {
    let mut s = Tiny::Llama.spec(CTX);
    s.arch = "frobnicator".to_string();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model.gguf");
    write_checkpoint(&s, &path);
    let g = Gguf::open(&path).unwrap();
    assert_eq!(g.meta.get("general.architecture").and_then(|v| v.as_str()), Some("frobnicator"));
    assert!(g.meta.contains_key("frobnicator.embedding_length"), "its keys carry its name");
    assert!(ModelSpec::from_gguf(&g).is_err(), "no arm reads it");
}

/// Granite's shape (`models/granite.cpp`): Llama's, with the embedding times `embedding_scale`, every score times
/// `attention.scale`, each sublayer's output times `residual_scale` before the residual, and the logits divided by
/// `logit_scale` after the head.
fn granite() -> ModelSpec {
    let mut s = Tiny::Llama.spec(CTX);
    s.arch = "granite".to_string();
    for l in &mut s.layers {
        if let Mixer::Attention(a) = &mut l.mixer {
            a.scale = 0.0078125;
        }
    }
    ModelSpec { embedding_scale: Some(12.0), residual_scale: Some(0.22), logit_scale: Some(LogitScale::AfterHead(0.125)), ..s }
}

/// MiniCPM's shape (`models/minicpm.cpp`, the HF graph's head): Llama's, with the embedding times `embedding_scale`,
/// each sublayer's output times `residual_scale`, and the output norm's result divided by `logit_scale` before the head.
fn minicpm() -> ModelSpec {
    let mut s = Tiny::Llama.spec(CTX);
    s.arch = "minicpm".to_string();
    ModelSpec { embedding_scale: Some(12.0), residual_scale: Some(0.35), logit_scale: Some(LogitScale::BeforeHead(0.125)), ..s }
}

#[test]
fn a_granite_checkpoint_reads_back_as_its_spec() {
    let s = granite();
    assert_eq!(round_trip(&s, "Granite"), s, "Granite");
}

#[test]
fn a_minicpm_checkpoint_reads_back_as_its_spec() {
    let s = minicpm();
    assert_eq!(round_trip(&s, "MiniCPM"), s, "MiniCPM");
}

#[test]
fn a_minicpm_checkpoint_without_its_multipliers_reads_llama_cpps_defaults() {
    // `models/minicpm.cpp`: an embedding scale of 12, a residual scale of 1.4/√layers and a logit scale of 256/d where
    // the file states none.
    let s = minicpm();
    let read = read_with(&s, |m| m.retain(|(k, _)| !k.ends_with("embedding_scale") && !k.ends_with("residual_scale") && !k.ends_with("logit_scale")));
    assert_eq!(read.embedding_scale, Some(12.0));
    assert_eq!(read.residual_scale, Some(1.4 / (s.layers.len() as f64).sqrt()));
    assert_eq!(read.logit_scale, Some(LogitScale::BeforeHead(s.d as f64 / 256.0)));
}

#[test]
fn a_granite_checkpoint_without_its_optional_multipliers_reads_none() {
    // `models/granite.cpp`: a residual or embedding scale of zero (absent) applies none; an attention scale of zero
    // scores by 1/√hd.
    let s = granite();
    let read = read_with(&s, |m| m.retain(|(k, _)| !k.ends_with("embedding_scale") && !k.ends_with("residual_scale") && !k.ends_with("attention.scale")));
    assert_eq!((read.embedding_scale, read.residual_scale), (None, None));
    let Mixer::Attention(a) = &read.layers[0].mixer else { panic!("an attention layer") };
    assert_eq!(a.scale, 1.0 / (a.head_dim as f64).sqrt());
}

/// Arcee's shape (`models/arcee.cpp`): Llama's, with an FFN of relu² of the up projection and no gate projection.
fn arcee() -> ModelSpec {
    let mut s = Tiny::Llama.spec(CTX);
    s.arch = "arcee".to_string();
    for l in &mut s.layers {
        if let Ffn::Dense { dff, .. } = l.ffn {
            l.ffn = Ffn::Dense { dff, act: Act::ReluSquared };
        }
    }
    s
}

#[test]
fn an_arcee_checkpoint_reads_back_as_its_spec() {
    let s = arcee();
    assert_eq!(round_trip(&s, "Arcee"), s, "Arcee");
}

/// OLMo 3's shape (`models/olmo2.cpp` with a window): OLMo 2's, four layers, all but every fourth attending a sliding
/// window of 4, turning by the global base unscaled.
fn olmo3() -> ModelSpec {
    let mut s = Tiny::Olmo2.spec(CTX);
    let last = s.layers.last().cloned().expect("a layer");
    s.layers.push(last);
    for (l, layer) in s.layers.iter_mut().enumerate() {
        if let Mixer::Attention(a) = &mut layer.mixer {
            a.window = (l % 4 < 3).then_some(Window::Sliding(4));
        }
    }
    s
}

#[test]
fn an_olmo2_checkpoint_reads_back_as_its_spec() {
    for (s, what) in [(Tiny::Olmo2.spec(CTX), "OLMo 2"), (Tiny::Olmo2.wide(CTX), "OLMo 2 four times as wide"), (olmo3(), "OLMo 3")] {
        let read = round_trip(&s, what);
        assert_eq!(read, s, "{what}");
        assert!(read.layers.iter().all(|l| !l.pre_norms && l.post_mixer_norm && l.post_ffn_norm), "{what}: a norm after each sublayer, none before");
    }
}

#[test]
fn an_olmo2_checkpoint_whose_qk_norm_is_a_heads_is_refused_by_name() {
    // OLMo 2 norms every head's queries at once: a checkpoint whose `attn_q_norm` is one head wide is another model.
    let mut s = Tiny::Olmo2.spec(CTX);
    for l in &mut s.layers {
        if let Mixer::Attention(a) = &mut l.mixer {
            a.qk_norm_whole = false;
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model.gguf");
    write_checkpoint(&s, &path);
    let err = ModelSpec::from_gguf(&Gguf::open(&path).unwrap()).expect_err("a head-wide QK norm is not OLMo 2's");
    let why = err.to_string();
    assert!(why.contains("attn_q_norm") && why.contains("at once"), "the refusal names the norm and what OLMo 2 norms: {why}");
}

/// Ministral 3's shape (`models/mistral3.cpp`): Llama's, under YaRN over an original context of 4, every layer's rotated
/// queries taking the attention temperature, its floor that original context and its positions counted from zero.
fn mistral3() -> ModelSpec {
    let mut s = Tiny::Llama.spec(CTX);
    s.arch = "mistral3".to_string();
    let magnitude = 0.1 * 16f64.ln() + 1.0;
    for l in &mut s.layers {
        if let Mixer::Attention(a) = &mut l.mixer {
            if let Some(r) = a.rope.as_mut() {
                r.scaling = RopeScaling::Yarn { factor: 16.0, original_ctx: 4, beta_fast: 32.0, beta_slow: 1.0, magnitude };
            }
            a.temperature = Some(Temperature { floor: 4.0, scale: 0.1, offset: 0.0 });
        }
    }
    s
}

/// A mixture a `mistral3` checkpoint carries: Mixtral's, its experts as wide as the FFN.
fn mistral3_mixture() -> ModelSpec {
    let mut s = llama_mixture();
    s.arch = "mistral3".to_string();
    s
}

#[test]
fn a_mistral3_checkpoint_reads_back_as_its_spec() {
    let s = mistral3();
    assert_eq!(round_trip(&s, "Ministral 3"), s, "Ministral 3");
    // Mistral Small 3.1 and 3.2: the arm with neither YaRN nor a temperature.
    let mut plain = Tiny::Llama.spec(CTX);
    plain.arch = "mistral3".to_string();
    assert_eq!(round_trip(&plain, "Mistral Small 3"), plain, "Mistral Small 3");
    let m = mistral3_mixture();
    assert_eq!(round_trip(&m, "a mistral3 mixture"), m, "a mistral3 mixture");
}

#[test]
fn a_mistral3_temperature_floors_by_the_original_context() {
    // `models/mistral3.cpp`: the floor is llama.cpp's `n_ctx_orig_yarn`, YaRN's original context where the file writes
    // one and the trained context where it does not.
    let s = mistral3();
    let floors = |spec: &ModelSpec| -> Vec<Option<Temperature>> {
        spec.layers.iter().map(|l| match &l.mixer {
            Mixer::Attention(a) => a.temperature,
            other => panic!("an attention layer, not {other:?}"),
        }).collect()
    };
    let unscaled = read_with(&s, |meta| meta.retain(|(k, _)| !k.contains(".rope.scaling.")));
    assert_eq!(
        floors(&unscaled),
        vec![Some(Temperature { floor: s.ctx_len as f64, scale: 0.1, offset: 0.0 }); s.layers.len()],
        "the trained context where YaRN writes none"
    );
    let untempered = read_with(&s, |meta| meta.retain(|(k, _)| !k.ends_with(".attention.temperature_scale")));
    assert_eq!(floors(&untempered), vec![None; s.layers.len()], "no temperature where the file sets no scale");
}

#[test]
fn a_mistral3_mixture_is_unscaled_and_as_wide_as_the_ffn() {
    // llama.cpp's mistral3 reads its experts' width from `feed_forward_length` and no routed scale.
    let plain = mistral3_mixture();
    let mut scaled = plain.clone();
    for l in &mut scaled.layers {
        if let Ffn::Moe(m) = &mut l.ffn {
            m.weights_scale = 2.0;
        }
    }
    assert_eq!(round_trip(&scaled, "a scaled mistral3 mixture"), plain, "the routed scale is not read");
    let Ffn::Moe(m) = &plain.layers[0].ffn else { unreachable!("a mixture") };
    let wider = read_with(&plain, |meta| {
        for (k, v) in meta.iter_mut() {
            if k.ends_with(".expert_feed_forward_length") {
                *v = MetaValue::U32(4 * m.expert_dff as u32);
            }
        }
    });
    assert_eq!(wider, plain, "an expert width of their own is not read");
}

#[test]
fn a_mistral3_checkpoint_llama_cpp_would_compute_otherwise_is_refused() {
    // The graph rotates every dimension of a head and adds a dense FFN's biases where the file carries them; the spec
    // reads neither a partial rotation nor FFN biases.
    let s = mistral3();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("partial.gguf");
    let mut meta = checkpoint_meta(&s);
    for (k, v) in meta.iter_mut() {
        if k.ends_with(".rope.dimension_count") {
            *v = MetaValue::U32(8);
        }
    }
    let list = tensors(&s);
    let outs: Vec<TensorOut> = list.iter().map(|(n, d)| TensorOut { name: n.clone(), dims: d.clone(), ty: GgmlType::F32 }).collect();
    gguf::write(&path, &meta, &outs, |k| Ok(vec![0u8; 4 * list[k].1.iter().product::<u64>() as usize])).unwrap();
    let err = ModelSpec::from_gguf(&Gguf::open(&path).unwrap()).expect_err("a partial rotation is refused");
    assert!(err.to_string().contains("rotates 8"), "{err}");
    let path = dir.path().join("biased.gguf");
    let mut list = tensors(&s);
    list.push(("blk.0.ffn_up.bias".to_string(), vec![96]));
    let outs: Vec<TensorOut> = list.iter().map(|(n, d)| TensorOut { name: n.clone(), dims: d.clone(), ty: GgmlType::F32 }).collect();
    gguf::write(&path, &checkpoint_meta(&s), &outs, |k| Ok(vec![0u8; 4 * list[k].1.iter().product::<u64>() as usize])).unwrap();
    let err = ModelSpec::from_gguf(&Gguf::open(&path).unwrap()).expect_err("an FFN bias is refused");
    assert!(err.to_string().contains("ffn_up.bias"), "{err}");
}

/// PhiMoE's shape, as transformers computes it: LayerNorms with their biases, biased attention under a NEOX rotation and a
/// window of 4, sparsemixer's two of four experts as wide as the FFN, and a biased head.
fn phimoe() -> ModelSpec {
    let mut s = Tiny::Qwen3Moe.spec(CTX);
    s.arch = "phimoe".to_string();
    for l in &mut s.layers {
        if let Mixer::Attention(a) = &mut l.mixer {
            (a.qk_norm, a.bias, a.output_bias) = (false, true, true);
            a.window = Some(Window::Sliding(4));
        }
        if let Ffn::Moe(m) = &mut l.ffn {
            (m.gating, m.norm_top_k) = (Gating::SparseMixer { jitter: 0.01 }, false);
        }
    }
    ModelSpec { norm: Norm::Layer { bias: true }, head_bias: true, ..s }
}

#[test]
fn a_phimoe_checkpoint_reads_back_as_its_spec() {
    let s = phimoe();
    assert_eq!(round_trip(&s, "PhiMoE"), s, "PhiMoE");
    // A window as long as the context is none.
    let unwindowed = read_with(&s, |meta| {
        for (k, v) in meta.iter_mut() {
            if k.ends_with(".attention.sliding_window") {
                *v = MetaValue::U32(CTX as u32);
            }
        }
    });
    for (l, layer) in unwindowed.layers.iter().enumerate() {
        let Mixer::Attention(a) = &layer.mixer else { panic!("block {l} attends") };
        assert_eq!(a.window, None, "block {l}: a window of the whole context");
    }
}

#[test]
fn a_phimoe_checkpoint_routing_other_than_two_experts_is_refused() {
    // sparsemixer chooses two experts whatever the configuration says.
    let s = phimoe();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("three.gguf");
    let mut meta = checkpoint_meta(&s);
    for (k, v) in meta.iter_mut() {
        if k.ends_with(".expert_used_count") {
            *v = MetaValue::U32(3);
        }
    }
    let list = tensors(&s);
    let outs: Vec<TensorOut> = list.iter().map(|(n, d)| TensorOut { name: n.clone(), dims: d.clone(), ty: GgmlType::F32 }).collect();
    gguf::write(&path, &meta, &outs, |k| Ok(vec![0u8; 4 * list[k].1.iter().product::<u64>() as usize])).unwrap();
    let err = ModelSpec::from_gguf(&Gguf::open(&path).unwrap()).expect_err("three experts a token is refused");
    assert!(err.to_string().contains("sparsemixer chooses two"), "{err}");
}

/// DeepSeek-V3.2's shape (`models/deepseek32.cpp`): DeepSeek-V2's split latent attention with a low-rank query, under
/// YaRN with V3.2's log multiplier of 1 (the score scale `(1 + 0.1·ln 40)²/√key`), and a lightning indexer on every layer
/// choosing 4 positions; GLM-5.2's (`models/glm-dsa.cpp`) with its rotation interleaved and every other layer's indexer
/// shared.
fn dsa(arch: &str) -> ModelSpec {
    let mut s = Tiny::DeepSeek2Split.spec(CTX);
    s.arch = arch.to_string();
    let n = s.layers.len();
    for (l, layer) in s.layers.iter_mut().enumerate() {
        if let Mixer::Mla(m) = &mut layer.mixer {
            let rope = m.rope.as_mut().expect("DeepSeek's latent attention rotates");
            if let RopeScaling::Yarn { factor, magnitude, .. } = &mut rope.scaling {
                *magnitude = 1.0;
                let mscale = 1.0 + 0.1 * factor.ln();
                m.scale = mscale * mscale / ((m.qk_nope + m.qk_rope) as f64).sqrt();
            }
            let full = arch == "deepseek32" || l % 2 == 0 || n == 1;
            m.indexer = Some(Dsa { heads: 2, head_dim: 2 * m.qk_rope, top_k: 4, neox: arch == "deepseek32", full });
        }
    }
    s
}

#[test]
fn the_dsa_checkpoints_read_back_as_their_specs() {
    for arch in ["deepseek32", "glm-dsa"] {
        let s = dsa(arch);
        assert_eq!(round_trip(&s, arch), s, "{arch}");
    }
}

/// What reading `spec`'s checkpoint gives once `edit` has changed its keys and `rename` its tensors' names (`None`
/// dropping one), with the tensors `extra` beside them.
fn read_edited(
    spec: &ModelSpec,
    edit: impl Fn(&mut Vec<(String, MetaValue)>),
    rename: impl Fn(&str) -> Option<String>,
    extra: &[(&str, Vec<u64>)],
) -> Result<ModelSpec, SpecError> {
    let mut meta = checkpoint_meta(spec);
    edit(&mut meta);
    let mut list: Vec<(String, Vec<u64>)> = tensors(spec).into_iter().filter_map(|(n, d)| rename(&n).map(|n| (n, d))).collect();
    list.extend(extra.iter().map(|(n, d)| (n.to_string(), d.clone())));
    let outs: Vec<TensorOut> = list.iter().map(|(n, d)| TensorOut { name: n.clone(), dims: d.clone(), ty: GgmlType::F32 }).collect();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model.gguf");
    gguf::write(&path, &meta, &outs, |k| Ok(vec![0u8; 4 * list[k].1.iter().product::<u64>() as usize])).expect("the checkpoint writes");
    ModelSpec::from_gguf(&Gguf::open(&path).expect("the checkpoint opens"))
}

/// Every tensor of a checkpoint under its own name.
fn same(name: &str) -> Option<String> {
    Some(name.to_string())
}

/// A key of every architecture set to `v` wherever it ends in `suffix`, or added under `arch` where none does.
fn set(arch: &'static str, suffix: &'static str, v: MetaValue) -> impl Fn(&mut Vec<(String, MetaValue)>) {
    move |meta| match meta.iter_mut().find(|(k, _)| k.ends_with(suffix)) {
        Some((_, x)) => *x = v.clone(),
        None => meta.push((format!("{arch}{suffix}"), v.clone())),
    }
}

/// LFM2's shape (`models/lfm2.cpp`): gated short convolutions of 3 taps on blocks 0 and 2, Qwen3's attention (per-head
/// QK norms, a NEOX rotation) on blocks 1 and 3, the last with as many KV heads as query heads, a dense SwiGLU after
/// every block, the head tied.
fn lfm2() -> ModelSpec {
    let mut s = Tiny::Qwen3.spec(CTX);
    s.arch = "lfm2".to_string();
    let a = s
        .layers
        .iter()
        .find_map(|l| match &l.mixer {
            Mixer::Attention(a) => Some(a.clone()),
            _ => None,
        })
        .expect("Qwen3 attends");
    let ffn = s.layers[0].ffn.clone();
    let block = |mixer: Mixer| Layer { mixer, ffn: ffn.clone(), pre_norms: true, post_mixer_norm: false, post_ffn_norm: false };
    let conv = || Mixer::Recurrent(Recurrent::ShortConv(ShortConv { taps: 3 }));
    s.layers = vec![
        block(conv()),
        block(Mixer::Attention(a.clone())),
        block(conv()),
        block(Mixer::Attention(Attention { kv_heads: a.heads, ..a })),
    ];
    s.tied_output = true;
    s
}

#[test]
fn an_lfm2_checkpoint_reads_back_as_its_spec() {
    let s = lfm2();
    assert_eq!(round_trip(&s, "LFM2"), s, "LFM2");
}

#[test]
fn an_lfm2_checkpoint_llama_cpp_would_compute_otherwise_is_refused() {
    // The final norm under the converter's name for it (`model::layout` presents it as `output_norm`), a decision head, a
    // bidirectional LFM2, LFM2-ColBERT's projection and a convolution of one tap: each a model the spec's causal decoder
    // of short convolutions is not.
    let s = lfm2();
    let raw = |n: &str| Some(if n == "output_norm.weight" { "token_embd_norm.weight".to_string() } else { n.to_string() });
    let colbert = [("dense_2_out.weight", vec![s.d as u64, 8])];
    let refusals = [
        ("token_embd_norm", read_edited(&s, |_| {}, raw, &[])),
        ("decision_block_count", read_edited(&s, set("lfm2", ".decision_block_count", MetaValue::U32(1)), same, &[])),
        ("attention.causal", read_edited(&s, set("lfm2", ".attention.causal", MetaValue::Bool(false)), same, &[])),
        ("dense_2_out", read_edited(&s, |_| {}, same, &colbert)),
        ("a short convolution of 1 taps", read_edited(&s, set("lfm2", ".shortconv.l_cache", MetaValue::U32(1)), same, &[])),
    ];
    for (what, read) in refusals {
        let err = read.expect_err(what);
        assert!(err.to_string().contains(what), "{what}: {err}");
    }
}

/// Mamba-2's shape (`models/mamba2.cpp`): every block the scan alone, sixteen heads of 8 over an inner width of twice
/// the model's in two groups of B and C of 4 states, a convolution of 4 taps, the grouped gated norm, no FFN.
fn mamba2() -> ModelSpec {
    let mut s = Tiny::Llama.spec(CTX);
    s.arch = "mamba2".to_string();
    let scan = Ssd { heads: 2 * s.d / 8, head_dim: 8, state: 4, groups: 2, conv_taps: 4, gated_norm: true };
    for layer in &mut s.layers {
        *layer = Layer { mixer: Mixer::Recurrent(Recurrent::Ssd(scan.clone())), ffn: Ffn::None, pre_norms: true, post_mixer_norm: false, post_ffn_norm: false };
    }
    s
}

#[test]
fn a_mamba2_checkpoint_reads_back_as_its_spec() {
    let s = mamba2();
    assert_eq!(round_trip(&s, "Mamba-2"), s, "Mamba-2");
}

#[test]
fn a_mamba2_checkpoint_llama_cpp_would_not_load_or_would_compute_otherwise_is_refused() {
    // llama.cpp's mamba2 requires the gated norm; a head reads its group's B and C as `h / (heads / groups)`, which is
    // transformers' `repeat_interleave` only where the groups split the heads.
    let s = mamba2();
    let normless = read_edited(&s, |_| {}, |n| (!n.contains("ssm_norm")).then(|| n.to_string()), &[]).expect_err("a scan with no gated norm");
    assert!(normless.to_string().contains("ssm_norm"), "{normless}");
    for (what, suffix, v) in [
        ("three groups over sixteen heads", ".ssm.group_count", 3),
        ("no heads", ".ssm.time_step_rank", 0),
        ("no state", ".ssm.state_size", 0),
        ("no convolution", ".ssm.conv_kernel", 0),
    ] {
        let err = read_edited(&s, set("mamba2", suffix, MetaValue::U32(v)), same, &[]).expect_err(what);
        assert!(err.to_string().contains("a scan of"), "{what}: {err}");
    }
}

/// Falcon-H1's shape (`models/falcon-h1.cpp`): every block attention under a NEOX rotation (Qwen3's, without its QK
/// norms) beside Mamba-2's scan of sixteen heads of 8 in two groups, then a dense SwiGLU.
fn falcon_h1() -> ModelSpec {
    let mut s = Tiny::Qwen3.spec(CTX);
    s.arch = "falcon-h1".to_string();
    let scan = Ssd { heads: 2 * s.d / 8, head_dim: 8, state: 4, groups: 2, conv_taps: 4, gated_norm: true };
    for layer in &mut s.layers {
        let Mixer::Attention(a) = &layer.mixer else { panic!("Qwen3 attends") };
        let a = Attention { qk_norm: false, ..a.clone() };
        layer.mixer = Mixer::Recurrent(Recurrent::Parallel(Box::new(a), scan.clone()));
    }
    s
}

#[test]
fn a_falcon_h1_checkpoint_reads_back_as_its_spec() {
    let s = falcon_h1();
    assert_eq!(round_trip(&s, "Falcon-H1"), s, "Falcon-H1");
}

#[test]
fn a_falcon_h1_checkpoint_llama_cpp_would_compute_otherwise_is_refused() {
    // The FFN norm under the name llama.cpp loads it by (`model::layout` presents it with its `.weight`), and the biases
    // llama.cpp's falcon-h1 graph adds (the FFN's) or leaves out (the attention output's).
    let s = falcon_h1();
    let raw = |n: &str| Some(n.strip_suffix(".ffn_norm.weight").map_or(n.to_string(), |stem| format!("{stem}.ffn_norm")));
    let raw_read = read_edited(&s, |_| {}, raw, &[]).expect_err("a raw FFN norm");
    assert!(raw_read.to_string().contains("ffn_norm"), "{raw_read}");
    let d = s.d as u64;
    for (bias, dims) in [("blk.0.ffn_up.bias", vec![2 * d]), ("blk.0.attn_output.bias", vec![d])] {
        let err = read_edited(&s, |_| {}, same, &[(bias, dims)]).expect_err(bias);
        assert!(err.to_string().contains(&bias["blk.0.".len()..]), "{bias}: {err}");
    }
}

/// Nemotron-H's shape (`models/nemotron-h.cpp`): a scan block (sixteen heads of 8 in two groups), an FFN block of relu²,
/// an attention block with no rotation, and an FFN block, each one sublayer under its one norm.
fn nemotron_h() -> ModelSpec {
    let mut s = Tiny::Llama.spec(CTX);
    s.arch = "nemotron_h".to_string();
    let Mixer::Attention(a) = s.layers[0].mixer.clone() else { panic!("Llama attends") };
    let scan = Ssd { heads: 2 * s.d / 8, head_dim: 8, state: 4, groups: 2, conv_taps: 4, gated_norm: true };
    let block = |mixer: Mixer, ffn: Ffn| Layer { mixer, ffn, pre_norms: true, post_mixer_norm: false, post_ffn_norm: false };
    let relu = Ffn::Dense { dff: 2 * s.d, act: Act::ReluSquared };
    s.layers = vec![
        block(Mixer::Recurrent(Recurrent::Ssd(scan)), Ffn::None),
        block(Mixer::None, relu.clone()),
        block(Mixer::Attention(Attention { rope: None, ..a }), Ffn::None),
        block(Mixer::None, relu),
    ];
    s
}

#[test]
fn a_nemotron_h_checkpoint_reads_back_as_its_spec() {
    let s = nemotron_h();
    assert_eq!(round_trip(&s, "Nemotron-H"), s, "Nemotron-H");
    // The converter writes the epsilon as a LayerNorm's.
    let layer_norm = read_edited(
        &s,
        |m| {
            m.retain(|(k, _)| !k.ends_with(".attention.layer_norm_rms_epsilon"));
            m.push(("nemotron_h.attention.layer_norm_epsilon".into(), MetaValue::F64(1e-5)));
        },
        same,
        &[],
    )
    .unwrap();
    assert_eq!(layer_norm.rms_eps, 1e-5, "the LayerNorm epsilon, which llama.cpp's RMSNorms take");
}

#[test]
fn a_nemotron_h_checkpoint_llama_cpp_would_compute_otherwise_is_refused() {
    // Nemotron 3's mixture blocks, an FFN bias, and a block of both KV heads and an FFN width, which is no block of
    // Nemotron-H's one sublayer.
    let s = nemotron_h();
    let d = s.d as u64;
    let experts = read_edited(&s, |_| {}, same, &[("blk.1.ffn_gate_inp.weight", vec![d, 4])]).expect_err("experts");
    assert!(experts.to_string().contains("ffn_gate_inp"), "{experts}");
    let biased = read_edited(&s, |_| {}, same, &[("blk.1.ffn_up.bias", vec![2 * d])]).expect_err("an FFN bias");
    assert!(biased.to_string().contains("ffn_up.bias"), "{biased}");
    let both = read_edited(
        &s,
        |m| {
            for (_, v) in m.iter_mut().filter(|(k, _)| k.ends_with(".feed_forward_length")) {
                *v = MetaValue::Array { ty: 4, items: vec![MetaValue::U32(0), MetaValue::U32(128), MetaValue::U32(128), MetaValue::U32(128)] };
            }
        },
        same,
        &[],
    )
    .expect_err("attention with an FFN");
    assert!(both.to_string().contains("one sublayer a block"), "{both}");
}

/// Jamba's shape (`models/jamba.cpp`): Mamba-1's selective scan (an inner width of twice the model's, a state of 4, a
/// time step of rank 4, its dt, B and C normed) then a mixture of four experts, two a token, unnormalized; attention with
/// no rotation then a dense SwiGLU; the two again.
fn jamba() -> ModelSpec {
    let mut s = Tiny::Qwen3Moe.spec(CTX);
    s.arch = "jamba".to_string();
    let scan = Selective { inner: 2 * s.d, state: 4, dt_rank: 4, conv_taps: 4, inner_norms: true };
    for (l, layer) in s.layers.iter_mut().enumerate() {
        let Mixer::Attention(a) = &layer.mixer else { panic!("Qwen3-MoE attends") };
        let a = Attention { rope: None, qk_norm: false, ..a.clone() };
        let Ffn::Moe(m) = &layer.ffn else { panic!("Qwen3-MoE routes") };
        let m = Moe { gating: Gating::Softmax, norm_top_k: false, weights_scale: 1.0, ..m.clone() };
        let dff = m.expert_dff;
        (layer.mixer, layer.ffn) = if l % 2 == 0 {
            (Mixer::Recurrent(Recurrent::Selective(scan.clone())), Ffn::Moe(m))
        } else {
            (Mixer::Attention(a), Ffn::Dense { dff, act: Act::Silu })
        };
    }
    s
}

#[test]
fn a_jamba_checkpoint_reads_back_as_its_spec() {
    let s = jamba();
    assert!(s.layers.len() >= 2, "a scan block and an attention block");
    assert_eq!(round_trip(&s, "Jamba"), s, "Jamba");
}

/// Kimi-Linear's shape (`models/kimi-linear.cpp`): Kimi delta attention (four heads, a 4-tap convolution) on every block
/// but the last, DeepSeek-V2's split latent attention with a full-rank query and no rotation on the last; a dense block,
/// then DeepSeek's mixture under sigmoid scores with a selection bias, renormalized and scaled, its shared expert as
/// wide as one routed expert.
fn kimi_linear() -> ModelSpec {
    let mut s = Tiny::DeepSeek2Split.spec(CTX);
    s.arch = "kimi-linear".to_string();
    let last = s.layers.len() - 1;
    for (l, layer) in s.layers.iter_mut().enumerate() {
        let Mixer::Mla(m) = &mut layer.mixer else { panic!("DeepSeek-V2 attends through its latent") };
        (m.rope, m.q_lora_rank) = (None, None);
        m.scale = 1.0 / ((m.qk_nope + m.qk_rope) as f64).sqrt();
        let heads = m.heads;
        if l < last {
            layer.mixer = Mixer::Recurrent(Recurrent::DeltaAttention(DeltaAttention { heads, head_dim: 16, conv_taps: 4 }));
        }
        if let Ffn::Moe(m) = &mut layer.ffn {
            (m.gating, m.norm_top_k, m.weights_scale, m.groups) = (Gating::Sigmoid { bias: true }, true, 2.5, None);
            m.shared_dff = m.expert_dff * m.shared_experts.max(1);
        }
    }
    s
}

#[test]
fn a_kimi_linear_checkpoint_reads_back_as_its_spec() {
    let s = kimi_linear();
    assert!(s.layers.len() >= 2, "a delta attention block and a latent attention block");
    assert_eq!(round_trip(&s, "Kimi-Linear"), s, "Kimi-Linear");
}

#[test]
fn every_latent_attention_reads_its_latent_norms_at_a_millionth_whatever_the_models_epsilon() {
    // The publishers' code builds `q_a_layernorm` and `kv_a_layernorm` at 10⁻⁶ whatever `rms_norm_eps` says; llama.cpp
    // norms them at the model's epsilon, the one the GGUF carries. Each arm reads the model's, and the latent's at 10⁻⁶.
    let specs = [(Tiny::DeepSeek2.spec(CTX), "deepseek2"), (Tiny::DeepSeek2Split.spec(CTX), "deepseek2 split"), (dsa("deepseek32"), "deepseek32"), (dsa("glm-dsa"), "glm-dsa"), (kimi_linear(), "kimi-linear")];
    for (spec, what) in specs {
        for eps in [1e-5, 1e-6, 1e-12] {
            let s = ModelSpec { rms_eps: eps, ..spec.clone() };
            let read = round_trip(&s, what);
            assert_eq!(read.rms_eps, eps, "{what} at {eps:e}: the model's epsilon");
            let latent: Vec<f64> = read.layers.iter().filter_map(|l| if let Mixer::Mla(m) = &l.mixer { Some(m.latent_norm_eps) } else { None }).collect();
            assert!(!latent.is_empty(), "{what}: a latent attention block");
            assert!(latent.iter().all(|&e| e == 1e-6), "{what} at {eps:e}: the latent norms at {latent:?}");
            assert_eq!(read, s, "{what} at {eps:e}");
        }
    }
}

/// MiniMax-01's shape (`models/minimax-01.cpp`): lightning attention (four heads of 16) on every block but the last,
/// attention under a NEOX rotation of half of each head on the last; in every block a mixture of four experts, two a
/// token, the softmax's kept weights renormalized; each sublayer's residual its normed input times 3.5.
fn minimax_01() -> ModelSpec {
    let mut s = Tiny::Qwen3Moe.spec(CTX);
    s.arch = "minimax-01".to_string();
    let last = s.layers.len() - 1;
    for (l, layer) in s.layers.iter_mut().enumerate() {
        let Mixer::Attention(a) = &mut layer.mixer else { panic!("Qwen3-MoE attends") };
        a.qk_norm = false;
        a.rope.as_mut().expect("Qwen3-MoE rotates").dims = a.head_dim / 2;
        if l < last {
            layer.mixer = Mixer::Recurrent(Recurrent::Lightning(Lightning { heads: a.heads, head_dim: a.head_dim }));
        }
    }
    s.parts = vec![Part::NormedResidual { scale: 3.5 }];
    s
}

#[test]
fn a_minimax_01_checkpoint_reads_back_as_its_spec() {
    let s = minimax_01();
    assert!(s.layers.len() >= 2, "a lightning block and an attention block");
    assert_eq!(round_trip(&s, "MiniMax-01"), s, "MiniMax-01");
    // With no `attention.recurrent_layers` llama.cpp attends fully on every `full_attention_interval`-th block, every
    // eighth where that is unwritten too.
    let blocks = s.layers.len();
    let interval = read_edited(
        &s,
        |m| {
            m.retain(|(k, _)| !k.ends_with(".attention.recurrent_layers"));
            m.push(("minimax-01.full_attention_interval".into(), MetaValue::U32(blocks as u32)));
        },
        same,
        &[],
    );
    assert_eq!(interval.as_ref(), Ok(&s), "the last block attends at an interval of the stack's depth");
    let eighth = read_edited(&s, |m| m.retain(|(k, _)| !k.ends_with(".attention.recurrent_layers")), same, &[]).unwrap();
    let kinds: Vec<bool> = eighth.layers.iter().map(|l| matches!(l.mixer, Mixer::Recurrent(Recurrent::Lightning(_)))).collect();
    assert_eq!(kinds, vec![true; blocks], "short of eight blocks, every block is a lightning block");
    let all_lightning = read_edited(&s, set("minimax-01", ".attention.recurrent_layers", MetaValue::Bool(true)), same, &[]).unwrap();
    assert!(
        all_lightning.layers.iter().all(|l| matches!(l.mixer, Mixer::Recurrent(Recurrent::Lightning(_)))),
        "one flag for every block, as llama.cpp's `get_key_or_arr` reads a scalar"
    );
}

#[test]
fn a_minimax_01_checkpoint_llama_cpp_would_compute_otherwise_is_refused() {
    // Values unlike the keys (llama.cpp's graph asserts them equal), one block whose lightning decay's depth scale is
    // 0/0, a residual without its scale, and an interval of 0.
    let s = minimax_01();
    let mut one = s.clone();
    one.layers.truncate(1);
    let refusals = [
        ("beside keys of", read_edited(&s, set("minimax-01", ".attention.value_length", MetaValue::U32(8)), same, &[])),
        ("0/0", read_edited(&one, |_| {}, same, &[])),
        ("minimax-01.residual_scale", read_edited(&s, |m| m.retain(|(k, _)| !k.ends_with(".residual_scale")), same, &[])),
        (
            "full_attention_interval of 0",
            read_edited(
                &s,
                |m| {
                    m.retain(|(k, _)| !k.ends_with(".attention.recurrent_layers"));
                    m.push(("minimax-01.full_attention_interval".into(), MetaValue::U32(0)));
                },
                same,
                &[],
            ),
        ),
    ];
    for (what, read) in refusals {
        let err = read.expect_err(what);
        assert!(err.to_string().contains(what), "{what}: {err}");
    }
}

/// RWKV-7's shape (`models/rwkv7.cpp`): three blocks of the time mix (four heads of 16, low-rank pairs of 16, 16, 8 and
/// 32, the first block's value the one the others mix with) and a channel mix of 128, the embedding normed, every norm
/// a LayerNorm with its bias at 10⁻⁵, the channel mix's `attn_norm_2`.
fn rwkv7() -> ModelSpec {
    let mut s = Tiny::Llama.spec(CTX);
    s.arch = "rwkv7".to_string();
    let time = Rwkv7 { head_size: 16, decay_lora: 16, iclr_lora: 16, value_res_lora: 8, gate_lora: 32, first: false };
    for (l, layer) in s.layers.iter_mut().enumerate() {
        *layer = Layer {
            mixer: Mixer::Recurrent(Recurrent::Rwkv7(Rwkv7 { first: l == 0, ..time.clone() })),
            ffn: Ffn::ChannelMix { dff: 128 },
            pre_norms: true,
            post_mixer_norm: false,
            post_ffn_norm: false,
        };
    }
    ModelSpec { norm: Norm::Layer { bias: true }, ffn_norm: "attn_norm_2", rms_eps: 1e-5, parts: vec![Part::EmbeddingNorm], ..s }
}

#[test]
fn an_rwkv7_checkpoint_reads_back_as_its_spec() {
    let s = rwkv7();
    assert!(s.layers.len() >= 2, "a first block and a block that mixes with its value");
    assert_eq!(round_trip(&s, "RWKV-7"), s, "RWKV-7");
}

#[test]
fn an_rwkv7_checkpoint_llama_cpp_would_not_load_is_refused() {
    // llama.cpp's rwkv7 asserts two token shifts a block and heads that divide the width; its LayerNorms read an epsilon
    // its loader gives no default.
    let s = rwkv7();
    let refusals = [
        ("asserts 2", read_edited(&s, set("rwkv7", ".token_shift_count", MetaValue::U32(3)), same, &[])),
        ("heads of 24 over a width of", read_edited(&s, set("rwkv7", ".wkv.head_size", MetaValue::U32(24)), same, &[])),
        ("heads of 0 over a width of", read_edited(&s, set("rwkv7", ".wkv.head_size", MetaValue::U32(0)), same, &[])),
        ("rwkv7.attention.layer_norm_epsilon", read_edited(&s, |m| m.retain(|(k, _)| !k.ends_with(".attention.layer_norm_epsilon")), same, &[])),
        ("rwkv7.attention.gate_lora_rank", read_edited(&s, |m| m.retain(|(k, _)| !k.ends_with(".attention.gate_lora_rank")), same, &[])),
    ];
    for (what, read) in refusals {
        let err = read.expect_err(what);
        assert!(err.to_string().contains(what), "{what}: {err}");
    }
}

/// Gemma 4's blocks over `windowed` (`models/gemma4.cpp`): a windowed block four heads of 16 over `kv_local` KV heads
/// turning whole by 10⁴, a global one four heads of 32 over `kv_global` under `rope_freqs`' rotation by 10⁶; QK norms,
/// scores unscaled, sandwich norms, each block's `ffn` its own, the head tied and capped at 30, the embedding scaled.
fn gemma4_stack(windowed: &[bool], kv_local: usize, kv_global: usize, ffn: impl Fn(usize) -> Ffn, parts: Vec<Part>) -> ModelSpec {
    let base = Tiny::Gemma3.spec(CTX);
    let Mixer::Attention(a) = base.layers[0].mixer.clone() else { panic!("Gemma 3 attends") };
    let rope = |dims: usize, theta: f64, scaling: RopeScaling| Some(Rope { dims, theta, neox: true, scaling, sections: None });
    let layers = windowed
        .iter()
        .enumerate()
        .map(|(l, &w)| Layer {
            mixer: Mixer::Attention(Attention {
                heads: 4,
                kv_heads: if w { kv_local } else { kv_global },
                head_dim: if w { 16 } else { 32 },
                value_dim: if w { 16 } else { 32 },
                rope: if w { rope(16, 10000.0, RopeScaling::None) } else { rope(32, 1_000_000.0, RopeScaling::Factors) },
                qk_norm: true,
                window: w.then_some(Window::Sliding(4)),
                scale: 1.0,
                softcap: None,
                ..a.clone()
            }),
            ffn: ffn(l),
            pre_norms: true,
            post_mixer_norm: true,
            post_ffn_norm: true,
        })
        .collect();
    ModelSpec {
        arch: "gemma4".to_string(),
        layers,
        parts,
        final_logit_softcap: Some(30.0),
        embedding_scale: Some((base.d as f64).sqrt()),
        tied_output: true,
        ..base
    }
}

/// Gemma 4 shaped as its 26B-A4B: every block's GELU FFN of 96 beside four experts of 32, two a token; the global
/// blocks' one KV head reading its keys as its values; every block's output scaled.
fn gemma4() -> ModelSpec {
    let moe = Moe {
        experts: 4,
        top_k: 2,
        expert_dff: 32,
        act: Act::GeluTanh,
        gating: Gating::Softmax,
        norm_top_k: true,
        weights_scale: 1.0,
        shared_experts: 0,
        shared_dff: 0,
        shared_gate: false,
        biases: false,
        groups: None,
        latent: None,
    };
    let windowed = [true, true, false, true, true, false];
    gemma4_stack(
        &windowed,
        2,
        1,
        |_| Ffn::DenseAndMoe { dff: 96, act: Act::GeluTanh, moe: moe.clone() },
        vec![Part::ValueNorm, Part::KeyAsValue { layers: vec![2, 5] }, Part::LayerScale],
    )
}

/// Gemma 4 shaped as its E2B and E4B: per-layer inputs of 16, blocks 4 and 5 attending the caches of blocks 2 and 3 with
/// FFNs twice as wide, one KV head everywhere.
fn gemma4_e() -> ModelSpec {
    let windowed = [true, true, true, false, true, false];
    gemma4_stack(
        &windowed,
        1,
        1,
        |l| Ffn::Dense { dff: if l >= 4 { 192 } else { 96 }, act: Act::GeluTanh },
        vec![Part::PerLayerInput { dim: 16 }, Part::SharedKv { from: 4 }, Part::ValueNorm, Part::LayerScale],
    )
}

#[test]
fn a_gemma4_checkpoint_reads_back_as_its_spec() {
    for (s, what) in [(gemma4(), "Gemma 4 26B-A4B"), (gemma4_e(), "Gemma 4 E2B")] {
        assert_eq!(round_trip(&s, what), s, "{what}");
    }
}

#[test]
fn a_gemma4_checkpoint_llama_cpp_would_not_load_is_refused() {
    // llama.cpp's gemma4 requires values as wide as keys, a window pattern of a block each, and the two blocks before the
    // shared ones a windowed block and a global one.
    let (s, e) = (gemma4(), gemma4_e());
    let refusals = [
        ("values unlike the keys", read_edited(&s, set("gemma4", ".attention.value_length_swa", MetaValue::U32(8)), same, &[])),
        (
            "a window pattern of 5",
            read_edited(
                &s,
                set("gemma4", ".attention.sliding_window_pattern", MetaValue::Array { ty: 7, items: vec![MetaValue::Bool(true); 5] }),
                same,
                &[],
            ),
        ),
        ("a windowed block and a global one", read_edited(&e, set("gemma4", ".attention.shared_kv_layers", MetaValue::U32(3)), same, &[])),
        ("gemma4.embedding_length_per_layer_input", read_edited(&s, |m| m.retain(|(k, _)| !k.ends_with(".embedding_length_per_layer_input")), same, &[])),
    ];
    for (what, read) in refusals {
        let err = read.expect_err(what);
        assert!(err.to_string().contains(what), "{what}: {err}");
    }
}
