//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! qwen4_exp_ple_lock — Qwen3.8-Flash-Next's per-layer n-gram embedding (QWEN4-3, QWEN4-4, QWEN4-5 of
//! `benchmarks/PREPARE_FOR_SAMBA.md`), written by the author and computed as the reference computes it.
//!
//! On its layer, before the mixer reads the lanes: each token's n-grams hashed into rows of one table (`the ngram ids of
//! …`, the hash's token history carried), read side by side (`rows … side by side`) and projected to a key for each lane
//! and one value; each lane's gate `σ(sign(g)·√|g|)` of `g`, its normed key's agreement with its normed lane over `√d`
//! (`the lane sums of …`, `the signed root of …`); the value spread over the lanes by the gates, normed in lanes, and the
//! dilated causal convolution of that (an ordinary one over every position its taps span, the binder laying its kernel
//! out dilated, the ring carried) through the SiLU added to it.
//!
//!   • the author writes it on Qwen3.8's tiny hybrid and on a dense attention model, and a prompt that crosses an end of
//!     document equals the reference's logits and tokens prefilled whole, in two chunks, and a token at a time from a
//!     fresh state — and so do the decode steps after it;
//!   • sixty-four decode steps after the prompt equal the reference's, the dilated ring and the history carried through;
//!   • a batch's sequences each hash after their own history and convolve over their own ring, one of them joining the
//!     batch at its first token;
//!   • the embedding moves the logits, so it is computed and not dropped;
//!   • the binder lays the dilated kernel out as the reference reads it: tap `k` in row `k·ngram`, zero between;
//!   • an embedding off a hyper-connected residual is refused by name, and every per-layer program of its layer declares
//!     the token its hash reads as the step's runtime input;
//!   • a decode step lowers onto the SN50 whole, the hash on its 32-bit lanes, and computes the VM's logits on an fp32
//!     twin of the chip, within 2⁻¹⁶ of the largest; what the chip's bf16 adds is the numerics contract's (NUM-1).
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test qwen4_exp_ple_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;
#[path = "common/model_harness.rs"]
mod model_harness;
#[path = "common/spatial.rs"]
mod spatial_support;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use logicaffeine_compile::model::author::{self, RowState};
use logicaffeine_compile::model::bind::{bind, values, Binding, Source, View};
use logicaffeine_compile::model::ngram_hash::NgramHash;
use logicaffeine_compile::model::reference::{argmax, Reference, Tensors};
use logicaffeine_compile::model::spec::{GateActivation, HyperConnections, Mixer, ModelSpec, Ple};
use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::spatial::eval::{eval_choreography_every_tensor, Value};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use model_harness::{assert_close, carried, carried_at, checkpoint, filled, filled_at, filled_batch, run, run_batch, CTX};
use spatial_support::inputs_from_fills;
use std::collections::HashMap;
use tiny_models::Tiny;

/// The end of document in the tiny models' vocabulary.
const EOS: u32 = 3;
/// A prompt that crosses an end of document.
const PROMPT: &[u32] = &[5, 17, EOS, 2, 30, 9];

/// `spec` with four lanes and the n-gram embedding on `layer`: trigrams, two heads an order of four values each, a
/// convolution of four taps dilated by three, hashed as Qwen3.8-Flash-Next hashes (`model::ngram_hash`).
fn with_ple(mut spec: ModelSpec, layer: usize) -> ModelSpec {
    spec.hyper = Some(HyperConnections { lanes: 4, rank: 4 });
    let hash = NgramHash::new(spec.vocab as u64, 3, 2, 7, 1, 0, 1234, EOS);
    spec.ple = Some(Ple {
        layer,
        ngram: 3,
        heads_per_ngram: 2,
        head_dim: 4,
        conv_taps: 4,
        eos: EOS,
        multipliers: hash.multipliers().to_vec(),
        head_offsets: hash.head_offsets().to_vec(),
        head_vocab: hash.head_primes().to_vec(),
    });
    spec
}

