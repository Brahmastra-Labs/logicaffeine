//! The decode engine's machine: a step as the jobs its sequencer runs over the card's memory.
//!
//! `spatial::decode_engine::machine` lays a checkpoint's weight image, rotary tables and sequence regions out in one
//! memory, lowers each command of the program to one job, and runs the jobs reading and writing every value as the
//! memory stores it. These tests hold it to the engine's model bit for bit — every knob point, every block format the
//! array reads, a batch of sequences with prompts and single tokens, across a lazy flush — and its memory to its
//! layout: nothing overlaps, everything starts on a line, a step writes only the regions of the sequences it runs, and
//! a sequence's region starts empty.

#![cfg(feature = "codegen")]

mod common;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::path::{Path, PathBuf};

use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::quant::GgmlType;
use logicaffeine_compile::model::reference::WeightSource;
use logicaffeine_compile::model::bind::rope_tables;
use logicaffeine_compile::model::spec::{Mixer, ModelSpec, Rope};
use logicaffeine_compile::spatial::decode::GgufHeader;
use logicaffeine_compile::spatial::decode_engine::machine::{
    decode, encode, fold, fold_delta_norms, lower, BlockState, CachePart, DeltaNorm, Fold, Job, Layout, Load, LogFields, Machine, Order, Put, Reg,
    Sink, Source, VectorJob, LANES, LINE, SREGS, VREGS,
};
use logicaffeine_compile::spatial::decode_engine::model::{argmax, CacheFormat, EngineError, EngineModel, EngineNumerics, Replay, FLUSH_TOKENS, TRAILING_TOKENS};
use logicaffeine_compile::spatial::decode_engine::vector::Op;
use logicaffeine_compile::spatial::serving::StateFlush;
use logicaffeine_compile::spatial::decode_engine::program::Command;
use tiny_models::Tiny;

const PROMPT: [u32; 6] = [3, 17, 41, 8, 29, 60];
/// Tokens past the prompt: the prompt and these reach past a lazy flush's window, and a trailing flush's third.
const STEPS: usize = FLUSH_TOKENS as usize + 6;

/// Every way the knob points carry a Gated-DeltaNet memory: rewritten every pass, flushed every [`FLUSH_TOKENS`], and
/// flushed a window of [`TRAILING_TOKENS`] behind.
const FLUSHES: [StateFlush; 3] = [StateFlush::EveryPass, StateFlush::Every { tokens: FLUSH_TOKENS }, StateFlush::Trailing { tokens: TRAILING_TOKENS }];

/// The tiny qwen35 with its attention grouped as the 27B's is, four query heads over two cached heads, so every cache
/// row a head of its own.
fn grouped() -> ModelSpec {
    let mut spec = Tiny::Qwen35.wide(64);
    for layer in &mut spec.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            a.heads = 4;
            a.kv_heads = 2;
        }
    }
    spec
}

fn checkpoint(dir: &Path, name: &str, format_of: &dyn Fn(&str) -> GgmlType) -> (ModelSpec, PathBuf) {
    let spec = grouped();
    let path = dir.join(format!("{name}.gguf"));
    tiny_models::write_checkpoint_as(&spec, &path, format_of);
    (spec, path)
}

fn q8(dir: &Path) -> (ModelSpec, PathBuf) {
    let spec = grouped();
    let path = dir.join("qwen35-q8.gguf");
    tiny_models::write_checkpoint(&spec, &path);
    (spec, path)
}

fn bits(logits: &[f32]) -> Vec<u32> {
    logits.iter().map(|v| v.to_bits()).collect()
}

/// The prompt in one step, then the model's greedy tokens one a step, on the model and on the machine: every step's
/// logits equal bit for bit.
fn agree(spec: &ModelSpec, path: &Path, n: EngineNumerics, what: &str) {
    let mut model = EngineModel::new(spec.clone(), Gguf::open(path).unwrap(), n).unwrap();
    let mut machine = Machine::new(spec.clone(), Gguf::open(path).unwrap(), n, 1, spec.ctx_len).unwrap();
    let (s, m) = (model.sequence(), machine.sequence().unwrap());
    let want = model.step(&[(s, PROMPT.to_vec())]).unwrap().remove(0);
    let got = machine.step(&[(m, PROMPT.to_vec())]).unwrap().remove(0);
    assert_eq!(want.len(), got.len());
    for (i, (w, g)) in want.iter().zip(&got).enumerate() {
        assert_eq!(bits(w), bits(g), "{what} {n:?}: the prompt's row {i}");
    }
    let mut token = argmax(want.last().unwrap());
    for step in 0..STEPS {
        let w = model.step(&[(s, vec![token])]).unwrap().remove(0).pop().unwrap();
        let g = machine.step(&[(m, vec![token])]).unwrap().remove(0).pop().unwrap();
        assert_eq!(bits(&w), bits(&g), "{what} {n:?}: step {step}");
        token = argmax(&w);
    }
    assert_eq!(machine.position(m), model.position(s));
}

/// [`agree`] at every knob point whose memory is carried as `flush`, one of the [`FLUSHES`] every knob point's is.
fn agree_at_every_knob_point_of(flush: StateFlush) {
    assert!(EngineNumerics::all().iter().all(|n| FLUSHES.contains(&n.flush)), "every knob point's memory carried as one of {FLUSHES:?}");
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    let points: Vec<EngineNumerics> = EngineNumerics::all().into_iter().filter(|n| n.flush == flush).collect();
    assert!(!points.is_empty(), "knob points carried as {flush:?}");
    for n in points {
        agree(&spec, &path, n, "Q8_0");
    }
}

#[test]
fn the_machine_computes_what_the_model_computes_at_every_knob_point_rewritten_every_pass() {
    agree_at_every_knob_point_of(FLUSHES[0]);
}

#[test]
fn the_machine_computes_what_the_model_computes_at_every_knob_point_flushed_every_window() {
    agree_at_every_knob_point_of(FLUSHES[1]);
}

#[test]
fn the_machine_computes_what_the_model_computes_at_every_knob_point_flushed_a_window_behind() {
    agree_at_every_knob_point_of(FLUSHES[2]);
}

#[test]
fn the_machine_computes_what_the_model_computes_in_every_block_format() {
    let dir = tempfile::tempdir().unwrap();
    let formats = [
        GgmlType::Q4_0,
        GgmlType::Q4_1,
        GgmlType::Q5_0,
        GgmlType::Q5_1,
        GgmlType::Q2K,
        GgmlType::Q3K,
        GgmlType::Q4K,
        GgmlType::Q5K,
        GgmlType::Q6K,
        GgmlType::Iq4Nl,
        GgmlType::Iq4Xs,
    ];
    for ty in formats {
        let matrix = |name: &str| {
            name.ends_with(".weight")
                && ["token_embd", "output.weight", "attn_q.", "attn_k.", "attn_v.", "attn_output", "attn_qkv", "attn_gate", "ssm_out", "ffn_gate.", "ffn_up.", "ffn_down."]
                    .iter()
                    .any(|p| name.contains(p))
        };
        let (spec, path) = checkpoint(dir.path(), ty.name(), &|name| if matrix(name) { ty } else { GgmlType::F32 });
        for n in EngineNumerics::real_points() {
            agree(&spec, &path, n, ty.name());
        }
    }
}

/// A tiny Llama, an interleaved rotation and no QK-norm, turned by the checkpoint's frequency factors when `factored`
/// (`rope_freqs.weight`, as Llama 3's are).
fn llama(dir: &Path, factored: bool) -> (ModelSpec, PathBuf) {
    let spec = if factored { tiny_models::with_rope_factors(Tiny::Llama.wide(64)) } else { Tiny::Llama.wide(64) };
    let path = dir.join(if factored { "llama3.gguf" } else { "llama.gguf" });
    tiny_models::write_checkpoint(&spec, &path);
    (spec, path)
}

/// The checkpoint's frequency factors, as it stores them.
fn factors(path: &Path) -> Vec<f32> {
    let mut g = Gguf::open(path).unwrap();
    let (ty, dims) = g.describe("rope_freqs.weight").unwrap();
    let bytes = g.row_bytes("rope_freqs.weight", 0, 1).unwrap();
    let mut out = Vec::new();
    ty.dequantize(&bytes, &mut out);
    assert_eq!(out.len(), dims[0] as usize);
    out
}

/// The words of rotary table `k` in the machine's memory: per position, its cosines then its sines.
fn table(machine: &Machine, k: usize) -> Vec<u32> {
    let (rope, at) = &machine.layout.rotary[k];
    let words = machine.layout.ctx * rope.dims;
    machine.memory()[*at as usize..*at as usize + 4 * words].chunks_exact(4).map(|w| u32::from_le_bytes(w.try_into().unwrap())).collect()
}

/// The words a table of `rope` over `ctx` positions holds with `factor` dividing pair `i`'s angle.
fn turned(rope: &Rope, ctx: usize, factor: impl Fn(usize) -> Option<f64>) -> Vec<u32> {
    let pairs = rope.dims / 2;
    let mut out = Vec::new();
    for p in 0..ctx {
        let (cos, sin): (Vec<u32>, Vec<u32>) = (0..pairs)
            .map(|i| {
                let (c, s) = rope.rotation(i, p, factor(i));
                ((c as f32).to_bits(), (s as f32).to_bits())
            })
            .unzip();
        out.extend(cos);
        out.extend(sin);
    }
    out
}

/// A checkpoint without frequency factors keeps the tables it had: each pair's unscaled angle, bit for bit, for a
/// Llama and for the 27B's partial multi-section rotation alike.
#[test]
fn a_rotation_without_frequency_factors_is_its_unscaled_angle_bit_for_bit() {
    let dir = tempfile::tempdir().unwrap();
    for (spec, path) in [llama(dir.path(), false), q8(dir.path())] {
        let machine = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), EngineNumerics::fastest(), 1, spec.ctx_len).unwrap();
        assert!(!machine.layout.rotary.is_empty());
        for (k, (rope, _)) in machine.layout.rotary.iter().enumerate() {
            assert_eq!(table(&machine, k), turned(rope, machine.layout.ctx, |_| None), "{}: table {k}", spec.arch);
        }
    }
}

/// A rotation scaled by the checkpoint's frequency factors turns each pair by its angle divided by the pair's factor:
/// the image's table is the lanes' (`model::bind::rope_tables`) in f32, and not the unscaled one.
#[test]
fn a_rotation_scaled_by_the_checkpoints_frequency_factors_turns_each_pair_by_its_factor() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = llama(dir.path(), true);
    let f = factors(&path);
    assert!(f.iter().any(|&x| x != 1.0) && f.iter().all(|&x| x >= 1.0), "factors that change the angles: {f:?}");
    let machine = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), EngineNumerics::fastest(), 1, spec.ctx_len)
        .expect("the machine runs a rotation scaled by frequency factors");
    assert_eq!(machine.layout.rotary.len(), 1, "one rotation every layer turns by");
    let (rope, _) = &machine.layout.rotary[0];
    let ctx = machine.layout.ctx;
    let got = table(&machine, 0);
    assert_eq!(got, turned(rope, ctx, |i| Some(f64::from(f[i]))), "each pair's angle divided by its factor");
    assert_ne!(got, turned(rope, ctx, |_| None), "the factors turn the table");
    let factors: Vec<f64> = f.iter().map(|&x| f64::from(x)).collect();
    let (cos, sin) = rope_tables(rope, ctx, Some(&factors));
    let pairs = rope.dims / 2;
    let lanes: Vec<u32> = (0..ctx)
        .flat_map(|p| {
            let row = |t: &[f64]| t[p * pairs..(p + 1) * pairs].iter().map(|&v| (v as f32).to_bits()).collect::<Vec<u32>>();
            [row(&cos), row(&sin)].concat()
        })
        .collect();
    assert_eq!(got, lanes, "the lanes' rotary tables");
}

