//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE REFERENCE FORWARD: A CHECKPOINT'S ARCHITECTURE READ FROM ITS FILE, ITS COMPUTATION IN f64.
//!
//! `ModelSpec::from_gguf` reads what a checkpoint computes from its own metadata; `model::reference` evaluates that
//! spec over the checkpoint's weights in `f64`. Nothing in either is written for one model: the Qwen3-8B on this
//! box is read, not described. This lock holds both to llama.cpp — the spec to the checkpoint's own numbers, and the
//! forward to llama.cpp's greedy continuation and its top-5 logits, read from `benchmarks/silicon/qwen_golden.json`
//! (written by `scripts/accel/qwen_golden.py` with llama.cpp), never copied into this file.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::quant::GgmlType;
use logicaffeine_compile::model::reference::{argmax, Activations, Reference};
use logicaffeine_compile::model::spec::{Act, Ffn, Mixer, ModelSpec, RopeScaling};
use logicaffeine_compile::model::tokenizer::Vocab;
use tiny_models::Tiny;

fn golden() -> serde_json::Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/silicon/qwen_golden.json");
    serde_json::from_str(&std::fs::read_to_string(&path).expect("qwen_golden.json")).expect("json")
}

fn ids(v: &serde_json::Value) -> Vec<u32> {
    v.as_array()
        .expect("a list of token ids")
        .iter()
        .map(|t| t.as_u64().expect("a token id") as u32)
        .collect()
}

#[test]
fn the_spec_of_the_local_qwen3_8b_is_read_from_its_metadata() {
    if !common::require_tool("qwen3-8b-checkpoint") {
        return;
    }
    let g = Gguf::open(&common::qwen3_8b_checkpoint()).expect("open");
    let spec = ModelSpec::from_gguf(&g).expect("qwen3 is described");
    assert_eq!(spec.arch, "qwen3");
    assert_eq!((spec.d, spec.vocab, spec.layers.len()), (4096, 151936, 36));
    assert!(!spec.tied_output && spec.mtp_blocks == 0);
    for layer in &spec.layers {
        let Mixer::Attention(a) = &layer.mixer else {
            panic!("every Qwen3 layer is attention")
        };
        assert_eq!((a.heads, a.kv_heads, a.head_dim, a.value_dim), (32, 8, 128, 128));
        assert!(a.qk_norm && !a.output_gate && !a.fused_qkv && !a.bias);
        let rope = a.rope.as_ref().expect("rotary");
        assert!(rope.neox && rope.dims == 128 && rope.scaling == RopeScaling::None);
        assert_eq!(rope.theta, 1.0e6);
        assert_eq!(layer.ffn, Ffn::Dense { dff: 12288, act: Act::Silu });
    }
    assert_eq!(spec.rms_eps, 1e-6);
}

