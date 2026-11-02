//! The decode engine's model: a hybrid Gated-DeltaNet checkpoint run in the engine's arithmetic.
//!
//! `spatial::decode_engine::model` runs a `qwen35` checkpoint over a batch of sequences with every number from
//! `decode_engine::numerics`. These tests hold it to the f64 reference (`model::reference`) at every knob point within
//! the bound that point earns, and to its own determinism bit for bit: a sequence's logits do not depend on its batch
//! mates, and a prompt run in one step is the prompt run a token at a time. On the real checkpoints — Qwen3.5-0.8B and
//! Qwen3.8-27B — each knob point is held to llama.cpp's own runs of a corpus (scripts/run-model-bars-tests.sh).

#![cfg(feature = "codegen")]

mod common;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::path::{Path, PathBuf};

use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::quant::GgmlType;
use logicaffeine_compile::model::reference::{Activations, Reference};
use logicaffeine_compile::model::spec::ModelSpec;
use logicaffeine_compile::spatial::decode_engine::model::{argmax, CacheFormat, EngineModel, EngineNumerics, StateFormat, FLUSH_TOKENS, TRAILING_TOKENS};
use logicaffeine_compile::spatial::decode_engine::numerics::{FpUnit, Words};
use logicaffeine_compile::spatial::serving::{AttentionMath, StateFlush};
use tiny_models::Tiny;

const PROMPT: [u32; 6] = [3, 17, 41, 8, 29, 60];
const STEPS: usize = 10;

fn checkpoint(dir: &Path, name: &str, format_of: &dyn Fn(&str) -> GgmlType) -> (ModelSpec, PathBuf) {
    let spec = Tiny::Qwen35.wide(64);
    let path = dir.join(format!("{name}.gguf"));
    tiny_models::write_checkpoint_as(&spec, &path, format_of);
    (spec, path)
}

fn q8(dir: &Path) -> (ModelSpec, PathBuf) {
    let spec = Tiny::Qwen35.wide(64);
    let path = dir.join("qwen35-q8.gguf");
    tiny_models::write_checkpoint(&spec, &path);
    (spec, path)
}

/// The reference's logits along its own greedy continuation.
fn reference_run(spec: &ModelSpec, path: &Path) -> (Vec<u32>, Vec<Vec<f64>>) {
    let mut r = Reference::new(spec.clone(), Gguf::open(path).expect("open")).expect("the reference runs qwen35");
    let mut logits = vec![r.step(&PROMPT).expect("prefill")];
    let mut tokens = Vec::new();
    for _ in 0..STEPS {
        let t = logits.last().map(|l| logicaffeine_compile::model::reference::argmax(l)).unwrap();
        tokens.push(t);
        logits.push(r.step(&[t]).expect("a step"));
    }
    (tokens, logits)
}

/// The engine's logits fed the reference's tokens, so both see the same positions.
fn engine_run(spec: &ModelSpec, path: &Path, numerics: EngineNumerics, tokens: &[u32]) -> Vec<Vec<f32>> {
    let mut e = EngineModel::new(spec.clone(), Gguf::open(path).expect("open"), numerics).expect("the engine runs qwen35");
    let s = e.sequence();
    let mut logits = vec![e.step(&[(s, PROMPT.to_vec())]).expect("prefill").remove(0).pop().unwrap()];
    for &t in tokens {
        logits.push(e.step(&[(s, vec![t])]).expect("a step").remove(0).pop().unwrap());
    }
    logits
}

/// The largest difference of two runs' logits, relative to the reference's largest magnitude.
fn apart(engine: &[Vec<f32>], reference: &[Vec<f64>]) -> f64 {
    let top = reference.iter().flatten().fold(0f64, |m, v| m.max(v.abs()));
    engine.iter().flatten().zip(reference.iter().flatten()).map(|(&e, &r)| (e as f64 - r).abs()).fold(0f64, f64::max) / top
}

/// The reference's logits under llama.cpp's activation quantization for the same word count, along the same tokens.
fn reference_quantized(spec: &ModelSpec, path: &Path, words: Words, tokens: &[u32]) -> Vec<Vec<f64>> {
    let mode = match words {
        Words::One => Activations::Int8,
        Words::Two => Activations::Int8TwoPass,
    };
    let mut r = Reference::new(spec.clone(), Gguf::open(path).expect("open")).unwrap().with_activations(mode);
    let mut logits = vec![r.step(&PROMPT).unwrap()];
    for &t in tokens {
        logits.push(r.step(&[t]).unwrap());
    }
    logits
}

/// What each stored format adds to a knob point's distance from the reference on the tiny model: the f16 state's
/// and cache's 11-bit significands, the int8 cache's 7 bits, the array's quantized queries and probabilities. Under
/// one word the activations' own error compounds with them, so they earn more room there.
fn storage_allowance(n: EngineNumerics) -> f64 {
    let (state, f16_cache, int8_vector, int8_array) = match n.activations {
        Words::Two => (1e-3, 3e-4, 2e-3, 2.5e-3),
        Words::One => (2e-3, 1e-3, 1e-2, 2e-2),
    };
    let state = match n.state {
        StateFormat::F32 => 0.0,
        StateFormat::F16 => state,
    };
    let cache = match (n.cache, n.attention) {
        (CacheFormat::F16, _) => f16_cache,
        (CacheFormat::Int8 | CacheFormat::Int8Hadamard, AttentionMath::Vector) => int8_vector,
        (CacheFormat::Int8 | CacheFormat::Int8Hadamard, AttentionMath::Array) => int8_array,
    };
    // The lazy memory reads each token through its log instead of the rewritten memory: the same updates, summed in
    // another order.
    let lazy = match n.flush {
        StateFlush::EveryPass => 0.0,
        StateFlush::Every { .. } | StateFlush::Trailing { .. } => 3e-4,
    };
    state + cache + lazy
}