/// A Llama runs on the machine as the model runs it at every knob point, its rotation scaled by frequency factors or
/// not.
#[test]
fn the_machine_computes_what_the_model_computes_for_a_llama_with_and_without_frequency_factors() {
    let dir = tempfile::tempdir().unwrap();
    for factored in [false, true] {
        let (spec, path) = llama(dir.path(), factored);
        for n in EngineNumerics::all() {
            agree(&spec, &path, n, if factored { "Llama, frequency factors" } else { "Llama" });
        }
    }
}

/// A rotation scaled by frequency factors the checkpoint does not hold, or holds one for other than each pair, is
/// refused by name: by the machine as it writes its tables, and by the model as it turns its first head.
#[test]
fn frequency_factors_the_checkpoint_does_not_hold_for_every_pair_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (_, plain) = llama(dir.path(), false);
    let factored = tiny_models::with_rope_factors(Tiny::Llama.wide(64));
    let missing = |e: EngineError| matches!(&e, EngineError::Io(io) if io.to_string().contains("rope_freqs.weight"));
    match Machine::new(factored.clone(), Gguf::open(&plain).unwrap(), EngineNumerics::fastest(), 1, factored.ctx_len) {
        Err(e) => assert!(missing(e), "the machine names the missing table"),
        Ok(_) => panic!("the machine wrote a factored rotation's table without the checkpoint's factors"),
    }
    let mut model = EngineModel::new(factored.clone(), Gguf::open(&plain).unwrap(), EngineNumerics::fastest()).unwrap();
    let s = model.sequence();
    match model.step(&[(s, PROMPT.to_vec())]) {
        Err(e) => assert!(missing(e), "the model names the missing table"),
        Ok(_) => panic!("the model turned a factored rotation without the checkpoint's factors"),
    }
    // The checkpoint's table is for the whole head; a rotation of half the head has half as many pairs.
    let (_, path) = llama(dir.path(), true);
    let mut half = factored;
    for layer in &mut half.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            a.rope.as_mut().unwrap().dims /= 2;
        }
    }
    let pairs = Tiny::Llama.wide(64).layers.iter().find_map(|l| match &l.mixer {
        Mixer::Attention(a) => a.rope.as_ref().map(|r| r.dims / 2),
        _ => None,
    });
    let pairs = pairs.unwrap();
    let wrong = |e: &EngineError| matches!(e, EngineError::Factors { found, pairs: p } if *found == pairs && *p == pairs / 2);
    match Machine::new(half.clone(), Gguf::open(&path).unwrap(), EngineNumerics::fastest(), 1, half.ctx_len) {
        Err(e) => assert!(wrong(&e), "the machine: {e:?}"),
        Ok(_) => panic!("the machine took a factor table of {pairs} for {} pairs", pairs / 2),
    }
    let mut model = EngineModel::new(half, Gguf::open(&path).unwrap(), EngineNumerics::fastest()).unwrap();
    let s = model.sequence();
    match model.step(&[(s, PROMPT.to_vec())]) {
        Err(e) => assert!(wrong(&e), "the model: {e:?}"),
        Ok(_) => panic!("the model took a factor table of {pairs} for {} pairs", pairs / 2),
    }
}

/// Two sequences ride one step, one with a prompt and one with a token, then three steps of a token each: each
/// sequence's logits are the model's, and the machine's regions keep the two apart, its step folded or not.
#[test]
fn a_batch_of_sequences_runs_as_the_model_runs_it() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    for (n, folded) in [EngineNumerics::precise(), EngineNumerics::fastest()].into_iter().flat_map(|n| [(n, false), (n, true)]) {
        let mut model = EngineModel::new(spec.clone(), Gguf::open(&path).unwrap(), n).unwrap();
        let machine = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), n, 2, spec.ctx_len).unwrap();
        let mut machine = if folded { machine.folded() } else { machine };
        let (a, b) = (model.sequence(), model.sequence());
        let (ma, mb) = (machine.sequence().unwrap(), machine.sequence().unwrap());
        let mut entries = vec![(PROMPT.to_vec(), vec![7u32])];
        for i in 0..3u32 {
            entries.push((vec![11 + i], vec![50 - i]));
        }
        for (step, (ta, tb)) in entries.into_iter().enumerate() {
            let want = model.step(&[(a, ta.clone()), (b, tb.clone())]).unwrap();
            let got = machine.step(&[(ma, ta), (mb, tb)]).unwrap();
            for (e, (w, g)) in want.iter().zip(&got).enumerate() {
                for (r, (wr, gr)) in w.iter().zip(g).enumerate() {
                    assert_eq!(bits(wr), bits(gr), "{n:?}, folded {folded}: step {step}, entry {e}, row {r}");
                }
            }
        }
    }
}

/// The machine's store is the engine's: before each job it holds exactly the slots the step has written that are live
/// out of the job before it — a slot gone once no later job reads it before it is made again, its beats another
/// slot's in the engine's store — and after the last job the logits and whatever is live round into the next step.
/// Two sequences at both ends of the knobs, the prompt and single tokens past a lazy flush, every job of every step, the
/// step folded or not.
#[test]
fn the_machine_keeps_a_slot_only_while_it_is_live() {
    use logicaffeine_compile::spatial::decode_engine::machine::StoreLayout;
    use logicaffeine_compile::spatial::decode_engine::program::Slot;
    let names = |m: u32| Slot::ALL.iter().filter(|s| m >> s.code() & 1 == 1).map(|s| format!("{s:?}")).collect::<Vec<_>>().join(", ");
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    for (n, folded) in [EngineNumerics::precise(), EngineNumerics::fastest()].into_iter().flat_map(|n| [(n, false), (n, true)]) {
        let machine = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), n, 2, spec.ctx_len).unwrap();
        let mut machine = if folded { machine.folded() } else { machine };
        let jobs = machine.jobs.clone();
        let live = StoreLayout::live_out(&jobs);
        let made: Vec<u32> = jobs
            .iter()
            .scan(0u32, |m, job| {
                let (fresh, partial) = job.writes();
                *m |= fresh.iter().chain(&partial).fold(0u32, |w, s| w | 1u32 << s.code());
                Some(*m)
            })
            .collect();
        let (a, b) = (machine.sequence().unwrap(), machine.sequence().unwrap());
        let (mut ta, mut tb) = (PROMPT.to_vec(), vec![7u32]);
        let (mut dropped, mut wrong) = (0usize, Vec::new());
        for step in 0..STEPS {
            let out = machine
                .step_traced(&[(a, ta.clone()), (b, tb.clone())], &mut |t| {
                    let (want, ever) = match t.job {
                        0 => (0, 0),
                        j => (made[j - 1] & live[j - 1], made[j - 1]),
                    };
                    let held = t.store.keys().fold(0u32, |m, s| m | 1u32 << s.code());
                    dropped += (ever & !want).count_ones() as usize;
                    if held != want {
                        wrong.push(format!(
                            "{n:?} folded {folded} step {step}, before job {}: held [{}], live [{}]",
                            t.job,
                            names(held),
                            names(want)
                        ));
                    }
                })
                .unwrap();
            ta = vec![argmax(out[0].last().unwrap())];
            tb = vec![argmax(out[1].last().unwrap())];
        }
        assert!(wrong.is_empty(), "{} points hold other than the live slots:\n{}", wrong.len(), wrong.iter().take(8).cloned().collect::<Vec<_>>().join("\n"));
        assert!(dropped > 0, "{n:?}: no slot the step made ever died, so nothing here was checked");
    }
}

/// Folding a step's residual adds and gated products into the matrix jobs that alone feed them ([`fold`]) leaves every
/// value the step computes as it was: at every knob point, the prompt and then single tokens past a lazy flush, the
/// folded machine's logits are the unfolded machine's bit for bit and so is its memory, every residual add and every
/// gated product of the tiny model folded, a job fewer for each.
#[test]
fn folding_the_residual_adds_and_gated_products_into_their_matrices_leaves_every_value_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    let program = logicaffeine_compile::spatial::decode_engine::program::lower(&spec, EngineNumerics::fastest(), spec.ctx_len).unwrap().program;
    let adds = program.commands.iter().filter(|c| matches!(c, Command::Add { .. })).count();
    let products = program.commands.iter().filter(|c| matches!(c, Command::SwiGlu { .. })).count();
    assert!(adds > 0 && products > 0, "the tiny model has residual adds and gated products to fold");
    let folds = |m: &Machine, f: Fold| m.jobs.iter().filter(|j| matches!(j, Job::Matvec { fold, .. } if *fold == f)).count();
    for n in EngineNumerics::all() {
        let mut plain = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), n, 1, spec.ctx_len).unwrap();
        let mut folded = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), n, 1, spec.ctx_len).unwrap().folded();
        assert_eq!((folds(&folded, Fold::Add), folds(&folded, Fold::SwiGlu)), (adds, products), "{n:?}: every add and product folds");
        assert_eq!(folded.jobs.len() + adds + products, plain.jobs.len(), "{n:?}: a job fewer a fold");
        let (a, b) = (plain.sequence().unwrap(), folded.sequence().unwrap());
        let mut tokens = PROMPT.to_vec();
        for step in 0..=STEPS {
            let want = plain.step(&[(a, tokens.clone())]).unwrap().remove(0);
            let got = folded.step(&[(b, tokens.clone())]).unwrap().remove(0);
            assert_eq!(want.len(), got.len());
            for (r, (w, g)) in want.iter().zip(&got).enumerate() {
                assert_eq!(bits(w), bits(g), "{n:?}: step {step}, row {r}");
            }
            assert!(plain.memory() == folded.memory(), "{n:?}: the memory after step {step}");
            tokens = vec![argmax(want.last().unwrap())];
        }
    }
}