/// The reference continues "The capital of France is" exactly as llama.cpp does, and ranks llama.cpp's top five
/// next tokens in llama.cpp's order with logits within 1% of its. Text goes in and comes out: the prompt is
/// tokenized and the continuation detokenized by the checkpoint's own vocabulary (`model::tokenizer`).
#[test]
fn the_reference_continues_the_prompt_as_llama_cpp_does() {
    if !common::require_tool("qwen3-8b-checkpoint") {
        return;
    }
    let gold = golden();
    let continuation = ids(&gold["generated_tokens"]);
    let g = Gguf::open(&common::qwen3_8b_checkpoint()).expect("open");
    let vocab = Vocab::from_gguf(&g).expect("the vocabulary reads");
    let prompt = vocab.tokenize(gold["prompt"].as_str().unwrap().as_bytes(), true, false).expect("tokenize");
    assert_eq!(prompt, ids(&gold["prompt_tokens"]), "the prompt tokenizes as llama.cpp tokenized it");
    let spec = ModelSpec::from_gguf(&g).expect("spec");
    let mut model = Reference::new(spec, g).expect("the reference evaluates qwen3");

    let logits = model.step(&prompt).expect("prefill");
    assert_eq!(argmax(&logits), gold["next_token_id"].as_u64().unwrap() as u32);
    let mut order: Vec<usize> = (0..logits.len()).collect();
    order.sort_by(|&a, &b| logits[b].partial_cmp(&logits[a]).unwrap());
    for (rank, entry) in gold["top5"].as_array().unwrap().iter().enumerate() {
        let (id, value) = (entry[0].as_u64().unwrap() as usize, entry[1].as_f64().unwrap());
        assert_eq!(order[rank], id, "rank {rank}: llama.cpp ranks token {id} here");
        assert!(
            (logits[id] - value).abs() <= 0.01 * value.abs(),
            "token {id}: logit {} here, {value} in llama.cpp",
            logits[id]
        );
    }

    model.reset();
    let got = model
        .generate(&prompt, continuation.len())
        .expect("generation");
    assert_eq!(got, continuation, "the greedy continuation is llama.cpp's");
    assert_eq!(
        String::from_utf8(vocab.detokenize(&got, false, false)).unwrap(),
        gold["generated_text"].as_str().unwrap(),
        "the continuation reads as llama.cpp's text"
    );
}

/// The latent's expansion halves read their activations unquantized under every precision: the key half, which takes a
/// head's unrotated query into the latent's space, and the value half, which takes the head's attended latent out. So
/// the absorbed attention computes what the expanded one computes, and a lane may run either. A tiny DeepSeek-V2 in
/// both of its conversions, every matrix `f32` but the expansion's, continues its prompt under the one- and two-word
/// precisions with the exact reference's logits bit for bit. With the latent's own projection quantized too, both
/// precisions part from the exact run, since a projection by quantized weights reads its activation quantized.
#[test]
fn a_latent_attentions_expansion_reads_its_activations_unquantized() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let run = |spec: &ModelSpec, path: &std::path::Path, mode: Activations| -> Vec<Vec<f64>> {
        let mut model = Reference::new(spec.clone(), Gguf::open(path).expect("open")).expect("the reference evaluates it").with_activations(mode);
        let mut logits = vec![model.step(&[3, 17, 41, 8, 29]).expect("prefill")];
        for _ in 0..4 {
            let next = argmax(logits.last().unwrap());
            logits.push(model.step(&[next]).expect("a step"));
        }
        logits
    };
    let expansion = |name: &str| ["attn_kv_b", "attn_k_b", "attn_v_b"].iter().any(|p| name.contains(p));
    for tiny in [Tiny::DeepSeek2, Tiny::DeepSeek2Split] {
        let spec = tiny.spec(32);
        let only = dir.path().join(format!("{}-expansion.gguf", tiny.name()));
        tiny_models::write_checkpoint_as(&spec, &only, &|name| if expansion(name) { GgmlType::Q8_0 } else { GgmlType::F32 });
        let also = dir.path().join(format!("{}-latent.gguf", tiny.name()));
        tiny_models::write_checkpoint_as(&spec, &also, &|name| {
            if expansion(name) || name.contains("attn_kv_a_mqa") {
                GgmlType::Q8_0
            } else {
                GgmlType::F32
            }
        });
        // The largest difference between two runs' logits over every step.
        let apart = |a: &[Vec<f64>], b: &[Vec<f64>]| a.iter().flatten().zip(b.iter().flatten()).map(|(x, y)| (x - y).abs()).fold(0f64, f64::max);
        let (exact, exact_also) = (run(&spec, &only, Activations::Exact), run(&spec, &also, Activations::Exact));
        for mode in [Activations::Int8, Activations::Int8TwoPass] {
            let d = apart(&run(&spec, &only, mode), &exact);
            assert!(d == 0.0, "{} under {mode:?}: the expansion's halves read their activations unquantized, yet the run parts from the exact one by {d:e}", tiny.name());
            let d = apart(&run(&spec, &also, mode), &exact_also);
            assert!(d > 0.0, "{} under {mode:?}: the latent's projection reads its activation quantized, so the run parts from the exact one", tiny.name());
        }
    }
}