/// Every knob point is held to the bar it earns: no farther from the exact reference than 1.25 times what
/// llama.cpp's activation quantization of the same word count costs the same model, plus what its stored formats add.
#[test]
fn every_knob_point_runs_within_its_bound_of_the_reference() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    let (tokens, reference) = reference_run(&spec, &path);
    let competitor = |w: Words| apart_f64(&reference_quantized(&spec, &path, w, &tokens), &reference);
    let (one, two) = (competitor(Words::One), competitor(Words::Two));
    let mut rows = Vec::new();
    for n in EngineNumerics::all() {
        let d = apart(&engine_run(&spec, &path, n, &tokens), &reference);
        let bar = 1.25 * match n.activations {
            Words::One => one,
            Words::Two => two,
        } + storage_allowance(n);
        rows.push((n, d, bar));
    }
    let report: Vec<String> = rows.iter().map(|(n, d, b)| format!("{n:?}: {d:.3e} (bar {b:.3e})")).collect();
    eprintln!("llama.cpp one word {one:.3e}, two words {two:.3e}\n{}", report.join("\n"));
    for (n, d, b) in &rows {
        assert!(d <= b, "{n:?} parts from the reference by {d:.3e}, past its bar {b:.3e}\n{}", report.join("\n"));
    }
    // The precise point earns more than the bar: it is nearer than llama.cpp's own two-word quantization.
    let precise = rows.iter().find(|r| r.0 == EngineNumerics::precise()).unwrap().1;
    assert!(precise < two, "the precise point {precise:.3e} is not nearer than llama.cpp's two words {two:.3e}");
}

/// A Llama turned by its checkpoint's frequency factors (`rope_freqs.weight`, as Llama 3's are) turns as the reference
/// turns it: at the precise point it parts from the reference by a hundredth of what the same checkpoint turned
/// without its factors does.
#[test]
fn a_llama_turned_by_its_checkpoints_frequency_factors_turns_as_the_reference_turns_it() {
    let dir = tempfile::tempdir().unwrap();
    let factored = tiny_models::with_rope_factors(Tiny::Llama.wide(64));
    let path = dir.path().join("llama3.gguf");
    tiny_models::write_checkpoint(&factored, &path);
    let (tokens, reference) = reference_run(&factored, &path);
    let mut unscaled = factored.clone();
    for layer in &mut unscaled.layers {
        if let logicaffeine_compile::model::spec::Mixer::Attention(a) = &mut layer.mixer {
            a.rope.as_mut().unwrap().scaling = logicaffeine_compile::model::spec::RopeScaling::None;
        }
    }
    let on = apart(&engine_run(&factored, &path, EngineNumerics::precise(), &tokens), &reference);
    let off = apart(&engine_run(&unscaled, &path, EngineNumerics::precise(), &tokens), &reference);
    eprintln!("turned by its factors {on:.3e}, without them {off:.3e}");
    assert!(off > 100.0 * on, "turned by its factors {on:.3e}, without them {off:.3e}");
}

fn apart_f64(a: &[Vec<f64>], reference: &[Vec<f64>]) -> f64 {
    let top = reference.iter().flatten().fold(0f64, |m, v| m.max(v.abs()));
    a.iter().flatten().zip(reference.iter().flatten()).map(|(&x, &r)| (x - r).abs()).fold(0f64, f64::max) / top
}

#[test]
fn the_precise_point_continues_as_the_reference_does() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    let (tokens, _) = reference_run(&spec, &path);
    let mut e = EngineModel::new(spec, Gguf::open(&path).unwrap(), EngineNumerics::precise()).unwrap();
    let s = e.sequence();
    assert_eq!(e.generate(s, &PROMPT, STEPS).unwrap(), tokens, "the engine's greedy continuation is the reference's");
}

/// A sequence's numbers are its own: run beside another, in either order, or alone, its logits are the same bits.
#[test]
fn a_sequences_logits_do_not_depend_on_its_batch_mates() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    for n in [EngineNumerics::precise(), EngineNumerics::fastest()] {
        let alone = |prompt: &[u32]| {
            let mut e = EngineModel::new(spec.clone(), Gguf::open(&path).unwrap(), n).unwrap();
            let s = e.sequence();
            let first = e.step(&[(s, prompt.to_vec())]).unwrap().remove(0);
            let second = e.step(&[(s, vec![7])]).unwrap().remove(0);
            (first, second)
        };
        let (a, b) = (alone(&PROMPT), alone(&[50, 2, 77]));
        let mut e = EngineModel::new(spec.clone(), Gguf::open(&path).unwrap(), n).unwrap();
        let (sa, sb) = (e.sequence(), e.sequence());
        let mut first = e.step(&[(sb, vec![50, 2, 77]), (sa, PROMPT.to_vec())]).unwrap();
        let mut second = e.step(&[(sa, vec![7]), (sb, vec![7])]).unwrap();
        let bits = |rows: &[Vec<f32>]| rows.iter().flatten().map(|v| v.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&first.remove(1)), bits(&a.0), "{n:?}: A's prompt beside B");
        assert_eq!(bits(&first.remove(0)), bits(&b.0), "{n:?}: B's prompt beside A");
        assert_eq!(bits(&second.remove(0)), bits(&a.1), "{n:?}: A's next step");
        assert_eq!(bits(&second.remove(0)), bits(&b.1), "{n:?}: B's next step");
    }
}

