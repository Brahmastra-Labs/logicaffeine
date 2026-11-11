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

/// A product by a matrix in a 16-bit float format reads its activation as the matrix's own words under the quantized
/// precisions, as llama.cpp's 16-bit products and the lane's tensor cores read it: the exact precision takes the values
/// as computed; the one-word precision each value rounded to `f32` and then to the nearest bfloat16 or binary16, ties to
/// even; the two-word precision that word and the rest it leaves in `f32`, rounded so too. Every word is the numerics
/// contract's own rounding (`Keep::keep`), bit for bit — at the ties of both formats, a unit of `f32` either side of
/// them, through binary16's subnormals, past its range and among `f32`'s own subnormals. A BF16 and an F16 Llama part
/// from the exact run under one word, and under two lie nearer it.
#[test]
fn a_sixteen_bit_product_reads_its_activation_in_its_matrixs_own_words() {
    use logicaffeine_compile::model::reference::sixteen_bit_activations;
    use logicaffeine_compile::tensor_graph::numerics::Keep;
    let mut x: Vec<f64> = Vec::new();
    for base in [1.0f32, 3.0, 0.15625, 1e-3, 6.103_515_6e-5, 3e-6, 4.0e4, 65504.0, 2.5e-30, 1.2e-38] {
        for sign in [1.0f32, -1.0] {
            let bits = (base * sign).to_bits();
            // The bfloat16 tie is 0x8000 past a value whose low sixteen bits are clear, binary16's 0x1000 past one whose
            // low thirteen are.
            for d in [0u32, 1, 0x0fff, 0x1000, 0x1001, 0x1fff, 0x2000, 0x7fff, 0x8000, 0x8001, 0xffff] {
                x.push(f64::from(f32::from_bits(bits + d)));
            }
        }
    }
    x.extend([0.0, -0.0, 65519.0, 65520.0, -7e4, f64::from(f32::MAX), 1e-45, -3e-41, 0.1, -2.0 / 3.0]);
    let same = |a: f64, b: f64| a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan());
    for (ty, keep) in [(GgmlType::Bf16, Keep::Bf16), (GgmlType::F16, Keep::F16)] {
        let word = |v: f64| keep.keep(v).expect("a float keeps a value at a time");
        let exact = sixteen_bit_activations(&x, ty, Activations::Exact);
        assert!(exact.iter().zip(&x).all(|(a, b)| same(*a, *b)), "{ty:?}: the exact precision reads the values as computed");
        let one = sixteen_bit_activations(&x, ty, Activations::Int8);
        let two = sixteen_bit_activations(&x, ty, Activations::Int8TwoPass);
        for (i, &v) in x.iter().enumerate() {
            let hi = word(v);
            assert!(same(one[i], hi), "{ty:?}: {v:e} reads as the word {hi:e} under one word, not {:e}", one[i]);
            let lo = word(f64::from(v as f32 - hi as f32));
            assert!(same(two[i], hi + lo), "{ty:?}: {v:e} reads as {hi:e} and its rest {lo:e} under two words, not {:e}", two[i]);
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let prompt: Vec<u32> = (0..12u32).map(|i| (i * 37 + 5) % tiny_models::VOCAB as u32).collect();
    for ty in [GgmlType::Bf16, GgmlType::F16] {
        let spec = Tiny::Llama.spec(32);
        let path = dir.path().join(format!("llama-{}.gguf", ty.name()));
        tiny_models::write_checkpoint_in(&spec, &path, ty);
        let run = |mode: Activations| Reference::new(spec.clone(), Gguf::open(&path).unwrap()).unwrap().with_activations(mode).step(&prompt).unwrap();
        let exact = run(Activations::Exact);
        let apart = |a: Vec<f64>| a.iter().zip(&exact).map(|(x, y)| (x - y).abs()).fold(0f64, f64::max);
        let (one, two) = (apart(run(Activations::Int8)), apart(run(Activations::Int8TwoPass)));
        assert!(one > 0.0, "{ty:?}: one word rounds the activations, so the run parts from the exact one");
        assert!(two > 0.0 && two < one, "{ty:?}: two words keep more of each value: {two:e} from the exact run, against one word's {one:e}");
    }
}

/// Under the one-word precision a 16-bit product's roundings that lie near a half-way point are near calls, made once
/// an activation and measured as a cached value's are — in levels of the row's scale, its largest magnitude over 127 —
/// since a run's `f32` arithmetic errs by a share of the scale it computes at. The exact precision rounds nothing, and
/// the two-word precision makes no call: a first word rounded the other way leaves a rest the second word takes, so the
/// pair holds sixteen bits of the value either way. Flipped, a call takes the other neighbour and the logits move.
#[test]
fn a_sixteen_bit_products_near_roundings_are_calls_a_run_may_take_the_other_way() {
    use logicaffeine_compile::model::reference::{NearCall, RoundingCall, BF16_WORD, F16_WORD, NEAR_ROUNDING};
    let dir = tempfile::tempdir().unwrap();
    let prompt: Vec<u32> = (0..24u32).map(|i| (i * 37 + 5) % tiny_models::VOCAB as u32).collect();
    for (ty, word) in [(GgmlType::Bf16, BF16_WORD), (GgmlType::F16, F16_WORD)] {
        let spec = Tiny::Llama.spec(32);
        let path = dir.path().join(format!("llama-{}.gguf", ty.name()));
        tiny_models::write_checkpoint_in(&spec, &path, ty);
        let open = |mode: Activations| Reference::new(spec.clone(), Gguf::open(&path).unwrap()).unwrap().with_activations(mode);
        let calls = |r: &Reference| -> Vec<RoundingCall> {
            r.near_calls()
                .iter()
                .filter_map(|c| match c {
                    NearCall::Rounding(r) if r.word == BF16_WORD || r.word == F16_WORD => Some(*r),
                    _ => None,
                })
                .collect()
        };
        for mode in [Activations::Exact, Activations::Int8TwoPass] {
            let mut r = open(mode);
            r.step(&prompt).unwrap();
            assert!(calls(&r).is_empty(), "{ty:?} under {mode:?}: no 16-bit rounding is a call");
        }
        let mut kept = open(Activations::Int8);
        let logits = kept.step(&prompt).unwrap();
        let made = calls(&kept);
        assert!(!made.is_empty(), "{ty:?}: a prompt of 24 positions reads values near a half-way point");
        assert!(
            made.iter().all(|c| c.word == word && c.margin <= NEAR_ROUNDING && c.margin <= c.step / 2.0 && c.position == prompt.len() - 1),
            "{ty:?}: the calls are the matrix format's, of the prompt's step, within the bound: {made:?}"
        );
        let mut keys: Vec<(u64, usize)> = made.iter().map(|c| (c.activation, c.element)).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), made.len(), "{ty:?}: each value of an activation is one call, however many products read it");
        let call = *made.iter().max_by(|a, b| a.step.total_cmp(&b.step)).expect("a call");
        let mut flipped = open(Activations::Int8);
        flipped.flip(NearCall::Rounding(call));
        assert_ne!(flipped.step(&prompt).unwrap(), logits, "{ty:?}: the other neighbour moves the logits");
    }
}