/// Folding each Gated-DeltaNet block's gated norm into its recurrence ([`fold_delta_norms`]) leaves every value the step
/// computes as it was: at every knob point, the prompt and then single tokens past a lazy flush, one sequence and two
/// in a step, the machine whose recurrences norm their read-outs gives the plain machine's logits bit for bit and its
/// memory, folded or not; every gated norm of the tiny model goes into its recurrence, a job fewer for each, and the
/// recurrence then reads the gate's slot.
#[test]
fn folding_the_gated_norms_into_their_recurrences_leaves_every_value_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    let program = logicaffeine_compile::spatial::decode_engine::program::lower(&spec, EngineNumerics::fastest(), spec.ctx_len).unwrap().program;
    let norms = program.commands.iter().filter(|c| matches!(c, Command::GatedNorm { .. })).count();
    assert!(norms > 0, "the tiny model has gated norms to fold");
    let normed = |m: &Machine| m.jobs.iter().filter(|j| matches!(j, Job::Delta { norm: Some(_), .. })).count();
    for n in EngineNumerics::all() {
        for folded in [false, true] {
            for sequences in [1usize, 2] {
                let machine = |normed: bool| {
                    let m = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), n, sequences, spec.ctx_len).unwrap();
                    let m = if folded { m.folded() } else { m };
                    if normed { m.delta_normed() } else { m }
                };
                let (mut plain, mut gated) = (machine(false), machine(true));
                assert_eq!(normed(&gated), norms, "{n:?} folded {folded}: every gated norm goes into its recurrence");
                assert_eq!(gated.jobs.len() + norms, plain.jobs.len(), "{n:?} folded {folded}: a job fewer a norm");
                for job in gated.jobs.iter() {
                    if let Job::Delta { norm: Some(norm), to, .. } = job {
                        assert!(job.reads().contains(&norm.z) && norm.z != *to, "{n:?}: the recurrence reads the gate, a slot of its own");
                    }
                }
                let ids: Vec<(usize, usize)> = (0..sequences).map(|_| (plain.sequence().unwrap(), gated.sequence().unwrap())).collect();
                let mut tokens: Vec<Vec<u32>> = (0..sequences).map(|k| if k == 0 { PROMPT.to_vec() } else { vec![7 + k as u32] }).collect();
                for step in 0..=STEPS {
                    let want = plain.step(&ids.iter().zip(&tokens).map(|(&(a, _), t)| (a, t.clone())).collect::<Vec<_>>()).unwrap();
                    let got = gated.step(&ids.iter().zip(&tokens).map(|(&(_, b), t)| (b, t.clone())).collect::<Vec<_>>()).unwrap();
                    for (e, (w, g)) in want.iter().zip(&got).enumerate() {
                        for (r, (wr, gr)) in w.iter().zip(g).enumerate() {
                            assert_eq!(bits(wr), bits(gr), "{n:?} folded {folded}, {sequences} sequences: step {step}, entry {e}, row {r}");
                        }
                    }
                    assert!(plain.memory() == gated.memory(), "{n:?} folded {folded}: the memory after step {step}");
                    tokens = want.iter().map(|w| vec![argmax(w.last().unwrap())]).collect();
                }
            }
        }
    }
}

/// A gated norm folds only where it is the recurrence's: one that reads another slot than the read-outs, writes
/// elsewhere, gates by the read-outs' own slot, runs another program than a gated norm's, or is any other job after the
/// recurrence stays its own job, and a recurrence that carries a norm takes no second. A norm of another epsilon is a
/// gated norm still: it folds carrying its epsilon, which the head lanes take from the job's word.
#[test]
fn a_gated_norm_folds_only_into_the_recurrence_whose_read_outs_it_norms_in_place() {
    use logicaffeine_compile::spatial::decode_engine::machine::gated_norm_job;
    use logicaffeine_compile::spatial::decode_engine::numerics::FpUnit;
    use logicaffeine_compile::spatial::decode_engine::program::Slot;
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    let n = EngineNumerics::fastest();
    let machine = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), n, 1, spec.ctx_len).unwrap();
    let layout = machine.layout.clone();
    let at = machine.jobs.iter().position(|j| matches!(j, Job::Delta { .. })).expect("a recurrence");
    let (delta, norm) = (machine.jobs[at].clone(), machine.jobs[at + 1].clone());
    let Job::Delta { layer, to, .. } = delta else { unreachable!() };
    let BlockState::Delta { v_heads, width, .. } = layout.blocks[layer] else { unreachable!() };
    let Job::Vector(ref v) = norm else { panic!("the recurrence's next job is its gated norm: {norm:?}") };
    let (Source::Tensor { tensor: weight, .. }, Source::Slot { slot: z, .. }) = (v.loads[1].from, v.loads[2].from) else { panic!("{v:?}") };
    let eps = spec.rms_eps as f32;
    let fp = layout.numerics.fp_unit;
    assert_eq!(*v, gated_norm_job(fp, weight, to, z, width, v_heads * width, to, eps), "the lowered norm is the one the fold looks for");
    let folds = |second: Job| fold_delta_norms(&[delta.clone(), second], &layout).len() == 1;
    assert!(folds(norm.clone()), "the recurrence's own gated norm folds");
    let other = Slot::ALL.iter().copied().find(|s| ![to, z].contains(s)).unwrap();
    for (what, job) in [
        ("another input", gated_norm_job(fp, weight, other, z, width, v_heads * width, to, eps)),
        ("another output", gated_norm_job(fp, weight, to, z, width, v_heads * width, other, eps)),
        ("the read-outs as the gate", gated_norm_job(fp, weight, to, to, width, v_heads * width, to, eps)),
        ("a program short of its last instruction", { let mut j = gated_norm_job(fp, weight, to, z, width, v_heads * width, to, eps); j.program.pop(); j }),
        ("the other float unit", gated_norm_job(if fp == FpUnit::Soft { FpUnit::Hard } else { FpUnit::Soft }, weight, to, z, width, v_heads * width, to, eps)),
    ] {
        if job != *v {
            assert!(!folds(Job::Vector(job)), "a norm of {what} folded");
        }
    }
    assert!(!folds(Job::Advance), "a job that is no norm folded");
    let twice = fold_delta_norms(&[delta.clone(), Job::Vector(gated_norm_job(fp, weight, to, z, width, v_heads * width, to, eps * 2.0))], &layout);
    assert!(
        matches!(twice.as_slice(), [Job::Delta { norm: Some(DeltaNorm { eps: e, .. }), .. }] if e.to_bits() == (eps * 2.0).to_bits()),
        "a norm of another epsilon folds carrying it: {twice:?}"
    );
    let once = fold_delta_norms(&[delta.clone(), norm.clone()], &layout);
    assert_eq!(fold_delta_norms(&[once[0].clone(), norm.clone()], &layout).len(), 2, "a recurrence took a second norm");
}

/// A fold changes the element its sum lands on exactly as the job it replaces would: a sum added into a slot that holds
/// another job's output is that output plus the sum, and a product through a slot that holds a gate is the gate's SiLU
/// times the sum — so a fold whose arithmetic were another (the sum alone, the operands turned round) would differ.
#[test]
fn a_fold_is_the_add_or_the_gated_product_it_replaces() {
    use logicaffeine_compile::spatial::decode_engine::numerics::{fadd, silu, FpUnit};
    let values = [0.0f32, -0.0, 1.0, -2.5, 3.0e-39, 1.0e30, f32::MIN_POSITIVE, 7.25, -0.125];
    for fp in [FpUnit::Soft, FpUnit::Hard] {
        for &old in &values {
            for &sum in &values {
                assert_eq!(Fold::Put.apply(fp, old, sum).to_bits(), sum.to_bits());
                assert_eq!(Fold::Add.apply(fp, old, sum).to_bits(), fadd(old, sum).to_bits());
                assert_eq!(Fold::SwiGlu.apply(fp, old, sum).to_bits(), fp.mul(silu(fp, old), sum).to_bits());
            }
        }
    }
    assert_ne!(Fold::SwiGlu.apply(FpUnit::Soft, 2.0, 3.0).to_bits(), Fold::SwiGlu.apply(FpUnit::Soft, 3.0, 2.0).to_bits(), "the gate is the slot's");
}

/// The 27B's step, its adds and products folded, folds each of its 48 Gated-DeltaNet blocks' gated norms into the
/// block's recurrence: none is left a vector job, a job fewer for each, the store's row no wider, and its words round-trip.
#[test]
fn the_27b_folds_every_gated_norm_into_its_recurrence() {
    use logicaffeine_compile::spatial::decode_engine::machine::StoreLayout;
    let (spec, source) = qwen38_27b();
    for n in EngineNumerics::real_points().into_iter().chain([EngineNumerics::fastest()]) {
        let program = logicaffeine_compile::spatial::decode_engine::program::lower(&spec, n, spec.ctx_len).unwrap().program;
        let layout = Layout::of(&spec, n, &program, &source, 1, 4096).unwrap();
        let jobs = fold(&lower(&program, &layout, spec.rms_eps as f32), &layout);
        let normed = fold_delta_norms(&jobs, &layout);
        let norms = program.commands.iter().filter(|c| matches!(c, Command::GatedNorm { .. })).count();
        assert_eq!(norms, 48, "{n:?}: a gated norm a Gated-DeltaNet block");
        assert_eq!(normed.iter().filter(|j| matches!(j, Job::Delta { norm: Some(_), .. })).count(), norms, "{n:?}: every norm folds");
        assert_eq!(normed.len() + norms, jobs.len(), "{n:?}: a job fewer a norm");
        let (before, after) = (StoreLayout::greedy(&jobs, &layout, 1), StoreLayout::greedy(&normed, &layout, 1));
        assert!(after.beats <= before.beats, "{n:?}: the store's row grew, {} to {} beats", before.beats, after.beats);
        assert_eq!(decode(&encode(&normed)).unwrap(), normed, "{n:?}: the words of the normed step");
    }
}

/// The 27B's step folds every residual add into the matrix job before it and every gated product into its up
/// projection's: none is left a vector job, and the slots only they read — the gate's, the up projection's, a block's
/// output before its add — are gone from its store. The gated product held the residual, the gate's, the up
/// projection's and its own output at once, 3,584 beats a token-row, the store's row at every point; folded, the up
/// projection's job holds the residual, its input and the product, and the store's row is less: at the fastest point
/// the Gated-DeltaNet block's 2,249 beats, the widest set of slots a point of the folded step holds together.
#[test]
fn the_27b_folds_every_residual_add_and_gated_product_into_its_matrix() {
    use logicaffeine_compile::spatial::decode_engine::machine::StoreLayout;
    use logicaffeine_compile::spatial::decode_engine::program::Slot;
    let (spec, source) = qwen38_27b();
    for n in EngineNumerics::real_points().into_iter().chain([EngineNumerics::fastest()]) {
        let program = logicaffeine_compile::spatial::decode_engine::program::lower(&spec, n, spec.ctx_len).unwrap().program;
        let layout = Layout::of(&spec, n, &program, &source, 1, 4096).unwrap();
        let jobs = lower(&program, &layout, spec.rms_eps as f32);
        let folded = fold(&jobs, &layout);
        let adds = program.commands.iter().filter(|c| matches!(c, Command::Add { .. })).count();
        let products = program.commands.iter().filter(|c| matches!(c, Command::SwiGlu { .. })).count();
        assert_eq!((adds, products), (128, 64), "{n:?}: two residual adds and a gated product a layer");
        let folds = |f: Fold| folded.iter().filter(|j| matches!(j, Job::Matvec { fold, .. } if *fold == f)).count();
        assert_eq!((folds(Fold::Add), folds(Fold::SwiGlu)), (adds, products), "{n:?}: every add and product folds");
        assert_eq!(folded.len() + adds + products, jobs.len(), "{n:?}: a job fewer a fold");
        let (before, after) = (StoreLayout::greedy(&jobs, &layout, 1), StoreLayout::greedy(&folded, &layout, 1));
        for slot in [Slot::FfnGate, Slot::FfnUp, Slot::Out] {
            assert!(before.slots[slot.code() as usize].1 > 0 && after.slots[slot.code() as usize].1 == 0, "{n:?}: {slot:?} left the store");
        }
        assert_eq!(before.beats, 3584, "{n:?}: a token-row of the unfolded store");
        assert!(after.beats < before.beats, "{n:?}: a token-row of the folded store, {} beats", after.beats);
        let live = StoreLayout::live_out(&folded);
        for (i, job) in folded.iter().enumerate().filter(|(_, j)| matches!(j, Job::Matvec { fold: Fold::SwiGlu, .. })) {
            let mut held = live[i - 1] | live[i];
            for slot in job.reads().into_iter().chain(job.writes().0) {
                held |= 1 << slot.code();
            }
            let want = [Slot::Residual, Slot::Normed, Slot::Hidden].iter().fold(0u32, |m, s| m | 1 << s.code());
            assert_eq!(held, want, "{n:?}: the up projection's job {i} holds the residual, its input and the product alone");
        }
        if n == EngineNumerics::fastest() {
            assert_eq!((after.beats, after.widest(&folded)), (2249, 2249), "{n:?}: a token-row of the folded store, and its widest point");
        }
    }
}