/// A prompt run in one step leaves the logits and the state a run a token at a time leaves, bit for bit.
#[test]
fn a_prompt_in_one_step_is_the_prompt_a_token_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    for n in EngineNumerics::all() {
        let mut whole = EngineModel::new(spec.clone(), Gguf::open(&path).unwrap(), n).unwrap();
        let s = whole.sequence();
        let all = whole.step(&[(s, PROMPT.to_vec())]).unwrap().remove(0);
        let after = whole.step(&[(s, vec![11])]).unwrap().remove(0);
        let mut single = EngineModel::new(spec.clone(), Gguf::open(&path).unwrap(), n).unwrap();
        let t = single.sequence();
        let one: Vec<Vec<f32>> = PROMPT.iter().map(|&p| single.step(&[(t, vec![p])]).unwrap().remove(0).remove(0)).collect();
        let after_one = single.step(&[(t, vec![11])]).unwrap().remove(0);
        let bits = |rows: &[Vec<f32>]| rows.iter().flatten().map(|v| v.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&all), bits(&one), "{n:?}: the prompt's logits");
        assert_eq!(bits(&after), bits(&after_one), "{n:?}: the state it leaves");
    }
}

/// Every block format the array reads, in the layout a K-quant mixture uses: the engine follows the reference.
#[test]
fn every_block_format_runs_within_its_bound_of_the_reference() {
    let dir = tempfile::tempdir().unwrap();
    for ty in [GgmlType::Q4_0, GgmlType::Q4_1, GgmlType::Q5_0, GgmlType::Q5_1, GgmlType::Q2K, GgmlType::Q3K, GgmlType::Q4K, GgmlType::Q5K, GgmlType::Q6K] {
        let (spec, path) = checkpoint(dir.path(), ty.name(), &|name| {
            let matrix = name.ends_with(".weight")
                && ["token_embd", "output.weight", "attn_q.", "attn_k.", "attn_v.", "attn_output", "attn_qkv", "attn_gate", "ssm_out", "ffn_gate.", "ffn_up.", "ffn_down."]
                    .iter()
                    .any(|p| name.contains(p));
            if matrix {
                ty
            } else {
                GgmlType::F32
            }
        });
        let (tokens, reference) = reference_run(&spec, &path);
        let n = EngineNumerics::precise();
        let d = apart(&engine_run(&spec, &path, n, &tokens), &reference);
        let two = apart_f64(&reference_quantized(&spec, &path, Words::Two, &tokens), &reference);
        let bar = 1.25 * two + storage_allowance(n);
        assert!(d <= bar, "{}: {d:.3e} past its bar {bar:.3e} (llama.cpp two words {two:.3e})", ty.name());
    }
}

/// The IQ4 formats, whose nibbles index a non-linear table, in the same layout: the engine follows the reference.
#[test]
fn the_iq4_formats_run_within_their_bound_of_the_reference() {
    let dir = tempfile::tempdir().unwrap();
    for ty in [GgmlType::Iq4Nl, GgmlType::Iq4Xs] {
        let (spec, path) = checkpoint(dir.path(), ty.name(), &|name| {
            let matrix = name.ends_with(".weight")
                && ["token_embd", "output.weight", "attn_q.", "attn_k.", "attn_v.", "attn_output", "attn_qkv", "attn_gate", "ssm_out", "ffn_gate.", "ffn_up.", "ffn_down."]
                    .iter()
                    .any(|p| name.contains(p));
            if matrix {
                ty
            } else {
                GgmlType::F32
            }
        });
        let (tokens, reference) = reference_run(&spec, &path);
        for n in [EngineNumerics::precise(), EngineNumerics { activations: Words::Two, ..EngineNumerics::fastest() }] {
            let d = apart(&engine_run(&spec, &path, n, &tokens), &reference);
            let two = apart_f64(&reference_quantized(&spec, &path, Words::Two, &tokens), &reference);
            let bar = 1.25 * two + storage_allowance(n);
            assert!(d <= bar, "{} {n:?}: {d:.3e} past its bar {bar:.3e} (llama.cpp two words {two:.3e})", ty.name());
        }
    }
}