/// The cache in int8 rows (`KvStorage::Int8Rows`, `Int8RowsRotated`): each head's row of keys and of values kept as
/// the meaning keeps it (`Keep::keep_row`) as it enters the cache, every later read taking the kept row. The first
/// layer's keys and values come before any attention, so each head's row is the exact run's, kept — bit for bit; every
/// head's row of every layer is codes within ±127 under one binary16 scale, its largest code 127 — turned first, under
/// the rotated scheme, whose row turned back by the transform is that; the logits move, and stay within a tenth of their
/// largest: each kept value lies within 1/254 of its row's largest, and on the tiny model those moves reach the logits
/// as some 5%.
#[test]
fn a_cache_in_int8_rows_keeps_each_heads_row_as_the_meaning_does() {
    use logicaffeine_compile::model::quant::{f16_to_f32, f32_to_f16};
    use logicaffeine_compile::model::reference::KvStorage;
    use logicaffeine_compile::tensor_graph::numerics::{hadamard, Keep};
    let dir = tempfile::tempdir().unwrap();
    let spec = Tiny::Llama.wide(64);
    let path = dir.path().join("llama.gguf");
    tiny_models::write_checkpoint(&spec, &path);
    let open = || Reference::new(spec.clone(), Gguf::open(&path).unwrap()).unwrap();
    let Mixer::Attention(a) = &spec.layers[0].mixer else { panic!("Llama attends") };
    let (hd, vd) = (a.head_dim, a.value_dim);
    let prompt = [3u32, 17, 41, 8, 29, 60];
    let mut exact = open();
    let le = exact.step(&prompt).unwrap();
    for (kv, keep) in [(KvStorage::Int8Rows, Keep::Int8Rows), (KvStorage::Int8RowsRotated, Keep::Int8RowsRotated)] {
        let mut rows = open().with_kv_storage(kv);
        let lr = rows.step(&prompt).unwrap();
        let kept = |row: &[f64], unit: usize| -> Vec<f64> { row.chunks(unit).flat_map(|h| keep.keep_row(h).expect("a head of a power of two")).collect() };
        let ((ek, ev), (rk, rv)) = (exact.cached(0), rows.cached(0));
        assert_eq!(rk.len(), prompt.len());
        for (e, r) in ek.iter().zip(rk) {
            assert_eq!(r, &kept(e, hd), "{kv:?}: the first layer's keys, each head's row kept");
        }
        for (e, r) in ev.iter().zip(rv) {
            assert_eq!(r, &kept(e, vd), "{kv:?}: the first layer's values, each head's row kept");
        }
        for l in 0..spec.layers.len() {
            let (k, v) = rows.cached(l);
            for (row, unit) in k.iter().map(|r| (r, hd)).chain(v.iter().map(|r| (r, vd))) {
                for head in row.chunks(unit) {
                    let stored = if keep == Keep::Int8RowsRotated { hadamard(head) } else { head.to_vec() };
                    let top = stored.iter().fold(0f64, |m, x| m.max(x.abs()));
                    if top == 0.0 {
                        continue;
                    }
                    let s = top / 127.0;
                    assert_eq!(f64::from(f16_to_f32(f32_to_f16(s as f32))), s, "{kv:?} layer {l}: the head's scale is a binary16");
                    assert!(stored.iter().all(|x| (x / s).fract() == 0.0 && (x / s).abs() <= 127.0), "{kv:?} layer {l}: every value a code within ±127 times the scale");
                }
            }
        }
        assert_ne!(le, lr, "{kv:?}: the kept cache moves the logits");
        let top = le.iter().fold(0f64, |m, v| m.max(v.abs()));
        let apart = le.iter().zip(&lr).map(|(a, b)| (a - b).abs()).fold(0f64, f64::max) / top;
        assert!(apart < 0.1, "{kv:?}: the kept cache keeps the logits within a tenth of their largest: {apart:.3e}");
    }
}