/// Every store is laid as short as a row can be: its row is the widest set of slots a point of its step holds together
/// ([`StoreLayout::widest`]) — no layout's row is shorter — for the tiny model at every knob point and the 27B at the
/// card's points, their steps folded and unfolded, at one row and at sixteen, greedy or not; and every slot a point holds
/// lies apart from every other slot it holds.
#[test]
fn every_store_is_laid_at_its_steps_widest_point() {
    use logicaffeine_compile::spatial::decode_engine::machine::StoreLayout;
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    let (big, source) = qwen38_27b();
    let mut steps: Vec<(String, Vec<Job>, Layout)> = Vec::new();
    for n in EngineNumerics::all() {
        let m = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), n, 1, spec.ctx_len).unwrap();
        let folded = fold(&m.jobs, &m.layout);
        steps.push((format!("tiny {n:?}"), m.jobs.clone(), m.layout.clone()));
        steps.push((format!("tiny {n:?} folded"), folded, m.layout));
    }
    for n in EngineNumerics::real_points().into_iter().chain([EngineNumerics::fastest()]) {
        let program = logicaffeine_compile::spatial::decode_engine::program::lower(&big, n, big.ctx_len).unwrap().program;
        let layout = Layout::of(&big, n, &program, &source, 1, 4096).unwrap();
        let jobs = lower(&program, &layout, big.rms_eps as f32);
        steps.push((format!("27B {n:?} folded"), fold(&jobs, &layout), layout.clone()));
        steps.push((format!("27B {n:?}"), jobs, layout));
    }
    let mut wrong = Vec::new();
    for (what, jobs, layout) in &steps {
        let clash = StoreLayout::live_together(jobs);
        for rows in [1, 16] {
            for store in [StoreLayout::of(jobs, layout, rows), StoreLayout::greedy(jobs, layout, rows)] {
                let widest = store.widest(jobs) * rows as u32;
                if store.beats != widest {
                    wrong.push(format!("{what}, {rows} rows, greedy {}: a row of {} beats a row, its widest point {}", store.greedy, store.beats, widest));
                }
                for (c, &(at, b)) in store.slots.iter().enumerate() {
                    for (o, &(at2, b2)) in store.slots.iter().enumerate() {
                        let (end, end2) = (at + b * rows as u32, at2 + b2 * rows as u32);
                        if c != o && b > 0 && b2 > 0 && clash[c] >> o & 1 == 1 && at < end2 && at2 < end {
                            wrong.push(format!("{what}, {rows} rows: slots {c} and {o}, live together, overlap"));
                        }
                    }
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{} stores past their widest point or overlapping:\n{}", wrong.len(), wrong.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
}

/// After each token `n` of a sequence lazily flushed as `flush`, every block's log has `n`'s entry in slot `(n − 1) mod
/// capacity` and no other slot changed, and its count line holds the tokens the log holds and the slot of its oldest,
/// `count(n)` and `oldest(n)`.
fn a_lazy_log_takes_each_token_in_its_slot(flush: StateFlush, count: &dyn Fn(usize) -> usize, oldest: &dyn Fn(usize) -> usize) {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    let n = EngineNumerics { flush, ..EngineNumerics::fastest() };
    let capacity = Replay::capacity(flush);
    let mut machine = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), n, 1, spec.ctx_len).unwrap();
    let m = machine.sequence().unwrap();
    let region = machine.layout.region_of(m) as usize;
    let logs: Vec<(usize, usize)> = machine
        .layout
        .blocks
        .iter()
        .filter_map(|b| match *b {
            BlockState::Delta { log: Some(log), entry, .. } => Some((region + log as usize, entry as usize)),
            _ => None,
        })
        .collect();
    assert!(!logs.is_empty(), "a lazily flushed block");
    let word = |memory: &[u8], at: usize| u32::from_le_bytes(memory[at..at + 4].try_into().unwrap()) as usize;
    let mut token = 3;
    for n in 1..=3 * capacity / 2 + 3 {
        let before = machine.memory().to_vec();
        let logits = machine.step(&[(m, vec![token])]).unwrap().remove(0).pop().unwrap();
        token = argmax(&logits);
        let after = machine.memory();
        for &(log, entry) in &logs {
            assert_eq!((word(after, log), word(after, log + 4)), (count(n), oldest(n)), "{flush:?}, token {n}: the count and the oldest slot");
            for slot in 0..capacity {
                let at = log + LINE as usize + slot * entry;
                let changed = before[at..at + entry] != after[at..at + entry];
                assert_eq!(changed, slot == (n - 1) % capacity, "{flush:?}, token {n}: slot {slot} of the log at {log}");
            }
        }
    }
}

/// A flush every window's log fills its slots in order from the first and empties at the window's end: the oldest
/// token is always in the first slot, where the recurrence unit reads it.
#[test]
fn a_flush_every_windows_log_fills_from_its_first_slot() {
    let t = FLUSH_TOKENS as usize;
    a_lazy_log_takes_each_token_in_its_slot(StateFlush::Every { tokens: FLUSH_TOKENS }, &|n| n % t, &|_| 0);
}

/// A trailing flush's log is a ring of two windows: it holds the tokens past the window being replayed and that window
/// too until its last share is replayed, its oldest a window on each time a window is dropped.
#[test]
fn a_trailing_flushs_log_is_a_ring_of_two_windows() {
    let t = TRAILING_TOKENS as usize;
    a_lazy_log_takes_each_token_in_its_slot(
        StateFlush::Trailing { tokens: TRAILING_TOKENS },
        &|n| if n <= t { n } else { t + n % t },
        &|n| if n < 2 * t { 0 } else { (n / t - 1) % 2 * t },
    );
}

/// The weight image, the rotary tables and the regions never overlap and each starts on a line; each region holds
/// every block's state, a block's ring, memory and log, or keys and values, apart and within it.
#[test]
fn the_layout_places_everything_apart_and_on_lines() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    for n in EngineNumerics::all() {
        let machine = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), n, 3, spec.ctx_len).unwrap();
        let l = &machine.layout;
        let mut spans: Vec<(u64, u64, String)> = l.tensors.iter().map(|t| (t.at, t.at + t.bytes, t.name.clone())).collect();
        for (rope, at) in &l.rotary {
            spans.push((*at, at + (l.ctx * rope.dims / 2 * 8) as u64, "a rotary table".into()));
        }
        for s in 0..l.sequences {
            spans.push((l.region_of(s), l.region_of(s) + l.region, format!("region {s}")));
        }
        spans.sort();
        for w in spans.windows(2) {
            assert!(w[0].1 <= w[1].0, "{n:?}: {} overlaps {}", w[0].2, w[1].2);
        }
        for (at, _, what) in &spans {
            assert_eq!(at % LINE, 0, "{n:?}: {what} starts off a line");
        }
        assert!(spans.iter().all(|s| s.1 <= l.bytes) && l.bytes as usize == machine.memory().len());
        let state = if n.state == logicaffeine_compile::spatial::decode_engine::model::StateFormat::F16 { 2 } else { 4 };
        let mut parts: Vec<(u64, u64)> = Vec::new();
        for b in &l.blocks {
            match *b {
                BlockState::Delta { ring, channels, taps, memory, log, entry, k_heads, v_heads, width } => {
                    parts.push((ring, ring + ((taps - 1) * channels * state) as u64));
                    parts.push((memory, memory + (v_heads * width * width * state) as u64));
                    let f = LogFields::of(k_heads, v_heads, width);
                    assert!(entry >= (f.elems * state) as u64 && entry % LINE == 0, "{n:?}: a log entry of {entry} bytes");
                    assert_eq!(log.is_some(), n.flush != StateFlush::EveryPass, "{n:?}: a log exactly for a lazy flush");
                    if let Some(log) = log {
                        parts.push((log, log + LINE + Replay::capacity(n.flush) as u64 * entry));
                    }
                }
                BlockState::Attention { keys, values, key_scales, value_scales, kv_heads, width, value_width } => {
                    let row = |w: usize| (if n.cache.is_int8() { w } else { 2 * w }) as u64;
                    parts.push((keys, keys + l.ctx as u64 * kv_heads as u64 * row(width)));
                    parts.push((values, values + l.ctx as u64 * kv_heads as u64 * row(value_width)));
                    assert_eq!(key_scales.is_some(), n.cache.is_int8(), "{n:?}: scales exactly for an int8 cache");
                    for s in [key_scales, value_scales].into_iter().flatten() {
                        parts.push((s, s + l.ctx as u64 * kv_heads as u64 * 2));
                    }
                }
            }
        }
        parts.sort();
        for w in parts.windows(2) {
            assert!(w[0].1 <= w[1].0, "{n:?}: two block states overlap in a region");
        }
        assert!(parts.iter().all(|p| p.0 % LINE == 0 && p.1 <= l.region), "{n:?}: a block state off a line or past its region");
    }
}

/// A step reads the weight image and the rotary tables and never writes them, and writes only the regions of the
/// sequences it runs.
#[test]
fn a_step_writes_only_its_sequences_regions() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    for n in [EngineNumerics::precise(), EngineNumerics::fastest()] {
        let mut machine = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), n, 3, spec.ctx_len).unwrap();
        let (a, b, c) = (machine.sequence().unwrap(), machine.sequence().unwrap(), machine.sequence().unwrap());
        machine.step(&[(c, vec![9])]).unwrap();
        let before = machine.memory().to_vec();
        machine.step(&[(a, PROMPT.to_vec())]).unwrap();
        let l = machine.layout.clone();
        let after = machine.memory();
        let span = |s: usize| l.region_of(s) as usize..(l.region_of(s) + l.region) as usize;
        assert_eq!(&before[..l.regions as usize], &after[..l.regions as usize], "{n:?}: the step wrote the weight image or a rotary table");
        assert_eq!(&before[span(b)], &after[span(b)], "{n:?}: the step wrote a region it does not run");
        assert_eq!(&before[span(c)], &after[span(c)], "{n:?}: the step wrote a region it does not run");
        assert_ne!(&before[span(a)], &after[span(a)], "{n:?}: the step kept nothing of its sequence's state");
    }
}

/// A sequence admitted into the region a released sequence ran in starts empty: its logits are a fresh machine's,
/// and the machine holds no more sequences than it has regions.
#[test]
fn a_sequence_starts_from_an_empty_region() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    for n in [EngineNumerics::precise(), EngineNumerics::fastest()] {
        let mut fresh = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), n, 1, spec.ctx_len).unwrap();
        let s = fresh.sequence().unwrap();
        let want = fresh.step(&[(s, PROMPT.to_vec())]).unwrap();
        let mut used = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), n, 1, spec.ctx_len).unwrap();
        let first = used.sequence().unwrap();
        used.step(&[(first, vec![5, 6, 7, 8])]).unwrap();
        assert!(used.sequence().is_err(), "{n:?}: a second sequence in a machine of one region");
        used.release(first);
        assert!(used.step(&[(first, vec![1])]).is_err(), "{n:?}: a released sequence still runs");
        let second = used.sequence().unwrap();
        assert_eq!(second, first, "{n:?}: the released region is the one taken");
        assert_eq!(used.position(second), 0);
        assert_eq!(used.step(&[(second, PROMPT.to_vec())]).unwrap(), want, "{n:?}: the reused region kept its last sequence's state");
    }
}