/// The knob points a family is held to follow whether its attention norms its keys: a model whose every attention layer
/// norms them (the Qwens) keeps the standard points exactly; one with any layer that does not (Llama) holds its int8
/// caches rotated, the rotation spreading the outlier channels un-normed keys carry, and its fastest point's attention on
/// the vector units: the points Llama-3.2-1B met its bars at against llama.cpp's runs of its real checkpoint.
#[test]
fn the_knob_points_a_family_is_held_to_follow_whether_its_keys_are_normed() {
    use logicaffeine_compile::model::spec::Mixer;
    use logicaffeine_compile::spatial::decode_engine::numerics::ActScaling;
    for tiny in [Tiny::Qwen35, Tiny::Qwen3] {
        let spec = tiny.wide(64);
        assert_eq!(EngineNumerics::real_points_for(&spec), EngineNumerics::real_points(), "{}", tiny.name());
        assert_eq!(EngineNumerics::fastest_for(&spec), EngineNumerics::fastest(), "{}", tiny.name());
    }
    let unnormed_fastest = EngineNumerics {
        activations: Words::One,
        scaling: ActScaling::PerBlock,
        state: StateFormat::F16,
        cache: CacheFormat::Int8Hadamard,
        attention: AttentionMath::Vector,
        flush: StateFlush::Every { tokens: FLUSH_TOKENS },
        fp_unit: FpUnit::Soft,
    };
    let real = EngineNumerics::real_points();
    let unnormed = [real[0], EngineNumerics { cache: CacheFormat::Int8Hadamard, ..real[1] }, real[2], unnormed_fastest];
    let mut one_layer = Tiny::Qwen3.wide(64);
    if let Mixer::Attention(a) = &mut one_layer.layers[0].mixer {
        a.qk_norm = false;
    }
    for (name, spec) in [("llama", Tiny::Llama.wide(64)), ("llama-tied", Tiny::LlamaTied.wide(64)), ("one layer un-normed", one_layer)] {
        assert_eq!(EngineNumerics::real_points_for(&spec), unnormed, "{name}");
        assert_eq!(EngineNumerics::fastest_for(&spec), unnormed_fastest, "{name}");
    }
    assert_eq!(unnormed[1], EngineNumerics { activations: Words::Two, scaling: ActScaling::Sub32, state: StateFormat::F16, cache: CacheFormat::Int8Hadamard, attention: AttentionMath::Array, flush: StateFlush::EveryPass, fp_unit: FpUnit::Soft });
}

#[test]
fn what_the_engine_does_not_run_yet_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    for tiny in [Tiny::DeepSeek2, Tiny::Qwen3Moe, Tiny::Gemma3, Tiny::GptOss] {
        let spec = tiny.wide(64);
        let path = dir.path().join(format!("{}.gguf", tiny.name()));
        tiny_models::write_checkpoint(&spec, &path);
        match EngineModel::new(spec, Gguf::open(&path).unwrap(), EngineNumerics::precise()) {
            Err(logicaffeine_compile::spatial::decode_engine::model::EngineError::NotYet(what)) => assert!(!what.is_empty()),
            Err(other) => panic!("{}: refused for the wrong reason: {other:?}", tiny.name()),
            Ok(_) => panic!("{} has features the engine does not run, yet it was accepted", tiny.name()),
        }
    }
    let _ = argmax;
}

// ── Real checkpoints, held to llama.cpp's runs of them ─────────────────────────────────────────────

struct Golden {
    tokens: Vec<u32>,
    top: Vec<(u32, f64)>,
    continuation: Vec<u32>,
}

fn goldens_at(path: &Path) -> Vec<Golden> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).expect("the golden")).expect("json");
    let ids = |v: &serde_json::Value| v.as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as u32).collect::<Vec<_>>();
    v["prompts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| Golden {
            tokens: ids(&p["tokens"]),
            top: p["top"].as_array().unwrap().iter().map(|e| (e[0].as_u64().unwrap() as u32, e[1].as_f64().unwrap())).collect(),
            continuation: ids(&p["continuation"]),
        })
        .collect()
}