/// Qwen3.8's hybrid at the tiny width — Gated DeltaNet gated by sigmoids — with the embedding on its second layer.
fn qwen38_shaped() -> ModelSpec {
    qwen38_over(CTX)
}

/// [`qwen38_shaped`] over a context of `ctx` positions.
fn qwen38_over(ctx: usize) -> ModelSpec {
    let mut spec = with_ple(Tiny::Qwen35.spec(ctx), 1);
    for layer in &mut spec.layers {
        if let Mixer::GatedDelta(g) = &mut layer.mixer {
            g.gate = GateActivation::Sigmoid;
        }
    }
    spec
}

fn models() -> Vec<(&'static str, ModelSpec)> {
    vec![("Qwen3.8's hybrid", qwen38_shaped()), ("dense attention, the embedding first", with_ple(Tiny::Qwen3.spec(CTX), 0))]
}

fn rows(tokens: &[u32]) -> Vec<usize> {
    tokens.iter().map(|&t| t as usize + 1).collect()
}

/// A chunk of `tokens` from position `pos0`, after the state `reference` holds, against the reference's step.
fn chunk_equals(spec: &ModelSpec, ckpt: &Tensors, reference: &mut Reference, tokens: &[u32], pos0: usize, what: &str) -> u32 {
    let caches = if pos0 == 1 { HashMap::new() } else { carried(spec, reference) };
    let want = reference.step(tokens).expect("the reference's step");
    let prog = if tokens.len() == 1 {
        author::decode_step(spec, "/w", CTX, tokens[0] as usize + 1, pos0)
    } else {
        author::prefill(spec, "/w", CTX, &rows(tokens), pos0)
    };
    let (got, next) = run(&filled(spec, &prog, ckpt, &caches), spec.vocab);
    assert_close(&got, &want, what);
    assert_eq!(next, argmax(&want), "{what}: the next token");
    next
}

#[test]
fn a_prompt_across_an_end_of_document_equals_the_reference_whole_in_chunks_and_a_token_at_a_time() {
    for (what, spec) in models() {
        author::check(&spec).unwrap_or_else(|e| panic!("{what}: the author writes the embedding: {e}"));
        let ckpt = checkpoint(&spec);
        let fresh = || Reference::new(spec.clone(), ckpt.clone()).expect("reference");
        // Whole, then three decode steps after it.
        let mut r = fresh();
        let mut next = chunk_equals(&spec, &ckpt, &mut r, PROMPT, 1, &format!("{what}: the prompt whole"));
        for k in 0..3 {
            let pos = PROMPT.len() + 1 + k;
            next = chunk_equals(&spec, &ckpt, &mut r, &[next], pos, &format!("{what}: decode step at {pos}"));
        }
        // In two chunks, the end of document in the first.
        let mut r = fresh();
        chunk_equals(&spec, &ckpt, &mut r, &PROMPT[..3], 1, &format!("{what}: the prompt's first chunk"));
        chunk_equals(&spec, &ckpt, &mut r, &PROMPT[3..], 4, &format!("{what}: the prompt's second chunk"));
        // A token at a time from a fresh state.
        let mut r = fresh();
        for (p, &t) in PROMPT.iter().enumerate() {
            chunk_equals(&spec, &ckpt, &mut r, &[t], p + 1, &format!("{what}: token {} of the prompt alone", p + 1));
        }
    }
}