/// Each command is one job of its unit — a float matrix the vector unit's, weight row after weight row — and a
/// rotated cache's attention a second, the vector unit's rotation back; every vector job's registers within the
/// vector unit's, a ring loaded and stored only by the convolution's jobs, and those the only jobs whose rows must run
/// in order.
#[test]
fn every_command_is_the_jobs_of_its_unit() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    for n in EngineNumerics::all() {
        let machine = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), n, 1, spec.ctx_len).unwrap();
        let program = logicaffeine_compile::spatial::decode_engine::program::lower(&spec, n, spec.ctx_len).unwrap().program;
        let mut jobs = machine.jobs.iter();
        let float = |tensor: &str| machine.layout.tensors.iter().find(|t| t.name == tensor).is_some_and(|t| t.ty == GgmlType::F32);
        for c in &program.commands {
            let job = jobs.next().unwrap_or_else(|| panic!("{n:?}: no job for {c:?}"));
            match (c, job) {
                (Command::Embed { .. }, Job::Embed { .. }) | (Command::Delta { .. }, Job::Delta { .. }) | (Command::Advance, Job::Advance) => {}
                (Command::Matvec { tensor, .. }, Job::Matvec { .. }) => assert!(!float(tensor), "{tensor}: a float matrix on the array"),
                (Command::Matvec { tensor, .. }, Job::Vector(v)) => {
                    assert!(float(tensor), "{tensor}: a block-format matrix on the vector unit");
                    assert_eq!(v.order, Order::Heads, "{tensor}: a weight row read once for every token-row");
                }
                (Command::Attend { .. }, Job::Attend { .. }) => {
                    if n.cache == CacheFormat::Int8Hadamard {
                        let back = jobs.next();
                        assert!(
                            matches!(back, Some(Job::Vector(v)) if v.program.iter().any(|o| matches!(o, Op::Hadamard { .. }))),
                            "{n:?}: a rotated cache's attention without its rotation back"
                        );
                    }
                }
                (c, Job::Vector(v)) => {
                    assert!(!matches!(c, Command::Embed { .. } | Command::Delta { .. } | Command::Attend { .. } | Command::Advance), "{c:?} on the vector unit");
                    vector_job_within_the_unit(c, v);
                }
                (c, j) => panic!("{c:?} lowered to {j:?}"),
            }
        }
        assert!(jobs.next().is_none(), "{n:?}: a job past the program");
        for job in &machine.jobs {
            if let Job::Vector(v) = job {
                assert!(v.heads >= 1);
            }
        }
    }
}

fn vector_job_within_the_unit(c: &Command, v: &VectorJob) {
    let conv = matches!(c, Command::Conv { .. });
    assert_eq!(v.ordered(), conv, "{c:?}: rows in order");
    for l in &v.loads {
        assert!((l.reg as usize) < VREGS && l.len > 0, "{c:?}: {l:?}");
        assert!(!matches!(l.from, Source::Ring { .. }) || conv, "{c:?} reads a ring");
    }
    for p in &v.puts {
        match p.reg {
            Reg::V(r) => assert!((r as usize) < VREGS, "{c:?}: {p:?}"),
            Reg::S(s) => assert!((s as usize) < SREGS && p.len == 1, "{c:?}: {p:?}"),
        }
        assert!(!matches!(p.to, Sink::Ring { .. }) || conv, "{c:?} writes a ring");
    }
}

/// The registers every field of a vector job's words names that the vector unit's side reads one from: each load's
/// and store's register, and each instruction's four fields, vector and scalar alike.
fn register_fields(v: &VectorJob) -> Vec<usize> {
    let mut fields: Vec<usize> = v.loads.iter().map(|l| l.reg as usize).collect();
    fields.extend(v.puts.iter().map(|p| match p.reg {
        Reg::V(r) | Reg::S(r) => r as usize,
    }));
    for op in &v.program {
        let word = logicaffeine_compile::spatial::decode_engine::rtl::simd::encode(op);
        fields.extend([8, 12, 16, 20].iter().map(|&at| (word >> at & 0xf) as usize));
    }
    fields
}

/// A header ledger's architecture, and the ledger as a source of its tensors' formats.
fn described(ledger: &str) -> (ModelSpec, Described) {
    let h = GgufHeader::read_ledger(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/decode/headers").join(ledger)).unwrap();
    (h.spec().unwrap(), Described(h))
}

/// The vector unit of a side whose convolutions run on their path holds
/// [`CONV_PATH_VREGS`](logicaffeine_compile::spatial::decode_engine::rtl::engine::CONV_PATH_VREGS) registers, and the
/// side refuses a job bound for its unit whose words name one past them: every vector job the machine lowers but a
/// convolution names fewer in every field the side reads a register from — at every knob point of the tiny model, and
/// the 0.8B's and the 27B's at their real points and the fastest, as lowered, folded and with their gated norms folded,
/// as the card runs them. A convolution's taps name more; only its path runs it.
#[test]
fn every_vector_job_but_a_convolution_names_fewer_registers_than_a_conv_path_sides_unit_holds() {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::CONV_PATH_VREGS;
    let check = |what: &str, jobs: &[Job]| {
        let mut convolutions = 0;
        for (j, job) in jobs.iter().enumerate() {
            let Job::Vector(v) = job else { continue };
            let most = register_fields(v).into_iter().max().unwrap_or(0);
            if v.ordered() {
                convolutions += 1;
            } else {
                assert!(most < CONV_PATH_VREGS, "{what}: job {j} names register {most}: {v:?}");
            }
        }
        assert!(convolutions > 0, "{what}: the step convolves");
    };
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    for n in EngineNumerics::all() {
        let machine = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), n, 1, spec.ctx_len).unwrap();
        check(&format!("the tiny model at {n:?}"), &machine.jobs);
    }
    for ledger in ["qwen3.5-0.8b-q8_0.json", "qwen3.8-27b-q4_k_m.json"] {
        let (spec, source) = described(ledger);
        for n in EngineNumerics::real_points().into_iter().chain([EngineNumerics::fastest()]) {
            let program = logicaffeine_compile::spatial::decode_engine::program::lower(&spec, n, spec.ctx_len).unwrap().program;
            let layout = Layout::of(&spec, n, &program, &source, 1, 4096).unwrap();
            let jobs = lower(&program, &layout, spec.rms_eps as f32);
            let folded = fold(&jobs, &layout);
            let normed = fold_delta_norms(&folded, &layout);
            for (how, jobs) in [("lowered", &jobs), ("folded", &folded), ("normed", &normed)] {
                check(&format!("{ledger} at {n:?}, {how}"), jobs);
            }
        }
    }
}

/// Every row the engine moves fills whole lines of the card's memory — a ring row, a memory row, a log entry, a cached
/// head row, a rotary table's position — and every span a vector job moves to or from a token-row starts on a beat and
/// a vector store ends on one or at its vector's end: the stream engines move lines, and the store moves beats, a
/// scalar store its one element.
#[test]
fn every_row_fills_whole_lines_and_every_span_starts_on_a_beat() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    for n in EngineNumerics::all() {
        let machine = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), n, 1, spec.ctx_len).unwrap();
        let l = &machine.layout;
        let state = if n.state == logicaffeine_compile::spatial::decode_engine::model::StateFormat::F16 { 2 } else { 4 };
        for b in &l.blocks {
            match *b {
                BlockState::Delta { channels, entry, k_heads, v_heads, width, .. } => {
                    let f = LogFields::of(k_heads, v_heads, width);
                    for (field, at) in [("values", f.values), ("decay exponents", f.log_decays), ("gates", f.gates), ("end", f.elems)] {
                        assert_eq!(at % LANES, 0, "{n:?}: a log entry's {field} from element {at}");
                    }
                    assert!(f.values >= k_heads * width && f.log_decays >= f.values + v_heads * width && f.gates >= f.log_decays + v_heads);
                    assert!(f.elems >= f.gates + v_heads, "{n:?}: a log entry's fields overlap");
                    assert_eq!((channels * state) as u64 % LINE, 0, "{n:?}: a ring row");
                    assert_eq!((width * state) as u64 % LINE, 0, "{n:?}: a memory row");
                    assert_eq!(entry % LINE, 0, "{n:?}: a log entry");
                }
                BlockState::Attention { width, value_width, .. } => {
                    for w in [width, value_width] {
                        let row = if n.cache.is_int8() { w } else { 2 * w } as u64;
                        assert_eq!(row % LINE, 0, "{n:?}: a cached head row of {w}");
                    }
                }
            }
        }
        for (rope, _) in &l.rotary {
            assert_eq!((rope.dims / 2 * 8) as u64 % LINE, 0, "{n:?}: a rotary table's position");
        }
        for job in &machine.jobs {
            let Job::Vector(v) = job else { continue };
            for ld in &v.loads {
                if let Source::Slot { at, stride, .. } = ld.from {
                    assert!(at % LANES == 0 && stride % LANES == 0, "{n:?}: a load from element {at} by {stride}");
                }
            }
            for p in &v.puts {
                if let (Sink::Slot { slot, at, stride }, Reg::V(_)) = (p.to, p.reg) {
                    assert!(at % LANES == 0 && stride % LANES == 0, "{n:?}: a store to element {at} by {stride}");
                    // A vector store ends on a beat or at its vector's end, so the lanes of its last beat past it are
                    // no vector's: the engine needs no mask to keep them.
                    let width = v.fresh.iter().find(|(s, _)| *s == slot).map(|(_, w)| *w);
                    let ends = |h: usize| at + h * stride + p.len;
                    for h in 0..v.heads {
                        assert!(
                            ends(h) % LANES == 0 || Some(ends(h)) == width || (stride == 0 && v.heads == 1),
                            "{n:?}: {slot:?} stored to element {} mid-beat", ends(h)
                        );
                    }
                }
            }
        }
    }
}

/// A header ledger as a weight source: its tensors' formats and dims, and no bytes.
struct Described(GgufHeader);

impl WeightSource for Described {
    fn describe(&self, name: &str) -> std::io::Result<(GgmlType, Vec<u64>)> {
        let t = self.0.tensors.iter().find(|t| t.name == name).ok_or_else(|| std::io::Error::other(format!("no {name}")))?;
        let ty = GgmlType::from_id(t.type_id).map_err(|id| std::io::Error::other(format!("type {id}")))?;
        Ok((ty, t.dims.clone()))
    }
    fn row_bytes(&mut self, name: &str, _: usize, _: usize) -> std::io::Result<Vec<u8>> {
        Err(std::io::Error::other(format!("a header ledger holds no bytes of {name}")))
    }
    fn has(&self, name: &str) -> bool {
        self.0.tensors.iter().any(|t| t.name == name)
    }
}

/// The 27B's architecture from its header ledger, and the ledger as a source of its tensors' formats.
fn qwen38_27b() -> (ModelSpec, Described) {
    let ledger = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/decode/headers/qwen3.8-27b-q4_k_m.json");
    let h = GgufHeader::read_ledger(&ledger).unwrap();
    (h.spec().unwrap(), Described(h))
}