/// A cache in int8 rows makes two kinds of call a run may take the other way: a code whose value over its scale lies
/// within `NEAR_ROUNDING` of a half-way point, in levels (word `CACHE_WORD`), and a head's scale whose `f32` value lies
/// within it of its binary16 half-way point, in 127ths of itself (word `CACHE_SCALE_WORD`, the element the head). The
/// calls are the cache's own: within the bound, each first made at the prompt's step, none under the exact cache.
/// Flipped, a code moves one element of one head's row by one level — the turned scheme's moves that head's row alone —
/// and a scale moves one head's row to the scale's other binary16 neighbour; each moves the logits.
#[test]
fn a_cache_in_int8_rows_makes_its_codes_and_scales_calls_a_run_may_take_the_other_way() {
    use logicaffeine_compile::model::quant::{f16_to_f32, f32_to_f16};
    use logicaffeine_compile::model::reference::{KvStorage, NearCall, RoundingCall, CACHE_SCALE_WORD, CACHE_WORD, NEAR_ROUNDING};
    use logicaffeine_compile::tensor_graph::numerics::hadamard;
    let dir = tempfile::tempdir().unwrap();
    let spec = Tiny::Llama.wide(64);
    let path = dir.path().join("llama.gguf");
    tiny_models::write_checkpoint(&spec, &path);
    let open = || Reference::new(spec.clone(), Gguf::open(&path).unwrap()).unwrap();
    let Mixer::Attention(a) = &spec.layers[0].mixer else { panic!("Llama attends") };
    let hd = a.head_dim;
    let prompt: Vec<u32> = (0..40u32).map(|i| (i * 37 + 5) % tiny_models::VOCAB as u32).collect();
    let mut exact = open();
    exact.step(&prompt).unwrap();
    let cached = |c: &NearCall| matches!(c, NearCall::Rounding(r) if r.word == CACHE_WORD || r.word == CACHE_SCALE_WORD);
    assert!(!exact.near_calls().iter().any(cached), "the exact cache keeps no row in int8");
    // The heads' rows of every layer, keys then values, each `hd` wide.
    let heads = |r: &Reference| -> Vec<Vec<Vec<f64>>> {
        (0..spec.layers.len()).map(|l| { let (k, v) = r.cached(l); k.iter().chain(v).flat_map(|row| row.chunks(hd).map(<[f64]>::to_vec)).collect() }).collect()
    };
    for kv in [KvStorage::Int8Rows, KvStorage::Int8RowsRotated] {
        let turned = kv == KvStorage::Int8RowsRotated;
        let mut kept = open().with_kv_storage(kv);
        let logits = kept.step(&prompt).unwrap();
        let calls = |word: u8| -> Vec<RoundingCall> {
            kept.near_calls().iter().filter_map(|c| match c { NearCall::Rounding(r) if r.word == word => Some(*r), _ => None }).collect()
        };
        let (codes, scales) = (calls(CACHE_WORD), calls(CACHE_SCALE_WORD));
        assert!(!codes.is_empty() && !scales.is_empty(), "{kv:?}: a prompt of 40 positions keeps codes and scales near their other outcome: {} codes, {} scales", codes.len(), scales.len());
        assert!(codes.iter().chain(&scales).all(|c| c.margin <= NEAR_ROUNDING && c.position == prompt.len() - 1), "{kv:?}: the calls of the prompt's step, within the bound");
        assert!(codes.iter().all(|c| c.step == 1.0), "{kv:?}: a code's other outcome is a level away");
        let before = heads(&kept);
        for (call, what) in [(codes[0], "a code"), (scales[0], "a scale")] {
            let mut flipped = open().with_kv_storage(kv);
            flipped.flip(NearCall::Rounding(call));
            assert_ne!(flipped.step(&prompt).unwrap(), logits, "{kv:?}: {what} the other way moves the logits");
            let after = heads(&flipped);
            let layer = (0..spec.layers.len()).find(|&l| before[l] != after[l]).expect("the flipped call's layer");
            let moved: Vec<usize> = (0..before[layer].len()).filter(|&h| before[layer][h] != after[layer][h]).collect();
            assert_eq!(moved.len(), 1, "{kv:?} layer {layer}: {what} moves one head's row: {moved:?}");
            let (was, now) = (&before[layer][moved[0]], &after[layer][moved[0]]);
            let (was, now) = if turned { (hadamard(was), hadamard(now)) } else { (was.clone(), now.clone()) };
            let scale = |row: &[f64]| row.iter().fold(0f64, |m, x| m.max(x.abs())) / 127.0;
            if call.word == CACHE_WORD {
                assert_eq!(scale(&was), scale(&now), "{kv:?} layer {layer}: a code leaves its row's scale");
                let apart: Vec<f64> = was.iter().zip(&now).filter(|(a, b)| a != b).map(|(a, b)| (a - b).abs() / scale(&was)).collect();
                assert_eq!(apart, vec![1.0], "{kv:?} layer {layer}: one code moves by one level");
            } else {
                let (h, other) = (f32_to_f16(scale(&was) as f32), f32_to_f16(scale(&now) as f32));
                assert!(
                    [h.wrapping_add(1), h.wrapping_sub(1)].contains(&other),
                    "{kv:?} layer {layer}: the scale {} to its binary16 neighbour, not {}",
                    f16_to_f32(h),
                    f16_to_f32(other)
                );
            }
        }
    }
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

/// fp8 e4m3 as the model rounds to it (`model::quant`): the 256 codes decode to 0 and the 126 positive finite values
/// `m/8 · 2⁻⁶` below the smallest normal and `(1 + m/8) · 2^(e − 7)` above it to 448, their mirror images, and NaN at
/// `0x7f`/`0xff`; each finite code encodes back to itself; every value between two neighbours rounds to the nearer, a
/// value half-way to the one of even significand; past ±448, infinities included, it saturates to ±448; a NaN encodes
/// as `0x7f`.
#[test]
fn e4m3_rounds_every_f32_to_its_nearest_code_ties_to_even() {
    use logicaffeine_compile::model::quant::{e4m3_to_f32, f32_to_e4m3, E4M3_MAX};
    let values: Vec<f32> = (0u8..=0x7e).map(e4m3_to_f32).collect();
    assert!(e4m3_to_f32(0x7f).is_nan() && e4m3_to_f32(0xff).is_nan(), "0x7f and 0xff are NaN");
    assert_eq!(values[0], 0.0);
    assert_eq!(values[1], 2f32.powi(-9), "the smallest subnormal");
    assert_eq!(values[8], 2f32.powi(-6), "the smallest normal");
    assert_eq!(*values.last().unwrap(), E4M3_MAX, "the largest finite value");
    assert!(values.windows(2).all(|w| w[0] < w[1]), "the positive codes run in order of value");
    for (code, &v) in values.iter().enumerate() {
        assert_eq!(f32_to_e4m3(v), code as u8, "{v} encodes back to its code");
        assert_eq!(e4m3_to_f32(code as u8 | 0x80), -v, "code {code:#x} with its sign bit is its mirror image");
        if code > 0 {
            assert_eq!(f32_to_e4m3(-v), code as u8 | 0x80, "-{v} encodes back to its code");
        }
    }
    for code in 0..values.len() - 1 {
        let (lo, hi) = (values[code], values[code + 1]);
        let mid = (lo + hi) / 2.0;
        let even = if code % 2 == 0 { code } else { code + 1 } as u8;
        assert_eq!(f32_to_e4m3(mid), even, "{mid}, half-way between {lo} and {hi}, rounds to the even code");
        assert_eq!(f32_to_e4m3(mid.next_down()), code as u8, "just below half-way rounds down");
        assert_eq!(f32_to_e4m3(mid.next_up()), code as u8 + 1, "just above half-way rounds up");
        assert_eq!(f32_to_e4m3(-mid.next_up()), (code as u8 + 1) | 0x80, "the mirror image rounds alike");
    }
    for v in [448.0f32.next_up(), 464.0, 1e3, f32::MAX, f32::INFINITY] {
        assert_eq!(f32_to_e4m3(v), 0x7e, "{v} saturates to 448");
        assert_eq!(f32_to_e4m3(-v), 0xfe, "-{v} saturates to -448");
    }
    assert_eq!(f32_to_e4m3(f32::NAN), 0x7f, "a NaN encodes as 0x7f");
    assert_eq!(f32_to_e4m3(-0.0), 0x80, "negative zero keeps its sign");
    assert_eq!(f32_to_e4m3(f32::from_bits(1)), 0, "a subnormal f32 rounds to zero");
}

/// The cache as vLLM's `fp8_e4m3` cache keeps it at a scale of one (`KvStorage::E4m3`): each key and value row rounded
/// to `f32`, then to fp8 e4m3, as it enters the cache, and every later read taking the rounded row. The first layer's
/// keys and values come before any attention, so they are the exact run's, rounded; every element cached is an e4m3
/// value; the logits move, and stay within a few of e4m3's relative half-steps (2⁻⁴).
#[test]
fn an_e4m3_cache_keeps_each_key_and_value_as_vllms_fp8_cache_does() {
    use logicaffeine_compile::model::quant::{e4m3_to_f32, f32_to_e4m3};
    use logicaffeine_compile::model::reference::KvStorage;
    let dir = tempfile::tempdir().unwrap();
    let spec = Tiny::Llama.wide(64);
    let path = dir.path().join("llama.gguf");
    tiny_models::write_checkpoint(&spec, &path);
    let open = || Reference::new(spec.clone(), Gguf::open(&path).unwrap()).unwrap();
    let prompt = [3u32, 17, 41, 8, 29, 60];
    let (mut exact, mut fp8) = (open(), open().with_kv_storage(KvStorage::E4m3));
    let (le, lf) = (exact.step(&prompt).unwrap(), fp8.step(&prompt).unwrap());
    let round = |v: f64| f64::from(e4m3_to_f32(f32_to_e4m3(v as f32)));
    let ((ek, ev), (fk, fv)) = (exact.cached(0), fp8.cached(0));
    assert_eq!(fk.len(), prompt.len());
    for (e, f) in ek.iter().chain(ev).zip(fk.iter().chain(fv)) {
        assert_eq!(f, &e.iter().map(|&x| round(x)).collect::<Vec<f64>>(), "the first layer's rows, rounded");
    }
    for l in 0..spec.layers.len() {
        let (k, v) = fp8.cached(l);
        assert!(k.iter().chain(v).flatten().all(|&x| round(x) == x), "layer {l}: every element an e4m3 value");
    }
    assert_ne!(le, lf, "the rounded cache moves the logits");
    let top = le.iter().fold(0f64, |m, v| m.max(v.abs()));
    let apart = le.iter().zip(&lf).map(|(a, b)| (a - b).abs()).fold(0f64, f64::max) / top;
    // Each cached value moves by at most half its e4m3 step, 2⁻⁴ of it: the logits by a few of those.
    assert!(apart < 4.0 * 2f64.powi(-4), "the rounded cache keeps the logits near: {apart:.3e}");
}