/// The decode steps a cache of `ctx` positions holds after the prompt: each one's logits and token the reference's.
#[test]
fn sixty_four_decode_steps_after_the_prompt_equal_the_reference() {
    const STEPS: usize = 64;
    let ctx = PROMPT.len() + STEPS;
    let spec = qwen38_over(ctx);
    let ckpt = checkpoint(&spec);
    let mut r = Reference::new(spec.clone(), ckpt.clone()).expect("reference");
    let mut next = argmax(&r.step(PROMPT).expect("the reference's prefill"));
    for k in 0..STEPS {
        let pos = PROMPT.len() + 1 + k;
        let state = carried_at(&spec, &r, ctx);
        let want = r.step(&[next]).expect("the reference's step");
        let prog = author::decode_step(&spec, "/w", ctx, next as usize + 1, pos);
        let (got, token) = run(&filled_at(&spec, &prog, &ckpt, &state, ctx), spec.vocab);
        assert_close(&got, &want, &format!("decode step {} at position {pos}", k + 1));
        assert_eq!(token, argmax(&want), "decode step {}'s token", k + 1);
        next = token;
    }
}

#[test]
fn a_batch_hashes_and_convolves_each_sequence_after_its_own_history() {
    for (what, spec) in models() {
        let ckpt = checkpoint(&spec);
        // Two sequences under way — one past an end of document — and one joining the batch at its first token.
        let prompts: [&[u32]; 3] = [&[5, 17], &[9, EOS, 2, 30], &[]];
        let mut state = HashMap::new();
        let mut seqs = Vec::new();
        let mut want = Vec::new();
        for (b, prompt) in prompts.iter().enumerate() {
            let mut r = Reference::new(spec.clone(), ckpt.clone()).expect("reference");
            let tok = if prompt.is_empty() { 7 } else { argmax(&r.step(prompt).expect("the reference's prefill")) };
            state.extend(carried(&spec, &r).into_iter().map(|(name, v)| (format!("{name}_s{b}"), v)));
            want.push(r.step(&[tok]).expect("the reference's step"));
            seqs.push((tok as usize + 1, prompt.len() + 1));
        }
        let prog = author::decode_batch(&spec, "/w", CTX, &seqs);
        let got = run_batch(&filled_batch(&spec, &prog, &ckpt, &state, seqs.len()), spec.vocab, seqs.len());
        for (b, ((logits, next), want)) in got.iter().zip(&want).enumerate() {
            assert_close(logits, want, &format!("{what}: sequence {b} of the batch"));
            assert_eq!(*next, argmax(want), "{what}: sequence {b}'s next token");
        }
    }
}

#[test]
fn the_embedding_moves_the_logits() {
    for (what, spec) in models() {
        let ckpt = checkpoint(&spec);
        let logits = |s: &ModelSpec| Reference::new(s.clone(), ckpt.clone()).expect("reference").step(PROMPT).expect("prefill");
        let (with, without) = (logits(&spec), logits(&ModelSpec { ple: None, ..spec.clone() }));
        let scale = with.iter().fold(0.0f64, |m, x| m.max(x.abs()));
        assert!(with.iter().zip(&without).any(|(a, b)| (a - b).abs() > 1e-3 * scale), "{what}: the embedding moves the logits");
    }
}

#[test]
fn the_binder_lays_the_dilated_kernel_out_as_the_reference_reads_it() {
    let spec = qwen38_shaped();
    let ple = spec.ple.clone().unwrap();
    let width = 4 * spec.d;
    let Ok(Source::Weight(b)) = bind(&spec, &format!("Wpc{}", ple.layer)) else { panic!("the kernel binds a weight") };
    assert_eq!(b.view, View::Dilated { by: ple.ngram }, "dilated by the n-gram's length");
    let taps = (ple.conv_taps - 1) * ple.ngram + 1;
    assert_eq!(b.view.declared(ple.conv_taps, width), (taps, width), "{taps} rows of the lanes' width");
    let mut ckpt = checkpoint(&spec);
    let laid = values(&b, &mut ckpt).expect("the kernel reads");
    let stored = values(&Binding { tensor: b.tensor.clone(), view: View::Same }, &mut ckpt).unwrap();
    for row in 0..taps {
        for c in 0..width {
            let want = if row % ple.ngram == 0 { stored[c * ple.conv_taps + row / ple.ngram] } else { 0.0 };
            assert_eq!(laid[row * width + c], want, "row {row}, channel {c}: tap {} or zero", row / ple.ngram);
        }
    }
    for state in ["Ph", "Cp"] {
        assert_eq!(bind(&spec, &format!("{state}{}", ple.layer)), Ok(Source::State), "`{state}` is carried state");
    }
}