fn goldens(model: &str, which: &str) -> Vec<Golden> {
    goldens_at(&Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../benchmarks/models/{model}/golden-{which}.json")))
}

fn agreed(a: &[u32], b: &[u32]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

fn mean_and_variance(xs: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    let m = xs.iter().sum::<f64>() / n;
    (m, xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0) / n)
}

/// llama.cpp's runs of a corpus: the `f32` run, the oracle; how far the shipped run follows its greedy continuation,
/// summed over the corpus; and the shipped run's top-logit gap from it after each prompt.
struct Corpus {
    f32g: Vec<Golden>,
    theirs: usize,
    shipped_gap: Vec<f64>,
}

impl Corpus {
    fn of(f32g: Vec<Golden>, cpu: &[Golden]) -> Corpus {
        assert_eq!(f32g.len(), cpu.len(), "the two runs are of one corpus");
        let theirs = f32g.iter().zip(cpu).map(|(f, c)| agreed(&c.continuation, &f.continuation)).sum();
        let shipped_gap = f32g
            .iter()
            .zip(cpu)
            .map(|(f, c)| {
                assert_eq!(f.tokens, c.tokens, "the two runs read the same prompt");
                let at: std::collections::HashMap<u32, f64> = c.top.iter().copied().collect();
                f.top.iter().filter_map(|(id, v)| at.get(id).map(|s| (s - v).abs())).fold(0f64, f64::max)
            })
            .collect();
        Corpus { f32g, theirs, shipped_gap }
    }
}

/// One knob point on a real checkpoint, held to the bar the CUDA lane is held to against llama.cpp's runs of the
/// corpus: a two-word point follows llama.cpp's `f32` run, summed over the corpus, at least as far as llama.cpp's
/// shipped run does; a one-word point (llama.cpp's own activation precision) keeps its logits after the prompt no
/// further from the `f32` run's top five, on average, than the shipped run's, beyond two standard errors.
///
/// The corpus runs as one batch, as the card runs it: every prompt its own sequence, the prompts in one step, then a
/// token a step for each sequence still following the `f32` run, until its greedy token parts from the run's or the
/// run ends. A sequence's logits do not depend on its batch mates (locked above), so each is the run it makes alone.
fn meets_its_bar(spec: &ModelSpec, ckpt: &Path, n: EngineNumerics, corpus: &Corpus, what: &str) {
    let started = std::time::Instant::now();
    let mut e = EngineModel::new(spec.clone(), Gguf::open(ckpt).unwrap(), n).unwrap();
    let f32g = &corpus.f32g;
    let seqs: Vec<usize> = f32g.iter().map(|_| e.sequence()).collect();
    let prompts: Vec<(usize, Vec<u32>)> = seqs.iter().zip(f32g).map(|(&s, f)| (s, f.tokens.clone())).collect();
    let mut last: Vec<Vec<f32>> = e.step(&prompts).unwrap().into_iter().map(|mut l| l.pop().unwrap()).collect();
    let gap: Vec<f64> = f32g
        .iter()
        .zip(&last)
        .map(|(f, first)| f.top.iter().map(|&(id, v)| (first[id as usize] as f64 - v).abs()).fold(0f64, f64::max))
        .collect();
    let mut ours = 0;
    let mut following: Vec<usize> = (0..f32g.len()).collect();
    let mut i = 0;
    while !following.is_empty() {
        let mut still = Vec::new();
        let mut rows = Vec::new();
        for &p in &following {
            let run = &f32g[p].continuation;
            if i >= run.len() {
                continue;
            }
            let t = argmax(&last[p]);
            if t != run[i] {
                continue;
            }
            ours += 1;
            if i + 1 < run.len() {
                still.push(p);
                rows.push((seqs[p], vec![t]));
            }
        }
        if !rows.is_empty() {
            for (&p, mut l) in still.iter().zip(e.step(&rows).unwrap()) {
                last[p] = l.pop().unwrap();
            }
        }
        following = still;
        i += 1;
    }
    let ((m_ours, v_ours), (m_theirs, v_theirs)) = (mean_and_variance(&gap), mean_and_variance(&corpus.shipped_gap));
    let theirs = corpus.theirs;
    eprintln!(
        "{what}: follows the f32 run {ours} steps (llama.cpp shipped {theirs}); mean top-logit gap {m_ours:.4} (shipped {m_theirs:.4}); {:.0} s",
        started.elapsed().as_secs_f64()
    );
    match n.activations {
        Words::One => assert!(
            m_ours <= m_theirs + 2.0 * (v_ours + v_theirs).sqrt(),
            "{what}: the mean top-logit gap {m_ours:.4} is past llama.cpp's {m_theirs:.4} by more than two standard errors"
        ),
        Words::Two => assert!(ours >= theirs, "{what}: follows the f32 run {ours} steps, llama.cpp's shipped run {theirs}"),
    }
}

/// Every knob point of [`EngineNumerics::real_points_for`] the checkpoint's family on a checkpoint under `models/`, its
/// products by the multiplier `unit`, held to its bar over llama.cpp's runs of the corpus in `benchmarks/models/<model>/`.
/// Minutes a knob point, they run on purpose through scripts/run-model-bars-tests.sh rather than in the routine baseline.
fn meets_its_bar_over(model: &str, corpus: &str, unit: FpUnit) {
    if !common::require_tool(&format!("checkpoint:{model}")) {
        return;
    }
    let ckpt = common::checkpoint(model).unwrap();
    let spec = ModelSpec::from_gguf(&Gguf::open(&ckpt).unwrap()).unwrap();
    let runs = Corpus::of(goldens(model, &format!("{corpus}f32")), &goldens(model, &format!("{corpus}cpu")));
    for n in EngineNumerics::real_points_for(&spec) {
        let n = EngineNumerics { fp_unit: unit, ..n };
        meets_its_bar(&spec, &ckpt, n, &runs, &format!("{model} {corpus}{n:?}"));
    }
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_5_0_8b_meets_every_knob_points_bar() {
    meets_its_bar_over("qwen3.5-0.8b", "", FpUnit::Soft);
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_5_0_8b_meets_every_knob_points_bar_on_long_prompts() {
    meets_its_bar_over("qwen3.5-0.8b", "long-", FpUnit::Soft);
}

/// The same bars with every product the card's floating-point DSP block's ([`FpUnit::Hard`]).
#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_5_0_8b_meets_every_knob_points_bar_on_the_cards_multiplier() {
    meets_its_bar_over("qwen3.5-0.8b", "", FpUnit::Hard);
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_5_0_8b_meets_every_knob_points_bar_on_long_prompts_on_the_cards_multiplier() {
    meets_its_bar_over("qwen3.5-0.8b", "long-", FpUnit::Hard);
}

/// The fastest point of a checkpoint's family with its memories flushed a window of [`TRAILING_TOKENS`] behind, a knob
/// point of its own, on a checkpoint under `models/` held to the fastest point's bar over llama.cpp's runs of the corpus.
fn trailing_point_meets_its_bar_over(model: &str, corpus: &str) {
    if !common::require_tool(&format!("checkpoint:{model}")) {
        return;
    }
    let ckpt = common::checkpoint(model).unwrap();
    let spec = ModelSpec::from_gguf(&Gguf::open(&ckpt).unwrap()).unwrap();
    let runs = Corpus::of(goldens(model, &format!("{corpus}f32")), &goldens(model, &format!("{corpus}cpu")));
    let n = EngineNumerics { flush: StateFlush::Trailing { tokens: TRAILING_TOKENS }, ..EngineNumerics::fastest_for(&spec) };
    meets_its_bar(&spec, &ckpt, n, &runs, &format!("{model} {corpus}{n:?}"));
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_5_0_8b_fastest_point_flushed_a_window_behind_meets_its_bar() {
    trailing_point_meets_its_bar_over("qwen3.5-0.8b", "");
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_5_0_8b_fastest_point_flushed_a_window_behind_meets_its_bar_on_long_prompts() {
    trailing_point_meets_its_bar_over("qwen3.5-0.8b", "long-");
}

/// Llama-3.2-1B: grouped attention on every layer, keys not QK-normed, its rotation scaled by the checkpoint's
/// frequency factors, its head tied to the embedding.
#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn llama3_2_1b_meets_every_knob_points_bar() {
    meets_its_bar_over("llama3.2-1b", "", FpUnit::Soft);
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn llama3_2_1b_meets_every_knob_points_bar_on_long_prompts() {
    meets_its_bar_over("llama3.2-1b", "long-", FpUnit::Soft);
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn llama3_2_1b_meets_every_knob_points_bar_on_the_cards_multiplier() {
    meets_its_bar_over("llama3.2-1b", "", FpUnit::Hard);
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn llama3_2_1b_meets_every_knob_points_bar_on_long_prompts_on_the_cards_multiplier() {
    meets_its_bar_over("llama3.2-1b", "long-", FpUnit::Hard);
}

/// Qwen3.8-27B's goldens: llama.cpp's runs of the checkpoint their provenance names.
const QWEN38_27B_GOLDENS: &str = "../../benchmarks/silicon/d5005/goldens/qwen3.8-27b";

/// The SHA-256 of a file, read a few megabytes at a time.
fn sha256_of(path: &Path) -> String {
    use std::io::Read as _;
    let mut f = std::fs::File::open(path).expect("open the checkpoint");
    let mut h = common::ttsim::Sha256::new();
    let mut buf = vec![0u8; 8 << 20];
    loop {
        let n = f.read(&mut buf).expect("read the checkpoint");
        if n == 0 {
            return h.hex();
        }
        h.update(&buf[..n]);
    }
}

/// The 27B checkpoint and its `corpus` ("" or "long-"), as the goldens' provenance records them: the checkpoint the
/// shipped run read, held to the SHA-256 recorded with it; the shipped run, one prompt at a time; the `f32` run of its
/// dequantized copy, the corpus decoded as one batch, which the provenance's batch-equivalence record (no continuation
/// parting, every top logit within `1e-6` of its own magnitude) makes the run of each prompt alone.
fn qwen38_27b(corpus: &str) -> Option<(ModelSpec, PathBuf, Corpus)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(QWEN38_27B_GOLDENS);
    let json = |name: &str| -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))).expect("json")
    };
    let provenance = json("provenance.json");
    let (cpu, f32) = (format!("golden-{corpus}cpu.json"), format!("golden-{corpus}f32.json"));
    let shipped = provenance["goldens"][&cpu]["checkpoint"].as_str().expect("the shipped run's checkpoint");
    let name = shipped.strip_prefix("models/").expect("a checkpoint under models/");
    if !common::require_tool(&format!("checkpoint:{name}")) {
        return None;
    }
    let ckpt = common::checkpoint(name).unwrap();
    let recorded = provenance["checkpoints"][shipped]["sha256"].as_str().expect("the checkpoint's recorded SHA-256");
    assert_eq!(sha256_of(&ckpt), recorded, "{shipped} is not the checkpoint the goldens ran on");
    let prompts = json(&f32)["prompts"].as_array().expect("the f32 run's prompts").len() as u64;
    assert_eq!(json(&f32)["batch"].as_u64(), Some(prompts), "{f32}: the f32 run decoded the corpus as one batch");
    assert_eq!(provenance["goldens"][&f32]["batch"].as_u64(), Some(prompts), "the provenance records the f32 run's batch");
    let equivalence = &provenance["batch_equivalence"][if corpus.is_empty() { "short" } else { "long" }];
    assert_eq!(equivalence["partings"].as_u64(), Some(0), "a batched continuation parts from the prompt run alone");
    assert!(equivalence["worst_rel"].as_f64().expect("the worst relative logit difference") <= 1e-6, "a batched logit past 1e-6 of its own magnitude");
    let spec = ModelSpec::from_gguf(&Gguf::open(&ckpt).unwrap()).unwrap();
    Some((spec, ckpt, Corpus::of(goldens_at(&dir.join(&f32)), &goldens_at(&dir.join(&cpu)))))
}

/// Knob point `point` of [`EngineNumerics::real_points`] on Qwen3.8-27B, its products by the multiplier `unit`, held to
/// its bar over the corpus; a knob point a test, each an hour or more.
fn qwen3_8_27b_point_meets_its_bar(point: usize, corpus: &str, unit: FpUnit) {
    let Some((spec, ckpt, runs)) = qwen38_27b(corpus) else { return };
    let n = EngineNumerics { fp_unit: unit, ..EngineNumerics::real_points()[point] };
    meets_its_bar(&spec, &ckpt, n, &runs, &format!("qwen3.8-27b {corpus}{n:?}"));
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_8_27b_precise_point_meets_its_bar() {
    qwen3_8_27b_point_meets_its_bar(0, "", FpUnit::Soft);
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_8_27b_two_word_int8_point_meets_its_bar() {
    qwen3_8_27b_point_meets_its_bar(1, "", FpUnit::Soft);
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_8_27b_one_word_point_meets_its_bar() {
    qwen3_8_27b_point_meets_its_bar(2, "", FpUnit::Soft);
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_8_27b_fastest_point_meets_its_bar() {
    qwen3_8_27b_point_meets_its_bar(3, "", FpUnit::Soft);
}

/// Qwen3.8-27B's fastest point with its memories flushed a window of [`TRAILING_TOKENS`] behind, held to the fastest
/// point's bar over the corpus.
fn qwen3_8_27b_trailing_point_meets_its_bar(corpus: &str) {
    let Some((spec, ckpt, runs)) = qwen38_27b(corpus) else { return };
    let n = EngineNumerics { flush: StateFlush::Trailing { tokens: TRAILING_TOKENS }, ..EngineNumerics::real_points()[3] };
    meets_its_bar(&spec, &ckpt, n, &runs, &format!("qwen3.8-27b {corpus}{n:?}"));
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_8_27b_fastest_point_flushed_a_window_behind_meets_its_bar() {
    qwen3_8_27b_trailing_point_meets_its_bar("");
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_8_27b_fastest_point_flushed_a_window_behind_meets_its_bar_on_long_prompts() {
    qwen3_8_27b_trailing_point_meets_its_bar("long-");
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_8_27b_precise_point_meets_its_bar_on_long_prompts() {
    qwen3_8_27b_point_meets_its_bar(0, "long-", FpUnit::Soft);
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_8_27b_two_word_int8_point_meets_its_bar_on_long_prompts() {
    qwen3_8_27b_point_meets_its_bar(1, "long-", FpUnit::Soft);
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_8_27b_one_word_point_meets_its_bar_on_long_prompts() {
    qwen3_8_27b_point_meets_its_bar(2, "long-", FpUnit::Soft);
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_8_27b_fastest_point_meets_its_bar_on_long_prompts() {
    qwen3_8_27b_point_meets_its_bar(3, "long-", FpUnit::Soft);
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_8_27b_precise_point_meets_its_bar_on_the_cards_multiplier() {
    qwen3_8_27b_point_meets_its_bar(0, "", FpUnit::Hard);
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_8_27b_precise_point_meets_its_bar_on_long_prompts_on_the_cards_multiplier() {
    qwen3_8_27b_point_meets_its_bar(0, "long-", FpUnit::Hard);
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_8_27b_two_word_int8_point_meets_its_bar_on_the_cards_multiplier() {
    qwen3_8_27b_point_meets_its_bar(1, "", FpUnit::Hard);
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_8_27b_two_word_int8_point_meets_its_bar_on_long_prompts_on_the_cards_multiplier() {
    qwen3_8_27b_point_meets_its_bar(1, "long-", FpUnit::Hard);
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_8_27b_one_word_point_meets_its_bar_on_the_cards_multiplier() {
    qwen3_8_27b_point_meets_its_bar(2, "", FpUnit::Hard);
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_8_27b_one_word_point_meets_its_bar_on_long_prompts_on_the_cards_multiplier() {
    qwen3_8_27b_point_meets_its_bar(2, "long-", FpUnit::Hard);
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_8_27b_fastest_point_meets_its_bar_on_the_cards_multiplier() {
    qwen3_8_27b_point_meets_its_bar(3, "", FpUnit::Hard);
}

#[test]
#[ignore = "a real-checkpoint bar, minutes to hours: scripts/run-model-bars-tests.sh runs it on purpose"]
fn qwen3_8_27b_fastest_point_meets_its_bar_on_long_prompts_on_the_cards_multiplier() {
    qwen3_8_27b_point_meets_its_bar(3, "long-", FpUnit::Hard);
}

/// A lazily flushed memory's flush replays the logged updates exactly as the token-by-token memory makes them, so
/// with an f32 state the memory it leaves is the token-by-token memory, bit for bit; between flushes it holds the
/// memory of its last flush and a log of the tokens since.
#[test]
fn a_flush_leaves_the_token_by_token_memory_bit_for_bit() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    let at = |flush| EngineNumerics { flush, ..EngineNumerics::precise() };
    let mut every = EngineModel::new(spec.clone(), Gguf::open(&path).unwrap(), at(StateFlush::EveryPass)).unwrap();
    let mut lazy = EngineModel::new(spec.clone(), Gguf::open(&path).unwrap(), at(StateFlush::Every { tokens: FLUSH_TOKENS })).unwrap();
    let (a, b) = (every.sequence(), lazy.sequence());
    let gdn = (0..spec.layers.len()).find(|&l| every.memory(a, l).is_some()).expect("a Gated-DeltaNet block");
    let tokens: Vec<u32> = (0..2 * FLUSH_TOKENS + 3).map(|i| (i * 7 + 3) % 90).collect();
    let mut flushes = 0;
    for (i, &t) in tokens.iter().enumerate() {
        every.step(&[(a, vec![t])]).unwrap();
        lazy.step(&[(b, vec![t])]).unwrap();
        let (_, pending) = lazy.memory(b, gdn).unwrap();
        assert_eq!(pending, (i + 1) % FLUSH_TOKENS as usize, "token {i}: the log holds the tokens since the last flush");
        if pending == 0 {
            flushes += 1;
            let bits = |m: &[f32]| m.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
            assert_eq!(bits(lazy.memory(b, gdn).unwrap().0), bits(every.memory(a, gdn).unwrap().0), "token {i}: the flushed memory");
        }
    }
    assert_eq!(flushes, 2);
}

/// A trailing flush replays each window into its share of every head's rows a token after the window: after `n` tokens,
/// `n = k·T + m` for `m` in `1..=T`, a row of the shares below `m` holds the token-by-token memory after `k·T` tokens,
/// bit for bit, and every other row the memory after `(k−1)·T` (none of it yet before the second window); the log holds
/// the tokens past the window being replayed, and that window too until its last share is replayed.
#[test]
fn a_trailing_flush_leaves_each_row_the_token_by_token_memory_a_window_later() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    let t = TRAILING_TOKENS as usize;
    let at = |flush| EngineNumerics { flush, ..EngineNumerics::precise() };
    let mut every = EngineModel::new(spec.clone(), Gguf::open(&path).unwrap(), at(StateFlush::EveryPass)).unwrap();
    let mut trailing = EngineModel::new(spec.clone(), Gguf::open(&path).unwrap(), at(StateFlush::Trailing { tokens: TRAILING_TOKENS })).unwrap();
    let (a, b) = (every.sequence(), trailing.sequence());
    let (gdn, hd) = spec
        .layers
        .iter()
        .enumerate()
        .find_map(|(l, layer)| match &layer.mixer {
            logicaffeine_compile::model::spec::Mixer::GatedDelta(g) => Some((l, g.head_dim)),
            _ => None,
        })
        .expect("a Gated-DeltaNet block");
    let bits = |m: &[f32]| m.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
    let mut windows = vec![bits(every.memory(a, gdn).unwrap().0)];
    let tokens: Vec<u32> = (0..3 * t + 3).map(|i| (i as u32 * 7 + 3) % 90).collect();
    for (i, &token) in tokens.iter().enumerate() {
        every.step(&[(a, vec![token])]).unwrap();
        trailing.step(&[(b, vec![token])]).unwrap();
        let n = i + 1;
        if n % t == 0 {
            windows.push(bits(every.memory(a, gdn).unwrap().0));
        }
        let (k, m) = ((n - 1) / t, (n - 1) % t + 1);
        let (memory, logged) = trailing.memory(b, gdn).unwrap();
        assert_eq!(logged, if n <= t { n } else { t + m % t }, "token {n}: the tokens the log holds");
        let memory = bits(memory);
        for (row, got) in memory.chunks(hd).enumerate() {
            let r = row % hd;
            let window = if k == 0 { 0 } else if r < m * hd / t { k } else { k - 1 };
            assert_eq!(got, &windows[window][row * hd..(row + 1) * hd], "token {n}: head {} row {r}, the memory after {} tokens", row / hd, window * t);
        }
    }
    assert_eq!(windows.len(), 4, "three windows replayed");
}

/// A trailing flush reads every token nearly as a flush every window does: a row it has not replayed reads through up to
/// two windows of tokens, twice the fold of a flush every `T` tokens, so its logits part from the token-by-token
/// memory's by no more than twice what that flush's do, at the precise point.
#[test]
fn a_trailing_flush_reads_as_near_the_token_by_token_memory_as_a_flush_every_window() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    let (tokens, _) = reference_run(&spec, &path);
    let tokens: Vec<u32> = tokens.iter().cycle().take(3 * TRAILING_TOKENS as usize).copied().collect();
    let run = |flush| engine_run(&spec, &path, EngineNumerics { flush, ..EngineNumerics::precise() }, &tokens);
    let eager: Vec<Vec<f64>> = run(StateFlush::EveryPass).iter().map(|l| l.iter().map(|&v| v as f64).collect()).collect();
    let every = apart(&run(StateFlush::Every { tokens: TRAILING_TOKENS }), &eager);
    let trailing = apart(&run(StateFlush::Trailing { tokens: TRAILING_TOKENS }), &eager);
    eprintln!("from the token-by-token memory: every {TRAILING_TOKENS} tokens {every:.3e}, trailing {trailing:.3e}");
    assert!(every > 0.0, "a flush every window sums the same updates in another order");
    assert!(trailing <= 2.0 * every, "a trailing flush parts from the token-by-token memory by {trailing:.3e}, past twice a flush every window's {every:.3e}");
}