/// The 27B's architecture, read from its header ledger, lays out at every knob point it runs: its rows fill whole
/// lines, the card's 32 GiB hold its weight image, and a sequence of 4,096 positions takes what the pricer holds for
/// it with its caches whole, never more and less by no more than 2% — the pricer counts one ring row and one log
/// entry the machine writes over in place.
#[test]
fn the_27b_lays_out_on_the_cards_lines() {
    let (spec, source) = qwen38_27b();
    let h = &source.0;
    for n in EngineNumerics::all() {
        let program = logicaffeine_compile::spatial::decode_engine::program::lower(&spec, n, spec.ctx_len).unwrap().program;
        let layout = Layout::of(&spec, n, &program, &source, 1, 4096).unwrap_or_else(|e| panic!("{n:?}: {e:?}"));
        let card = 32u64 << 30;
        assert!(layout.regions < card, "{n:?}: the weight image of {} bytes", layout.regions);
        let serving = logicaffeine_compile::spatial::serving::Serving {
            kv_page: None,
            ..logicaffeine_compile::spatial::decode_engine::twin::serving_of(&spec, n, 1, 2048.0)
        };
        let held = logicaffeine_compile::spatial::serving::PassShape::of(h, &serving).unwrap().held_per_sequence;
        let region = layout.region as f64;
        assert!(region <= held && region >= 0.98 * held, "{n:?}: a region of {region} bytes against the pricer's {held}");
    }
}

/// The 27B's token-row store at sixteen rows lays two slots over the same beats only when no point of its step holds
/// both. Taken round the step twice, as one step follows another and the host reads the logits after the last job,
/// every slot a job reads holds, over all its beats, what that slot's own last write left there, never a beat another
/// slot wrote over since. And a row of the store is the largest sum of slots any point of the step holds together —
/// those a job reads or writes and those live across it — where laying every slot apart took the sum of them all.
#[test]
fn the_27b_store_lays_slots_never_live_together_over_the_same_beats() {
    use logicaffeine_compile::spatial::decode_engine::machine::StoreLayout;
    use logicaffeine_compile::spatial::decode_engine::program::Slot;
    use std::collections::BTreeSet;
    const ROWS: usize = 16;
    let (spec, source) = qwen38_27b();
    let n = EngineNumerics::fastest();
    let program = logicaffeine_compile::spatial::decode_engine::program::lower(&spec, n, spec.ctx_len).unwrap().program;
    let layout = Layout::of(&spec, n, &program, &source, ROWS, 4096).unwrap();
    let jobs = lower(&program, &layout, spec.rms_eps as f32);
    let store = StoreLayout::greedy(&jobs, &layout, ROWS);
    let region = |s: Slot| {
        let (at, beats) = store.slots[s.code() as usize];
        at as usize..(at + beats * ROWS as u32) as usize
    };
    // The ownership walk: each write claims its slot's beats; each read, the second time round, finds them its own.
    let mut owner: Vec<Option<Slot>> = vec![None; store.beats as usize];
    let mut stolen: BTreeSet<String> = BTreeSet::new();
    for round in 0..2 {
        let mut check = |s: Slot, at: usize, owner: &[Option<Slot>]| {
            if round == 1 {
                if let Some(b) = region(s).find(|&b| owner[b] != Some(s)) {
                    stolen.insert(format!("job {at} reads {s:?}, whose beat {b} {:?} wrote over", owner[b]));
                }
            }
        };
        for (i, job) in jobs.iter().enumerate() {
            for s in job.reads() {
                check(s, i, &owner);
            }
            let (fresh, partial) = job.writes();
            for s in fresh.iter().chain(&partial) {
                for b in region(*s) {
                    owner[b] = Some(*s);
                }
            }
        }
        check(Slot::Logits, jobs.len(), &owner);
    }
    assert!(stolen.is_empty(), "{} reads of beats another slot wrote over:\n{}", stolen.len(), stolen.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
    // Each point's live slots, the step taken round until nothing changes: a slot read by a job, or written by it in
    // part, is live into it; one live out of it is live into it unless the job makes it afresh.
    let sets: Vec<(Vec<Slot>, Vec<Slot>, Vec<Slot>)> = jobs.iter().map(|j| (j.reads(), j.writes().0, j.writes().1)).collect();
    let mut live_in: Vec<BTreeSet<u8>> = vec![BTreeSet::new(); jobs.len()];
    let live_out = |live_in: &[BTreeSet<u8>], i: usize| -> BTreeSet<u8> {
        if i + 1 < live_in.len() {
            live_in[i + 1].clone()
        } else {
            let mut s = live_in[0].clone();
            s.insert(Slot::Logits.code());
            s
        }
    };
    loop {
        let mut changed = false;
        for i in (0..jobs.len()).rev() {
            let (reads, fresh, partial) = &sets[i];
            let mut set: BTreeSet<u8> = live_out(&live_in, i).into_iter().filter(|c| !fresh.iter().any(|f| f.code() == *c)).collect();
            set.extend(reads.iter().chain(partial).map(|s| s.code()));
            if set != live_in[i] {
                live_in[i] = set;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let beats = |c: u8| store.slots[c as usize].1;
    let peak = (0..jobs.len())
        .map(|i| {
            let (reads, fresh, partial) = &sets[i];
            let mut held = live_in[i].clone();
            held.extend(live_out(&live_in, i));
            held.extend(reads.iter().chain(fresh).chain(partial).map(|s| s.code()));
            held.into_iter().map(beats).sum::<u32>()
        })
        .max()
        .unwrap();
    let apart: u32 = Slot::ALL.iter().map(|s| beats(s.code())).sum();
    assert!(peak < apart, "the step holds every slot at once: {peak} of {apart} beats a row");
    assert_eq!(store.beats as usize, peak as usize * ROWS, "a row of the store, where its step holds at most {peak} beats together and every slot apart is {apart}");
}

/// The sequencer's words decode to the jobs they encode: every knob point's jobs on the tiny model, and the 27B's own.
#[test]
fn the_sequencers_words_decode_to_the_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    let (big, ledger) = qwen38_27b();
    for n in EngineNumerics::all() {
        let machine = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), n, 1, spec.ctx_len).unwrap();
        assert_eq!(decode(&encode(&machine.jobs)).unwrap(), machine.jobs, "{n:?}: the tiny model's jobs");
        let program = logicaffeine_compile::spatial::decode_engine::program::lower(&big, n, big.ctx_len).unwrap().program;
        let layout = Layout::of(&big, n, &program, &ledger, 1, 4096).unwrap();
        let jobs = lower(&program, &layout, big.rms_eps as f32);
        assert_eq!(decode(&encode(&jobs)).unwrap(), jobs, "{n:?}: the 27B's jobs");
    }
}

/// Every field of every word round-trips at its extremes: the largest layer, tensor, element and head count each
/// field holds, every slot, every register, every source, sink and cache part, a NaN scale's bits, a recurrence's norm
/// beside one without.
#[test]
fn every_field_of_a_job_word_round_trips_at_its_extremes() {
    use logicaffeine_compile::spatial::decode_engine::program::Slot;
    let big = u32::MAX as usize;
    let mut jobs = vec![
        Job::Embed { tensor: big },
        Job::Matvec { tensor: (1 << 20) - 1, from: Slot::Logits, to: Slot::Residual, fold: Fold::SwiGlu },
        Job::Matvec { tensor: 0, from: Slot::Residual, to: Slot::Logits, fold: Fold::Add },
        Job::Matvec { tensor: 1, from: Slot::Hidden, to: Slot::Hidden, fold: Fold::Put },
        Job::Attend { layer: 255, query: Slot::Hidden, heads: (1 << 16) - 1, scale: f32::from_bits(0xffc0_0001), to: Slot::Logits },
        Job::Advance,
    ];
    for (i, s) in Slot::ALL.iter().enumerate() {
        let o = |k: usize| Slot::ALL[(i + k) % Slot::ALL.len()];
        jobs.push(Job::Delta { layer: 255 - i, query: *s, key: o(1), value: o(2), value_at: (1 << 20) - 1 - i, log_decay: o(3), decay: o(4), gate: o(5), to: o(6), norm: None });
        let norm = Some(DeltaNorm { weight: (1 << 20) - 1 - i, z: o(7), eps: f32::from_bits(0xffc0_0001 - i as u32) });
        jobs.push(Job::Delta { layer: i, query: o(6), key: o(5), value: o(4), value_at: i, log_decay: o(3), decay: o(2), gate: o(1), to: *s, norm });
        assert_eq!(Slot::of_code(s.code()), Some(*s));
    }
    let sources = [
        Source::Slot { slot: Slot::Logits, at: big, stride: big },
        Source::Tensor { tensor: (1 << 16) - 1, at: big, stride: big },
        Source::Rotary { table: (1 << 16) - 1, sines: true },
        Source::Rotary { table: 0, sines: false },
        Source::Ring { layer: (1 << 16) - 1, age: 255 },
    ];
    let sinks = [
        Sink::Slot { slot: Slot::Logits, at: big, stride: big },
        Sink::Ring { layer: (1 << 16) - 1 },
        Sink::Cache { layer: (1 << 16) - 1, values: true, part: CachePart::Halves },
        Sink::Cache { layer: 0, values: false, part: CachePart::Codes },
        Sink::Cache { layer: 7, values: true, part: CachePart::Scale },
    ];
    let program: Vec<Op> = (0..u16::MAX as usize).map(|i| Op::Const { value: f32::from_bits(i as u32 * 65_537), d: (i % 8) as u8 }).collect();
    jobs.push(Job::Vector(VectorJob {
        heads: big,
        order: Order::Heads,
        fresh: Slot::ALL.iter().map(|&s| (s, big)).chain(std::iter::repeat((Slot::Residual, 0)).take(255 - Slot::ALL.len())).collect(),
        loads: (0..255).map(|i| Load { reg: (i % 16) as u8, from: sources[i % sources.len()], len: big - i }).collect(),
        program,
        puts: (0..255)
            .map(|i| Put { reg: if i % 2 == 0 { Reg::V((i % 16) as u8) } else { Reg::S((i % 16) as u8) }, to: sinks[i % sinks.len()], len: big - i })
            .collect(),
    }));
    // NaN constants and scales compare unequal as floats: the decoded jobs are held to the words they encode, bit for
    // bit, and to the jobs field by field wherever a float is not a NaN.
    let words = encode(&jobs);
    let back = decode(&words).unwrap();
    assert_eq!(encode(&back), words);
    assert_eq!(back.len(), jobs.len());
    for (a, b) in back.iter().zip(&jobs) {
        match (a, b) {
            (Job::Attend { scale: x, .. }, Job::Attend { scale: y, .. }) => assert_eq!(x.to_bits(), y.to_bits()),
            (Job::Delta { norm: Some(x), .. }, Job::Delta { norm: Some(y), .. }) => {
                assert_eq!((x.weight, x.z, x.eps.to_bits()), (y.weight, y.z, y.eps.to_bits()));
                let unnormed = |j: &Job| {
                    let mut j = j.clone();
                    if let Job::Delta { norm, .. } = &mut j {
                        *norm = None;
                    }
                    j
                };
                assert_eq!(unnormed(a), unnormed(b));
            }
            (Job::Vector(x), Job::Vector(y)) => {
                assert_eq!((x.heads, x.order, &x.fresh, &x.loads, &x.puts), (y.heads, y.order, &y.fresh, &y.loads, &y.puts));
                assert_eq!(x.program.len(), y.program.len());
                for (p, q) in x.program.iter().zip(&y.program) {
                    match (p, q) {
                        (Op::Const { value: u, d: e }, Op::Const { value: v, d: f }) => assert_eq!((u.to_bits(), e), (v.to_bits(), f)),
                        _ => assert_eq!(p, q),
                    }
                }
            }
            _ => assert_eq!(a, b),
        }
    }
}

/// A word no job encodes to is refused by name, never read as some other job: an unknown unit, source, sink, cache
/// part or fold, a slot past the last, an instruction no instruction encodes to, words that end inside a job.
#[test]
fn a_word_no_job_encodes_to_is_refused() {
    use logicaffeine_compile::spatial::decode_engine::program::Slot;
    let good = encode(&[Job::Vector(VectorJob {
        heads: 1,
        order: Order::Rows,
        fresh: vec![],
        loads: vec![Load { reg: 0, from: Source::Slot { slot: Slot::Residual, at: 0, stride: 0 }, len: 16 }],
        program: vec![Op::Hadamard { a: 0, d: 1 }],
        puts: vec![Put { reg: Reg::V(1), to: Sink::Slot { slot: Slot::Residual, at: 0, stride: 0 }, len: 16 }],
    })]);
    assert!(decode(&good).is_ok());
    let broken = |i: usize, f: &dyn Fn(u64) -> u64| {
        let mut w = good.clone();
        w[i] = f(w[i]);
        decode(&w)
    };
    assert!(broken(0, &|h| h & !0xf | 9).is_err(), "unit 9");
    assert!(broken(2, &|l| l & !0xf0 | 7 << 4).is_err(), "load source 7");
    assert!(broken(2, &|l| l & !(0xffff << 8) | 31 << 8).is_err(), "slot 31");
    assert!(broken(4, &|p| p & !(0x7 << 5) | 6 << 5).is_err(), "store sink 6");
    assert!(broken(6, &|_| 0xf).is_err(), "instruction 15");
    let mut cache = good.clone();
    cache[4] = cache[4] & !(0x7 << 5) | 2 << 5 | 3 << 25;
    assert!(decode(&cache).is_err(), "cache part 3");
    let matrix = encode(&[Job::Matvec { tensor: 7, from: Slot::Normed, to: Slot::Residual, fold: Fold::Add }]);
    assert!(decode(&matrix).is_ok());
    assert!(decode(&[matrix[0], matrix[1] | 3 << 30]).is_err(), "fold 3");
    for end in 1..good.len() {
        assert!(decode(&good[..end]).is_err(), "words ending at {end}");
    }
}

/// A load is kept ([`VectorJob::kept`]: loaded on a job's first pass alone) exactly when it brings a tensor's span at
/// stride 0 into a register no other load and no instruction writes: no other source is, a tensor at a stride is not,
/// every instruction that writes a vector register unkeeps the one it writes and no instruction that only reads it
/// does, and two loads into one register are neither kept. The flag is the load's word's `[31:24]`, and a word whose
/// flag says other than its job is refused.
#[test]
fn a_load_is_kept_only_from_a_tensor_at_stride_zero_into_a_register_nothing_else_writes() {
    use logicaffeine_compile::spatial::decode_engine::program::Slot;
    use logicaffeine_compile::spatial::decode_engine::vector::{Bin, Func};
    let tensor = |reg: u8, stride: usize| Load { reg, from: Source::Tensor { tensor: 3, at: 0, stride }, len: 32 };
    let slot = |reg: u8| Load { reg, from: Source::Slot { slot: Slot::Residual, at: 0, stride: 0 }, len: 32 };
    let job = |loads: Vec<Load>, program: Vec<Op>| VectorJob { heads: 2, order: Order::Rows, fresh: vec![], loads, program, puts: vec![] };
    let sources = job(
        vec![
            tensor(0, 0),
            tensor(1, 32),
            slot(2),
            Load { reg: 3, from: Source::Rotary { table: 0, sines: false }, len: 32 },
            Load { reg: 4, from: Source::Ring { layer: 0, age: 0 }, len: 32 },
        ],
        vec![],
    );
    assert_eq!((0..5).map(|i| sources.kept(i)).collect::<Vec<_>>(), [true, false, false, false, false]);
    let writers = [
        Op::Vec { op: Bin::Add, a: 1, b: 1, d: 0 },
        Op::VecScalar { op: Bin::Mul, a: 1, s: 0, d: 0 },
        Op::Map { f: Func::Exp, a: 1, d: 0 },
        Op::Hadamard { a: 1, d: 0 },
        Op::Rotate { a: 1, cos: 1, sin: 1, d: 0, neox: false },
        Op::Codes { a: 1, s: 0, d: 0 },
        Op::Scale { a: 1, n: 3, d: 0 },
    ];
    for w in writers {
        assert_eq!(w.writes(), Some(0), "{w:?}");
        assert!(!job(vec![tensor(0, 0)], vec![w]).kept(0), "{w:?} writes the tensor's register");
        assert!(!job(vec![tensor(0, 0)], vec![Op::Dot { a: 0, b: 0, d: 0 }, w, Op::Sum { a: 0, d: 1 }]).kept(0), "{w:?} among readers");
    }
    let readers = [
        Op::Dot { a: 0, b: 0, d: 0 },
        Op::Sum { a: 0, d: 0 },
        Op::Max { a: 0, d: 1 },
        Op::Scalar { op: Bin::Add, a: 0, b: 0, d: 0 },
        Op::ScalarMap { f: Func::Exp, a: 0, d: 0 },
        Op::Const { value: 1.0, d: 0 },
        Op::ArgMax { a: 0, d: 0 },
        Op::AbsMax { a: 0, d: 0 },
        Op::Vec { op: Bin::Add, a: 0, b: 0, d: 1 },
        Op::Map { f: Func::Silu, a: 0, d: 1 },
        Op::Rotate { a: 0, cos: 0, sin: 0, d: 1, neox: true },
        Op::Codes { a: 0, s: 0, d: 1 },
    ];
    for r in readers {
        assert!(job(vec![tensor(0, 0)], vec![r]).kept(0), "{r:?} leaves the tensor's register");
    }
    let twice = job(vec![tensor(0, 0), slot(0)], vec![]);
    assert!(!twice.kept(0) && !twice.kept(1), "two loads into one register");
    let words = encode(&[Job::Vector(sources.clone())]);
    for i in 0..5 {
        assert_eq!(words[2 + 2 * i] >> 24 & 0xff, u64::from(sources.kept(i)), "load {i}'s flag");
    }
    for i in [0, 1] {
        let mut w = words.clone();
        w[2 + 2 * i] ^= 1 << 24;
        assert!(decode(&w).is_err(), "load {i}'s flag flipped");
    }
    let mut w = words.clone();
    w[2] |= 2 << 24;
    assert!(decode(&w).is_err(), "a flag past one");
}

/// Every tensor a job reads at stride 0 — the norms' weights, the convolution's taps, the gates' constants — is kept:
/// no micro-program writes the register it brings, so the vector unit reads it on a job's first pass alone. Every knob
/// point's jobs on the tiny model, and the 27B's, at least two a layer (each layer's two norms).
#[test]
fn every_tensor_a_job_reads_at_stride_zero_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    let (big, ledger) = qwen38_27b();
    for n in EngineNumerics::all() {
        let machine = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), n, 1, spec.ctx_len).unwrap();
        let program = logicaffeine_compile::spatial::decode_engine::program::lower(&big, n, big.ctx_len).unwrap().program;
        let layout = Layout::of(&big, n, &program, &ledger, 1, 4096).unwrap();
        let jobs = lower(&program, &layout, big.rms_eps as f32);
        for (what, jobs, layers) in [("the tiny model", &machine.jobs, spec.layers.len()), ("the 27B", &jobs, big.layers.len())] {
            let mut kept = 0;
            for (j, job) in jobs.iter().enumerate() {
                let Job::Vector(v) = job else { continue };
                for (i, l) in v.loads.iter().enumerate() {
                    if matches!(l.from, Source::Tensor { stride: 0, .. }) {
                        assert!(v.kept(i), "{n:?}: {what}'s job {j}, load {i} at stride 0 is not kept: {v:?}");
                        kept += 1;
                    }
                }
            }
            assert!(kept >= 2 * layers, "{n:?}: {what} keeps {kept} loads over {layers} layers");
        }
    }
}