#[test]
fn an_embedding_off_the_lanes_is_refused_and_every_program_of_its_layer_reads_the_steps_token() {
    let spec = qwen38_shaped();
    let refused = author::check(&ModelSpec { hyper: None, ..spec.clone() }).expect_err("an embedding without lanes");
    assert!(refused.to_string().contains("n-gram embedding"), "the refusal names the embedding: {refused}");
    let l = spec.ple.as_ref().unwrap().layer;
    let reads = |src: &str, decl: &str, hash: &str, what: &str| {
        assert!(src.lines().any(|line| line == decl), "{what} declares `{decl}`:\n{src}");
        assert!(src.contains(&format!("the ngram ids of {hash} after")), "{what} hashes `{hash}`");
    };
    reads(&author::layer(&spec, l, "/w").0, "Let tok be 1.", "tok", "the forward's layer");
    reads(&author::decode_layer(&spec, l, "/w", CTX, 3).0, "Let tok be 1.", "tok", "a decode step's layer");
    let prefill = author::stacked_layer(&spec, l, "/w", CTX, &[3, 4], RowState::Shared).0;
    reads(&prefill, "Let toks be a 1 by 2 tensor of Int.", "toks", "a prefill chunk's layer");
    let batch = author::stacked_layer(&spec, l, "/w", CTX, &[3, 4], RowState::PerSequence).0;
    for b in 0..2 {
        reads(&batch, &format!("Let tok_s{b} be 1."), &format!("tok_s{b}"), &format!("a batch's layer, sequence {b}"));
    }
    // The decode step's own token, and no other declaration of it.
    let step = author::decode_step(&spec, "/w", CTX, 6, 2).0;
    assert_eq!(step.lines().filter(|line| line.starts_with("Let tok be")).count(), 1, "the decode step declares its token once");
    assert!(step.contains("the ngram ids of tok after"), "the decode step hashes its token");
}

/// A chip with fp32 in place of bf16 on the matmul, pointwise and row units: every value of the step is fp32.
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

fn logits_on(src: &str, target: &SpatialTarget) -> Vec<f64> {
    let prog = compile_program_to_spatial(src, target).expect("a tensor program").into_whole().unwrap_or_else(|why| panic!("{why}"));
    let out = eval_choreography_every_tensor(&prog, &inputs_from_fills(&prog)).expect("the program evaluates");
    let logits = prog.tensors.iter().find(|t| t.name == "logits").expect("the logits").id;
    match out.tensors.get(&logits).expect("the evaluation hands back the logits") {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    }
}

#[test]
fn the_sn50_computes_a_decode_step_with_the_embedding() {
    for (what, spec) in models() {
        let ckpt = checkpoint(&spec);
        let mut reference = Reference::new(spec.clone(), ckpt.clone()).expect("reference");
        let tok = argmax(&reference.step(PROMPT).expect("reference prefill"));
        let caches = carried(&spec, &reference);
        let src = filled(&spec, &author::decode_step(&spec, "/w", CTX, tok as usize + 1, PROMPT.len() + 1), &ckpt, &caches);
        let (vm, _) = run(&src, spec.vocab);
        let scale = vm.iter().fold(0.0f64, |m, x| m.max(x.abs())).max(1e-9);
        let chip = logits_on(&src, &fp32_twin());
        assert_eq!(chip.len(), vm.len(), "{what}: the logits' count");
        for (k, (c, v)) in chip.iter().zip(&vm).enumerate() {
            assert!((c - v).abs() <= scale / 65536.0, "{what}: logit {k} is {c} on the SN50's fp32 twin, the VM's {v}");
        }
    }
}