/// The cache under llama.cpp's default storage: each key and value row rounded to `f32`, then to binary16, as it
/// enters the cache, and every later read taking the rounded row. The first layer's keys and values come before any
/// attention, so they are the exact run's, rounded; every element cached is a binary16; the logits move, and stay near.
/// The exact cache is the default.
#[test]
fn an_f16_cache_keeps_each_key_and_value_as_llama_cpps_default_cache_does() {
    use logicaffeine_compile::model::quant::{f16_to_f32, f32_to_f16};
    use logicaffeine_compile::model::reference::KvStorage;
    let dir = tempfile::tempdir().unwrap();
    let spec = Tiny::Llama.wide(64);
    let path = dir.path().join("llama.gguf");
    tiny_models::write_checkpoint(&spec, &path);
    let open = || Reference::new(spec.clone(), Gguf::open(&path).unwrap()).unwrap();
    let prompt = [3u32, 17, 41, 8, 29, 60];
    let (mut exact, mut f16) = (open(), open().with_kv_storage(KvStorage::F16));
    let (le, lf) = (exact.step(&prompt).unwrap(), f16.step(&prompt).unwrap());
    let round = |v: f64| f64::from(f16_to_f32(f32_to_f16(v as f32)));
    let ((ek, ev), (fk, fv)) = (exact.cached(0), f16.cached(0));
    assert_eq!(fk.len(), prompt.len());
    for (e, f) in ek.iter().chain(ev).zip(fk.iter().chain(fv)) {
        assert_eq!(f, &e.iter().map(|&x| round(x)).collect::<Vec<f64>>(), "the first layer's rows, rounded");
    }
    for l in 0..spec.layers.len() {
        let (k, v) = f16.cached(l);
        assert!(k.iter().chain(v).flatten().all(|&x| round(x) == x), "layer {l}: every element a binary16");
    }
    assert_ne!(le, lf, "the rounded cache moves the logits");
    let top = le.iter().fold(0f64, |m, v| m.max(v.abs()));
    let apart = le.iter().zip(&lf).map(|(a, b)| (a - b).abs()).fold(0f64, f64::max) / top;
    assert!(apart < 1e-2, "the rounded cache keeps the logits near: {apart:.3e}");
    let mut default = open();
    assert_eq!(default.step(&prompt).unwrap(), open().with_kv_storage(KvStorage::Exact).step(&prompt).unwrap());
    assert_eq!(open().step(&prompt).unwrap(), le, "the exact cache is the default");
}