/// A vector job runs in groups exactly when its passes stand alone, and then a pass's values are its own whatever its
/// registers start as: every job whose passes stand alone, of the 27B and of the tiny model at every knob point, run
/// from its loads once over zeroed registers and once over registers of garbage — NaNs, infinities, random bits, other
/// lengths — leaves every register its stores read the same, bit for bit. The 27B's gated norms (one a Gated-DeltaNet
/// layer) and its head norms run in groups; no job that loads or stores a ring row, turns or rotates does. The header
/// carries the flag, and a word with it flipped is refused.
#[test]
fn a_vector_job_runs_in_groups_exactly_when_its_passes_stand_alone() {
    use logicaffeine_compile::spatial::decode_engine::vector::{run, Bin, Func, Regs};
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = q8(dir.path());
    let (big, ledger) = qwen38_27b();
    let mut r = 0x6b00_0001u64;
    let mut next = || {
        r ^= r << 13;
        r ^= r >> 7;
        r ^= r << 17;
        r
    };
    let garbage = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.0, 1e-40, 3.4e38];
    for n in EngineNumerics::all() {
        let machine = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), n, 1, spec.ctx_len).unwrap();
        let program = logicaffeine_compile::spatial::decode_engine::program::lower(&big, n, big.ctx_len).unwrap().program;
        let layout = Layout::of(&big, n, &program, &ledger, 1, 4096).unwrap();
        let jobs = lower(&program, &layout, big.rms_eps as f32);
        for (what, jobs) in [("the tiny model", &machine.jobs), ("the 27B", &jobs)] {
            let (mut gated_norms, mut head_norms) = (0, 0);
            for (j, job) in jobs.iter().enumerate() {
                let Job::Vector(v) = job else { continue };
                let words = encode(std::slice::from_ref(job));
                assert_eq!(words[0] >> 54 & 1, v.grouped() as u64, "{n:?}: {what}'s job {j}'s header carries its flag");
                let mut flipped = words.clone();
                flipped[0] ^= 1 << 54;
                assert!(decode(&flipped).is_err(), "{n:?}: {what}'s job {j} with its group flag flipped is refused");
                let ring = v.loads.iter().any(|l| matches!(l.from, Source::Ring { .. })) || v.puts.iter().any(|p| matches!(p.to, Sink::Ring { .. }));
                let turns = v.program.iter().any(|op| matches!(op, Op::Hadamard { .. } | Op::Rotate { .. }));
                if ring || turns {
                    assert!(!v.grouped(), "{n:?}: {what}'s job {j} runs in groups: {v:?}");
                    continue;
                }
                let tail = &v.program[v.program.len().saturating_sub(2)..];
                if v.heads > 1 && matches!(tail, [Op::Map { f: Func::Silu, .. }, Op::Vec { op: Bin::Mul, .. }]) {
                    assert!(v.grouped(), "{n:?}: {what}'s gated norm, job {j}, runs alone: {v:?}");
                    gated_norms += 1;
                }
                if v.heads > 1 && v.loads.len() == 2 && matches!(v.program.first(), Some(Op::Dot { .. })) && matches!(v.program.last(), Some(Op::Vec { op: Bin::Mul, .. })) && v.program.len() == 8 {
                    assert!(v.grouped(), "{n:?}: {what}'s head norm, job {j}, runs alone: {v:?}");
                    head_norms += 1;
                }
                if !v.grouped() {
                    continue;
                }
                let len = v.loads[0].len;
                let fill = |r: &mut dyn FnMut() -> u64, wild: bool| -> Regs {
                    let mut regs = Regs::new(16, 8);
                    for reg in &mut regs.v {
                        *reg = if wild { (0..(r() % 300) as usize).map(|_| if r() % 3 == 0 { garbage[(r() % 6) as usize] } else { f32::from_bits(r() as u32) }).collect() } else { vec![0.0; len] };
                    }
                    for s in &mut regs.s {
                        *s = if wild { garbage[(r() % 6) as usize] } else { 0.0 };
                    }
                    regs
                };
                let (mut zeroed, mut wild) = (fill(&mut next, false), fill(&mut next, true));
                for l in &v.loads {
                    let values: Vec<f32> = (0..len).map(|_| (((next() >> 40) as f32 / (1u64 << 23) as f32) - 1.0) * 4.0).collect();
                    zeroed.v[l.reg as usize] = values.clone();
                    wild.v[l.reg as usize] = values;
                }
                run(n.fp_unit, &v.program, &mut zeroed);
                run(n.fp_unit, &v.program, &mut wild);
                for put in &v.puts {
                    match put.reg {
                        Reg::V(reg) => assert_eq!(bits(&zeroed.v[reg as usize]), bits(&wild.v[reg as usize]), "{n:?}: {what}'s job {j}: register {reg} depends on what the pass starts with"),
                        Reg::S(reg) => assert_eq!(zeroed.s[reg as usize].to_bits(), wild.s[reg as usize].to_bits(), "{n:?}: {what}'s job {j}: scalar {reg} depends on what the pass starts with"),
                    }
                }
            }
            if what == "the 27B" {
                assert_eq!(gated_norms, 48, "{n:?}: the 27B's gated norms in groups, one a Gated-DeltaNet layer");
                assert!(head_norms >= 32, "{n:?}: the 27B's head norms in groups: {head_norms}");
            }
        }
    }
}

