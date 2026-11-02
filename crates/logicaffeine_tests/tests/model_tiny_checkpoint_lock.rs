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
use logicaffeine_compile::model::spec::{Act, Ffn, Gating, Mixer, ModelSpec};
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
            Ffn::Dense { .. } => panic!("Qwen3-MoE's block {i} is dense"),
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