/// A cached value whose `f32` value lies within `NEAR_ROUNDING` of a half-way point between its two binary16 neighbours,
/// in levels of its row's scale (the yardstick of an int8 rounding of the row), is a near call (`RoundingCall` of word
/// `CACHE_WORD`): a run that computes the value in `f32` a hair the other side keeps the other neighbour, as an int8
/// rounding's near call says of an activation. The calls are the
/// cache's own: within the bound, each first made at a step, none under the exact cache. Flipped, the reference keeps
/// the other neighbour for that element of that row and nothing else in the call's layer — the rows of a layer are
/// computed before any attention reads them — and the logits move.
#[test]
fn an_f16_caches_near_roundings_are_calls_a_run_may_take_the_other_way() {
    use logicaffeine_compile::model::quant::{f16_to_f32, f32_to_f16};
    use logicaffeine_compile::model::reference::{KvStorage, NearCall, CACHE_WORD, NEAR_ROUNDING};
    let dir = tempfile::tempdir().unwrap();
    let spec = Tiny::Llama.wide(64);
    let path = dir.path().join("llama.gguf");
    tiny_models::write_checkpoint(&spec, &path);
    let open = || Reference::new(spec.clone(), Gguf::open(&path).unwrap()).unwrap();
    let prompt: Vec<u32> = (0..40u32).map(|i| (i * 37 + 5) % tiny_models::VOCAB as u32).collect();
    let mut exact = open();
    exact.step(&prompt).unwrap();
    assert!(
        !exact.near_calls().iter().any(|c| matches!(c, NearCall::Rounding(r) if r.word == CACHE_WORD)),
        "the exact cache rounds nothing"
    );
    let mut kept = open().with_kv_storage(KvStorage::F16);
    let logits = kept.step(&prompt).unwrap();
    let calls: Vec<_> = kept
        .near_calls()
        .iter()
        .filter_map(|c| match c {
            NearCall::Rounding(r) if r.word == CACHE_WORD => Some(*r),
            _ => None,
        })
        .collect();
    assert!(!calls.is_empty(), "a prompt of 40 positions caches values near a half-way point");
    assert!(calls.iter().all(|c| c.margin <= NEAR_ROUNDING && c.position == prompt.len() - 1), "the calls of the prompt's step, within the bound: {calls:?}");
    let call = calls[0];
    let mut flipped = open().with_kv_storage(KvStorage::F16);
    flipped.flip(NearCall::Rounding(call));
    assert_ne!(flipped.step(&prompt).unwrap(), logits, "the other neighbour moves the logits");
    // The first layer whose cache differs differs in one element of one row, by one binary16 step.
    let layer = (0..spec.layers.len())
        .find(|&l| kept.cached(l) != flipped.cached(l))
        .expect("the flipped call's layer");
    let ((ka, va), (kb, vb)) = (kept.cached(layer), flipped.cached(layer));
    let moved: Vec<(f64, f64)> = ka.iter().chain(va).flatten().zip(kb.iter().chain(vb).flatten()).filter(|(a, b)| a != b).map(|(a, b)| (*a, *b)).collect();
    assert_eq!(moved.len(), 1, "layer {layer}: one element moves: {moved:?}");
    let (a, b) = moved[0];
    let step = |h: u16| f64::from(f16_to_f32(h));
    let ha = f32_to_f16(a as f32);
    assert!(b == step(ha.wrapping_add(1)) || b == step(ha.wrapping_sub(1)), "layer {layer}: {a} to its binary16 neighbour, not {b}");
}

/// The front door's binary16 rounding is the engine's at every boundary binary16 has: each value, each midpoint
/// between neighbours (a tie, to even), a unit either side of it, and the specials.
#[test]
fn the_front_doors_binary16_rounding_is_the_engines_at_every_boundary() {
    use logicaffeine_compile::model::quant::{f16_to_f32, f32_to_f16};
    use logicaffeine_compile::spatial::decode_engine::numerics;
    let mut checked = 0usize;
    let mut check = |x: f32| {
        assert_eq!(f32_to_f16(x), numerics::f32_to_f16(x), "{x:e} ({:08x})", x.to_bits());
        checked += 1;
    };
    for h in 0..0x7bffu16 {
        for sign in [0u16, 0x8000] {
            let (a, b) = (f16_to_f32(h | sign), f16_to_f32((h + 1) | sign));
            let mid = ((f64::from(a) + f64::from(b)) / 2.0) as f32;
            for x in [a, mid, f32::from_bits(mid.to_bits() - 1), f32::from_bits(mid.to_bits() + 1)] {
                check(x);
            }
        }
    }
    for x in [f32::INFINITY, f32::NEG_INFINITY, f32::NAN, -f32::NAN, f32::MAX, f32::MIN, f32::MIN_POSITIVE, f32::from_bits(1), 65520.0, 65519.996] {
        check(x);
    }
    assert_eq!(checked, 0x7bff * 2 * 4 + 10, "each finite binary16 below the largest, both signs, four points each, and the specials");
    for h in (0..=0x7bffu16).chain(0x8000..=0xfbff) {
        assert_eq!(f32_to_f16(f16_to_f32(h)), h, "{h:04x}: every finite binary16 round trips");
    }
}