/// A group loads every pass before it stores any, so a job whose pass loads store elements an earlier pass of its row
/// stores runs alone, in either order of passes. Each pass loads four elements at eight apart, so the spans leave gaps;
/// every store is the loads' length, so only where it lands decides: a pass storing the next head's span, or reaching
/// into it by an element, runs alone; a store into the span two heads on runs alone at three heads or more and in groups
/// at two; a pass storing its own span in place, into the gap before the next head's span, over an earlier head's span
/// (a later pass's store over what an earlier pass has already read), every pass storing the first head's span, and a
/// store into another slot run in groups. A scalar stored into the next head's span runs alone, into its own in groups.
/// One head never conflicts with itself. (Each job reduces, so none runs a chunk at a time.)
#[test]
fn a_job_whose_pass_loads_what_an_earlier_pass_stores_runs_alone() {
    use logicaffeine_compile::spatial::decode_engine::program::Slot;
    use logicaffeine_compile::spatial::decode_engine::vector::Func;
    let job = |heads: usize, order: Order, load_at: usize, at: usize, stride: usize, to: Slot| VectorJob {
        heads,
        order,
        fresh: vec![],
        loads: vec![Load { reg: 0, from: Source::Slot { slot: Slot::Query, at: load_at, stride: 8 }, len: 4 }],
        program: vec![Op::Map { f: Func::Silu, a: 0, d: 1 }, Op::Dot { a: 0, b: 1, d: 0 }],
        puts: vec![Put { reg: Reg::V(1), to: Sink::Slot { slot: to, at, stride }, len: 4 }],
    };
    for order in [Order::Rows, Order::Heads] {
        for heads in [2, 3, 48] {
            assert!(!job(heads, order, 0, 8, 8, Slot::Query).grouped(), "{order:?}, {heads} heads: a pass stores the next head's span");
            assert!(!job(heads, order, 0, 5, 8, Slot::Query).grouped(), "{order:?}, {heads} heads: a pass's store reaches into the next head's span by an element");
            assert_eq!(job(heads, order, 0, 16, 8, Slot::Query).grouped(), heads == 2, "{order:?}, {heads} heads: a pass stores the span two heads on");
            assert!(job(heads, order, 0, 0, 8, Slot::Query).grouped(), "{order:?}, {heads} heads: each pass stores its own span in place");
            assert!(job(heads, order, 0, 4, 8, Slot::Query).grouped(), "{order:?}, {heads} heads: a pass stores into the gap before the next head's span");
            assert!(job(heads, order, 8, 0, 8, Slot::Query).grouped(), "{order:?}, {heads} heads: a pass stores over the span the head before it loaded");
            assert!(job(heads, order, 0, 0, 0, Slot::Query).grouped(), "{order:?}, {heads} heads: every pass stores the first head's span");
            assert!(job(heads, order, 0, 8, 8, Slot::Key).grouped(), "{order:?}, {heads} heads: a pass stores into another slot");
        }
        assert!(job(1, order, 0, 8, 8, Slot::Query).grouped(), "{order:?}: one head stores past its own span");
        assert!(job(1, order, 0, 0, 8, Slot::Query).grouped(), "{order:?}: one head stores its own span");
    }
    let mut scalar = job(3, Order::Rows, 0, 0, 8, Slot::Query);
    scalar.puts = vec![Put { reg: Reg::S(0), to: Sink::Slot { slot: Slot::Query, at: 8, stride: 1 }, len: 1 }];
    assert!(!scalar.grouped(), "a pass's scalar stored into the next head's span");
    scalar.puts[0].to = Sink::Slot { slot: Slot::Query, at: 0, stride: 8 };
    assert!(scalar.grouped(), "a pass's scalar stored into its own span");
}

/// The vector unit's instruction words decode to the instructions they encode, and a word decodes only when it is
/// exactly an instruction's encoding.
#[test]
fn every_instruction_word_decodes_to_its_instruction() {
    use logicaffeine_compile::spatial::decode_engine::rtl::simd;
    use logicaffeine_compile::spatial::decode_engine::vector::{Bin, Func};
    let funcs = [Func::Exp, Func::Recip, Func::Rsqrt, Func::Sigmoid, Func::Silu, Func::Softplus, Func::Ln1p, Func::Neg, Func::F16];
    let mut ops = Vec::new();
    for (a, b, d) in [(0u8, 0u8, 0u8), (15, 15, 15), (3, 9, 12)] {
        for op in [Bin::Mul, Bin::Add, Bin::Sub] {
            ops.extend([Op::Vec { op, a, b, d }, Op::VecScalar { op, a, s: b, d }, Op::Scalar { op, a, b, d }]);
        }
        for f in funcs {
            ops.extend([Op::Map { f, a, d }, Op::ScalarMap { f, a, d }]);
        }
        ops.extend([
            Op::Dot { a, b, d },
            Op::Sum { a, d },
            Op::Max { a, d },
            Op::Const { value: f32::from_bits(0xffc0_0001), d },
            Op::Const { value: -0.0, d },
            Op::Hadamard { a, d },
            Op::Rotate { a, cos: b, sin: d, d: a, neox: true },
            Op::Rotate { a, cos: b, sin: d, d, neox: false },
            Op::ArgMax { a, d },
            Op::AbsMax { a, d },
            Op::Codes { a, s: b, d },
            Op::Scale { a, n: -126, d },
            Op::Scale { a, n: 127, d: b },
            Op::MulAdd { a, b, c: d, d: a },
            Op::MulAdd { a, b, c: b, d },
            Op::MulAddScalar { a, b, s: d, d },
        ]);
    }
    for op in &ops {
        let w = simd::encode(op);
        let back = simd::decode(w).unwrap_or_else(|| panic!("{op:?} does not decode"));
        assert_eq!(simd::encode(&back), w, "{op:?}");
    }
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    for _ in 0..200_000 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let w = state ^ state >> 29;
        if let Some(op) = simd::decode(w) {
            assert_eq!(simd::encode(&op), w, "{w:#018x} decodes to {op:?}");
        }
    }
}

/// A pass that only stores into a ring still runs its rows in order, as one that only loads from one does: a ring
/// read or written is the row's sequence's state.
#[test]
fn a_job_that_touches_a_ring_either_way_runs_its_rows_in_order() {
    use logicaffeine_compile::spatial::decode_engine::program::Slot;
    let job = |loads: Vec<Load>, puts: Vec<Put>| VectorJob { heads: 1, order: Order::Rows, fresh: vec![], loads, program: vec![], puts };
    let slot = Load { reg: 0, from: Source::Slot { slot: Slot::Qkv, at: 0, stride: 0 }, len: 16 };
    let ring = Load { reg: 1, from: Source::Ring { layer: 0, age: 0 }, len: 16 };
    let to_slot = Put { reg: Reg::V(0), to: Sink::Slot { slot: Slot::Mixed, at: 0, stride: 0 }, len: 16 };
    let to_ring = Put { reg: Reg::V(0), to: Sink::Ring { layer: 0 }, len: 16 };
    assert!(!job(vec![slot], vec![to_slot]).ordered());
    assert!(job(vec![slot, ring], vec![to_slot]).ordered(), "a ring read");
    assert!(job(vec![slot], vec![to_slot, to_ring]).ordered(), "a ring written");
}

/// A vector job runs a chunk of its vectors at a time ([`VectorJob::chunked`]) exactly when its rows cross nowhere: one
/// head, every load a vector of one length from a slot, a tensor or a ring, every store that length from a vector
/// register into a slot or a ring, and every instruction elementwise or a scalar's — none that reduces, turns, rotates
/// or codes across elements. On the 27B those are its FFN products (17,408 elements), its convolutions through each
/// sequence's ring (10,240), its attention gates (6,144), its residual adds (5,120) and the small gates of its
/// recurrence (48); none of its norms, float matrices, rotations, cache rows or read-outs. The job's header carries the
/// flag at bit 53, and a word whose flag is not its job's is refused.
#[test]
fn a_vector_job_runs_a_chunk_at_a_time_exactly_when_its_rows_cross_nowhere() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let header = GgufHeader::read_ledger(&root.join("benchmarks/silicon/decode/headers/qwen3.8-27b-q4_k_m.json")).unwrap();
    let spec = header.spec().unwrap();
    let n = EngineNumerics::fastest_for(&spec);
    let steps = logicaffeine_compile::spatial::decode_engine::program::lower(&spec, n, spec.ctx_len.min(4096)).unwrap().program;
    let layout = Layout::of(&spec, n, &steps, &header, 16, spec.ctx_len.min(4096)).unwrap();
    let jobs = lower(&steps, &layout, spec.rms_eps as f32);
    let crosses = |op: &Op| matches!(op, Op::Dot { .. } | Op::Sum { .. } | Op::Max { .. } | Op::ArgMax { .. } | Op::AbsMax { .. } | Op::Hadamard { .. } | Op::Rotate { .. } | Op::Codes { .. });
    let mut widths: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
    for (j, job) in jobs.iter().enumerate() {
        let Job::Vector(v) = job else { continue };
        let len = v.loads[0].len;
        let plain = v.heads == 1
            && v.loads.iter().all(|l| l.len == len && matches!(l.from, Source::Slot { .. } | Source::Tensor { .. } | Source::Ring { .. }))
            && v.puts.iter().all(|p| p.len == len && matches!(p.reg, Reg::V(_)) && matches!(p.to, Sink::Slot { .. } | Sink::Ring { .. }));
        assert_eq!(v.chunked(), plain && !v.program.iter().any(crosses), "job {j}: {v:?}");
        if v.chunked() {
            *widths.entry(len).or_insert(0) += 1;
        }
        let words = encode(std::slice::from_ref(job));
        assert_eq!(words[0] >> 53 & 1, v.chunked() as u64, "job {j}'s header carries its flag");
        let mut flipped = words.clone();
        flipped[0] ^= 1 << 53;
        assert!(decode(&flipped).is_err(), "job {j} with its flag flipped is refused");
        assert_eq!(decode(&words).unwrap(), vec![job.clone()], "job {j} round-trips");
    }
    assert_eq!(widths.into_iter().collect::<Vec<_>>(), vec![(48, 48), (5120, 128), (6144, 16), (10240, 48), (17408, 64)], "the 27B's chunked jobs by width");
}
