//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE COMPILED GENERATION LANE GENERATES WHAT THE MODEL COMPUTES, ON THE GPU.
//!
//! Each tiny architecture's decode step, compiled by `codegen_cuda::lane` from its spec and its checkpoint, runs eight
//! prompts and generates; at every generated position its logits are held to the `f64` reference (`model::reference`)
//! computing at the lane's own precision (`Activations`: a quantized weight reads the activation quantized exactly as
//! the kernels quantize it), fed the lane's own tokens: 2e-5 of the logits' scale for the precise lane, 2e-3 for
//! two-pass int8, 5e-3 for int8. Where the lane's `f32` arithmetic takes a decision the other way that the reference
//! made near its other outcome — an activation's rounding within `NEAR_ROUNDING` of a half-way point between two
//! levels, a routing choice within `NEAR_ROUTING` of a tie — the reference is run with that decision taken the lane's
//! way, and a parting no such decision explains fails.
//!
//! The lane's token is the arg-max of its own logits, and parts from the reference's only at a tie inside the bound.
//! A prompt run as chunks continues exactly as one fed a token at a time, and every fusion computes what its launches
//! compute apart: the program with every rule held back generates the fused program's tokens and logits bit for bit. The real Qwen3-8B, compiled the same way,
//! continues llama.cpp's golden prompt with llama.cpp's tokens. On every real checkpoint the lane compiles, each point
//! of the `activations` knob is held to the bar it earns against llama.cpp's own runs of the 32-prompt corpus: the
//! precise and two-pass lanes follow llama.cpp's `f32` run at least as far as its shipped run does, and the int8 lane —
//! llama.cpp's own activation precision — sits as close to the `f32` run's logits as the shipped run does.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;
#[path = "common/scratch.rs"]
mod scratch;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::path::{Path, PathBuf};
use std::fmt::Write as _;
use std::process::Command;

use logicaffeine_compile::codegen_cuda::knobs::CudaKnobs;
use logicaffeine_compile::codegen_cuda::lane::plan::{kind, Launch, RULES};
use logicaffeine_compile::codegen_cuda::lane::residence::place;
use logicaffeine_compile::codegen_cuda::lane::{compile, compile_batch, compile_batch_holding, compile_holding, Form, Lane, LaneOptions};
use logicaffeine_compile::codegen_cuda::qwen_emit::Q8MatKind;
use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::reference::{argmax, Activations, KvStorage, NearCall, Reference, NEAR_ROUNDING, NEAR_ROUTING};
use logicaffeine_compile::model::spec::{Mixer, ModelSpec};
use tiny_models::{Tiny, ALL, BLOCK_FORMATS};

const CTX: usize = 24;
const PROMPT: &[u32] = &[5, 17, 2, 30];
const GENERATE: usize = 12;

fn dir() -> PathBuf {
    scratch::dir("logos_cuda_lane")
}

/// How far a lane's logits may sit from the reference computing at the lane's precision, as a fraction of the
/// reference logits' largest magnitude, once the near decisions the lane took the other way are taken its way: the
/// `f32` activations against `f64` (every sum in `f64`: typically 1–4e-6, 1.1e-5 at the most sensitive position of the
/// tiny corpora), and roundings the lane's arithmetic moves across a half-way point by more than `NEAR_ROUNDING` — for
/// two-pass's low word, 128 times finer, common (measured ≤ 4.5e-4).
fn bound(numerics: Q8MatKind) -> f64 {
    match numerics {
        Q8MatKind::Dequant => 2e-5,
        Q8MatKind::TwoPassInt8 => 2e-3,
        Q8MatKind::Int8Act => 5e-3,
    }
}

/// What a run of a compiled lane printed: the generated tokens, every generated position's logits, the decode rate, and
/// where it placed its matrices: the bytes it allocated on the device and in mapped host memory, and how many of its
/// matrices it moved there.
struct Run {
    tokens: Vec<u32>,
    logits: Vec<Vec<f32>>,
    tok_per_s: Option<f64>,
    placed: (usize, usize, usize),
}

/// The device and host bytes a program's `stdout` says it allocated, and the matrices it says it moved.
fn placed(stdout: &str) -> (usize, usize, usize) {
    let field = |key: &str| -> usize {
        stdout.lines().find_map(|l| l.strip_prefix(key)).unwrap_or_else(|| panic!("the program prints {key}")).trim().parse().unwrap()
    };
    (field("DEVICE_BYTES:"), field("HOST_BYTES:"), field("MOVED:"))
}

/// A lane's program built: the binary and its sidecar.
struct Built {
    bin: PathBuf,
    side: PathBuf,
    name: String,
}

fn build(lane: &Lane, name: &str) -> Built {
    let cu = dir().join(format!("{name}.cu"));
    let bin = dir().join(name);
    let side = dir().join(format!("{name}.side"));
    std::fs::write(&cu, &lane.program.cu).unwrap();
    std::fs::write(&side, &lane.program.sidecar).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "{name} builds:\n{}", String::from_utf8_lossy(&built.stderr));
    Built { bin, side, name: name.to_string() }
}

/// [`build`] every job on a pool as wide as the machine, each program in its job's place. nvcc compiles a program on one
/// thread, so programs built one after another wait on one core however many sit idle, and on a loaded box get one
/// runnable's share of it.
fn build_all(jobs: &[(&Lane, &str)]) -> Vec<Built> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    let width = std::thread::available_parallelism().map_or(1, |n| n.get()).min(jobs.len());
    let mut built: Vec<(usize, Built)> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..width)
            .map(|_| {
                s.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some((lane, name)) = jobs.get(i) else { break mine };
                        mine.push((i, build(lane, name)));
                    }
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().unwrap_or_else(|e| std::panic::resume_unwind(e))).collect()
    });
    built.sort_by_key(|(i, _)| *i);
    built.into_iter().map(|(_, b)| b).collect()
}

/// Build the lane's program and run it over `ckpt` from `prompt` for `n` tokens.
fn run(lane: &Lane, ckpt: &Path, prompt: &[u32], n: usize, name: &str, vocab: usize, logits: bool) -> Run {
    exec(&build(lane, name), ckpt, prompt, n, vocab, logits)
}

/// Run a built program over `ckpt` from `prompt` for `n` tokens.
fn exec(b: &Built, ckpt: &Path, prompt: &[u32], n: usize, vocab: usize, logits: bool) -> Run {
    exec_flagged(b, ckpt, prompt, n, vocab, logits, &[])
}

/// [`exec`], the program given `flags` too.
fn exec_flagged(b: &Built, ckpt: &Path, prompt: &[u32], n: usize, vocab: usize, logits: bool, flags: &[&str]) -> Run {
    let Built { bin, side, name } = b;
    let dump = dir().join(format!("{name}.logits"));
    let mut cmd = Command::new(bin);
    cmd.arg(ckpt).arg(side).arg(n.to_string()).args(flags);
    if logits {
        cmd.arg("--logits").arg(&dump);
    }
    let out = cmd.args(prompt.iter().map(u32::to_string)).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(out.status.success(), "{name} runs:\n{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    let field = |key: &str| stdout.lines().find_map(|l| l.strip_prefix(key)).map(str::trim);
    let tokens: Vec<u32> = field("TOKENS:").expect("the tokens").split_whitespace().map(|t| t.parse().unwrap()).collect();
    assert_eq!(tokens.len(), n, "{name}: {n} tokens");
    let logits = if logits {
        let bytes = std::fs::read(&dump).unwrap();
        let all: Vec<f32> = bytes.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
        assert_eq!(all.len(), n * vocab, "{name}: a row of logits per generated position");
        all.chunks(vocab).map(<[f32]>::to_vec).collect()
    } else {
        Vec::new()
    };
    Run { tokens, logits, tok_per_s: field("DECODE_TOK_PER_S:").map(|v| v.parse().unwrap()), placed: placed(&stdout) }
}

/// Run a built program once over `ckpt` for every `(prompt, n)` of `jobs` (its `--prompts` form): each prompt's run, in
/// order.
fn exec_many(b: &Built, ckpt: &Path, jobs: &[(Vec<u32>, usize)], vocab: usize, logits: bool) -> Vec<Run> {
    exec_listed(b, ckpt, jobs, vocab, logits, "--prompts")
}

/// Run a built program's batch step over `ckpt` for the `(prompt, n)` of `jobs` together (its `--batch` form): each
/// sequence's run, in order, and the rate the batch decoded at, every sequence's tokens counted.
fn exec_batch(b: &Built, ckpt: &Path, jobs: &[(Vec<u32>, usize)], vocab: usize, logits: bool) -> Vec<Run> {
    exec_listed(b, ckpt, jobs, vocab, logits, "--batch")
}

/// Run a built program once over a list of `jobs` under `form` (`--prompts` or `--batch`): each prompt's run, in order.
fn exec_listed(b: &Built, ckpt: &Path, jobs: &[(Vec<u32>, usize)], vocab: usize, logits: bool, form: &str) -> Vec<Run> {
    let Built { bin, side, name } = b;
    let tag = form.trim_start_matches('-');
    let (list, dump) = (dir().join(format!("{name}.{tag}")), dir().join(format!("{name}.{tag}.logits")));
    let lines: String = jobs
        .iter()
        .map(|(p, n)| format!("{n} {}\n", p.iter().map(u32::to_string).collect::<Vec<_>>().join(" ")))
        .collect();
    std::fs::write(&list, lines).unwrap();
    let mut cmd = Command::new(bin);
    cmd.arg(ckpt).arg(side).arg(form).arg(&list);
    if logits {
        cmd.arg("--logits").arg(&dump);
    }
    let out = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(out.status.success(), "{name} runs its prompts ({form}):\n{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    let rate = stdout.lines().find_map(|l| l.strip_prefix("DECODE_TOK_PER_S:")).map(|v| v.trim().parse::<f64>().unwrap());
    let tokens: Vec<Vec<u32>> = stdout
        .lines()
        .filter_map(|l| l.strip_prefix("TOKENS:"))
        .map(|t| t.split_whitespace().map(|v| v.parse().unwrap()).collect())
        .collect();
    assert_eq!(tokens.len(), jobs.len(), "{name}: a run a prompt");
    let at = placed(&stdout);
    let mut all: Vec<f32> = if logits {
        std::fs::read(&dump).unwrap().chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect()
    } else {
        Vec::new()
    };
    if logits {
        assert_eq!(all.len(), jobs.iter().map(|(_, n)| n * vocab).sum::<usize>(), "{name}: a row of logits per generated position");
    }
    jobs.iter()
        .zip(tokens)
        .map(|((_, n), tokens)| {
            assert_eq!(tokens.len(), *n, "{name}: {n} tokens");
            let rows: Vec<f32> = if logits { all.drain(..n * vocab).collect() } else { Vec::new() };
            Run { tokens, logits: rows.chunks(vocab).map(<[f32]>::to_vec).collect(), tok_per_s: rate.filter(|_| form == "--batch"), placed: at }
        })
        .collect()
}

/// The activations the reference reads to compute at a lane's precision.
fn precision(numerics: Q8MatKind) -> Activations {
    match numerics {
        Q8MatKind::Dequant => Activations::Exact,
        Q8MatKind::Int8Act => Activations::Int8,
        Q8MatKind::TwoPassInt8 => Activations::Int8TwoPass,
    }
}

/// The most near calls a run may decide the other way, and the most tried at one parting.
const MAX_FLIPS: usize = 8;
const CANDIDATES: usize = 64;

/// How a run compares with the reference with some near calls decided the other way: within the bound at every
/// position (the worst distance), or parted at a position — its logits past the bound from the reference's, or its
/// token not the reference's arg-max with the two further apart than the bound — by how far (the larger of the two),
/// with the near calls the reference made up to it.
enum Held {
    Within(f64),
    Parts { position: usize, by: f64, what: String, near: Vec<NearCall> },
}

fn compare(spec: &ModelSpec, ckpt: &Path, prompt: &[u32], r: &Run, numerics: Q8MatKind, kv: KvStorage, flips: &[NearCall], what: &str) -> Held {
    let mut reference =
        Reference::new(spec.clone(), Gguf::open(ckpt).unwrap()).unwrap().with_activations(precision(numerics)).with_kv_storage(kv);
    for &c in flips {
        reference.flip(c);
    }
    let mut want = reference.step(prompt).unwrap();
    let mut worst = 0f64;
    for (i, (got, &tok)) in r.logits.iter().zip(&r.tokens).enumerate() {
        let scale = want.iter().fold(1f64, |m, v| m.max(v.abs()));
        let diff = got.iter().zip(&want).map(|(g, w)| (*g as f64 - w).abs()).fold(0f64, f64::max) / scale;
        let own = argmax(&got.iter().map(|&v| v as f64).collect::<Vec<_>>());
        assert_eq!(tok, own, "{what} position {i}: the lane emits its own logits' arg-max");
        let theirs = argmax(&want);
        let gap = (want[theirs as usize] - want[tok as usize]) / scale;
        if diff > bound(numerics) || gap > bound(numerics) {
            let position = prompt.len() - 1 + i;
            return Held::Parts {
                position,
                by: diff.max(gap),
                what: format!(
                    "{what} position {i}: the logits are {diff:.3e} of their scale from the reference, the lane takes {tok} and the reference {theirs}, {gap:.3e} apart"
                ),
                near: reference.near_calls().iter().filter(|c| c.position() <= position).copied().collect(),
            };
        }
        worst = worst.max(diff);
        if i + 1 < r.tokens.len() {
            want = reference.step(&[tok]).unwrap();
        }
    }
    Held::Within(worst)
}

/// How near its other outcome a call was, as a fraction of how near it may be to be decided the other way and of how
/// far the other outcome moves its value (a whole level for an int8 word, a binary16 step for a cached value).
fn closeness(c: &NearCall) -> f64 {
    match c {
        NearCall::Rounding(c) => c.margin / (NEAR_ROUNDING * c.step),
        NearCall::Routing(c) => c.margin / (NEAR_ROUTING * c.score.abs()),
    }
}

/// Hold a run to the reference at the run's precision, fed the run's own tokens. Where the run parts from the
/// reference, the near calls the reference made up to that position — roundings within `NEAR_ROUNDING` of a half-way
/// point, routings within `NEAR_ROUTING` of a tie — are tried decided the other way beside the calls already kept,
/// those at the parting position first and the nearest first. A call that holds the run within the bound is kept at
/// once; otherwise the call that carries the parting furthest — to the latest position, then to the least distance
/// there — is kept, so the calls a position needs together are kept one after another. Each kept call is a decision
/// the run's `f32` arithmetic made the other way. A parting no near call carries fails. Returns the worst distance
/// over the run and the calls kept.
fn held_to_reference(spec: &ModelSpec, ckpt: &Path, prompt: &[u32], r: &Run, numerics: Q8MatKind, what: &str) -> (f64, usize) {
    let (worst, kept) = held(spec, ckpt, prompt, r, numerics, KvStorage::Exact, what);
    (worst, kept.len())
}

/// [`held_to_reference`] against the reference whose caches keep their rows as `kv` stores them — under binary16, its
/// near calls include each cached value rounded within `NEAR_ROUNDING` of a half-way point — returning the calls kept.
/// Where no one call carries a parting, the calls the run took that the parting at that position may have misled are
/// let go: from the calls kept before the run first parted there, each candidate is tried together with each near call
/// its own run makes — a cached value rounded the other way, and an activation rounding that rounding moves near its
/// half-way point — and the pair that holds the run, or carries the parting furthest, is kept. A position stuck twice
/// fails.
fn held(spec: &ModelSpec, ckpt: &Path, prompt: &[u32], r: &Run, numerics: Q8MatKind, kv: KvStorage, what: &str) -> (f64, Vec<NearCall>) {
    let compare = |calls: &[NearCall]| compare(spec, ckpt, prompt, r, numerics, kv, calls, what);
    let order = |near: &mut Vec<NearCall>| near.sort_by(|a, b| b.position().cmp(&a.position()).then(closeness(a).total_cmp(&closeness(b))));
    // The best of `calls` each taken beside `kept`: one that holds the run, or the one that carries a parting at
    // `position` by `by` furthest — to the latest position, then to the least distance there.
    let carry = |kept: &[NearCall], calls: Vec<NearCall>, position: usize, by: f64| -> Option<(NearCall, usize, f64)> {
        let mut best: Option<(NearCall, usize, f64)> = None;
        for c in calls.into_iter().filter(|c| !kept.contains(c)).take(CANDIDATES) {
            let tried: Vec<NearCall> = kept.iter().copied().chain([c]).collect();
            match compare(&tried) {
                Held::Within(_) => return Some((c, usize::MAX, 0.0)),
                Held::Parts { position: next, by: d, .. } => {
                    let carries = next > position || (next == position && d < by);
                    if carries && best.map_or(true, |(_, p, b)| next > p || (next == p && d < b)) {
                        best = Some((c, next, d));
                    }
                }
            }
        }
        best
    };
    // Each kept call, and the position the run parted at when it was kept.
    let mut flips: Vec<(NearCall, usize)> = Vec::new();
    let mut stuck: Vec<usize> = Vec::new();
    loop {
        let calls: Vec<NearCall> = flips.iter().map(|f| f.0).collect();
        match compare(&calls) {
            Held::Within(worst) => return (worst, calls),
            Held::Parts { position, by, what: parted, mut near } => {
                assert!(flips.len() < MAX_FLIPS, "{parted}, past {MAX_FLIPS} near calls decided the other way: {calls:?}");
                order(&mut near);
                if let Some((c, ..)) = carry(&calls, near, position, by) {
                    flips.push((c, position));
                    continue;
                }
                assert!(!stuck.contains(&position), "{parted}, and no near rounding or routing call the reference made carries it (taken: {calls:?})");
                stuck.push(position);
                flips.retain(|&(_, at)| at < position);
                let base: Vec<NearCall> = flips.iter().map(|f| f.0).collect();
                let Held::Parts { position: from, by: base_by, mut near, .. } = compare(&base) else { continue };
                order(&mut near);
                let mut best: Option<(NearCall, NearCall, usize, f64)> = None;
                for c in near.into_iter().filter(|c| !base.contains(c)).take(CANDIDATES) {
                    let with: Vec<NearCall> = base.iter().copied().chain([c]).collect();
                    let Held::Parts { near: mut induced, .. } = compare(&with) else {
                        best = Some((c, c, usize::MAX, 0.0));
                        break;
                    };
                    order(&mut induced);
                    if let Some((n, next, d)) = carry(&with, induced, from, base_by) {
                        if best.map_or(true, |(.., p, b)| next > p || (next == p && d < b)) {
                            best = Some((c, n, next, d));
                        }
                        if next == usize::MAX {
                            break;
                        }
                    }
                }
                match best {
                    Some((c, n, ..)) => {
                        flips.push((c, from));
                        if n != c {
                            flips.push((n, from));
                        }
                    }
                    None => panic!("{parted}, and no near rounding or routing call the reference made, alone or with another, carries it (taken: {calls:?})"),
                }
            }
        }
    }
}

/// The prompts every architecture runs: lengths 3 to 10, tokens spread over the vocabulary.
fn prompts() -> Vec<Vec<u32>> {
    (0..8u32).map(|k| (0..3 + k).map(|i| (i * 37 + k * 11 + 5) % tiny_models::VOCAB as u32).collect()).collect()
}

#[test]
fn every_tiny_architecture_generates_what_the_reference_computes() {
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let prompts = prompts();
    let jobs: Vec<(Vec<u32>, usize)> = prompts.iter().map(|p| (p.clone(), GENERATE)).collect();
    let mut report = Vec::new();
    for &t in ALL {
        let spec = t.spec(CTX);
        let ckpt = dir().join(format!("{}.gguf", t.name()));
        tiny_models::write_checkpoint(&spec, &ckpt);
        for numerics in [Q8MatKind::Dequant, Q8MatKind::TwoPassInt8, Q8MatKind::Int8Act] {
            let what = format!("{} {numerics:?}", t.name());
            let lane = compile(&spec, &ckpt, &LaneOptions { ctx: CTX, knobs: CudaKnobs { activations: numerics, ..CudaKnobs::default() }, prefill: 1 })
                .unwrap_or_else(|e| panic!("{what}: {e}"));
            let runs = exec_many(&build(&lane, &format!("{}-{numerics:?}", t.name())), &ckpt, &jobs, spec.vocab, true);
            let (mut worst, mut flips) = (0f64, 0);
            for (k, (prompt, r)) in prompts.iter().zip(&runs).enumerate() {
                let (w, f) = held_to_reference(&spec, &ckpt, prompt, r, numerics, &format!("{what} prompt {k}"));
                worst = worst.max(w);
                flips += f;
            }
            report.push(format!("{what}: logits within {worst:.3e} of their scale, {flips} near calls decided the other way"));
        }
    }
    eprintln!("{}", report.join("\n"));
}

/// A prompt run as chunks continues exactly as the same prompt fed a token at a time: every kernel computes each row of
/// a block in the order it computes a lone row, so the eleven-token prompt run as chunks of 8, 2 and 1 leaves the same
/// caches, rings and memories, and the same logits, bit for bit.
#[test]
fn a_prompt_run_in_chunks_continues_as_one_fed_a_token_at_a_time() {
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let prompt: Vec<u32> = vec![5, 17, 2, 30, 41, 8, 63, 12, 77, 3, 90];
    for &t in ALL {
        let spec = t.spec(CTX);
        let ckpt = dir().join(format!("chunks-{}.gguf", t.name()));
        tiny_models::write_checkpoint(&spec, &ckpt);
        for numerics in [Q8MatKind::Dequant, Q8MatKind::Int8Act] {
            let runs: Vec<Run> = [1, 8]
                .iter()
                .map(|&prefill| {
                    let opts = LaneOptions { ctx: CTX, knobs: CudaKnobs { activations: numerics, ..CudaKnobs::default() }, prefill };
                    let lane = compile(&spec, &ckpt, &opts).unwrap_or_else(|e| panic!("{} prefill {prefill}: {e}", t.name()));
                    run(&lane, &ckpt, &prompt, 6, &format!("chunks-{}-{numerics:?}-{prefill}", t.name()), spec.vocab, true)
                })
                .collect();
            assert_eq!(runs[1].tokens, runs[0].tokens, "{} {numerics:?}: the chunked prompt continues alike", t.name());
            assert_eq!(runs[1].logits, runs[0].logits, "{} {numerics:?}: bit for bit", t.name());
        }
    }
}

/// A long prompt crosses the attention's chunks. A row's span is cut into chunks by its length alone: the decode step
/// runs each chunk in a block of its own and folds them as the last block arrives, and a prefill chunk whose rows
/// already fill the chip runs every chunk of a row in the row's block, in the same arithmetic. The attentions that
/// take every path of the kernel — grouped heads, sinks beside windowed layers, latent attention's narrower values read
/// in the cache — each get a prompt long enough that its chunks outgrow one tile of positions. So the prompt fed a
/// token at a time, run in chunks of 16 (their rows' chunks a block each) and in chunks of 64 (a block a row)
/// continues alike, bit for bit, and as the reference computes.
#[test]
fn a_long_prompt_attends_in_chunks_alike_in_every_form() {
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    const LONG: usize = 4096;
    // Each attention over the whole cache (no window) of a form: its chunks and whether they run a block each.
    let global = |f: &Form| -> Vec<(usize, bool)> {
        f.schedule
            .launches
            .iter()
            .filter_map(|l| match l {
                Launch::Attention { splits, parallel, window: None, .. } => Some((*splits, *parallel)),
                _ => None,
            })
            .collect()
    };
    for t in [Tiny::Qwen3, Tiny::GptOss, Tiny::DeepSeek2] {
        let spec = t.spec(LONG);
        let ckpt = dir().join(format!("long-{}.gguf", t.name()));
        tiny_models::write_checkpoint(&spec, &ckpt);
        let lane = |prefill: usize| {
            let opts = LaneOptions { ctx: LONG, knobs: CudaKnobs { activations: Q8MatKind::Dequant, ..CudaKnobs::default() }, prefill };
            compile(&spec, &ckpt, &opts).unwrap_or_else(|e| panic!("{} prefill {prefill}: {e}", t.name()))
        };
        let lanes = [lane(1), lane(16), lane(64)];
        let step = global(lanes[0].decode());
        assert!(!step.is_empty() && step.iter().all(|&(s, p)| s > 1 && p), "{}: the decode step's chunks run a block each: {step:?}", t.name());
        for (l, prefill, parallel) in [(&lanes[1], 16, true), (&lanes[2], 64, false)] {
            let chunk = global(l.chunk(prefill).expect("the chunk form"));
            assert!(
                chunk.iter().all(|&(s, p)| s > 1 && p == parallel),
                "{} prefill {prefill}: the chunk's attentions take their chunks {}: {chunk:?}",
                t.name(),
                if parallel { "a block each" } else { "in their rows' blocks" }
            );
        }
        let splits = step.iter().map(|&(s, _)| s).max().expect("an attention");
        let prompt: Vec<u32> = (0..64 * splits as u32 + 50).map(|i| (i * 37 + 11) % tiny_models::VOCAB as u32).collect();
        assert!(prompt.len() + 6 <= LONG, "{}: the prompt fits the cache", t.name());
        let runs: Vec<Run> = lanes
            .iter()
            .zip([1, 16, 64])
            .map(|(l, prefill)| run(l, &ckpt, &prompt, 6, &format!("long-{}-{prefill}", t.name()), spec.vocab, true))
            .collect();
        for (r, prefill) in runs[1..].iter().zip([16, 64]) {
            assert_eq!(r.tokens, runs[0].tokens, "{} prefill {prefill}: the long prompt continues alike", t.name());
            assert_eq!(r.logits, runs[0].logits, "{} prefill {prefill}: bit for bit", t.name());
        }
        held_to_reference(&spec, &ckpt, &prompt, &runs[0], Q8MatKind::Dequant, &format!("{} long", t.name()));
    }
}

/// One program run over many prompts gives each prompt the run a process of its own gives it: every carried state —
/// a latent cache, the delta rule's memories and convolution rings, the generated run and the position — starts over
/// for each prompt, so the tokens and every logit match bit for bit.
#[test]
fn a_program_run_over_many_prompts_gives_each_the_run_it_gives_alone() {
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let jobs: Vec<(Vec<u32>, usize)> =
        vec![(vec![5, 17, 2, 30, 41, 8, 63, 12, 77, 3, 90], 6), (vec![9, 1], 4), (vec![44, 13, 70, 2, 2, 81, 6], 5)];
    for t in [Tiny::Qwen35, Tiny::DeepSeek2, Tiny::GptOss] {
        let spec = t.spec(CTX);
        let ckpt = dir().join(format!("many-{}.gguf", t.name()));
        tiny_models::write_checkpoint(&spec, &ckpt);
        let opts = LaneOptions { ctx: CTX, knobs: CudaKnobs::default(), prefill: 8 };
        let lane = compile(&spec, &ckpt, &opts).unwrap_or_else(|e| panic!("{}: {e}", t.name()));
        let built = build(&lane, &format!("many-{}", t.name()));
        let many = exec_many(&built, &ckpt, &jobs, spec.vocab, true);
        for ((prompt, n), r) in jobs.iter().zip(&many) {
            let alone = exec(&built, &ckpt, prompt, *n, spec.vocab, true);
            assert_eq!(r.tokens, alone.tokens, "{}: the prompt {prompt:?} continues alike", t.name());
            assert_eq!(r.logits, alone.logits, "{}: bit for bit", t.name());
        }
    }
}

/// A batch step decodes each of its sequences exactly as the decode step decodes it alone: four prompts of four lengths
/// through one program — each prefilled into its own sequence's state, then all four decoded a token each a step
/// together (`--batch`) — generate, sequence by sequence, the tokens and every logit, bit for bit, that the program gives
/// them run one after another (`--prompts`): for every tiny architecture, under every numerics — and so does the batch
/// step with its sequences' launches apart (`sequences` held), under the quantized numerics. The batch reports the rate
/// it decoded at, every sequence's tokens counted.
#[test]
fn a_batch_decodes_each_sequence_as_its_own_step_does() {
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let jobs: Vec<(Vec<u32>, usize)> = vec![(vec![5, 17, 2, 30, 41, 8, 63], 6), (vec![9, 1], 6), (vec![44, 13, 70, 2, 2], 6), (vec![7], 6)];
    let mut lanes = Vec::new();
    for &t in ALL {
        let spec = t.spec(CTX);
        let ckpt = dir().join(format!("batch-{}.gguf", t.name()));
        tiny_models::write_checkpoint(&spec, &ckpt);
        for numerics in [Q8MatKind::Dequant, Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
            let opts = LaneOptions { ctx: CTX, knobs: CudaKnobs { activations: numerics, ..CudaKnobs::default() }, prefill: 4 };
            let lane = compile_batch(&spec, &ckpt, &opts, jobs.len()).unwrap_or_else(|e| panic!("{} {numerics:?}: {e}", t.name()));
            lanes.push((t, numerics, spec.vocab, ckpt.clone(), lane, format!("batch-{}-{numerics:?}", t.name())));
            if numerics == Q8MatKind::Int8Act {
                let apart = compile_batch_holding(&spec, &ckpt, &opts, jobs.len(), &["sequences"]).unwrap_or_else(|e| panic!("{} apart: {e}", t.name()));
                lanes.push((t, numerics, spec.vocab, ckpt.clone(), apart, format!("batch-apart-{}-{numerics:?}", t.name())));
            }
        }
    }
    let built = build_all(&lanes.iter().map(|(_, _, _, _, lane, name)| (lane, name.as_str())).collect::<Vec<_>>());
    for ((t, numerics, vocab, ckpt, _, name), b) in lanes.iter().zip(&built) {
        let what = format!("{name} {} {numerics:?}", t.name());
        let alone = exec_many(b, ckpt, &jobs, *vocab, true);
        let together = exec_batch(b, ckpt, &jobs, *vocab, true);
        for (s, (one, all)) in alone.iter().zip(&together).enumerate() {
            assert_eq!(all.tokens, one.tokens, "{what}: sequence {s} decodes in the batch as it does alone");
            assert_eq!(all.logits, one.logits, "{what}: sequence {s}'s every logit, bit for bit");
            assert!(all.tok_per_s.is_some_and(|r| r > 0.0), "{what}: the batch reports its rate");
        }
    }
}

/// The `recurrent_state` knob at bf16 keeps every form of the step alike: a gated-delta model whose memories are stored
/// in bf16 between steps (the delta rule rounding each memory after every row it walks) continues a prompt run in chunks
/// of 8, 2 and 1 exactly as one fed a token at a time, and decodes each of a batch's sequences as it decodes alone,
/// tokens and every logit bit for bit, under the quantized and the precise numerics; under the precise numerics its
/// logits stay within 1e-2 of their scale of the reference's whose memory is rounded to bf16 at every step alike — the
/// distance one bf16 rounding the lane's `f32` arithmetic takes the other way moves them.
#[test]
fn a_bf16_recurrent_state_decodes_alike_in_every_form_and_near_its_reference() {
    use logicaffeine_compile::codegen_cuda::StatePrecision;
    use logicaffeine_compile::model::reference::StateStorage;
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let spec = Tiny::Qwen35.spec(CTX);
    let ckpt = dir().join("state-bf16-qwen35.gguf");
    tiny_models::write_checkpoint(&spec, &ckpt);
    let prompt: Vec<u32> = vec![5, 17, 2, 30, 41, 8, 63, 12, 77, 3, 90];
    let jobs: Vec<(Vec<u32>, usize)> = vec![(vec![5, 17, 2, 30, 41, 8, 63], 6), (vec![9, 1], 6), (vec![44, 13, 70, 2, 2], 6), (vec![7], 6)];
    for numerics in [Q8MatKind::Dequant, Q8MatKind::Int8Act] {
        let what = format!("bf16 state {numerics:?}");
        let knobs = CudaKnobs { activations: numerics, recurrent_state: StatePrecision::Bf16, ..CudaKnobs::default() };
        let lane = |prefill: usize| compile(&spec, &ckpt, &LaneOptions { ctx: CTX, knobs, prefill }).unwrap_or_else(|e| panic!("{what} prefill {prefill}: {e}"));
        let (one_at_a_time, chunked) = (lane(1), lane(8));
        let batch = compile_batch(&spec, &ckpt, &LaneOptions { ctx: CTX, knobs, prefill: 4 }, jobs.len()).unwrap_or_else(|e| panic!("{what} batch: {e}"));
        let built = build_all(&[
            (&one_at_a_time, &format!("state-bf16-1-{numerics:?}")),
            (&chunked, &format!("state-bf16-8-{numerics:?}")),
            (&batch, &format!("state-bf16-batch-{numerics:?}")),
        ]);
        let one = exec(&built[0], &ckpt, &prompt, 6, spec.vocab, true);
        let chunks = exec(&built[1], &ckpt, &prompt, 6, spec.vocab, true);
        assert_eq!(chunks.tokens, one.tokens, "{what}: the chunked prompt continues alike");
        assert_eq!(chunks.logits, one.logits, "{what}: bit for bit");
        let alone = exec_many(&built[2], &ckpt, &jobs, spec.vocab, true);
        let together = exec_batch(&built[2], &ckpt, &jobs, spec.vocab, true);
        for (s, (a, t)) in alone.iter().zip(&together).enumerate() {
            assert_eq!(t.tokens, a.tokens, "{what}: sequence {s} decodes in the batch as it does alone");
            assert_eq!(t.logits, a.logits, "{what}: sequence {s}'s every logit, bit for bit");
        }
        if numerics == Q8MatKind::Dequant {
            let mut reference = Reference::new(spec.clone(), Gguf::open(&ckpt).unwrap())
                .unwrap()
                .with_activations(Activations::Exact)
                .with_state_storage(StateStorage::Bf16);
            let mut want = reference.step(&prompt).unwrap();
            for (i, (got, &tok)) in one.logits.iter().zip(&one.tokens).enumerate() {
                let scale = want.iter().fold(1f64, |m, v| m.max(v.abs()));
                let diff = got.iter().zip(&want).map(|(g, w)| (*g as f64 - w).abs()).fold(0f64, f64::max) / scale;
                assert!(diff <= 1e-2, "{what} position {i}: the logits are {diff:.3e} of their scale from the bf16-state reference");
                if i + 1 < one.tokens.len() {
                    want = reference.step(&[tok]).unwrap();
                }
            }
        }
    }
}

/// The `cache_precision` knob at binary16 keeps every form of the step alike, on every architecture: a model whose
/// attention caches are stored in binary16 (each row rounded to the nearest as it is written, every read taking the
/// rounded row) continues a prompt run in chunks of 8, 2 and 1 exactly as one fed a token at a time, and decodes each of
/// a batch's sequences as it decodes alone, tokens and every logit bit for bit, under every numerics. Each run is held
/// to the reference whose caches keep binary16 rows alike as the `f32` cache's runs are held to theirs (`held`): within
/// its numerics' bound once the near calls it decided the other way are taken its way — a cached value its `f32`
/// arithmetic rounded to the other binary16 neighbour among them. Latent attention is not held: its first layer's cache
/// is the reference's but for a straddle or two, and each straddle moves every later layer's inputs, so the later
/// layers' caches part from the reference's by more than any set of near calls says. Under the precise numerics, where
/// the cache's rounding is all that sets the references apart, the run sits nearer the binary16 reference than the one
/// whose caches keep the rows as computed: the lane rounds every row it caches.
#[test]
fn an_f16_cache_decodes_alike_in_every_form_and_near_its_reference() {
    use logicaffeine_compile::codegen_cuda::CachePrecision;
    use logicaffeine_compile::model::reference::CACHE_WORD;
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let prompt: Vec<u32> = vec![5, 17, 2, 30, 41, 8, 63, 12, 77, 3, 90];
    let jobs: Vec<(Vec<u32>, usize)> = vec![(vec![5, 17, 2, 30, 41, 8, 63], 6), (vec![9, 1], 6), (vec![44, 13, 70, 2, 2], 6), (vec![7], 6)];
    let mut report = Vec::new();
    for &t in ALL {
        let spec = t.spec(CTX);
        let ckpt = dir().join(format!("cache-f16-{}.gguf", t.name()));
        tiny_models::write_checkpoint(&spec, &ckpt);
        for numerics in [Q8MatKind::Dequant, Q8MatKind::TwoPassInt8, Q8MatKind::Int8Act] {
            let what = format!("{} f16 cache {numerics:?}", t.name());
            let knobs = CudaKnobs { activations: numerics, cache_precision: CachePrecision::F16, ..CudaKnobs::default() };
            let lane = |prefill: usize| compile(&spec, &ckpt, &LaneOptions { ctx: CTX, knobs, prefill }).unwrap_or_else(|e| panic!("{what} prefill {prefill}: {e}"));
            let (one_at_a_time, chunked) = (lane(1), lane(8));
            let batch = compile_batch(&spec, &ckpt, &LaneOptions { ctx: CTX, knobs, prefill: 4 }, jobs.len()).unwrap_or_else(|e| panic!("{what} batch: {e}"));
            let tag = format!("cache-f16-{}-{numerics:?}", t.name());
            let built = build_all(&[(&one_at_a_time, &format!("{tag}-1")), (&chunked, &format!("{tag}-8")), (&batch, &format!("{tag}-batch"))]);
            let one = exec(&built[0], &ckpt, &prompt, 6, spec.vocab, true);
            let chunks = exec(&built[1], &ckpt, &prompt, 6, spec.vocab, true);
            assert_eq!(chunks.tokens, one.tokens, "{what}: the chunked prompt continues alike");
            assert_eq!(chunks.logits, one.logits, "{what}: bit for bit");
            let alone = exec_many(&built[2], &ckpt, &jobs, spec.vocab, true);
            let together = exec_batch(&built[2], &ckpt, &jobs, spec.vocab, true);
            for (s, (a, t)) in alone.iter().zip(&together).enumerate() {
                assert_eq!(t.tokens, a.tokens, "{what}: sequence {s} decodes in the batch as it does alone");
                assert_eq!(t.logits, a.logits, "{what}: sequence {s}'s every logit, bit for bit");
            }
            let latent = spec.layers.iter().any(|l| matches!(l.mixer, Mixer::Mla(_)));
            let (worst, kept) = if latent { (f64::NAN, Vec::new()) } else { held(&spec, &ckpt, &prompt, &one, numerics, KvStorage::F16, &what) };
            // The distance over the run from the reference keeping its caches' rows as `kv` stores them, the kept calls
            // taken.
            let distance = |kv: KvStorage| -> f64 {
                let mut reference = Reference::new(spec.clone(), Gguf::open(&ckpt).unwrap()).unwrap().with_activations(precision(numerics)).with_kv_storage(kv);
                for &c in &kept {
                    reference.flip(c);
                }
                let mut want = reference.step(&prompt).unwrap();
                let mut sum = 0f64;
                for (i, (got, &tok)) in one.logits.iter().zip(&one.tokens).enumerate() {
                    let scale = want.iter().fold(1f64, |m, v| m.max(v.abs()));
                    sum += got.iter().zip(&want).map(|(g, w)| (*g as f64 - w).abs()).fold(0f64, f64::max) / scale;
                    if i + 1 < one.tokens.len() {
                        want = reference.step(&[tok]).unwrap();
                    }
                }
                sum
            };
            if !latent {
                let cache = kept.iter().filter(|c| matches!(c, NearCall::Rounding(r) if r.word == CACHE_WORD)).count();
                report.push(format!("{what}: within {worst:.3e} of the binary16-cache reference, {} near calls decided the other way ({cache} cached values)", kept.len()));
            }
            // Under the precise numerics the cache's rounding is all that sets the two references apart.
            if numerics == Q8MatKind::Dequant {
                let (near, far) = (distance(KvStorage::F16), distance(KvStorage::Exact));
                assert!(near < far, "{what}: the run sits nearer the binary16-cache reference ({near:.3e} over its positions) than the exact one ({far:.3e})");
                report.push(format!("{what}: {near:.3e} from the binary16-cache reference over the run, {far:.3e} from the exact one"));
            }
        }
    }
    eprintln!("{}", report.join("\n"));
}

/// A block that takes a row whole weighs its chunks side by side (`lk_attn_side`) in exactly the operations it weighs
/// them one after another and the decode step's blocks weigh them apart: `lk_attention` with `held` chunks side by side
/// (every width `kernels::attn_side_chunks` gives, and two) writes, bit for bit, what it writes with one at a time and
/// what its split blocks write (each chunk a block, the last to arrive folding them). Grouped heads of 64-wide values
/// (eight side by side), Qwen3's 128 (four) and Gemma's 256 (two), one head of 32 (a row's every chunk at once); spans
/// of one chunk, of chunks whose last is short, and of chunks longer than a tile (several steps each); a window with
/// sinks; `f64` sums; a chunk's rows, and a batch's sequences each at its own position. Every output written (they
/// start as NaN).
#[test]
fn a_rows_chunks_weighed_side_by_side_compute_what_they_compute_one_after_another() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{attn_held_bytes, attn_side_bytes, attn_side_chunks, attn_smem_bytes, attn_values_per_thread, Kernel, ATTN_MIN_SPLIT, PRELUDE};
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    // (name, heads, kv heads, hd, vd, window, sinks, rows, positions (one a row for a batch, the first for a chunk),
    // splits, accumulator, a batch)
    type Case = (&'static str, usize, usize, usize, usize, usize, bool, usize, Vec<usize>, usize, &'static str, bool);
    let cases: Vec<Case> = vec![
        ("grouped, 64 wide, one chunk", 8, 2, 64, 64, 0, false, 1, vec![20], 7, "float", false),
        ("grouped, 64 wide, a short last chunk", 8, 2, 64, 64, 0, false, 1, vec![221], 7, "float", false),
        ("grouped, 64 wide, chunks past a tile", 8, 2, 64, 64, 0, false, 1, vec![1000], 4, "float", false),
        ("qwen3, 128 wide", 32, 8, 128, 128, 0, true, 1, vec![300], 6, "float", false),
        ("gemma, 256 wide, windowed", 8, 1, 256, 256, 160, false, 1, vec![700], 7, "float", false),
        ("one head of 32", 4, 4, 32, 32, 0, false, 1, vec![400], 9, "float", false),
        ("precise sums", 8, 2, 64, 64, 96, true, 1, vec![333], 7, "double", false),
        ("a chunk's rows", 8, 2, 64, 64, 0, false, 5, vec![190], 7, "float", false),
        ("a batch's sequences", 8, 2, 64, 64, 0, false, 4, vec![70, 260, 5, 199], 7, "float", true),
    ];
    let dir = dir().join("attn-side");
    std::fs::create_dir_all(&dir).unwrap();
    let unit = |i: usize, salt: u64| {
        let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(31) ^ 0x2545_F491_4F6C_DD1D;
        ((h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32
    };
    let write = |name: String, v: &[f32]| std::fs::write(dir.join(name), v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    let mut calls = String::new();
    // Each case: its name and widths, its outputs a run, and its runs (the split blocks', then each width's).
    let mut sizes: Vec<(String, usize, usize)> = Vec::new();
    for (k, (name, heads, kvh, hd, vd, win, sinks, rows, positions, splits, acc_ty, batch)) in cases.into_iter().enumerate() {
        let (group, acc) = (heads / kvh, if acc_ty == "double" { 8 } else { 4 });
        let g = group.min(4);
        let kvw = kvh * hd;
        let vw = kvh * vd;
        let ctx = positions.iter().max().unwrap() + rows + 1;
        let tables = if batch { rows } else { 1 };
        write(format!("q{k}.bin"), &(0..rows * heads * hd).map(|i| unit(i, 10 + k as u64)).collect::<Vec<f32>>());
        write(format!("k{k}.bin"), &(0..tables * ctx * kvw).map(|i| unit(i, 20 + k as u64)).collect::<Vec<f32>>());
        write(format!("v{k}.bin"), &(0..tables * ctx * vw).map(|i| unit(i, 30 + k as u64)).collect::<Vec<f32>>());
        write(format!("s{k}.bin"), &(0..heads).map(|i| unit(i, 40 + k as u64) * 2.0).collect::<Vec<f32>>());
        let vpt = attn_values_per_thread(vd);
        let scale = 1.0 / (hd as f64).sqrt();
        // A batch's tables are `kz` values apart, its keys' and values' alike: the wider of the two a table.
        let kz = if batch { ctx * kvw.max(vw) } else { 0 };
        let widths: Vec<usize> = {
            let mut w = vec![1, 2, attn_side_chunks(vd, splits)];
            w.sort_unstable();
            w.dedup();
            w
        };
        let list = positions.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
        let n = rows * heads * vd;
        let mut code = format!(
            "  {{ // {name}\n    {{ int p0[] = {{{list}}}; CK(cudaMemcpy(pos, p0, sizeof p0, cudaMemcpyHostToDevice)); }}\n\
             \x20   float* q = dev(dir + \"/q{k}.bin\"); float* kc = dev(dir + \"/k{k}.bin\"); float* vc = dev(dir + \"/v{k}.bin\"); float* sk = {};\n",
            if sinks { format!("dev(dir + \"/s{k}.bin\")") } else { "nullptr".into() }
        );
        if batch {
            // Each sequence's keys and values in its own table: the files laid `ctx` rows a table, the tables `kz` apart.
            let _ = writeln!(
                code,
                "    {{ float* kt = (float*)junk({rows}L * {kz} * 4); float* vt = (float*)junk({rows}L * {kz} * 4);\n\
                 \x20     for(int r = 0; r < {rows}; ++r){{ CK(cudaMemcpy(kt + (long)r * {kz}, kc + (long)r * {ctx} * {kvw}, {ctx}L * {kvw} * 4, cudaMemcpyDeviceToDevice));\n\
                 \x20       CK(cudaMemcpy(vt + (long)r * {kz}, vc + (long)r * {ctx} * {vw}, {ctx}L * {vw} * 4, cudaMemcpyDeviceToDevice)); }}\n\
                 \x20     kc = kt; vc = vt; }}"
            );
        }
        // The split blocks: each chunk a block, the last to arrive folding them.
        let base = attn_smem_bytes(g, hd, splits, acc);
        let _ = writeln!(
            code,
            "    float* o0 = nan({n});\n    {{ {acc_ty}* part = ({acc_ty}*)junk({}); int* cnt = (int*)junk({}); CK(cudaMemset(cnt, 0, {}));\n\
             \x20     CK(cudaFuncSetAttribute(lk_attention<{g}, {vpt}, {acc_ty}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {base}));\n\
             \x20     lk_attention<{g}, {vpt}, {acc_ty}><<<dim3({}, {rows}, {splits}), 256, {base}>>>(q, kc, vc, pos, sk, o0, part, cnt, {ctx}, {win}, {hd}, {vd}, {group}, {heads}, {kvw}, {vw}, 0, {scale:e}f, {ATTN_MIN_SPLIT}, {splits}, {kz}, 0); }}\n\
             \x20   CK(cudaGetLastError()); CK(cudaDeviceSynchronize()); put(out, o0, {n});",
            rows * heads * splits * (vd + 2) * acc,
            rows * heads * 4,
            rows * heads * 4,
            heads / g
        );
        for &w in &widths {
            let smem = base + attn_held_bytes(g, vd, splits, acc) + attn_side_bytes(g, w, acc);
            let _ = writeln!(
                code,
                "    {{ float* o = nan({n});\n\
                 \x20     CK(cudaFuncSetAttribute(lk_attention<{g}, {vpt}, {acc_ty}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {smem}));\n\
                 \x20     lk_attention<{g}, {vpt}, {acc_ty}><<<dim3({}, {rows}, 1), 256, {smem}>>>(q, kc, vc, pos, sk, o, nullptr, nullptr, {ctx}, {win}, {hd}, {vd}, {group}, {heads}, {kvw}, {vw}, 0, {scale:e}f, {ATTN_MIN_SPLIT}, {splits}, {kz}, {w});\n\
                 \x20     CK(cudaGetLastError()); CK(cudaDeviceSynchronize()); put(out, o, {n}); }}",
                heads / g
            );
        }
        code.push_str("  }\n");
        calls.push_str(&code);
        assert!(widths.len() > 1, "{name}: widths beside one at a time");
        sizes.push((format!("{name}, widths {widths:?}"), n, 1 + widths.len()));
    }
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <string>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {PRELUDE}{}\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ fprintf(stderr, \"cuda %d %s\\n\", __LINE__, cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         static float* dev(const std::string& p){{ FILE* f = fopen(p.c_str(), \"rb\"); if(!f) exit(2); std::vector<char> b; char c[65536]; size_t n;\n\
           while((n = fread(c, 1, sizeof c, f)) > 0) b.insert(b.end(), c, c + n); fclose(f);\n\
           float* d; CK(cudaMalloc(&d, b.size())); CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice)); return d; }}\n\
         static float* nan(long n){{ float* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0xff, n * 4)); return p; }}\n\
         static signed char* junk(long n){{ signed char* p; CK(cudaMalloc(&p, n)); CK(cudaMemset(p, 0xff, n)); return p; }}\n\
         static void put(FILE* out, const float* d, long n){{ std::vector<float> h(n); CK(cudaMemcpy(h.data(), d, n * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, n, out); }}\n\
         int main(int argc, char** argv){{\n  std::string dir = argv[1];\n  int* pos; CK(cudaMalloc(&pos, 64));\n\
           FILE* out = fopen(argv[2], \"wb\");\n{calls}  fclose(out);\n  return 0;\n}}\n",
        Kernel::Attention.source()
    );
    let (cu, bin, out) = (dir.join("side.cu"), dir.join("side"), dir.join("out.bin"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "the attention builds:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).arg(&dir).arg(&out).output().unwrap();
    assert!(ran.status.success(), "it runs: {}", String::from_utf8_lossy(&ran.stderr));
    let all: Vec<u32> = std::fs::read(&out).unwrap().chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    assert_eq!(all.len(), sizes.iter().map(|(_, n, runs)| n * runs).sum::<usize>(), "every case's outputs");
    let mut at = 0;
    for (name, n, runs) in &sizes {
        let apart = &all[at..at + n];
        assert!(apart.iter().all(|&x| f32::from_bits(x).is_finite()), "{name}: the split blocks write every output, finite");
        for run in 1..*runs {
            let side = &all[at + run * n..at + (run + 1) * n];
            if let Some(i) = (0..*n).find(|&i| side[i] != apart[i]) {
                panic!("{name}, run {run}: output {i} is {} apart and {} in one block", f32::from_bits(apart[i]), f32::from_bits(side[i]));
            }
        }
        at += n * runs;
    }
}

/// A tile's scores (`lk_attn_scores`) sum each (key, head) dot exactly as one warp sum of it does: the dot a lane's
/// share of the head at a time (`f32` products into the accumulator, the head's fours `lane, lane + 32, …`), then the
/// lanes paired across 16, 8, 4, 2 and 1 (`lk_wsum`), the lanes past the head adding zeros — so however many keys a
/// warp scores at once, and however its lanes share them, every score is the same bits. Against that formulation
/// written out here, a key and a head at a time: heads 4 to 576 wide (one four a head up to 144), one to sixteen heads
/// a block, `f32` and `f64` sums, `f32` and binary16 keys, a key row wider than the head with the head inside it,
/// tiles of one key to a whole tile, and values with exact zeros of both signs, keys of zeros of both signs, a key
/// whose products with a query cancel in pairs, a query head and a key so small their products underflow to zeros of
/// either sign, and a key whose every product with that head is a negative zero. Every (key, head) of the tile written exactly once, and nothing past it.
#[test]
fn a_tiles_scores_sum_each_dot_as_one_warp_sum_sums_it() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{Kernel, ATTN_TILE, PRELUDE};
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let widths = [4usize, 8, 12, 16, 20, 24, 32, 36, 48, 64, 80, 96, 128, 160, 192, 256, 576];
    let tiles = [1usize, 5, 37, ATTN_TILE];
    // (heads a block, accumulator, key element, threads a block)
    let cases: [(usize, &str, &str, usize); 18] = [
        (1, "float", "float", 256),
        (2, "float", "float", 256),
        (3, "float", "float", 256),
        (4, "float", "float", 256),
        (6, "float", "float", 256),
        (8, "float", "float", 256),
        (16, "float", "float", 256),
        (1, "double", "float", 256),
        (2, "double", "float", 256),
        (3, "double", "float", 256),
        (4, "double", "float", 256),
        (6, "double", "float", 256),
        (8, "double", "float", 256),
        (16, "double", "float", 256),
        (4, "float", "__half", 256),
        (8, "float", "__half", 256),
        (4, "float", "float", 128),
        (2, "double", "__half", 128),
    ];
    let mut runs = String::new();
    for (g, acc, ke, threads) in cases {
        let _ = writeln!(runs, "  check<{g}, {acc}, {ke}>(\"{g} heads, {acc} sums, {ke} keys, {threads} threads\", {threads}, totals);");
    }
    let width_list = widths.iter().map(|w| w.to_string()).collect::<Vec<_>>().join(", ");
    let tile_list = tiles.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(", ");
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <cstring>\n#include <cmath>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {PRELUDE}{}\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ fprintf(stderr, \"cuda %d %s\\n\", __LINE__, cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         // The dot of key `j` and head `g`, a key and a head at a time: each lane's share of the head, then one warp sum.\n\
         template<int G, typename A, typename KE>\n\
         __global__ void one_at_a_time(const float* q, const KE* kc, int n, int hd, int kvw, long koff, A* out){{\n\
           extern __shared__ __align__(16) float qs[];\n\
           for(int i = threadIdx.x; i < G * hd; i += blockDim.x) qs[i] = q[i];\n\
           __syncthreads();\n\
           const int lane = threadIdx.x & 31, warp = threadIdx.x >> 5, nw = blockDim.x >> 5, h4 = hd >> 2;\n\
           const float4* q4 = (const float4*)qs;\n\
           for(int j = warp; j < n; j += nw)\n\
             for(int g = 0; g < G; ++g){{\n\
               A d = 0;\n\
               for(int e = lane; e < h4; e += 32){{\n\
                 float4 k = lk_from4(kc + (long)j * kvw + koff + 4 * e), a = q4[g * h4 + e];\n\
                 d = lk_fma((A)a.x, (A)k.x, d); d = lk_fma((A)a.y, (A)k.y, d); d = lk_fma((A)a.z, (A)k.z, d); d = lk_fma((A)a.w, (A)k.w, d);\n\
               }}\n\
               A s = lk_wsum(d);\n\
               if(lane == 0) out[g * {ATTN_TILE} + j] = s;\n\
             }}\n\
         }}\n\
         // The tile scored as the attention scores it: every write counted, the keys past the tile's `n` read as rows past it.\n\
         template<int G, typename A, typename KE>\n\
         __global__ void as_the_tile(const float* q, const KE* kc, int n, int hd, int kvw, long koff, A* out, int* writes){{\n\
           extern __shared__ __align__(16) float qs[];\n\
           for(int i = threadIdx.x; i < G * hd; i += blockDim.x) qs[i] = q[i];\n\
           __syncthreads();\n\
           lk_attn_scores<G, A>(qs, kc, n, hd, kvw, koff, [&](int j){{ return j < n ? j : -1; }},\n\
                                [&](int j, int g, A s){{ out[g * {ATTN_TILE} + j] = s; atomicAdd(writes + g * {ATTN_TILE} + j, 1); }});\n\
         }}\n\
         static unsigned long long seed = 0x9E3779B97F4A7C15ull;\n\
         static float draw(){{\n\
           seed = seed * 6364136223846793005ull + 1442695040888963407ull;\n\
           unsigned r = (unsigned)(seed >> 33);\n\
           if(r % 10 == 0) return 0.f;\n\
           if(r % 10 == 1) return -0.f;\n\
           return ((int)((r >> 4) % 20001) - 10000) * 1e-4f * (1 + (r % 7));\n\
         }}\n\
         template<typename KE> KE elem(float v){{ return (KE)v; }}\n\
         template<> __half elem<__half>(float v){{ return __float2half_rn(v); }}\n\
         template<int G, typename A, typename KE>\n\
         static void check(const char* name, int threads, long* totals){{\n\
           const int widths[] = {{{width_list}}}, tiles[] = {{{tile_list}}};\n\
           for(int hd : widths){{\n\
             const int kvw = 3 * hd; const long koff = hd;\n\
             std::vector<float> q(G * hd); for(auto& v : q) v = draw();\n\
             for(int e = 0; e < hd; ++e) q[(G - 1) * hd + e] *= 1e-39f;\n\
             std::vector<KE> k({ATTN_TILE} * kvw);\n\
             for(int j = 0; j < {ATTN_TILE}; ++j) for(int e = 0; e < kvw; ++e){{\n\
               float v = draw();\n\
               if(j == 1) v = 0.f;\n\
               if(j == 2) v = -0.f;\n\
               if(j == 4) v *= 1e-6f;\n\
               if(j == 5 && e >= koff && e < koff + hd) v = -copysignf(1e-8f, q[(G - 1) * hd + e - koff]);\n\
               if(j == 3 && e >= koff && e < koff + hd){{ const int x = e - koff, h = hd / 2; v = x < h ? q[(j % G) * hd + x + h] : -q[(j % G) * hd + x - h]; }}\n\
               k[j * kvw + e] = elem<KE>(v);\n\
             }}\n\
             float* dq; KE* dk; A *ra, *ta; int* wr;\n\
             CK(cudaMalloc(&dq, q.size() * 4)); CK(cudaMalloc(&dk, k.size() * sizeof(KE)));\n\
             CK(cudaMalloc(&ra, G * {ATTN_TILE} * sizeof(A))); CK(cudaMalloc(&ta, G * {ATTN_TILE} * sizeof(A))); CK(cudaMalloc(&wr, G * {ATTN_TILE} * 4));\n\
             CK(cudaMemcpy(dq, q.data(), q.size() * 4, cudaMemcpyHostToDevice)); CK(cudaMemcpy(dk, k.data(), k.size() * sizeof(KE), cudaMemcpyHostToDevice));\n\
             for(int n : tiles){{\n\
               CK(cudaMemset(ra, 0xff, G * {ATTN_TILE} * sizeof(A))); CK(cudaMemset(ta, 0xff, G * {ATTN_TILE} * sizeof(A))); CK(cudaMemset(wr, 0, G * {ATTN_TILE} * 4));\n\
               one_at_a_time<G, A, KE><<<1, threads, G * hd * 4>>>(dq, dk, n, hd, kvw, koff, ra);\n\
               as_the_tile<G, A, KE><<<1, threads, G * hd * 4>>>(dq, dk, n, hd, kvw, koff, ta, wr);\n\
               CK(cudaGetLastError()); CK(cudaDeviceSynchronize());\n\
               std::vector<A> r(G * {ATTN_TILE}), t(G * {ATTN_TILE}); std::vector<int> w(G * {ATTN_TILE});\n\
               CK(cudaMemcpy(r.data(), ra, r.size() * sizeof(A), cudaMemcpyDeviceToHost)); CK(cudaMemcpy(t.data(), ta, t.size() * sizeof(A), cudaMemcpyDeviceToHost));\n\
               CK(cudaMemcpy(w.data(), wr, w.size() * 4, cudaMemcpyDeviceToHost));\n\
               for(int g = 0; g < G; ++g) for(int j = 0; j < {ATTN_TILE}; ++j){{\n\
                 const int at = g * {ATTN_TILE} + j;\n\
                 if(j >= n){{ if(w[at] != 0){{ printf(\"PAST %s hd %d n %d head %d key %d written %d times\\n\", name, hd, n, g, j, w[at]); ++totals[2]; }} continue; }}\n\
                 ++totals[0];\n\
                 if(w[at] != 1){{ printf(\"WRITES %s hd %d n %d head %d key %d: %d\\n\", name, hd, n, g, j, w[at]); ++totals[2]; }}\n\
                 if(memcmp(&r[at], &t[at], sizeof(A))){{ printf(\"BITS %s hd %d n %d head %d key %d: %.17g one at a time, %.17g as the tile\\n\", name, hd, n, g, j, (double)r[at], (double)t[at]); ++totals[1]; }}\n\
               }}\n\
             }}\n\
             CK(cudaFree(dq)); CK(cudaFree(dk)); CK(cudaFree(ra)); CK(cudaFree(ta)); CK(cudaFree(wr));\n\
           }}\n\
         }}\n\
         int main(){{\n  long totals[3] = {{0, 0, 0}};\n{runs}  printf(\"SCORES %ld APART %ld WRITES %ld\\n\", totals[0], totals[1], totals[2]);\n  return 0;\n}}\n",
        Kernel::Attention.source()
    );
    let dir = dir().join("attn-scores");
    std::fs::create_dir_all(&dir).unwrap();
    let (cu, bin) = (dir.join("scores.cu"), dir.join("scores"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "the scores build:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).output().unwrap();
    let stdout = String::from_utf8_lossy(&ran.stdout);
    assert!(ran.status.success(), "it runs:\n{stdout}\n{}", String::from_utf8_lossy(&ran.stderr));
    let expected: usize = cases.iter().map(|&(g, ..)| g * widths.len() * tiles.iter().sum::<usize>()).sum();
    let line = stdout.lines().find(|l| l.starts_with("SCORES ")).unwrap_or_else(|| panic!("a summary:\n{stdout}"));
    let field = |key: &str| -> usize {
        let at = line.split_whitespace().position(|w| w == key).unwrap_or_else(|| panic!("{key} in {line}"));
        line.split_whitespace().nth(at + 1).unwrap().parse().unwrap()
    };
    assert_eq!(field("SCORES"), expected, "every (key, head) of every case checked");
    assert_eq!(field("APART"), 0, "every score the same bits as one warp sum's:\n{stdout}");
    assert_eq!(field("WRITES"), 0, "every score written once, nothing past the tile:\n{stdout}");
}

/// Every attention kernel reads a binary16 cache exactly as it reads the `f32` cache of the same values — the cache the
/// `cache_precision` knob stores, each value widened exactly as it is read: a launch over binary16 keys, values and a
/// split key's rest (and over binary16 keys beside `f32` values) writes, bit for bit, what the same launch over the `f32`
/// arrays holding those values writes. Every kernel, at every path it takes: `lk_attention` over one chunk, its chunks a
/// block each, a row's chunks in its block with their partials held and not, a chunk's rows, a batch's sequences, precise
/// sums and three blocks a multiprocessor; `lk_attention_shared` folded and in one chunk, its values in the key rows and
/// in rows of their own; and `lk_attention_tiled`, `lk_attention_flash`, `lk_attention_mma` and `lk_kv_two_words` (before
/// `lk_attention_i8`) over grouped heads, a window with sinks, a latent cache, wide heads and long spans, and split, the
/// rotated part every head shares read from the cache as an expanded latent attention reads it. Every output written
/// (they start as NaN).
#[test]
fn every_attention_reads_a_binary16_cache_as_the_f32_cache_of_its_values() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{
        attn_flash_queries, attn_flash_smem_bytes, attn_flash_threads, attn_held_bytes, attn_i8_rows, attn_i8_smem_bytes, attn_mma_smem_bytes,
        attn_shared_smem_bytes, attn_smem_bytes, attn_tile_heads, attn_tile_smem_bytes, attn_tile_values, attn_values_per_thread, Kernel,
        ATTN_FOLD_PAIRS, ATTN_I8_QUERIES, ATTN_MIN_SPLIT, ATTN_MMA_QUERIES, ATTN_TILE_Q, PRELUDE,
    };
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let dir = dir().join("cache-f16-attention");
    std::fs::create_dir_all(&dir).unwrap();
    let unit = |i: usize, salt: u64| {
        let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(31) ^ 0x2545_F491_4F6C_DD1D;
        ((h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32
    };
    let mut files = 0usize;
    // `n` values of salt `salt` (times `by`) in a file the program reads onto the device.
    let mut file = |n: usize, salt: u64, by: f32| -> String {
        let name = format!("x{files}.bin");
        files += 1;
        std::fs::write(dir.join(&name), (0..n).flat_map(|i| (unit(i, salt) * by).to_le_bytes()).collect::<Vec<u8>>()).unwrap();
        format!("dev(dir + \"/{name}\")")
    };
    // A case's arrays: the positions, the queries `q` (and a split query's rest `qe`), the sinks `sk`, and the keys, the
    // values and a split key's rest each as binary16 (`kh`, `vh`, `th`) and as the `f32` of the same values (`kf`, ...);
    // values in the key rows are the key arrays themselves.
    struct Arrays {
        pos: Vec<usize>,
        q: usize,
        qe: usize,
        sinks: usize,
        k: usize,
        v: usize,
        t: usize,
    }
    let mut setup = |salt: u64, a: Arrays| -> String {
        let list = a.pos.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
        let mut s = format!("    {{ int p0[] = {{{list}}}; CK(cudaMemcpy(pos, p0, sizeof p0, cudaMemcpyHostToDevice)); }}\n");
        let _ = writeln!(s, "    float* q = {};", file(a.q, salt, 1.0));
        let _ = writeln!(s, "    float* qe = {};", if a.qe > 0 { file(a.qe, salt + 1, 1.0) } else { "nullptr".into() });
        let _ = writeln!(s, "    float* sk = {};", if a.sinks > 0 { file(a.sinks, salt + 2, 2.0) } else { "nullptr".into() });
        let _ = writeln!(s, "    __half* kh; float* kf; halves({}, {}, kh, kf);", file(a.k, salt + 3, 1.0), a.k);
        if a.v > 0 {
            let _ = writeln!(s, "    __half* vh; float* vf; halves({}, {}, vh, vf);", file(a.v, salt + 4, 1.0), a.v);
        } else {
            let _ = writeln!(s, "    __half* vh = kh; float* vf = kf;");
        }
        if a.t > 0 {
            let _ = writeln!(s, "    __half* th; float* tf; halves({}, {}, th, tf);", file(a.t, salt + 5, 1.0), a.t);
        }
        s
    };
    let mut calls = String::new();
    let mut cases: Vec<(String, usize)> = Vec::new();
    // A case: its arrays, its outputs, the arrays its kernel takes in binary16 (keys, values, a split key's rest), and its
    // launch over keys `k`, values `v` and rest `t` into `o`, `e` the element types as the kernel's last template
    // arguments — once over the `f32` arrays (the default types), once over the binary16 ones.
    let mut case = |name: String, arrays: String, outputs: usize, half: &[bool], launch: &dyn Fn(&str, &str, &str, &str, &str) -> String| {
        let pick = |i: usize, f: &'static str, h: &'static str| if half.get(i) == Some(&true) { h } else { f };
        let e: String = half.iter().map(|&h| if h { ", __half" } else { ", float" }).collect();
        let _ = write!(
            calls,
            "  {{ // {name}\n{arrays}    float *a = nan({outputs}), *b = nan({outputs});\n    {{ {} }}\n    CK(cudaGetLastError());\n    {{ {} }}\n\
             \x20   CK(cudaGetLastError()); CK(cudaDeviceSynchronize());\n    put(out, a, {outputs}); put(out, b, {outputs});\n  }}\n",
            launch("kf", "vf", "tf", "a", ""),
            launch(pick(0, "kf", "kh"), pick(1, "vf", "vh"), pick(2, "tf", "th"), "b", &e)
        );
        cases.push((name, outputs));
    };
    let mut salt = 100u64;
    let mut next = || {
        salt += 10;
        salt
    };
    // `lk_attention`: (name, heads, kv heads, hd, vd, window, sinks, values in the key rows, rows, cached, heads a block,
    // splits, path — 0 one chunk, 1 chunks a block each, 2 a row's chunks in its block held, 3 not held — the
    // accumulator, blocks a multiprocessor, binary16 values).
    let decode: [(&str, usize, usize, usize, usize, usize, bool, bool, usize, usize, usize, usize, u8, &str, usize, bool); 9] = [
        ("decode, one chunk", 8, 2, 64, 64, 0, false, false, 1, 70, 4, 1, 0, "float", 2, true),
        ("decode, chunks a block each, windowed with sinks", 8, 2, 64, 64, 300, true, false, 1, 400, 4, 4, 1, "float", 2, true),
        ("decode, chunks a block each, latent", 16, 1, 576, 512, 0, false, true, 1, 300, 4, 4, 1, "float", 2, true),
        ("chunk rows, chunks in the row's block, held", 8, 2, 64, 64, 0, false, false, 4, 400, 4, 4, 2, "float", 2, true),
        ("chunk rows, chunks in the row's block, not held", 8, 2, 64, 64, 0, true, false, 4, 400, 4, 4, 3, "float", 2, true),
        ("precise sums, chunks a block each", 8, 2, 64, 64, 0, true, false, 1, 400, 4, 4, 1, "double", 2, true),
        ("precise sums, held", 8, 2, 128, 128, 0, false, false, 4, 400, 2, 4, 2, "double", 2, true),
        ("three blocks a multiprocessor", 8, 2, 64, 64, 0, false, false, 1, 70, 4, 1, 0, "float", 3, true),
        ("values apart in f32, narrower", 8, 2, 64, 32, 0, false, false, 1, 70, 4, 1, 0, "float", 2, false),
    ];
    for (name, heads, kvh, hd, vd, win, sinks, latent, rows, cached, g, splits, path, acc_ty, mb, half_v) in decode {
        let (group, len, acc) = (heads / kvh, cached + rows, if acc_ty == "double" { 8 } else { 4 });
        let kvw = kvh * hd;
        let vw = if latent { kvw } else { kvh * vd };
        let held = path == 2;
        let smem = attn_smem_bytes(g, hd, splits, acc) + if held { attn_held_bytes(g, vd, splits, acc) } else { 0 };
        let z = if path == 1 { splits } else { 1 };
        let vpt = attn_values_per_thread(vd);
        let scale = 1.0 / (hd as f64).sqrt();
        let arrays = setup(next(), Arrays { pos: vec![cached], q: rows * heads * hd, qe: 0, sinks: if sinks { heads } else { 0 }, k: len * kvw, v: if latent { 0 } else { len * vw }, t: 0 });
        let launch = |k: &str, v: &str, _: &str, o: &str, e: &str| {
            let tail = if e.is_empty() && mb == 2 { String::new() } else { format!(", {mb}{e}") };
            let kernel = format!("lk_attention<{g}, {vpt}, {acc_ty}{tail}>");
            let (part, cnt) = if path == 1 {
                ("part".to_string(), "cnt".to_string())
            } else {
                ("nullptr".to_string(), "nullptr".to_string())
            };
            format!(
                "{acc_ty}* part = ({acc_ty}*)junk({}); int* cnt = (int*)junk({}); CK(cudaMemset(cnt, 0, {}));\n    \
                 CK(cudaFuncSetAttribute({kernel}, cudaFuncAttributeMaxDynamicSharedMemorySize, {smem}));\n    \
                 {kernel}<<<dim3({}, {rows}, {z}), 256, {smem}>>>(q, {k}, {v}, pos, sk, {o}, {part}, {cnt}, {len}, {win}, {hd}, {vd}, {group}, {heads}, {kvw}, {vw}, 0, {scale:e}f, {ATTN_MIN_SPLIT}, {splits}, 0, {});",
                rows * heads * splits * (vd + 2) * acc,
                rows * heads * 4,
                rows * heads * 4,
                heads / g,
                u8::from(held)
            )
        };
        case(format!("lk_attention, {name}"), arrays, rows * heads * vd, &[true, half_v], &launch);
    }
    // A batch's sequences, each its own caches `kz` values on from the last's and its own position.
    {
        let (heads, kvh, hd, vd, seqs, ctx, g) = (8usize, 2usize, 64usize, 64usize, 3usize, 160usize, 4usize);
        let (group, kvw, kz) = (heads / kvh, kvh * hd, ctx * kvh * hd);
        let vpt = attn_values_per_thread(vd);
        let smem = attn_smem_bytes(g, hd, 4, 4);
        let arrays = setup(next(), Arrays { pos: vec![70, 33, 151], q: seqs * heads * hd, qe: 0, sinks: 0, k: seqs * kz, v: seqs * kz, t: 0 });
        let launch = |k: &str, v: &str, _: &str, o: &str, e: &str| {
            let kernel = if e.is_empty() { format!("lk_attention<{g}, {vpt}, float>") } else { format!("lk_attention<{g}, {vpt}, float, 2{e}>") };
            format!(
                "CK(cudaFuncSetAttribute({kernel}, cudaFuncAttributeMaxDynamicSharedMemorySize, {smem}));\n    \
                 {kernel}<<<dim3({}, {seqs}, 1), 256, {smem}>>>(q, {k}, {v}, pos, nullptr, {o}, nullptr, nullptr, {ctx}, 0, {hd}, {vd}, {group}, {heads}, {kvw}, {kvw}, 0, {:e}f, {ATTN_MIN_SPLIT}, 4, {kz}, 0);",
                heads / g,
                1.0 / (hd as f64).sqrt()
            )
        };
        case("lk_attention, a batch's sequences".into(), arrays, seqs * heads * vd, &[true, true], &launch);
    }
    // `lk_attention_shared`: (name, heads, hd, vd, window, sinks, rows, cached, splits, values in the key rows).
    let shared: [(&str, usize, usize, usize, usize, bool, usize, usize, usize, bool); 3] = [
        ("latent, folded", 16, 576, 512, 0, false, 1, 1100, 17, true),
        ("tiny latent, one chunk, chunk rows", 4, 72, 64, 0, false, 4, 70, 1, true),
        ("values in rows of their own, windowed with sinks, folded", 8, 64, 32, 200, true, 1, 1100, 7, false),
    ];
    for (name, heads, hd, vd, win, sinks, rows, cached, splits, vk) in shared {
        let len = cached + rows;
        let vw = if vk { hd } else { vd };
        let smem = attn_shared_smem_bytes(heads, hd);
        let vpt = attn_values_per_thread(vd);
        let scale = 1.0 / (hd as f64).sqrt();
        let arrays = setup(next(), Arrays { pos: vec![cached], q: rows * heads * hd, qe: 0, sinks: if sinks { heads } else { 0 }, k: len * hd, v: if vk { 0 } else { len * vd }, t: 0 });
        let launch = |k: &str, v: &str, _: &str, o: &str, e: &str| {
            let kernel = format!("lk_attention_shared<{heads}, {vpt}, {}, {vk}{e}>", hd / 4);
            let mut code = format!(
                "float* part = (float*)junk({});\n    CK(cudaFuncSetAttribute({kernel}, cudaFuncAttributeMaxDynamicSharedMemorySize, {smem}));\n    \
                 {kernel}<<<dim3(1, {rows}, {splits}), 256, {smem}>>>(q, {k}, {v}, pos, sk, {o}, part, {len}, {win}, {vd}, {heads}, {hd}, {vw}, 0, {scale:e}f, {ATTN_MIN_SPLIT}, {splits});",
                rows * heads * splits * (vd + 2) * 4
            );
            if splits > 1 {
                let _ = write!(
                    code,
                    "\n    CK(cudaGetLastError());\n    lk_attention_fold<<<dim3({heads}, {rows}, {}), {ATTN_FOLD_PAIRS}, {}>>>(part, pos, sk, {o}, {len}, {win}, {vd}, {heads}, {ATTN_MIN_SPLIT}, {splits});",
                    vd.div_ceil(2 * ATTN_FOLD_PAIRS),
                    splits * 4
                );
            }
            code
        };
        case(format!("lk_attention_shared, {name}"), arrays, rows * heads * vd, &[true, true], &launch);
    }
    // The tile kernels: (name, heads, kv heads, hd, vd, window, sinks, values in the key rows, rows, cached).
    type Shape = (&'static str, usize, usize, usize, usize, usize, bool, bool, usize, usize);
    let grouped: Shape = ("grouped", 8, 2, 64, 64, 0, false, false, 40, 70);
    let windowed: Shape = ("windowed with sinks", 8, 2, 64, 64, 32, true, false, 40, 70);
    let long: Shape = ("qwen3, long", 32, 8, 128, 128, 0, true, false, 96, 400);
    let gemma: Shape = ("gemma, windowed", 4, 1, 256, 256, 64, false, false, 40, 70);
    let tiles: [(&str, Vec<Shape>); 4] = [
        ("tiled", vec![grouped, windowed, ("latent", 16, 1, 576, 512, 0, false, true, 40, 70), long]),
        ("flash", vec![grouped, windowed, gemma, ("tiny latent", 4, 1, 72, 64, 0, false, true, 40, 70), long]),
        ("mma", vec![grouped, windowed, gemma, ("small latent", 4, 1, 64, 48, 0, false, true, 40, 70), long]),
        ("two words", vec![grouped, windowed, gemma, ("small latent", 4, 1, 64, 32, 0, false, true, 40, 70), long]),
    ];
    // A tile kernel's launch over one shape, `split` the expanded latent's parts `(q's rest pitch, rope, k pitch, kw,
    // lora, own)` when it is split.
    let tile_launch = |which: &'static str, heads: usize, kvh: usize, hd: usize, vd: usize, win: usize, rows: usize, len: usize, vw: usize, split: Option<(usize, usize, usize, usize)>| {
        let (group, hg) = (heads / kvh, attn_tile_heads(heads / kvh));
        let kvw = kvh * split.map_or(hd, |(_, _, _, own)| own);
        let scale = 1.0 / (hd as f64).sqrt();
        move |k: &str, v: &str, t: &str, o: &str, e: &str| -> String {
            let parts = split.map_or(String::new(), |(rope, kw, lora, own)| format!(", qe, {hd}, {rope}, {t}, {kw}, {lora}, {own}"));
            let sk = if split.is_some() { "nullptr" } else { "sk" };
            match which {
                "tiled" => {
                    let kernel = format!("lk_attention_tiled<{hg}, {}{e}>", attn_tile_values(vd));
                    format!(
                        "CK(cudaFuncSetAttribute({kernel}, cudaFuncAttributeMaxDynamicSharedMemorySize, {}));\n    \
                         {kernel}<<<dim3({}, {}), 256, {}>>>(q, {k}, {v}, pos, {sk}, {o}, {len}, {win}, {hd}, {vd}, {group}, {heads}, {kvw}, {vw}, 0, {scale:e}f, {rows}{parts});",
                        attn_tile_smem_bytes(vd),
                        rows.div_ceil(ATTN_TILE_Q / hg),
                        heads / hg,
                        attn_tile_smem_bytes(vd)
                    )
                }
                "flash" => {
                    let (bq, nt, smem) = (attn_flash_queries(hd, vd), attn_flash_threads(hd, vd), attn_flash_smem_bytes(hd, vd));
                    let kernel = format!("lk_attention_flash<{hg}, {hd}, {vd}, {bq}, {nt}{e}>");
                    format!(
                        "CK(cudaFuncSetAttribute({kernel}, cudaFuncAttributeMaxDynamicSharedMemorySize, {smem}));\n    \
                         {kernel}<<<dim3({}, {}), {nt}, {smem}>>>(q, {k}, {v}, pos, {sk}, {o}, {len}, {win}, {group}, {heads}, {kvw}, {vw}, 0, {scale:e}f, {rows}{parts});",
                        rows.div_ceil(bq / hg),
                        heads / hg
                    )
                }
                "mma" => {
                    let smem = attn_mma_smem_bytes(hd, vd);
                    let kernel = format!("lk_attention_mma<{hg}, {hd}, {vd}{e}>");
                    format!(
                        "CK(cudaFuncSetAttribute({kernel}, cudaFuncAttributeMaxDynamicSharedMemorySize, {smem}));\n    \
                         {kernel}<<<dim3({}, {}), 128, {smem}>>>(q, {k}, {v}, pos, {sk}, {o}, {len}, {win}, {group}, {heads}, {kvw}, {vw}, 0, {scale:e}f, {rows}{parts});",
                        rows.div_ceil(ATTN_MMA_QUERIES / hg),
                        heads / hg
                    )
                }
                _ => {
                    let lpad = attn_i8_rows(len);
                    let words = kvh * lpad;
                    let kparts = split.map_or(String::new(), |(_, kw, lora, own)| format!(", {t}, {kw}, {lora}, {own}"));
                    let qparts = split.map_or(String::new(), |(rope, _, _, own)| format!(", qe, {hd}, {rope}, {own}"));
                    format!(
                        "signed char* k8 = junk({}); signed char* w8 = junk({}); float* s8 = (float*)junk({});\n    \
                         lk_kv_two_words<{hd}, {vd}{e}><<<dim3({}, {kvh}), 256>>>({k}, {v}, pos, {rows}, {len}, {kvw}, {vw}, 0, {lpad}, k8, k8 + {}, s8, w8, w8 + {}, s8 + {words}{kparts});\n    \
                         CK(cudaGetLastError());\n    \
                         lk_attention_i8<{hg}, {hd}, {vd}><<<dim3({}, {}), 128, {}>>>(q, k8, k8 + {}, s8, w8, w8 + {}, s8 + {words}, {lpad}, pos, {sk}, {o}, {len}, {win}, {group}, {heads}, {scale:e}f, {rows}{qparts});",
                        2 * words * hd,
                        2 * words * vd,
                        2 * words * 4,
                        lpad / 32,
                        words * hd,
                        words * vd,
                        rows.div_ceil(ATTN_I8_QUERIES / hg),
                        heads / hg,
                        attn_i8_smem_bytes(hd, vd),
                        words * hd,
                        words * vd
                    )
                }
            }
        }
    };
    for (which, shapes) in &tiles {
        for &(name, heads, kvh, hd, vd, win, sinks, latent, rows, cached) in shapes {
            let len = cached + rows;
            let kvw = kvh * hd;
            let vw = if latent { kvw } else { kvh * vd };
            let arrays = setup(next(), Arrays { pos: vec![cached], q: rows * heads * hd, qe: 0, sinks: if sinks { heads } else { 0 }, k: len * kvw, v: if latent { 0 } else { len * vw }, t: 0 });
            let launch = tile_launch(*which, heads, kvh, hd, vd, win, rows, len, vw, None);
            case(format!("{which}, {name}"), arrays, rows * heads * vd, &[true, true, false], &launch);
        }
        // Split: eight heads a group each, each query its own 128 and its rotated 64 apart; each key its head's own 128
        // (expanded, `f32`) and the rotated 64 every head shares at 512 in a 576-wide cache row, read in binary16.
        let (h, own, rope, vd, rows, cached, kw, lora) = (8usize, 128usize, 64usize, 128usize, 40usize, 70usize, 576usize, 512usize);
        let (qk, len) = (own + rope, cached + rows);
        let arrays = setup(next(), Arrays { pos: vec![cached], q: rows * h * qk, qe: rows * h * rope, sinks: 0, k: len * h * own, v: len * h * vd, t: len * kw });
        let launch = tile_launch(*which, h, h, qk, vd, 0, rows, len, h * vd, Some((rope, kw, lora, own)));
        case(format!("{which}, split"), arrays, rows * h * vd, &[false, false, true], &launch);
    }
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <string>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {PRELUDE}{}{}{}{}{}{}{}\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ fprintf(stderr, \"cuda %d %s\\n\", __LINE__, cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         static float* dev(const std::string& p){{ FILE* f = fopen(p.c_str(), \"rb\"); if(!f) exit(2); std::vector<char> b; char c[65536]; size_t n;\n\
           while((n = fread(c, 1, sizeof c, f)) > 0) b.insert(b.end(), c, c + n); fclose(f);\n\
           float* d; CK(cudaMalloc(&d, b.size())); CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice)); return d; }}\n\
         static float* nan(long n){{ float* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0xff, n * 4)); return p; }}\n\
         static signed char* junk(long n){{ signed char* p; CK(cudaMalloc(&p, n)); CK(cudaMemset(p, 0xff, n)); return p; }}\n\
         static void put(FILE* out, const float* d, long n){{ std::vector<float> h(n); CK(cudaMemcpy(h.data(), d, n * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, n, out); }}\n\
         // `x` rounded to binary16: `h` its values, `f` the same values as `f32`.\n\
         __global__ void round16(const float* x, __half* h, float* f, long n){{\n\
           long i = (long)blockIdx.x * 256 + threadIdx.x; if(i >= n) return; __half v = __float2half_rn(x[i]); h[i] = v; f[i] = __half2float(v);\n\
         }}\n\
         static void halves(const float* x, long n, __half*& h, float*& f){{\n\
           CK(cudaMalloc(&h, n * 2)); CK(cudaMalloc(&f, n * 4)); round16<<<(n + 255) / 256, 256>>>(x, h, f, n); CK(cudaGetLastError());\n\
         }}\n\
         int main(int argc, char** argv){{\n  std::string dir = argv[1];\n  int* pos; CK(cudaMalloc(&pos, 64));\n\
           FILE* out = fopen(argv[2], \"wb\");\n{calls}  fclose(out);\n  return 0;\n}}\n",
        Kernel::Mma.source(),
        Kernel::Attention.source(),
        Kernel::AttentionShared.source(),
        Kernel::AttentionTiled.source(),
        Kernel::AttentionFlash.source(),
        Kernel::AttentionMma.source(),
        Kernel::AttentionI8.source()
    );
    let (cu, bin, out) = (dir.join("attend.cu"), dir.join("attend"), dir.join("out.bin"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "every attention builds over both caches:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).arg(&dir).arg(&out).output().unwrap();
    assert!(ran.status.success(), "they run: {}", String::from_utf8_lossy(&ran.stderr));
    let all: Vec<u32> = std::fs::read(&out).unwrap().chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    assert_eq!(all.len(), cases.iter().map(|(_, n)| 2 * n).sum::<usize>(), "every case's outputs");
    let mut at = 0;
    for (name, n) in &cases {
        let (a, b) = (&all[at..at + n], &all[at + n..at + 2 * n]);
        at += 2 * n;
        assert!(a.iter().all(|&x| f32::from_bits(x).is_finite()), "{name}: every output written over the f32 cache, and finite");
        if let Some(i) = (0..*n).find(|&i| a[i] != b[i]) {
            panic!("{name}: output {i} is {} over the f32 cache and {} over the binary16 one", f32::from_bits(a[i]), f32::from_bits(b[i]));
        }
    }
    eprintln!("{} cases, every output alike bit for bit", cases.len());
}

/// Every kernel that writes a cache's rows stores into a binary16 cache exactly its `f32` result rounded to the nearest:
/// written through `lk_out`, the launch into a binary16 table leaves, bit for bit, what the same launch into an `f32`
/// table leaves rounded to binary16 — every row it writes, and nothing it does not. The Q8_0 GEMV under each numerics
/// over one and three rows; the block-format GEMV (Q4_0) a row and four rows a warp; the merged projections beside an
/// `f32` part, on the GEMV and on the tensor cores (`lk_gemv_mma`); the tensor-core GEMM on its streamed path and its
/// block path; the `f32` GEMV (one row cut over a block's warps, rows four at a time) and GEMM (whole tiles and cut along
/// `k`); the norm into rows; the rotation, normed and not, alone and beside an `f32` side, and a batch's rotations each
/// into its own table; a row set at a position, at a fixed row and a batch's rows into their tables; the join at a
/// position and a batch's; and the per-head matmuls — output rows, input rows, staged, tiled both ways. And the readers
/// a cache feeds besides the attentions: a slice and a tiled per-head matmul over a binary16 cache read what they read
/// over the `f32` cache of its values.
#[test]
fn every_cache_writer_stores_its_f32_result_rounded_into_a_binary16_cache() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{
        axpy_pitch, gemm_blk_launch_smem, gemm_blk_launch_threads, gemm_blk_smem_bytes, gemv_mma_launch, gemv_mma_smem_bytes, gemv_mma_threads,
        norm_threads, Kernel, AXPY_R, AXPY_SUMS, AXPY_W, GEMM_BLK_TILE_M, GEMM_TILE_N, GEMV_F32_ROWS, HEAD_TILE_M, PRELUDE,
    };
    use logicaffeine_compile::model::quant::GgmlType;
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let mut calls = String::new();
    let mut cases = 0usize;
    // A writer: its arrays, its table's length, and its launch into `{Y}` — once an `f32` table, once a binary16 one.
    let mut writer = |name: String, setup: String, table: String, launch: String| {
        let _ = writeln!(
            calls,
            "  {{ // {name}\n{setup}\n    const long TN = {table}; float* tf = zeros32(TN); __half* th = zeros16(TN);\n    {{ {} }}\n    CK(cudaGetLastError());\n    {{ {} }}\n    CK(cudaGetLastError()); same(\"{name}\", tf, th, TN);\n  }}",
            launch.replace("{Y}", "tf"),
            launch.replace("{Y}", "th")
        );
        cases += 1;
    };
    let quantized = |q: u32, rows: &str, k: &str| -> String {
        format!(
            "    float* x = falloc((long){rows} * {k}); fill_f<<<({rows} * {k} + 255) / 256, 256>>>(x, (long){rows} * {k}, 7);\n\
             \x20   signed char *xq = calloc8((long){rows} * {k}), *xl = calloc8((long){rows} * {k}); float* xs = falloc((long){rows} * {k} / 32);\n{}",
            if q > 0 { format!("    lk_quant<{q}><<<({rows} * {k} + 255) / 256, 256>>>(x, xq, {}, xs, (long){rows} * {k});\n", if q == 2 { "xl" } else { "nullptr" }) } else { String::new() }
        )
    };
    let acts = |q: u32| if q == 0 { "x, nullptr, nullptr, nullptr" } else if q == 1 { "nullptr, xq, nullptr, xs" } else { "nullptr, xq, xl, xs" };
    let q8 = |name: &str, n: &str, k: &str, seed: u32| {
        format!("    signed char* {name}q = calloc8((long){n} * {k}); __half* {name}s = (__half*)calloc8((long){n} * {k} / 16); fill_w<<<({n} * {k} + 255) / 256, 256>>>({name}q, {name}s, (long){n} * {k}, {seed});\n")
    };
    // The Q8_0 GEMV, each numerics, one and three rows, into rows from position 6.
    for q in 0..3u32 {
        for m in [1usize, 3] {
            let setup = format!("    const int K = 512, N = 64, M = {m};\n{}{}    float* bs = bias(N, 3);", quantized(q, "M", "K"), q8("w", "N", "K", 11));
            writer(
                format!("lk_gemv_q8 Q{q}, {m} rows"),
                setup,
                "(5 + M) * N".into(),
                format!("lk_gemv_q8<{q}, M><<<(N + 7) / 8, 256>>>(wq, ws, {}, bs, {{Y}}, rowat(6), N, K, N);", acts(q)),
            );
        }
    }
    // The block-format GEMV (Q4_0): a row a warp over three rows, four rows a warp over one.
    for (m, w) in [(3usize, 1usize), (1, 4)] {
        let setup = format!(
            "    const int K = 512, N = 64, M = {m};\n{}    unsigned char* wb = (unsigned char*)calloc8((long)N * (K / 32) * 18); fill_q40<<<(N * (K / 32) + 255) / 256, 256>>>(wb, (long)N * (K / 32), 13);\n    float* bs = bias(N, 5);",
            quantized(1, "M", "K")
        );
        let (args, grid) = if w == 1 { (String::new(), "(N + 7) / 8, 256".to_string()) } else { (format!(", {w}, 0"), format!("(N + {}) / {}, 128", 4 * w - 1, 4 * w)) };
        writer(
            format!("lk_gemv_blk Q4_0, {m} rows, {w} rows a warp"),
            setup,
            "(5 + M) * N".into(),
            format!("lk_gemv_blk<2, 1, M{args}><<<{grid}>>>(lk_w{{nullptr, nullptr, wb, (long)(K / 32) * 18}}, {}, bs, {{Y}}, rowat(6), N, K, N, nullptr, nullptr, nullptr);", acts(1)),
        );
    }
    // The merged projections: an `f32` part of 32 outputs, then 64 into the table's rows.
    for q in 0..3u32 {
        let setup = format!(
            "    const int K = 512, M = 2;\n{}{}{}    float* own = zeros32(2 * 32);",
            quantized(q, "M", "K"),
            q8("a", "32", "K", 17),
            q8("b", "64", "K", 19)
        );
        writer(
            format!("lk_gemv_parts Q{q}"),
            setup,
            "(5 + M) * 64".into(),
            format!(
                "lk_parts parts{{}}; parts.w[0] = lk_w{{aq, as, nullptr, 0}}; parts.w[1] = lk_w{{bq, bs, nullptr, 0}};\n    \
                 parts.end[0] = 32; parts.end[1] = parts.end[2] = parts.end[3] = 96; parts.y[0] = own; parts.y[1] = {{Y}}; parts.row[1] = rowat(6); parts.pitch[1] = 64;\n    \
                 lk_gemv_parts<8, {q}, M><<<12, 256>>>({}, K, 96, parts);",
                acts(q)
            ),
        );
    }
    // The batch's GEMV on the tensor cores, twenty rows: an `f32` part of 64, then 64 into the table's rows.
    for q in 1..3u32 {
        let (mt, nt, mw, nw) = gemv_mma_launch(20, 128, 512, 28);
        let smem = gemv_mma_smem_bytes(q == 2, mt, nt);
        let setup = format!(
            "    const int K = 512, M = 20, N = 128;\n{}{}{}    float* own = zeros32(20 * 64);",
            quantized(q, "M", "K"),
            q8("a", "64", "K", 23),
            q8("b", "64", "K", 29)
        );
        writer(
            format!("lk_gemv_mma Q{q}"),
            setup,
            "(5 + M) * 64".into(),
            format!(
                "lk_parts parts{{}}; parts.w[0] = lk_w{{aq, as, nullptr, 0}}; parts.w[1] = lk_w{{bq, bs, nullptr, 0}};\n    \
                 parts.end[0] = 64; parts.end[1] = parts.end[2] = parts.end[3] = 128; parts.y[0] = own; parts.y[1] = {{Y}}; parts.row[1] = rowat(6); parts.pitch[1] = 64;\n    \
                 CK(cudaFuncSetAttribute(lk_gemv_mma<{q}, {mt}, {nt}, {mw}, {nw}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {smem}));\n    \
                 lk_gemv_mma<{q}, {mt}, {nt}, {mw}, {nw}><<<{}, {}, {smem}>>>(xq, {}, xs, K, N, M, parts);",
                128usize.div_ceil(8 * nt),
                gemv_mma_threads(mt, nt, mw, nw),
                if q == 2 { "xl" } else { "nullptr" }
            ),
        );
    }
    // The tensor-core GEMM over forty rows: its streamed path (`k` whole scale groups) and its block path.
    for q in 1..3u32 {
        for k in [256usize, 160] {
            let bm = GEMM_BLK_TILE_M;
            let (threads, smem, attr) = (gemm_blk_launch_threads(8, bm, k), gemm_blk_launch_smem(8, q == 2, bm, k), gemm_blk_smem_bytes(8, q == 2, bm));
            let setup = format!("    const int K = {k}, N = 128, M = 40;\n{}{}    float* bs = bias(N, 31);", quantized(q, "M", "K"), q8("w", "N", "K", 37));
            writer(
                format!("lk_gemm_blk Q{q}, k {k}"),
                setup,
                "(5 + M) * N".into(),
                format!(
                    "CK(cudaFuncSetAttribute(lk_gemm_blk<8, {q}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {attr}));\n    \
                     lk_gemm_blk<8, {q}><<<dim3({}, {}), {threads}, {smem}>>>(lk_w{{wq, ws, nullptr, 0}}, xq, {}, xs, bs, {{Y}}, rowat(6), N, K, N, M);",
                    128usize.div_ceil(GEMM_TILE_N),
                    40usize.div_ceil(bm),
                    if q == 2 { "xl" } else { "nullptr" }
                ),
            );
        }
    }
    // The `f32` GEMV: one row cut over a block's warps, and three rows four at a time.
    for (m, split) in [(1usize, 4usize), (3, 1)] {
        let setup = format!(
            "    const int K = 256, N = 64, M = {m};\n    float* w = falloc((long)N * K); fill_f<<<(N * K + 255) / 256, 256>>>(w, (long)N * K, 41);\n\
             \x20   float* x = falloc((long)M * K); fill_f<<<(M * K + 255) / 256, 256>>>(x, (long)M * K, 43);\n    float* bs = bias(N, 47);"
        );
        writer(
            format!("lk_gemv_f32, {m} rows, {split} parts"),
            setup,
            "(5 + M) * N".into(),
            format!("lk_gemv_f32<<<dim3({}, {}), 256>>>(w, x, bs, {{Y}}, rowat(6), N, K, N, M{});", 64usize.div_ceil(8 / split), m.div_ceil(GEMV_F32_ROWS), if split > 1 { format!(", {split}") } else { String::new() }),
        );
    }
    // The `f32` GEMM over seventy rows: whole tiles, and each tile's `k` cut in two.
    for split in [1usize, 2] {
        let setup = "    const int K = 96, N = 128, M = 70;\n    float* w = falloc((long)N * K); fill_f<<<(N * K + 255) / 256, 256>>>(w, (long)N * K, 53);\n\
                     \x20   float* x = falloc((long)M * K); fill_f<<<(M * K + 255) / 256, 256>>>(x, (long)M * K, 59);\n    float* bs = bias(N, 61);\n\
                     \x20   float* part = zeros32(8L * 128 * 128); int* cnt = (int*)calloc8(64);"
            .to_string();
        writer(
            format!("lk_gemm_f32, {split} parts"),
            setup,
            "(5 + M) * N".into(),
            format!("lk_gemm_f32<<<dim3(2, 2, {split}), 256>>>(x, w, bs, {{Y}} + 5L * N, M, N, K{});", if split > 1 { format!(", {split}, part, cnt") } else { String::new() }),
        );
    }
    // The norm, two rows of 256 into the table's row at position 6, a row's two halves.
    {
        let threads = norm_threads(256);
        let setup = "    const int C = 256;\n    float* x = falloc(2 * C); fill_f<<<2, 256>>>(x, 2 * C, 67);\n    float* nw = falloc(C); fill_f<<<1, 256>>>(nw, C, 71);".to_string();
        writer(
            "lk_rmsnorm into a row".into(),
            setup,
            "6L * 2 * C".into(),
            format!("lk_rmsnorm<false, false, true, 0, {}, float><<<2, {threads}>>>(x, nullptr, nullptr, nw, {{Y}}, nullptr, nullptr, nullptr, C, 1e-6f, C, 0, rowat(6), 2 * C, C, 0);", 256usize.div_ceil(threads)),
        );
    }
    // The rotation: four heads of 64 into the table's row at position 6, normed and not.
    let rope_setup = "    const int H = 4, HD = 64, HALF = 32, P = 16;\n    float* x = falloc(16L * H * HD); fill_f<<<16, 256>>>(x, 16L * H * HD, 73);\n\
                      \x20   float* nw = falloc(HD); fill_f<<<1, 256>>>(nw, HD, 79);\n    float* cs = falloc((long)P * HALF); fill_f<<<2, 256>>>(cs, (long)P * HALF, 83);\n\
                      \x20   float* sn = falloc((long)P * HALF); fill_f<<<2, 256>>>(sn, (long)P * HALF, 89);\n    const int* at = rowat(7);"
        .to_string();
    for norm in [false, true] {
        let threads = if norm { norm_threads(64) } else { 128 };
        writer(
            format!("lk_rope{}", if norm { ", normed" } else { "" }),
            rope_setup.clone(),
            "6L * H * HD".into(),
            format!("lk_rope<{norm}, float><<<H, {threads}>>>(x, nw, cs, sn, at, {{Y}}, rowat(6), H * HD, HD, HALF, H, 1e-6f, HD, 0, HD, 0);"),
        );
    }
    // Two rotations in one launch: eight query heads into an `f32` buffer, two key heads into the table's row; and a
    // batch's three sequences, each sequence's key heads into its own table at its own row.
    writer(
        "lk_rope_parts beside an f32 side".into(),
        format!("{rope_setup}\n    float* own = zeros32(8L * HD);"),
        "6L * 2 * HD".into(),
        format!(
            "lk_rope_parts<true, float><<<10, {}>>>(lk_rope_side{{x, nw, own, nullptr, 0, 1e-6f, HD, 0, HD, 0, 8, 8, 0}}, \
             lk_rope_side{{x + 8L * HD, nw, {{Y}}, rowat(6), 2 * HD, 1e-6f, HD, 0, HD, 0, 2, 2, 0}}, cs, sn, at, HD, HALF, 0);",
            norm_threads(64)
        ),
    );
    writer(
        "lk_rope_parts, a batch's tables".into(),
        format!("{rope_setup}\n    float* own = zeros32(3L * 8 * HD); const int* where = rows3(4, 9, 2); const int* pos3 = rows3(5, 11, 3);"),
        "3L * 12 * 2 * HD".into(),
        "lk_rope_parts<false, float><<<30, 128>>>(lk_rope_side{x, nullptr, own, nullptr, 0, 0.f, HD, 0, HD, 0, 24, 8, 0}, \
         lk_rope_side{x + 24L * HD, nullptr, {Y}, where, 2 * HD, 0.f, HD, 0, HD, 0, 6, 2, 12L * 2 * HD}, cs, sn, pos3, HD, HALF, 1);"
            .into(),
    );
    // A row set at a position, at a fixed row, and a batch's rows each into its own table at its own row.
    let rows_setup = "    const int C = 96;\n    float* src = falloc(3L * C); fill_f<<<2, 256>>>(src, 3L * C, 97); const int* where = rows3(4, 9, 2);".to_string();
    writer("lk_set_row at a position".into(), rows_setup.clone(), "12L * C".into(), "lk_set_row<<<1, 256>>>({Y}, rowat(6), 0, src, 2L * C, C);".into());
    writer("lk_set_row at a fixed row".into(), rows_setup.clone(), "12L * C".into(), "lk_set_row<<<1, 256>>>({Y}, nullptr, 3, src, C, C);".into());
    writer("lk_set_row, a batch's tables".into(), rows_setup, "3L * 12 * C".into(), "lk_set_row<<<2, 256>>>({Y}, where, 0, src, 3L * C, C, 12L * C);".into());
    // A join at a position, and a batch's: four units of 48 and 16 a row.
    let join_setup = "    float* a = falloc(3L * 4 * 48); fill_f<<<3, 256>>>(a, 3L * 4 * 48, 101);\n    float* b = falloc(3L * 4 * 16); fill_f<<<1, 256>>>(b, 3L * 4 * 16, 103);\n\
                      \x20   const int* where = rows3(4, 9, 2);"
        .to_string();
    writer("lk_join at a position".into(), join_setup.clone(), "12L * 256".into(), "lk_join<<<1, 256>>>(a, b, {Y}, rowat(6), 256, 256, 48, 16);".into());
    writer("lk_join, a batch's tables".into(), join_setup, "3L * 12 * 256".into(), "lk_join<<<3, 256>>>(a, b, {Y}, where, 256, 3L * 256, 48, 16, 4, 12L * 256);".into());
    // The per-head matmuls over four heads of 64 by 64, into the table's rows from position 6.
    let heads_setup = |rows: usize| {
        format!(
            "    const int H = 4, A = 64, B = 64, M = {rows};\n{}    float* x = falloc((long)M * H * A); fill_f<<<(M * H * A + 255) / 256, 256>>>(x, (long)M * H * A, 107);",
            q8("w", "(H * B)", "A", 109)
        )
    };
    writer(
        "lk_head_gemv".into(),
        heads_setup(2),
        "(5L + M) * H * B".into(),
        "lk_head_gemv<8, 0, M><<<(H * B + 7) / 8, 256>>>(lk_w{wq, ws, nullptr, 0}, x, nullptr, nullptr, nullptr, {Y}, H, A, B, A, 0, rowat(6), H * B, B, 0);".into(),
    );
    writer(
        "lk_head_axpy".into(),
        heads_setup(2),
        "(5L + M) * H * B".into(),
        "lk_head_axpy<8, 0, M><<<dim3(B / 32, H), 256>>>(lk_w{wq, ws, nullptr, 0}, x, nullptr, nullptr, nullptr, {Y}, H, A, B, A, 0, rowat(6), H * B, B, 0);".into(),
    );
    {
        let pitch = axpy_pitch(GgmlType::Q8_0);
        let tile = 64usize.min((48 * 1024 - AXPY_SUMS) / pitch);
        writer(
            "lk_head_axpy_staged".into(),
            heads_setup(2),
            "(5L + M) * H * B".into(),
            format!(
                "lk_head_axpy_staged<8, 3, M, {AXPY_W}, {AXPY_R}><<<dim3({}, H), {}, {}>>>(lk_w{{wq, ws, nullptr, 0}}, x, nullptr, nullptr, nullptr, {{Y}}, H, A, B, A, 0, rowat(6), H * B, B, 0, {tile});",
                (64 / 32usize).div_ceil(AXPY_W),
                32 * AXPY_W * AXPY_R,
                tile * pitch
            ),
        );
    }
    for out in [true, false] {
        writer(
            format!("lk_head_tiled, {} rows", if out { "output" } else { "input" }),
            heads_setup(20),
            "(5L + M) * H * B".into(),
            format!(
                "lk_head_tiled<8, 3, {out}><<<dim3(1, {}, H), 256>>>(lk_w{{wq, ws, nullptr, 0}}, nullptr, nullptr, nullptr, {{Y}}, H, A, B, A, 0, rowat(6), H * B, B, 0, M, x, 0);",
                20usize.div_ceil(HEAD_TILE_M)
            ),
        );
    }
    // The readers: a slice of every row's first 48 of 64, and a tiled per-head matmul whose activation is the cache.
    let _ = writeln!(
        calls,
        "  {{ // lk_slice over a binary16 cache\n    float* c = falloc(20L * 64); fill_f<<<5, 256>>>(c, 20L * 64, 113); float* cf; __half* ch; halves(c, 20L * 64, ch, cf);\n\
         \x20   float *a = zeros32(20L * 48), *b = zeros32(20L * 48);\n\
         \x20   lk_slice<<<4, 256>>>(cf, a, 20L * 48, 64, 0, 48); lk_slice<<<4, 256>>>(ch, b, 20L * 48, 64, 0, 48); CK(cudaGetLastError());\n\
         \x20   same32(\"lk_slice over a binary16 cache\", a, b, 20L * 48);\n  }}"
    );
    let _ = writeln!(
        calls,
        "  {{ // lk_head_tiled over a binary16 cache\n{}\n    float* xf; __half* xh; halves(x, (long)M * H * A, xh, xf);\n\
         \x20   float *a = zeros32((long)M * H * B), *b = zeros32((long)M * H * B);\n\
         \x20   lk_head_tiled<8, 3, true><<<dim3(1, 1, H), 256>>>(lk_w{{wq, ws, nullptr, 0}}, nullptr, nullptr, nullptr, a, H, A, B, A, 0, nullptr, 0, B, 0, M, xf, 0);\n\
         \x20   lk_head_tiled<8, 3, true><<<dim3(1, 1, H), 256>>>(lk_w{{wq, ws, nullptr, 0}}, nullptr, nullptr, nullptr, b, H, A, B, A, 0, nullptr, 0, B, 0, M, xh, 0);\n\
         \x20   CK(cudaGetLastError()); same32(\"lk_head_tiled over a binary16 cache\", a, b, (long)M * H * B);\n  }}",
        heads_setup(20)
    );
    let readers = 2;
    let kernels: String = [
        Kernel::Blocks,
        Kernel::RmsNorm,
        Kernel::Quant,
        Kernel::GemvQ8,
        Kernel::GemvBlk,
        Kernel::GemvParts,
        Kernel::Mma,
        Kernel::GemmBlk,
        Kernel::GemmF32,
        Kernel::GemvF32,
        Kernel::Rope,
        Kernel::SetRow,
        Kernel::Slice,
        Kernel::Join,
        Kernel::Heads,
        Kernel::HeadTiled,
        Kernel::GemvMma,
    ]
    .iter()
    .map(|k| k.source())
    .collect();
    let main = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <cstring>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n{PRELUDE}{kernels}\n\
         #define CK(e) do {{ cudaError_t r_ = (e); if(r_ != cudaSuccess){{ printf(\"CUDA %s at %d\\n\", cudaGetErrorString(r_), __LINE__); exit(1); }} }} while(0)\n\
         __device__ unsigned long long hh(unsigned long long i, unsigned long long s){{ unsigned long long z = (i ^ (s * 0x9E3779B97F4A7C15ULL)) * 0xBF58476D1CE4E5B9ULL; z ^= z >> 31; z *= 0x94D049BB133111EBULL; return z ^ (z >> 29); }}\n\
         __device__ float unitf(unsigned long long i, unsigned long long s){{ return (float)((hh(i, s) >> 11) * (1.0 / 9007199254740992.0) * 2.0 - 1.0); }}\n\
         __global__ void fill_f(float* p, long n, unsigned long long s){{ for(long i = (long)blockIdx.x * blockDim.x + threadIdx.x; i < n; i += (long)gridDim.x * blockDim.x) p[i] = unitf(i, s); }}\n\
         __global__ void fill_w(signed char* q, __half* sc, long n, unsigned long long s){{ long i = (long)blockIdx.x * blockDim.x + threadIdx.x; if(i >= n) return;\n\
         \x20 q[i] = (signed char)((long)(hh(i, s) % 255) - 127);\n\
         \x20 if(i % 32 == 0){{ long b = i / 32; sc[b] = __ushort_as_half((unsigned short)(((5 + b % 4) << 10) | (hh(b, s + 1) & 0x3FF))); }} }}\n\
         // Q4_0 blocks: a normal f16 scale, then sixteen bytes of nibbles.\n\
         __global__ void fill_q40(unsigned char* b, long blocks, unsigned long long s){{ long i = (long)blockIdx.x * blockDim.x + threadIdx.x; if(i >= blocks) return;\n\
         \x20 unsigned short d = (unsigned short)(((5 + i % 4) << 10) | (hh(i, s) & 0x3FF)); b[i * 18] = d & 0xFF; b[i * 18 + 1] = d >> 8;\n\
         \x20 for(int j = 0; j < 16; ++j) b[i * 18 + 2 + j] = (unsigned char)(hh(i * 16 + j, s + 1) & 0xFF); }}\n\
         __global__ void to16(const float* f, __half* h, long n){{ long i = (long)blockIdx.x * blockDim.x + threadIdx.x; if(i < n) h[i] = __float2half_rn(f[i]); }}\n\
         __global__ void round16(const float* x, __half* h, float* f, long n){{ long i = (long)blockIdx.x * blockDim.x + threadIdx.x; if(i >= n) return; __half v = __float2half_rn(x[i]); h[i] = v; f[i] = __half2float(v); }}\n\
         float* falloc(long n){{ float* p; CK(cudaMalloc(&p, n * 4)); return p; }}\n\
         signed char* calloc8(long n){{ signed char* p; CK(cudaMalloc(&p, n)); CK(cudaMemset(p, 0, n)); return p; }}\n\
         float* zeros32(long n){{ return (float*)calloc8(n * 4); }}\n\
         __half* zeros16(long n){{ return (__half*)calloc8(n * 2); }}\n\
         __global__ void fill_b(float* p, long n, unsigned long long s){{ long i = (long)blockIdx.x * blockDim.x + threadIdx.x; if(i < n) p[i] = unitf(i, s) * 0.2f; }}\n\
         float* bias(int n, unsigned long long s){{ float* p = falloc(n); fill_b<<<(n + 255) / 256, 256>>>(p, n, s); return p; }}\n\
         const int* rowat(int r){{ int* p; CK(cudaMalloc(&p, 4)); CK(cudaMemcpy(p, &r, 4, cudaMemcpyHostToDevice)); return p; }}\n\
         const int* rows3(int a, int b, int c){{ int h[3] = {{a, b, c}}; int* p; CK(cudaMalloc(&p, 12)); CK(cudaMemcpy(p, h, 12, cudaMemcpyHostToDevice)); return p; }}\n\
         void halves(const float* x, long n, __half*& h, float*& f){{ CK(cudaMalloc(&h, n * 2)); CK(cudaMalloc(&f, n * 4)); round16<<<(n + 255) / 256, 256>>>(x, h, f, n); CK(cudaGetLastError()); }}\n\
         int bad = 0;\n\
         // The `f32` table rounded to binary16 against the binary16 table, bit for bit, and some of it written.\n\
         void same(const char* what, const float* f, const __half* h, long n){{\n\
         \x20 __half* r = zeros16(n); to16<<<(n + 255) / 256, 256>>>(f, r, n); CK(cudaDeviceSynchronize());\n\
         \x20 std::vector<unsigned short> a(n), b(n); CK(cudaMemcpy(a.data(), r, n * 2, cudaMemcpyDeviceToHost)); CK(cudaMemcpy(b.data(), h, n * 2, cudaMemcpyDeviceToHost));\n\
         \x20 long diff = 0, first = -1, written = 0; for(long i = 0; i < n; ++i){{ if(a[i] != b[i]){{ if(first < 0) first = i; ++diff; }} if(b[i]) ++written; }}\n\
         \x20 if(diff || !written){{ ++bad; printf(\"MISMATCH %s: %ld of %ld differ, first at %ld (%04x against %04x), %ld written\\n\", what, diff, n, first, first < 0 ? 0 : a[first], first < 0 ? 0 : b[first], written); }}\n\
         \x20 else printf(\"EQUAL %s: %ld written of %ld\\n\", what, written, n); }}\n\
         void same32(const char* what, const float* x, const float* y, long n){{\n\
         \x20 CK(cudaDeviceSynchronize()); std::vector<float> a(n), b(n); CK(cudaMemcpy(a.data(), x, n * 4, cudaMemcpyDeviceToHost)); CK(cudaMemcpy(b.data(), y, n * 4, cudaMemcpyDeviceToHost));\n\
         \x20 long diff = 0, written = 0; for(long i = 0; i < n; ++i){{ if(memcmp(&a[i], &b[i], 4)) ++diff; if(a[i] != 0.f) ++written; }}\n\
         \x20 if(diff || !written){{ ++bad; printf(\"MISMATCH %s: %ld of %ld differ, %ld written\\n\", what, diff, n, written); }} else printf(\"EQUAL %s: %ld\\n\", what, n); }}\n\
         int main(){{\n{calls}  printf(\"BAD %d\\n\", bad); return bad ? 2 : 0;\n}}\n"
    );
    let dir = dir().join("cache-f16-writers");
    std::fs::create_dir_all(&dir).unwrap();
    let (cu, bin) = (dir.join("writers.cu"), dir.join("writers"));
    std::fs::write(&cu, main).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "every writer builds into both tables:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).output().unwrap();
    let stdout = String::from_utf8_lossy(&ran.stdout);
    let equal = stdout.lines().filter(|l| l.starts_with("EQUAL")).count();
    assert!(ran.status.success() && equal == cases + readers, "every writer and reader alike bit for bit ({equal} of {}):\n{stdout}{}", cases + readers, String::from_utf8_lossy(&ran.stderr));
}

/// A batch's GEMV on the tensor cores (`lk_gemv_mma`) sums every (row, output) exactly as the decode step's GEMV sums its
/// one row: over Q8_0 matrices of normal f16 scales varied block to block and activations quantized by the lane's own
/// quantizer (`lk_quant`), each row of a batch of 1 to 64 (every count of m-tiles, rows past a tile's edge) by one to
/// three matrices side by side and by six, as many as a launch takes (each a whole number of 8-output tiles, one
/// biased, one into the rows of a table a position names), over `k` of no whole round of 512 values, whole rounds, and whole rounds and a tail of 32 to 256 —
/// launched at every shape it takes (one to sixteen n-tiles a block over each count of m-tiles, a warp one, two or four
/// of the m-tiles by one, two or four of the n-tiles, to sixteen warps, and the launch the lane makes,
/// `gemv_mma_launch`, its narrow projections of long rows in blocks of 64 outputs) — writes, bit for bit, what
/// `lk_gemv_q8<Q, 1>` writes for that row alone, under both quantized numerics.
#[test]
fn the_batch_gemv_on_the_tensor_cores_sums_as_the_decode_steps_gemv_does() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{gemv_mma_launch, gemv_mma_smem_bytes, gemv_mma_threads, gemv_mma_tiles, Kernel, GEMM_PARTS, PRELUDE};
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    // (rows, k, each part's outputs, the biased part, the part into a table's rows)
    let cases: [(usize, usize, &[usize], Option<usize>, Option<usize>); 13] = [
        (1, 512, &[8], None, None),
        (2, 1024, &[64, 32], Some(1), None),
        (5, 1152, &[24, 40], None, Some(0)),
        (16, 2048, &[128, 64, 64], Some(0), None),
        (17, 3584, &[96], None, None),
        (31, 512, &[8, 16, 8], Some(2), Some(1)),
        (32, 6912, &[64], None, None),
        (33, 2080, &[40], Some(0), None),
        (48, 2048, &[256], None, None),
        (64, 4096, &[128, 128], Some(1), None),
        (64, 160, &[16], None, None),
        (20, 1152, &[2048], None, None),
        (64, 1024, &[64, 64, 128, 128, 8, 8], Some(4), Some(5)),
    ];
    const AT: usize = 3;
    // Every width over the rows' m-tiles to sixteen warps, a warp one, two or four of the m-tiles by one, two or four
    // of the n-tiles (at most four products a fragment pair, its 64 accumulators), and the lane's own launch:
    // `(mt, nt, mw, nw)`.
    let shapes = |m: usize, n: usize, k: usize| {
        let mt = gemv_mma_tiles(m);
        let mut s: Vec<(usize, usize, usize, usize)> = [16usize, 8, 4, 2, 1]
            .into_iter()
            .flat_map(|nt| [1usize, 2, 4].into_iter().flat_map(move |mw| [1usize, 2, 4].into_iter().map(move |nw| (nt, mw, nw))))
            .filter(|&(nt, mw, nw)| mw <= mt && nt % nw == 0 && mw * nw <= 4 && (nt / nw) * (mt / mw) <= 16)
            .map(|(nt, mw, nw)| (mt, nt, mw, nw))
            .collect();
        if !s.contains(&gemv_mma_launch(m, n, k, 28)) {
            s.push(gemv_mma_launch(m, n, k, 28));
        }
        s
    };
    let mut calls = String::new();
    for (ci, &(m, k, ns, biased, rowed)) in cases.iter().enumerate() {
        for q in [1usize, 2] {
            let total: usize = ns.iter().sum();
            let _ = writeln!(calls, "  {{ // case {ci} Q{q}: {m} rows, k {k}, parts {ns:?}");
            let _ = writeln!(calls, "    const int M = {m}, K = {k}, N = {total}; float* x = falloc((long)M * K); fill_f<<<(M * K + 255) / 256, 256>>>(x, (long)M * K, {});", 100 + ci);
            let _ = writeln!(calls, "    signed char *xq = calloc8((long)M * K), *xl = calloc8((long)M * K); float* xs = falloc((long)M * K / 32);");
            let _ = writeln!(calls, "    lk_quant<{q}><<<(M * K + 255) / 256, 256>>>(x, xq, {}, xs, M * K);", if q == 2 { "xl" } else { "nullptr" });
            let _ = writeln!(calls, "    lk_parts parts{{}}; float* ref[{GEMM_PARTS}] = {{}}; long len[{GEMM_PARTS}] = {{}};");
            let mut end = 0;
            for (p, &n) in ns.iter().enumerate() {
                end += n;
                let rows = if rowed == Some(p) { AT + m } else { m };
                let _ = writeln!(
                    calls,
                    "    {{ signed char* wq = calloc8((long){n} * K); __half* ws = (__half*)calloc8((long){n} * K / 32 * 2); fill_w<<<({n} * K + 255) / 256, 256>>>(wq, ws, (long){n} * K, {});\n\
                     \x20     parts.w[{p}] = lk_w{{wq, ws, nullptr, 0}}; parts.end[{p}] = {end};\n\
                     \x20     parts.bias[{p}] = {}; parts.row[{p}] = {}; parts.pitch[{p}] = {};\n\
                     \x20     len[{p}] = (long){rows} * {n}; parts.y[{p}] = nanbuf(len[{p}]); ref[{p}] = nanbuf(len[{p}]);\n\
                     \x20     for(int t = 0; t < M; ++t) lk_gemv_q8<{q}, 1><<<({n} + 7) / 8, 256>>>(wq, ws, nullptr, xq + (long)t * K, {}, xs + (long)t * K / 32, parts.bias[{p}], ref[{p}] + (long)t * {n}, parts.row[{p}], parts.pitch[{p}], K, {n}); }}",
                    1000 * ci + p,
                    if biased == Some(p) { format!("bias({n}, {})", 2000 + ci) } else { "nullptr".into() },
                    if rowed == Some(p) { format!("rowat({})", AT + 1) } else { "nullptr".into() },
                    if rowed == Some(p) { n } else { 0 },
                    if q == 2 { "xl + (long)t * K" } else { "nullptr" },
                );
            }
            for p in ns.len()..GEMM_PARTS {
                let _ = writeln!(calls, "    parts.end[{p}] = N;");
            }
            // Every width over these m-tiles, each launch's outputs compared, then the outputs filled again.
            for (mt, nt, mw, nw) in shapes(m, total, k) {
                let smem = gemv_mma_smem_bytes(q == 2, mt, nt);
                let _ = writeln!(
                    calls,
                    "    CK(cudaFuncSetAttribute(lk_gemv_mma<{q}, {mt}, {nt}, {mw}, {nw}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {smem}));\n\
                     \x20   for(int p = 0; p < {np}; ++p) CK(cudaMemset(parts.y[p], 0xFF, len[p] * 4));\n\
                     \x20   lk_gemv_mma<{q}, {mt}, {nt}, {mw}, {nw}><<<(N + {o} - 1) / {o}, {threads}, {smem}>>>(xq, {}, xs, K, N, M, parts); CK(cudaGetLastError()); CK(cudaDeviceSynchronize());\n\
                     \x20   for(int p = 0; p < {np}; ++p) compare(\"case {ci} Q{q} {mt} m-tiles width {nt}, a warp {mw} by {nw}, part\", p, parts.y[p], ref[p], len[p]);",
                    if q == 2 { "xl" } else { "nullptr" },
                    np = ns.len(),
                    o = 8 * nt,
                    threads = gemv_mma_threads(mt, nt, mw, nw),
                );
            }
            calls.push_str("  }\n");
        }
    }
    let main = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <cstring>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n{PRELUDE}{}{}{}{}{}\n\
         #define CK(e) do {{ cudaError_t r_ = (e); if(r_ != cudaSuccess){{ printf(\"CUDA %s at %d\\n\", cudaGetErrorString(r_), __LINE__); exit(1); }} }} while(0)\n\
         __device__ unsigned long long hh(unsigned long long i, unsigned long long s){{ unsigned long long z = (i ^ (s * 0x9E3779B97F4A7C15ULL)) * 0xBF58476D1CE4E5B9ULL; z ^= z >> 31; z *= 0x94D049BB133111EBULL; return z ^ (z >> 29); }}\n\
         __device__ float unitf(unsigned long long i, unsigned long long s){{ return (float)((hh(i, s) >> 11) * (1.0 / 9007199254740992.0) * 2.0 - 1.0); }}\n\
         __global__ void fill_f(float* p, long n, unsigned long long s){{ long i = (long)blockIdx.x * blockDim.x + threadIdx.x; if(i < n) p[i] = unitf(i, s) * (1.0f + 3.0f * (float)((i / 32) % 5)); }}\n\
         __global__ void fill_w(signed char* q, __half* sc, long n, unsigned long long s){{ long i = (long)blockIdx.x * blockDim.x + threadIdx.x; if(i >= n) return;\n\
         \x20 q[i] = (signed char)((long)(hh(i, s) % 255) - 127);\n\
         \x20 if(i % 32 == 0){{ long b = i / 32; sc[b] = __ushort_as_half((unsigned short)(((5 + b % 4) << 10) | (hh(b, s + 1) & 0x3FF))); }} }}\n\
         __global__ void fill_b(float* p, long n, unsigned long long s){{ long i = (long)blockIdx.x * blockDim.x + threadIdx.x; if(i < n) p[i] = unitf(i, s) * 0.2f; }}\n\
         float* falloc(long n){{ float* p; CK(cudaMalloc(&p, n * 4)); return p; }}\n\
         signed char* calloc8(long n){{ signed char* p; CK(cudaMalloc(&p, n)); CK(cudaMemset(p, 0, n)); return p; }}\n\
         float* nanbuf(long n){{ float* p = falloc(n); CK(cudaMemset(p, 0xFF, n * 4)); return p; }}\n\
         float* bias(int n, unsigned long long s){{ float* p = falloc(n); fill_b<<<(n + 255) / 256, 256>>>(p, n, s); return p; }}\n\
         const int* rowat(int r){{ int* p; CK(cudaMalloc(&p, 4)); CK(cudaMemcpy(p, &r, 4, cudaMemcpyHostToDevice)); return p; }}\n\
         int bad = 0;\n\
         void compare(const char* what, int p, const float* got, const float* want, long n){{\n\
         \x20 CK(cudaDeviceSynchronize()); float* g = (float*)malloc(n * 4); float* w = (float*)malloc(n * 4);\n\
         \x20 CK(cudaMemcpy(g, got, n * 4, cudaMemcpyDeviceToHost)); CK(cudaMemcpy(w, want, n * 4, cudaMemcpyDeviceToHost));\n\
         \x20 long diff = 0, first = -1; for(long i = 0; i < n; ++i) if(memcmp(g + i, w + i, 4)){{ if(first < 0) first = i; ++diff; }}\n\
         \x20 if(diff){{ ++bad; printf(\"MISMATCH %s %d: %ld of %ld, first at %ld: %.9g against %.9g\\n\", what, p, diff, n, first, g[first], w[first]); }}\n\
         \x20 else printf(\"EQUAL %s %d: %ld\\n\", what, p, n);\n\
         \x20 free(g); free(w); }}\n\
         int main(){{\n{calls}  printf(\"BAD %d\\n\", bad); return bad ? 2 : 0;\n}}\n",
        Kernel::Blocks.source(),
        Kernel::Quant.source(),
        Kernel::GemvQ8.source(),
        Kernel::Mma.source(),
        Kernel::GemvMma.source(),
    );
    let dir = dir().join("gemv-mma");
    std::fs::create_dir_all(&dir).unwrap();
    let (cu, bin) = (dir.join("gemv_mma.cu"), dir.join("gemv_mma"));
    std::fs::write(&cu, main).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "the harness builds:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).output().unwrap();
    let stdout = String::from_utf8_lossy(&ran.stdout);
    let equal = stdout.lines().filter(|l| l.starts_with("EQUAL")).count();
    let expected: usize = cases.iter().map(|c| c.2.len() * 2 * shapes(c.0, c.2.iter().sum(), c.1).len()).sum();
    assert!(cases.iter().any(|c| gemv_mma_launch(c.0, c.2.iter().sum(), c.1, 28) == (4, 8, 2, 2)), "a case the lane takes in blocks of 64 outputs");
    assert!(cases.iter().any(|c| gemv_mma_launch(c.0, c.2.iter().sum(), c.1, 28) == (4, 16, 2, 2)), "a case the lane takes in blocks of 128 outputs");
    assert!(ran.status.success() && equal == expected, "every part of every case bit for bit ({equal} of {expected}):\n{stdout}{}", String::from_utf8_lossy(&ran.stderr));
}

/// The Q8_0 GEMV's quantized sum is one order, the order every kernel that sums as it sums takes (`lk_gemv_mma` on the
/// tensor cores too): each 32-value block's int8 dot exact in int32 (under two words the high and the low words' dots
/// `d` and `e`), block `b`'s step into partial `b mod 4` — `p ← fmaf(w·s, d, p)`, under two words `p ← fmaf(w, fmaf(s,
/// d, (s/128)·e), p)`, `w` the block's weight scale and `s` its activation's — a partial's blocks in order, and the
/// four partials summed `(p0 + p2) + (p1 + p3)`. Over Q8_0 matrices of normal f16 scales varied block to block and
/// activations quantized by the lane's own quantizer (`lk_quant`), the decode step's GEMV of one activation row, a
/// chunk's of two to sixteen, and the merged projections' (`lk_gemv_parts`, one output row a warp, and two over one
/// activation row), over `k` of fewer blocks than a 512-value round, of whole rounds, and of whole rounds and a ragged
/// tail, write, bit for bit, that sum computed on the host.
#[test]
fn the_q8_0_gemv_sums_each_output_in_four_interleaved_partials() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{Kernel, PRELUDE};
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    // (activation rows, k, outputs)
    let cases: [(usize, usize, usize); 10] =
        [(1, 512, 24), (1, 1152, 40), (1, 160, 8), (1, 12288, 16), (2, 2080, 16), (3, 1024, 8), (5, 3584, 16), (8, 6912, 8), (16, 2048, 16), (16, 1120, 8)];
    let mut calls = String::new();
    for (ci, &(m, k, n)) in cases.iter().enumerate() {
        for q in [1usize, 2] {
            let xl = if q == 2 { "xl" } else { "nullptr" };
            let _ = writeln!(
                calls,
                "  {{ // case {ci} Q{q}: {m} rows, k {k}, {n} outputs\n\
                 \x20   const int M = {m}, K = {k}, N = {n}; float* x = falloc((long)M * K); fill_f<<<(M * K + 255) / 256, 256>>>(x, (long)M * K, {});\n\
                 \x20   signed char *xq = calloc8((long)M * K), *xl = calloc8((long)M * K); float* xs = falloc((long)M * K / 32);\n\
                 \x20   lk_quant<{q}><<<(M * K + 255) / 256, 256>>>(x, xq, {xl}, xs, M * K);\n\
                 \x20   signed char* wq = calloc8((long)N * K); __half* ws = (__half*)calloc8((long)N * K / 32 * 2); fill_w<<<(N * K + 255) / 256, 256>>>(wq, ws, (long)N * K, {});\n\
                 \x20   Host h(M, K, N, xq, xl, xs, wq, ws, {q});\n\
                 \x20   float* y = falloc((long)M * N);\n\
                 \x20   CK(cudaMemset(y, 0xFF, (long)M * N * 4)); lk_gemv_q8<{q}, M><<<(N + 7) / 8, 256>>>(wq, ws, nullptr, xq, {xl}, xs, nullptr, y, nullptr, 0, K, N);\n\
                 \x20   h.compare(\"case {ci} Q{q} lk_gemv_q8\", y);\n\
                 \x20   lk_parts parts{{}}; parts.w[0] = lk_w{{wq, ws, nullptr, 0}}; parts.y[0] = y; for(int p = 0; p < 4; ++p) parts.end[p] = N;\n\
                 \x20   CK(cudaMemset(y, 0xFF, (long)M * N * 4)); lk_gemv_parts<8, {q}, M, 1><<<(N + 7) / 8, 256>>>(nullptr, xq, {xl}, xs, K, N, parts);\n\
                 \x20   h.compare(\"case {ci} Q{q} lk_gemv_parts, a row a warp\", y);\n\
                 {}\x20 }}",
                100 + ci,
                1000 + ci,
                if m == 1 {
                    format!(
                        "\x20   CK(cudaMemset(y, 0xFF, (long)M * N * 4)); lk_gemv_parts<8, {q}, M, 2><<<(N + 15) / 16, 256>>>(nullptr, xq, {xl}, xs, K, N, parts);\n\
                         \x20   h.compare(\"case {ci} Q{q} lk_gemv_parts, two rows a warp\", y);\n"
                    )
                } else {
                    String::new()
                },
            );
        }
    }
    let main = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <cstring>\n#include <cmath>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n{PRELUDE}{}{}{}{}\n\
         #define CK(e) do {{ cudaError_t r_ = (e); if(r_ != cudaSuccess){{ printf(\"CUDA %s at %d\\n\", cudaGetErrorString(r_), __LINE__); exit(1); }} }} while(0)\n\
         __device__ unsigned long long hh(unsigned long long i, unsigned long long s){{ unsigned long long z = (i ^ (s * 0x9E3779B97F4A7C15ULL)) * 0xBF58476D1CE4E5B9ULL; z ^= z >> 31; z *= 0x94D049BB133111EBULL; return z ^ (z >> 29); }}\n\
         __device__ float unitf(unsigned long long i, unsigned long long s){{ return (float)((hh(i, s) >> 11) * (1.0 / 9007199254740992.0) * 2.0 - 1.0); }}\n\
         __global__ void fill_f(float* p, long n, unsigned long long s){{ long i = (long)blockIdx.x * blockDim.x + threadIdx.x; if(i < n) p[i] = unitf(i, s) * (1.0f + 3.0f * (float)((i / 32) % 5)); }}\n\
         __global__ void fill_w(signed char* q, __half* sc, long n, unsigned long long s){{ long i = (long)blockIdx.x * blockDim.x + threadIdx.x; if(i >= n) return;\n\
         \x20 q[i] = (signed char)((long)(hh(i, s) % 255) - 127);\n\
         \x20 if(i % 32 == 0){{ long b = i / 32; sc[b] = __ushort_as_half((unsigned short)(((5 + b % 4) << 10) | (hh(b, s + 1) & 0x3FF))); }} }}\n\
         float* falloc(long n){{ float* p; CK(cudaMalloc(&p, n * 4)); return p; }}\n\
         signed char* calloc8(long n){{ signed char* p; CK(cudaMalloc(&p, n)); CK(cudaMemset(p, 0, n)); return p; }}\n\
         template<typename T> std::vector<T> back(const T* d, long n){{ std::vector<T> h(n); CK(cudaDeviceSynchronize()); CK(cudaMemcpy(h.data(), d, n * sizeof(T), cudaMemcpyDeviceToHost)); return h; }}\n\
         // The order on the host: block b's step into partial b mod 4, a partial's blocks in order, (p0 + p2) + (p1 + p3).\n\
         float order(const signed char* w, const __half* ws, const signed char* xh, const signed char* xl, const float* xs, int K, int q){{\n\
         \x20 float p[4] = {{0.f, 0.f, 0.f, 0.f}};\n\
         \x20 for(int b = 0; b < K / 32; ++b){{\n\
         \x20   int d = 0, e = 0;\n\
         \x20   for(int i = 0; i < 32; ++i){{ d += w[32 * b + i] * xh[32 * b + i]; if(q == 2) e += w[32 * b + i] * xl[32 * b + i]; }}\n\
         \x20   const float wsc = __half2float(ws[b]), s = xs[b];\n\
         \x20   if(q == 1){{ const float a = wsc * s; p[b % 4] = fmaf(a, (float)d, p[b % 4]); }}\n\
         \x20   else {{ const float lo = (s * 0.0078125f) * (float)e; p[b % 4] = fmaf(wsc, fmaf(s, (float)d, lo), p[b % 4]); }}\n\
         \x20 }}\n\
         \x20 return (p[0] + p[2]) + (p[1] + p[3]);\n\
         }}\n\
         int bad = 0, seen = 0;\n\
         struct Host {{\n\
         \x20 int M, K, N, q; std::vector<signed char> xq, xl, wq; std::vector<float> xs; std::vector<__half> ws;\n\
         \x20 Host(int M_, int K_, int N_, const signed char* dxq, const signed char* dxl, const float* dxs, const signed char* dwq, const __half* dws, int q_)\n\
         \x20   : M(M_), K(K_), N(N_), q(q_), xq(back(dxq, (long)M_ * K_)), xl(back(dxl, (long)M_ * K_)), wq(back(dwq, (long)N_ * K_)), xs(back(dxs, (long)M_ * K_ / 32)), ws(back(dws, (long)N_ * K_ / 32)) {{}}\n\
         \x20 void compare(const char* what, const float* dy){{\n\
         \x20   std::vector<float> y = back(dy, (long)M * N); long diff = 0, first = -1; float got = 0.f, want = 0.f;\n\
         \x20   for(int t = 0; t < M; ++t) for(int o = 0; o < N; ++o){{\n\
         \x20     const float r = order(wq.data() + (long)o * K, ws.data() + (long)o * K / 32, xq.data() + (long)t * K, xl.data() + (long)t * K, xs.data() + (long)t * K / 32, K, q);\n\
         \x20     if(memcmp(&r, &y[(long)t * N + o], 4)){{ if(first < 0){{ first = (long)t * N + o; got = y[first]; want = r; }} ++diff; }}\n\
         \x20   }}\n\
         \x20   ++seen; if(diff){{ ++bad; printf(\"MISMATCH %s: %ld of %ld, first at %ld: %.9g against %.9g\\n\", what, diff, (long)M * N, first, got, want); }}\n\
         \x20   else printf(\"EQUAL %s: %ld\\n\", what, (long)M * N);\n\
         \x20 }}\n\
         }};\n\
         int main(){{\n{calls}  printf(\"BAD %d of %d\\n\", bad, seen); return bad ? 2 : 0;\n}}\n",
        Kernel::Blocks.source(),
        Kernel::Quant.source(),
        Kernel::GemvQ8.source(),
        Kernel::GemvParts.source(),
    );
    let dir = dir().join("q8-order");
    std::fs::create_dir_all(&dir).unwrap();
    let (cu, bin) = (dir.join("q8_order.cu"), dir.join("q8_order"));
    std::fs::write(&cu, main).unwrap();
    let built = Command::new("nvcc")
        .args(["-O3", "-std=c++17", "-arch=sm_86", "-Xcompiler", "-ffp-contract=off", "-o"])
        .arg(&bin)
        .arg(&cu)
        .output()
        .unwrap();
    assert!(built.status.success(), "the harness builds:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).output().unwrap();
    let stdout = String::from_utf8_lossy(&ran.stdout);
    let equal = stdout.lines().filter(|l| l.starts_with("EQUAL")).count();
    let expected = cases.iter().map(|&(m, _, _)| if m == 1 { 3 } else { 2 }).sum::<usize>() * 2;
    assert!(ran.status.success() && equal == expected, "every kernel of every case in the order, bit for bit ({equal} of {expected}):\n{stdout}{}", String::from_utf8_lossy(&ran.stderr));
}

/// A batch of more sequences than the GEMV's rows (`GEMV_ROWS`) decodes each as the decode step decodes it alone: twenty
/// prompts of one to twelve tokens through one program, decoded a token each a step together, generate, sequence by
/// sequence, the tokens and every logit, bit for bit, that each gives alone — its projections summing its row as the
/// decode step sums its one row, never in the tensor cores' GEMM's order a chunk of as many rows takes — for every tiny
/// architecture, under every numerics.
#[test]
fn a_batch_past_the_gemvs_rows_decodes_each_sequence_as_alone() {
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let jobs: Vec<(Vec<u32>, usize)> = (0..20u32).map(|b| ((0..1 + b % 12).map(|i| (i * 29 + 5 + 11 * b) % 64).collect(), 5)).collect();
    let mut lanes = Vec::new();
    for &t in ALL {
        let spec = t.spec(CTX);
        let ckpt = dir().join(format!("batch-wide-{}.gguf", t.name()));
        tiny_models::write_checkpoint(&spec, &ckpt);
        for numerics in [Q8MatKind::Dequant, Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
            let opts = LaneOptions { ctx: CTX, knobs: CudaKnobs { activations: numerics, ..CudaKnobs::default() }, prefill: 4 };
            let lane = compile_batch(&spec, &ckpt, &opts, jobs.len()).unwrap_or_else(|e| panic!("{} {numerics:?}: {e}", t.name()));
            lanes.push((t, numerics, spec.vocab, ckpt.clone(), lane, format!("batch-wide-{}-{numerics:?}", t.name())));
        }
    }
    let built = build_all(&lanes.iter().map(|(_, _, _, _, lane, name)| (lane, name.as_str())).collect::<Vec<_>>());
    for ((t, numerics, vocab, ckpt, _, _), b) in lanes.iter().zip(&built) {
        let what = format!("{} {numerics:?}", t.name());
        let alone = exec_many(b, ckpt, &jobs, *vocab, true);
        let together = exec_batch(b, ckpt, &jobs, *vocab, true);
        for (s, (one, all)) in alone.iter().zip(&together).enumerate() {
            assert_eq!(all.tokens, one.tokens, "{what}: sequence {s} decodes in the batch of twenty as it does alone");
            assert_eq!(all.logits, one.logits, "{what}: sequence {s}'s every logit, bit for bit");
        }
    }
}

/// A batch's sequences each cut their own span into the attention's chunks: four prompts of four lengths, three of
/// them past one chunk of positions and each a different number of chunks, decoded together by the batch step — its
/// attentions over the whole cache taking their chunks a block each and folding them — generate, sequence by sequence,
/// the tokens and every logit, bit for bit, that each gives decoded alone: for the attentions that take every path of
/// the kernel (grouped heads; sinks beside windowed layers; latent attention, its rows shared under the quantized
/// numerics), under the precise numerics and the quantized.
#[test]
fn a_batch_of_long_spans_folds_each_sequence_as_it_folds_alone() {
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    const LONG: usize = 4096;
    let mut lanes = Vec::new();
    for t in [Tiny::Qwen3, Tiny::GptOss, Tiny::DeepSeek2] {
        let spec = t.spec(LONG);
        let ckpt = dir().join(format!("batch-long-{}.gguf", t.name()));
        tiny_models::write_checkpoint(&spec, &ckpt);
        for numerics in [Q8MatKind::Dequant, Q8MatKind::Int8Act] {
            let opts = LaneOptions { ctx: LONG, knobs: CudaKnobs { activations: numerics, ..CudaKnobs::default() }, prefill: 16 };
            let lane = compile_batch(&spec, &ckpt, &opts, 4).unwrap_or_else(|e| panic!("{} {numerics:?}: {e}", t.name()));
            let folds: Vec<usize> = lane
                .batch()
                .expect("the batch step")
                .schedule
                .launches
                .iter()
                .filter_map(|l| match l {
                    Launch::Attention { splits, parallel: true, window: None, rows: 4, .. } if *splits > 1 => Some(*splits),
                    _ => None,
                })
                .collect();
            assert!(!folds.is_empty(), "{} {numerics:?}: the batch's attentions over the whole cache fold their chunks", t.name());
            let splits = *folds.iter().max().expect("an attention");
            let lens = [64 * splits + 50, 32 * splits + 7, 40, 5];
            let jobs: Vec<(Vec<u32>, usize)> = lens
                .iter()
                .enumerate()
                .map(|(b, &n)| ((0..n as u32).map(|i| (i * 37 + 11 + 7 * b as u32) % tiny_models::VOCAB as u32).collect(), 6))
                .collect();
            assert!(jobs.iter().all(|(p, n)| p.len() + n <= LONG), "{}: every prompt fits the cache", t.name());
            lanes.push((t, numerics, spec.vocab, ckpt.clone(), lane, jobs, format!("batch-long-{}-{numerics:?}", t.name())));
        }
    }
    let built = build_all(&lanes.iter().map(|(_, _, _, _, lane, _, name)| (lane, name.as_str())).collect::<Vec<_>>());
    for ((t, numerics, vocab, ckpt, _, jobs, _), b) in lanes.iter().zip(&built) {
        let what = format!("{} {numerics:?}", t.name());
        let alone = exec_many(b, ckpt, jobs, *vocab, true);
        let together = exec_batch(b, ckpt, jobs, *vocab, true);
        for (s, (one, all)) in alone.iter().zip(&together).enumerate() {
            assert_eq!(all.tokens, one.tokens, "{what}: sequence {s} of {} positions decodes in the batch as it does alone", jobs[s].0.len());
            assert_eq!(all.logits, one.logits, "{what}: sequence {s}'s every logit, bit for bit");
        }
    }
}

/// A batch whose sequences' head blocks alone fill the chip takes each sequence's chunks of its span in the sequence's
/// own block, folded there (`parallel` off), the arithmetic the same as each chunk in a block of its own: sixty-four
/// sequences of 33 to 160 prompt tokens — each past one chunk of positions — decoded together generate, sequence by
/// sequence, the tokens and every logit, bit for bit, that each gives alone, for grouped heads and for sinks beside
/// windowed layers, under the precise numerics and the quantized.
#[test]
fn a_batch_that_fills_the_chip_folds_each_sequences_chunks_in_its_own_block() {
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    const LONG: usize = 512;
    let jobs: Vec<(Vec<u32>, usize)> = (0..64u32)
        .map(|b| ((0..33 + (b * 37) % 128).map(|i| (i * 29 + 7 + 13 * b) % tiny_models::VOCAB as u32).collect(), 4))
        .collect();
    let mut lanes = Vec::new();
    for t in [Tiny::Qwen3, Tiny::GptOss] {
        let spec = t.spec(LONG);
        let ckpt = dir().join(format!("batch-fill-{}.gguf", t.name()));
        tiny_models::write_checkpoint(&spec, &ckpt);
        for numerics in [Q8MatKind::Dequant, Q8MatKind::Int8Act] {
            let opts = LaneOptions { ctx: LONG, knobs: CudaKnobs { activations: numerics, ..CudaKnobs::default() }, prefill: 16 };
            let lane = compile_batch(&spec, &ckpt, &opts, jobs.len()).unwrap_or_else(|e| panic!("{} {numerics:?}: {e}", t.name()));
            let together: Vec<(usize, bool)> = lane
                .batch()
                .expect("the batch step")
                .schedule
                .launches
                .iter()
                .filter_map(|l| match l {
                    Launch::Attention { splits, parallel, rows: 64, window: None, .. } => Some((*splits, *parallel)),
                    _ => None,
                })
                .collect();
            assert!(
                !together.is_empty() && together.iter().all(|&(s, p)| s > 1 && !p),
                "{} {numerics:?}: the batch's attentions over the whole cache cut their spans and fold them in their rows' blocks: {together:?}",
                t.name()
            );
            lanes.push((t, numerics, spec.vocab, ckpt.clone(), lane, format!("batch-fill-{}-{numerics:?}", t.name())));
        }
    }
    let built = build_all(&lanes.iter().map(|(_, _, _, _, lane, name)| (lane, name.as_str())).collect::<Vec<_>>());
    for ((t, numerics, vocab, ckpt, _, _), b) in lanes.iter().zip(&built) {
        let what = format!("{} {numerics:?}", t.name());
        let alone = exec_many(b, ckpt, &jobs, *vocab, true);
        let together = exec_batch(b, ckpt, &jobs, *vocab, true);
        for (s, (one, all)) in alone.iter().zip(&together).enumerate() {
            assert_eq!(all.tokens, one.tokens, "{what}: sequence {s} of {} positions decodes in the batch as it does alone", jobs[s].0.len());
            assert_eq!(all.logits, one.logits, "{what}: sequence {s}'s every logit, bit for bit");
        }
    }
}

/// `--profile` times the decode step launch by launch: a line for each launch the step makes, in the step's order, with
/// its kind, the tensor it computes and its mean time over the generated tokens — the measurement every fusion and
/// kernel choice is made against, on any GPU, with no profiler installed.
#[test]
fn the_program_profiles_its_decode_step_launch_by_launch() {
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    for t in [Tiny::Qwen3, Tiny::DeepSeek2] {
        let spec = t.spec(CTX);
        let ckpt = dir().join(format!("profile-{}.gguf", t.name()));
        tiny_models::write_checkpoint(&spec, &ckpt);
        let lane = compile(&spec, &ckpt, &LaneOptions { ctx: CTX, knobs: CudaKnobs::default(), prefill: 4 }).unwrap();
        let built = build(&lane, &format!("profile-{}", t.name()));
        let out = Command::new(&built.bin).arg(&ckpt).arg(&built.side).arg("8").arg("--profile").args(["5", "17", "2"]).output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{} profiles:\n{stdout}\n{}", t.name(), String::from_utf8_lossy(&out.stderr));
        let rows: Vec<(String, String, f64)> = stdout
            .lines()
            .filter_map(|l| l.strip_prefix("PROFILE:"))
            .map(|l| {
                let f: Vec<&str> = l.split_whitespace().collect();
                (f[0].to_string(), f[1].to_string(), f[2].parse().unwrap())
            })
            .collect();
        let want: Vec<(String, String)> = lane
            .decode()
            .schedule
            .launches
            .iter()
            .filter(|l| !matches!(l, Launch::View { .. }))
            .map(|l| (kind(l).to_string(), l.writes()[0].to_string()))
            .collect();
        assert_eq!(rows.iter().map(|(k, o, _)| (k.clone(), o.clone())).collect::<Vec<_>>(), want, "{}: a line a launch, in order", t.name());
        assert!(rows.iter().all(|(_, _, us)| us.is_finite() && *us > 0.0), "{}: every launch takes time: {rows:?}", t.name());
        assert!(stdout.contains("TOKENS:"), "{}: the run still generates", t.name());
    }
}

/// `--profile` times every prefill chunk the prompt runs launch by launch as well: a `PROFILE_CHUNK:` line for each
/// launch of each chunk form, with the form's rows, the launch's kind, the tensor it computes and its mean time over the
/// chunk's runs, in the form's order — prefill measured the way decode is, on any GPU, with no profiler installed. The
/// decode step's `PROFILE:` lines are unchanged beside them.
#[test]
fn the_program_profiles_each_prefill_chunk_launch_by_launch() {
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    for t in [Tiny::Qwen3, Tiny::DeepSeek2] {
        let spec = t.spec(CTX);
        let ckpt = dir().join(format!("profile-chunks-{}.gguf", t.name()));
        tiny_models::write_checkpoint(&spec, &ckpt);
        let lane = compile(&spec, &ckpt, &LaneOptions { ctx: CTX, knobs: CudaKnobs::default(), prefill: 4 }).unwrap();
        let chunks: Vec<_> = lane.forms.iter().filter(|f| f.rows > 1).collect();
        assert!(!chunks.is_empty(), "{}: a prefill of 4 compiles chunk forms", t.name());
        // A prompt one token longer than every chunk together runs each chunk once, then one decode step.
        let prompt: Vec<String> = (0..chunks.iter().map(|f| f.rows).sum::<usize>() + 1).map(|i| ((5 + 7 * i) % 60).to_string()).collect();
        let built = build(&lane, &format!("profile-chunks-{}", t.name()));
        let out = Command::new(&built.bin).arg(&ckpt).arg(&built.side).arg("4").arg("--profile").args(&prompt).output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{} profiles:\n{stdout}\n{}", t.name(), String::from_utf8_lossy(&out.stderr));
        let rows: Vec<(usize, String, String, f64)> = stdout
            .lines()
            .filter_map(|l| l.strip_prefix("PROFILE_CHUNK:"))
            .map(|l| {
                let f: Vec<&str> = l.split_whitespace().collect();
                (f[0].parse().unwrap(), f[1].to_string(), f[2].to_string(), f[3].parse().unwrap())
            })
            .collect();
        for form in &chunks {
            let got: Vec<(String, String)> = rows.iter().filter(|r| r.0 == form.rows).map(|r| (r.1.clone(), r.2.clone())).collect();
            let want: Vec<(String, String)> = form
                .schedule
                .launches
                .iter()
                .filter(|l| !matches!(l, Launch::View { .. }))
                .map(|l| (kind(l).to_string(), l.writes()[0].to_string()))
                .collect();
            assert_eq!(got, want, "{}: a line a launch of the {}-row chunk, in order", t.name(), form.rows);
        }
        assert!(rows.iter().all(|r| r.3.is_finite() && r.3 > 0.0), "{}: every launch takes time: {rows:?}", t.name());
        assert_eq!(
            stdout.lines().filter(|l| l.starts_with("PROFILE:")).count(),
            lane.decode().schedule.launches.iter().filter(|l| !matches!(l, Launch::View { .. })).count(),
            "{}: the decode step's lines stand beside them",
            t.name()
        );
    }
}

/// A step's launches over several streams (`branches`) compute what they compute one after another: every tiny
/// architecture's program — and Qwen3 and DeepSeek-V2 at their medium widths in Q4_K, wide chunks among their forms —
/// under the quantized and the precise numerics, generating from a 40-token prompt run in a chunk of 32 rows and one of
/// 8, then decode steps, its logits
/// bit for bit those of the same program with every launch on the step's own stream (`--profile`, no graph): run as it
/// ships, and under four seeds of `--stagger`, each launch held back by a stall the seed draws, so launches the streams
/// leave unordered run in another order under each — a dependency the streams dropped parts them.
#[test]
fn a_branched_step_computes_what_its_launches_compute_in_order() {
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let ctx = 64;
    let prompt: Vec<u32> = (0..40).map(|i| (5 + 7 * i) % 60).collect();
    let mut models: Vec<(String, ModelSpec, bool)> = ALL.iter().map(|&t| (t.name().to_string(), t.spec(ctx), false)).collect();
    models.extend([Tiny::Qwen3, Tiny::DeepSeek2].map(|t| (format!("{}-medium", t.name()), t.medium(ctx), true)));
    let mut lanes: Vec<(String, String, &ModelSpec, PathBuf, Lane)> = Vec::new();
    for (name, spec, q4k) in &models {
        let ckpt = dir().join(format!("{name}-branched.gguf"));
        if *q4k {
            use logicaffeine_compile::model::quant::GgmlType;
            let matrices: Vec<String> = tiny_models::tensors(spec)
                .into_iter()
                .map(|(n, _)| n)
                .filter(|n| n.ends_with(".weight") && !n.contains("norm") && !n.contains("ffn_gate_inp"))
                .collect();
            tiny_models::write_checkpoint_as(spec, &ckpt, &|t| if matrices.iter().any(|m| m == t) { GgmlType::Q4K } else { GgmlType::F32 });
        } else {
            tiny_models::write_checkpoint(spec, &ckpt);
        }
        for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8, Q8MatKind::Dequant] {
            let what = format!("{name} {numerics:?}");
            let lane = compile(spec, &ckpt, &LaneOptions { ctx, knobs: CudaKnobs { activations: numerics, ..CudaKnobs::default() }, prefill: 32 })
                .unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_eq!(lane.forms.iter().map(|f| f.rows).max(), Some(32), "{what}: a chunk of 32 rows, wider than the GEMV takes");
            lanes.push((what, format!("branched-{name}-{numerics:?}"), spec, ckpt.clone(), lane));
        }
    }
    let jobs: Vec<(&Lane, &str)> = lanes.iter().map(|(_, bin, _, _, lane)| (lane, bin.as_str())).collect();
    for ((what, _, spec, ckpt, _), built) in lanes.iter().zip(&build_all(&jobs)) {
        let in_order = exec_flagged(built, ckpt, &prompt, GENERATE, spec.vocab, true, &["--profile"]);
        for seed in [None, Some("1"), Some("2"), Some("3"), Some("4")] {
            let flags: Vec<&str> = seed.into_iter().flat_map(|s| ["--stagger", s]).collect();
            let branched = exec_flagged(built, ckpt, &prompt, GENERATE, spec.vocab, true, &flags);
            assert_eq!(branched.tokens, in_order.tokens, "{what} stagger {seed:?}: the branched step generates the tokens in order");
            for (i, (x, y)) in branched.logits.iter().zip(&in_order.logits).enumerate() {
                let parted = x.iter().zip(y).position(|(p, q)| p.to_bits() != q.to_bits());
                assert!(parted.is_none(), "{what} stagger {seed:?}: position {i}'s logit {parted:?} parts from the launches in order");
            }
        }
    }
}

/// Where a matrix lives changes no value, and a program places its matrices as its plan does. Every tiny architecture
/// (its matrices in Q8_0), and the wide dense and expert models with their matrices in the block formats by turns,
/// compiled under every numerics with chunks up to 32 rows, is built once and run from a 40-token prompt on the device
/// memory it finds free and on `--device-bytes` of its whole, half its matrices' bytes past the arrays that cannot
/// move, and those arrays alone: every run generates the same tokens and logits bit for bit, and each placement — the
/// bytes on the device and in host memory, the matrices moved — is `residence::place` on its budget. A byte under the
/// arrays that cannot move, the program refuses by name before it allocates.
#[test]
fn a_matrix_in_host_memory_computes_what_it_computes_on_the_device() {
    use logicaffeine_compile::codegen_cuda::lane::LaneError;
    use logicaffeine_compile::model::quant::GgmlType;
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let ctx = 64;
    let prompt: Vec<u32> = (0..40).map(|i| (3 + 11 * i) % 60).collect();
    let mut models: Vec<(String, ModelSpec, bool)> = ALL.iter().map(|&t| (t.name().to_string(), t.spec(ctx), false)).collect();
    models.extend([Tiny::Qwen3, Tiny::Qwen3Moe, Tiny::GptOss, Tiny::DeepSeek2].map(|t| (format!("{}-wide", t.name()), t.wide(ctx), true)));
    for (name, spec, blocks) in &models {
        let ckpt = dir().join(format!("{name}-resident.gguf"));
        if *blocks {
            let matrices: Vec<String> = tiny_models::tensors(spec)
                .into_iter()
                .map(|(n, _)| n)
                .filter(|n| n.ends_with(".weight") && !n.contains("norm") && !n.contains("attn_sinks"))
                .filter(|n| !(spec.arch == "gpt-oss" && n.contains("ffn_gate_inp")))
                .collect();
            tiny_models::write_checkpoint_as(spec, &ckpt, &|t| match matrices.iter().position(|m| m == t) {
                Some(k) => BLOCK_FORMATS[k % BLOCK_FORMATS.len()],
                None => GgmlType::F32,
            });
        } else {
            tiny_models::write_checkpoint(spec, &ckpt);
        }
        let lanes: Vec<(Q8MatKind, Lane)> = [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8, Q8MatKind::Dequant]
            .into_iter()
            .map(|numerics| {
                let knobs = CudaKnobs { activations: numerics, ..CudaKnobs::default() };
                (numerics, compile(spec, &ckpt, &LaneOptions { ctx, knobs, prefill: 32 }).unwrap_or_else(|e| panic!("{name} {numerics:?}: {e}")))
            })
            .collect();
        let built: Vec<Built> = std::thread::scope(|s| {
            let handles: Vec<_> = lanes.iter().map(|(numerics, lane)| s.spawn(move || build(lane, &format!("resident-{name}-{numerics:?}")))).collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for ((numerics, lane), b) in lanes.iter().zip(&built) {
            let what = format!("{name} {numerics:?}");
            let plan = &lane.plan;
            let fixed = plan.arena + plan.fixed;
            let free = exec(b, &ckpt, &prompt, GENERATE, spec.vocab, true);
            assert_eq!(free.placed, (plan.whole(), 0, 0), "{what}: the free device holds every matrix");
            for (at, budget) in [("whole", plan.whole()), ("half", fixed + (plan.whole() - fixed) / 2), ("none", fixed)] {
                let r = exec_flagged(b, &ckpt, &prompt, GENERATE, spec.vocab, true, &["--device-bytes", &budget.to_string()]);
                let p = place(plan, budget).unwrap();
                assert_eq!(r.placed, (p.device_bytes, p.host_bytes, p.moved), "{what} {at}: the program places its matrices as its plan does");
                match at {
                    "whole" => assert_eq!(p.moved, 0, "{what}"),
                    "half" => assert!(p.moved > 0 && p.moved < plan.order.len(), "{what}: half moves some"),
                    _ => assert_eq!(p.moved, plan.order.len(), "{what}: none keeps no matrix"),
                }
                assert_eq!(r.tokens, free.tokens, "{what} {at}: the matrices in host memory generate the device's tokens");
                for (i, (x, y)) in r.logits.iter().zip(&free.logits).enumerate() {
                    let parted = x.iter().zip(y).position(|(p, q)| p.to_bits() != q.to_bits());
                    assert!(parted.is_none(), "{what} {at}: position {i}'s logit {parted:?} parts from the device's");
                }
            }
            let out = Command::new(&b.bin)
                .arg(&ckpt)
                .arg(&b.side)
                .arg(GENERATE.to_string())
                .args(["--device-bytes", &(fixed - 1).to_string()])
                .args(prompt.iter().map(u32::to_string))
                .output()
                .unwrap();
            assert_eq!(out.status.code(), Some(4), "{what}: a byte under the arrays that cannot move refuses");
            assert_eq!(
                String::from_utf8_lossy(&out.stderr).trim(),
                LaneError::Memory { needed: fixed, budget: fixed - 1 }.to_string(),
                "{what}: by name"
            );
            assert!(out.stdout.is_empty(), "{what}: before it allocates");
        }
    }
}

/// A program squeezed by another process places its matrices on the memory it finds: Llama-3.2-1B under the two-word
/// numerics, run while a process holds all of the device but the arrays that cannot move, half the matrices' bytes, its
/// spare and a context's share, moves some of its matrices to mapped host memory on its own, builds its graphs in the
/// spare it held back, and generates what it generates with the device to itself, bit for bit.
#[test]
fn a_program_squeezed_by_another_places_its_matrices_on_what_it_finds() {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    if !(common::require_tool("nvcc") && common::require_gpu() && common::require_tool("checkpoint:llama3.2-1b")) {
        return;
    }
    /// What a CUDA context and its modules take of the device, in the program's own measure: 113 MiB on the RTX 3060.
    const CONTEXT: usize = 160 << 20;
    let ckpt = common::checkpoint("llama3.2-1b").unwrap();
    let spec = ModelSpec::from_gguf(&Gguf::open(&ckpt).unwrap()).unwrap();
    let knobs = CudaKnobs { activations: Q8MatKind::TwoPassInt8, ..CudaKnobs::default() };
    let lane = compile(&spec, &ckpt, &LaneOptions { ctx: 128, knobs, prefill: 16 }).unwrap();
    let plan = &lane.plan;
    let fixed = plan.arena + plan.fixed;
    let b = build(&lane, "squeezed-llama");
    let prompt: Vec<u32> = (0..40).map(|i| 1000 + 37 * i).collect();
    let alone = exec(&b, &ckpt, &prompt, GENERATE, spec.vocab, true);
    assert_eq!(alone.placed.2, 0, "the device to itself holds every matrix");
    let hog_src = dir().join("hog.cu");
    let hog = dir().join("hog");
    std::fs::write(
        &hog_src,
        "#include <cstdio>\n#include <cstdlib>\n#include <cuda_runtime.h>\n\
         int main(int argc, char** argv){\n\
         \x20 size_t leave = strtoull(argv[1], nullptr, 10), f = 0, t = 0;\n\
         \x20 cudaFree(0); cudaMemGetInfo(&f, &t);\n\
         \x20 void* p = nullptr;\n\
         \x20 if(f <= leave || cudaMalloc(&p, f - leave) != cudaSuccess){ printf(\"REFUSED %zu\\n\", f); return 1; }\n\
         \x20 cudaMemGetInfo(&f, &t); printf(\"HELD %zu\\n\", f); fflush(stdout);\n\
         \x20 getchar();\n\
         }\n",
    )
    .unwrap();
    let built = Command::new("nvcc").args(["-O2", "-arch=sm_86", "-o"]).arg(&hog).arg(&hog_src).output().unwrap();
    assert!(built.status.success(), "the hog builds:\n{}", String::from_utf8_lossy(&built.stderr));
    let half = (plan.whole() - fixed) / 2;
    let mut holder = Command::new(&hog)
        .arg((fixed + half + plan.spare + CONTEXT).to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(holder.stdout.as_mut().unwrap()).read_line(&mut line).unwrap();
    assert!(line.starts_with("HELD "), "the hog holds the device: {line}");
    let squeezed = std::panic::catch_unwind(|| exec(&b, &ckpt, &prompt, GENERATE, spec.vocab, true));
    drop(holder.stdin.take());
    holder.wait().unwrap();
    let squeezed = squeezed.unwrap_or_else(|e| std::panic::resume_unwind(e));
    let (device, host, moved) = squeezed.placed;
    assert!(moved > 0 && moved < plan.order.len(), "squeezed, the program moves some of its {} matrices: {moved}", plan.order.len());
    let p = place(plan, device).unwrap();
    assert_eq!((p.device_bytes, p.host_bytes, p.moved), (device, host, moved), "the placement is the plan's on what it kept");
    assert_eq!(squeezed.tokens, alone.tokens, "squeezed, it generates the tokens it generates alone");
    for (i, (x, y)) in squeezed.logits.iter().zip(&alone.logits).enumerate() {
        let parted = x.iter().zip(y).position(|(p, q)| p.to_bits() != q.to_bits());
        assert!(parted.is_none(), "position {i}'s logit {parted:?} parts from the run alone");
    }
}

/// Every knob point compiles to a program that generates the same tokens: the step launched without a graph, the
/// activations quantized in their own launches.
#[test]
fn every_knob_point_generates_the_same_tokens() {
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let spec = Tiny::Qwen35.spec(CTX);
    let ckpt = dir().join("knobs.gguf");
    tiny_models::write_checkpoint(&spec, &ckpt);
    let base = CudaKnobs::default();
    let mut runs = Vec::new();
    for (name, knobs) in [
        ("default", base),
        ("no-graphs", CudaKnobs { cuda_graphs: false, ..base }),
        ("unfused-quant", CudaKnobs { fuse_quant: false, ..base }),
    ] {
        let lane = compile(&spec, &ckpt, &LaneOptions { ctx: CTX, knobs: CudaKnobs { activations: Q8MatKind::Int8Act, ..knobs }, prefill: 1 }).unwrap();
        runs.push((name, run(&lane, &ckpt, PROMPT, GENERATE, &format!("knobs-{name}"), spec.vocab, true)));
    }
    for (name, r) in &runs[1..] {
        assert_eq!(r.tokens, runs[0].1.tokens, "{name} generates the default's tokens");
        assert_eq!(r.logits, runs[0].1.logits, "{name} computes the default's logits bit for bit");
    }
}

/// Every fusion computes what its launches compute apart: each architecture's program with every rule held back
/// (`compile_holding` over `plan::RULES`) and every activation quantized in its own launch (`fuse_quant` off) generates
/// the fused program's tokens and logits bit for bit, under the quantized and the precise numerics, through a prompt
/// that runs a chunk of four, a chunk of two and a decode step — for every tiny architecture, for Qwen3 with heads of
/// 256, wider than the rotation kernel's own threads, and for Qwen3 and DeepSeek-V2 at their medium width in Q4_K,
/// whose projections, gated pairs and mixtures take four rows a warp and quantize in their epilogues.
#[test]
fn every_fusion_computes_what_its_launches_compute_apart() {
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let prompt = [5, 17, 2, 30, 9, 11, 3];
    let mut wide_heads = Tiny::Qwen3.spec(CTX);
    for layer in &mut wide_heads.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            (a.head_dim, a.value_dim, a.scale) = (256, 256, 1.0 / 16.0);
            if let Some(r) = &mut a.rope {
                r.dims = 256;
            }
        }
    }
    let mut models: Vec<(String, ModelSpec, bool)> = ALL.iter().map(|&t| (t.name().to_string(), t.spec(CTX), false)).collect();
    models.push(("qwen3-heads256".to_string(), wide_heads, false));
    models.extend([Tiny::Qwen3, Tiny::DeepSeek2].map(|t| (format!("{}-medium", t.name()), t.medium(CTX), true)));
    for (name, spec, q4k) in &models {
        let ckpt = dir().join(format!("{name}-apart.gguf"));
        if *q4k {
            use logicaffeine_compile::model::quant::GgmlType;
            let matrices: Vec<String> = tiny_models::tensors(spec)
                .into_iter()
                .map(|(n, _)| n)
                .filter(|n| n.ends_with(".weight") && !n.contains("norm") && !n.contains("ffn_gate_inp"))
                .collect();
            tiny_models::write_checkpoint_as(spec, &ckpt, &|t| if matrices.iter().any(|m| m == t) { GgmlType::Q4K } else { GgmlType::F32 });
        } else {
            tiny_models::write_checkpoint(spec, &ckpt);
        }
        let built: Vec<(String, Built, Built)> = std::thread::scope(|scope| {
            let jobs: Vec<_> = [Q8MatKind::Int8Act, Q8MatKind::Dequant]
                .into_iter()
                .map(|numerics| {
                    let ckpt = &ckpt;
                    scope.spawn(move || {
                        let what = format!("{name}-{numerics:?}");
                        let opts = LaneOptions { ctx: CTX, knobs: CudaKnobs { activations: numerics, ..CudaKnobs::default() }, prefill: 4 };
                        let fused = compile(spec, ckpt, &opts).unwrap_or_else(|e| panic!("{what}: {e}"));
                        let apart_opts = LaneOptions { knobs: CudaKnobs { fuse_quant: false, ..opts.knobs }, ..opts };
                        let apart = compile_holding(spec, ckpt, &apart_opts, &RULES).unwrap_or_else(|e| panic!("{what} apart: {e}"));
                        assert!(apart.forms.iter().all(|f| f.schedule.fusions.iter().all(|x| x.rule == "argmax_advance")), "{what}");
                        (what.clone(), build(&fused, &format!("fused-{what}")), build(&apart, &format!("apart-{what}")))
                    })
                })
                .collect();
            jobs.into_iter().map(|j| j.join().unwrap()).collect()
        });
        for (what, fused, apart) in &built {
            let (a, b) = (exec(fused, &ckpt, &prompt, GENERATE, spec.vocab, true), exec(apart, &ckpt, &prompt, GENERATE, spec.vocab, true));
            assert_eq!(b.tokens, a.tokens, "{what}: the program apart generates the fused one's tokens");
            for (i, (x, y)) in a.logits.iter().zip(&b.logits).enumerate() {
                let parted = x.iter().zip(y).position(|(p, q)| p.to_bits() != q.to_bits());
                assert!(parted.is_none(), "{what}: position {i}'s logit {parted:?} parts: {:?} fused, {:?} apart", parted.map(|v| x[v]), parted.map(|v| y[v]));
            }
        }
    }
}

/// A projection's last-arriving block norms the row it wrote as the norm's own launch does (PARITY-GEMV-NORM-EPILOGUE):
/// Llama, Gemma-3 (both its norms in the fold) and Qwen3 at 2048 wide, under the quantized and the precise numerics, with
/// their projections folding the add and norm and with `gemv_norm_epilogue` held, generate the same tokens and the same
/// logits bit for bit through a prompt of chunks and decode steps — the fold's reduction is `lk_rmsnorm`'s tree.
#[test]
fn the_last_block_norm_computes_what_the_norm_launch_computes() {
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let models = [("llama", Tiny::Llama.spec(CTX)), ("gemma3", Tiny::Gemma3.spec(CTX)), ("qwen3-medium", Tiny::Qwen3.medium(CTX))];
    let mut lanes = Vec::new();
    for (name, spec) in &models {
        let ckpt = dir().join(format!("{name}-fold.gguf"));
        tiny_models::write_checkpoint(spec, &ckpt);
        for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8, Q8MatKind::Dequant] {
            let what = format!("{name} {numerics:?}");
            let opts = LaneOptions { ctx: CTX, knobs: CudaKnobs { activations: numerics, ..CudaKnobs::default() }, prefill: 4 };
            let folded = compile(spec, &ckpt, &opts).unwrap_or_else(|e| panic!("{what}: {e}"));
            assert!(folded.decode().schedule.launches.iter().any(|l| kind(l) == "gemv_norm"), "{what}: the decode step folds");
            let apart = compile_holding(spec, &ckpt, &opts, &["gemv_norm_epilogue"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
            lanes.push((what, spec.vocab, ckpt.clone(), folded, apart));
        }
    }
    held_alike(&lanes, "fold");
}

/// A mixture's last step norms what the mixture wrote as the norm's own launch does (PARITY-MOE-DEFERRED-FINALIZE):
/// Qwen3-MoE, gpt-oss and DeepSeek-V2 tiny; Qwen3-MoE and DeepSeek-V2 at 2048 wide and gpt-oss at its own 2880 —
/// where the decode step's down block of a few warps plays the norm's block of 1024 threads, three values a thread
/// and the last ragged — under the quantized and the precise numerics, with their mixtures folding the add and norm
/// and with `moe_deferred_finalize` held, generate the same tokens and the same logits bit for bit through a prompt of
/// chunks and decode steps: the fold's reduction is `lk_rmsnorm`'s tree.
#[test]
fn the_mixtures_last_step_norms_as_the_norm_launch_does() {
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let mut oss = Tiny::GptOss.medium(CTX);
    oss.d = 2880;
    let models = [
        ("qwen3moe", Tiny::Qwen3Moe.spec(CTX)),
        ("gpt-oss", Tiny::GptOss.spec(CTX)),
        ("deepseek2", Tiny::DeepSeek2.spec(CTX)),
        ("qwen3moe-medium", Tiny::Qwen3Moe.medium(CTX)),
        ("deepseek2-medium", Tiny::DeepSeek2.medium(CTX)),
        ("gpt-oss-2880", oss),
    ];
    let mut lanes = Vec::new();
    for (name, spec) in &models {
        let ckpt = dir().join(format!("{name}-moe-fold.gguf"));
        tiny_models::write_checkpoint(spec, &ckpt);
        for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8, Q8MatKind::Dequant] {
            let what = format!("{name} {numerics:?}");
            let opts = LaneOptions { ctx: CTX, knobs: CudaKnobs { activations: numerics, ..CudaKnobs::default() }, prefill: 4 };
            let folded = compile(spec, &ckpt, &opts).unwrap_or_else(|e| panic!("{what}: {e}"));
            for f in &folded.forms {
                assert!(f.schedule.launches.iter().any(|l| kind(l) == "moe_norm"), "{what}: the form of {} rows folds", f.rows);
            }
            let apart = compile_holding(spec, &ckpt, &opts, &["moe_deferred_finalize"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
            lanes.push((what, spec.vocab, ckpt.clone(), folded, apart));
        }
    }
    held_alike(&lanes, "moe-fold");
}

/// The delta rule's step with the gated norm in its last block a head computes, bit for bit, what the step and then the
/// norm kernel compute (K-10): heads 32, 64, 128 and 256 wide (one warp of the step's four playing the norm's block, up
/// to two norm warps a step warp), four value heads over two key heads, three rows in turn and a batch of two
/// sequences a row each, the memory in `f32` and in bf16, under the precise numerics (`f64` sums, the norm written
/// `f32`) and the one- and two-word quantized ones (`f32` sums, the norm's words and scales, written `f32` too or not)
/// — two steps in a row, so each head's count is back at zero for the second — every output, memory value, norm,
/// word and scale (they start as NaN).
#[test]
fn the_stepped_delta_rules_last_block_norms_as_the_norm_kernel_does() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{Kernel, PRELUDE};
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    const HV: usize = 4;
    const NK: usize = 2;
    let unit = |i: usize, salt: u64| {
        let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(27) ^ 0x2545_F491_4F6C_DD1D;
        (h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    };
    let dir = dir().join("delta-norm");
    std::fs::create_dir_all(&dir).unwrap();
    let write = |name: String, v: &[f32]| std::fs::write(dir.join(name), v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    let mut calls = String::new();
    let mut cases = Vec::new();
    for d in [32usize, 64, 128, 256] {
        for (rows, seqs) in [(3usize, 1usize), (1, 2)] {
            let (n, tag) = (rows * seqs, format!("{d}-{rows}-{seqs}"));
            let normed = |salt: u64| -> Vec<f32> {
                (0..n * NK)
                    .flat_map(|u| {
                        let x: Vec<f64> = (0..d).map(|i| unit(u * d + i, salt)).collect();
                        let len = x.iter().map(|v| v * v).sum::<f64>().sqrt();
                        x.into_iter().map(move |v| (v / len) as f32)
                    })
                    .collect()
            };
            write(format!("q{tag}.bin"), &normed(1));
            write(format!("k{tag}.bin"), &normed(2));
            write(format!("v{tag}.bin"), &(0..n * HV * d).map(|i| unit(i, 3) as f32).collect::<Vec<f32>>());
            write(format!("b{tag}.bin"), &(0..n * HV).map(|i| (1.0 / (1.0 + (-2.0 * unit(i, 4)).exp())) as f32).collect::<Vec<f32>>());
            write(format!("a{tag}.bin"), &(0..n * HV).map(|i| (0.75 + 0.25 * unit(i, 5)) as f32).collect::<Vec<f32>>());
            write(format!("s{tag}.bin"), &(0..seqs * HV * d * d).map(|i| (0.1 * unit(i, 6)) as f32).collect::<Vec<f32>>());
            write(format!("z{tag}.bin"), &(0..n * HV * d).map(|i| (3.0 * unit(i, 7)) as f32).collect::<Vec<f32>>());
            write(format!("g{tag}.bin"), &(0..d).map(|i| (1.0 + 0.5 * unit(i, 8)) as f32).collect::<Vec<f32>>());
            for (ss, mem) in [("float", "f32"), ("__nv_bfloat16", "bf16")] {
                for (q, acc, outf) in [(0, "double", true), (1, "float", true), (1, "float", false), (2, "float", false)] {
                    let words = n * HV * d;
                    let _ = write!(
                        calls,
                        "  {{ // heads {d} wide, {rows} rows a sequence, {seqs} sequences, memory {mem}, Q {q}, f32 out {outf}\n\
                         \x20   float *q = dev(dir + \"/q{tag}.bin\"), *k = dev(dir + \"/k{tag}.bin\"), *v = dev(dir + \"/v{tag}.bin\"), *b = dev(dir + \"/b{tag}.bin\"), *a = dev(dir + \"/a{tag}.bin\");\n\
                         \x20   float *z = dev(dir + \"/z{tag}.bin\"), *g = dev(dir + \"/g{tag}.bin\");\n\
                         \x20   {ss} *sa = mem<{ss}>(dir + \"/s{tag}.bin\", {}), *sb = mem<{ss}>(dir + \"/s{tag}.bin\", {});\n\
                         \x20   float *oa = nan({words}), *ob = nan({words}), *ya = nan({words}), *yb = nan({words}), *xa = nan({words} / 32), *xb = nan({words} / 32);\n\
                         \x20   signed char *wa = bytes({words}), *wb = bytes({words}), *la = bytes({words}), *lb = bytes({words});\n\
                         \x20   int* cnt; CK(cudaMalloc(&cnt, {seqs} * {HV} * 4)); CK(cudaMemset(cnt, 0, {seqs} * {HV} * 4));\n\
                         \x20   for(int step = 0; step < 2; ++step){{\n\
                         \x20     lk_delta_step<{acc}, {dw}, {ss}><<<dim3({HV}, {blocks}, {seqs}), 128>>>(q, k, v, b, a, sa, oa, {d}, {NK}, {HV}, {rows}, {batch}); CK(cudaGetLastError());\n\
                         \x20     lk_delta_norm<{outf}, {q}, {acc}><<<{rows} * {HV} * {seqs}, {d}>>>(z, g, oa, {outa}, wa, la, xa, {d}, 1e-6f); CK(cudaGetLastError());\n\
                         \x20     lk_delta_step<{acc}, {dw}, {outf}, {q}, {vw}, {ss}><<<dim3({HV}, {blocks}, {seqs}), 128>>>(q, k, v, b, a, sb, ob, {d}, {NK}, {HV}, {rows}, {batch}, z, g, {outb}, wb, lb, xb, 1e-6f, cnt); CK(cudaGetLastError());\n\
                         \x20     CK(cudaDeviceSynchronize());\n\
                         \x20     put(out, oa, {words}); put(out, ob, {words}); put(out, ya, {words}); put(out, yb, {words});\n\
                         \x20     putb(out, wa, {words}); putb(out, wb, {words}); putb(out, la, {words}); putb(out, lb, {words}); put(out, xa, {words} / 32); put(out, xb, {words} / 32);\n\
                         \x20     putm(out, sa, {}); putm(out, sb, {}); puti(out, cnt, {seqs} * {HV});\n\
                         \x20   }}\n  }}\n",
                        seqs * HV * d * d,
                        seqs * HV * d * d,
                        seqs * HV * d * d,
                        seqs * HV * d * d,
                        dw = d.div_ceil(32),
                        vw = d.div_ceil(32).div_ceil(4),
                        blocks = d.div_ceil(4),
                        batch = u8::from(seqs > 1),
                        outa = if outf { "ya" } else { "nullptr" },
                        outb = if outf { "yb" } else { "nullptr" },
                    );
                    cases.push((format!("heads {d}, {rows} rows × {seqs} sequences, {mem} memory, Q {q}, f32 out {outf}"), words, seqs * HV * d * d, seqs * HV, q, outf));
                }
            }
        }
    }
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <string>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n#include <cuda_bf16.h>\n\
         {PRELUDE}{}{}{}\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ fprintf(stderr, \"cuda %d %s\\n\", __LINE__, cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         static std::vector<float> slurp(const std::string& p){{ FILE* f = fopen(p.c_str(), \"rb\"); if(!f) exit(2); std::vector<float> b; float c[16384]; size_t n;\n\
           while((n = fread(c, 4, 16384, f)) > 0) b.insert(b.end(), c, c + n); fclose(f); return b; }}\n\
         static float* dev(const std::string& p){{ auto b = slurp(p); float* d; CK(cudaMalloc(&d, b.size() * 4)); CK(cudaMemcpy(d, b.data(), b.size() * 4, cudaMemcpyHostToDevice)); return d; }}\n\
         template<typename T> static T* mem(const std::string& p, long n){{ auto b = slurp(p); std::vector<T> h(n); for(long i = 0; i < n; ++i) h[i] = (T)b[i];\n\
           T* d; CK(cudaMalloc(&d, n * sizeof(T))); CK(cudaMemcpy(d, h.data(), n * sizeof(T), cudaMemcpyHostToDevice)); return d; }}\n\
         static float* nan(long n){{ float* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0xff, n * 4)); return p; }}\n\
         static signed char* bytes(long n){{ signed char* p; CK(cudaMalloc(&p, n)); CK(cudaMemset(p, 0x5a, n)); return p; }}\n\
         static void put(FILE* out, const float* d, long n){{ std::vector<float> h(n); CK(cudaMemcpy(h.data(), d, n * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, n, out); }}\n\
         static void putb(FILE* out, const signed char* d, long n){{ std::vector<signed char> h(n); CK(cudaMemcpy(h.data(), d, n, cudaMemcpyDeviceToHost)); for(auto c : h){{ float f = c; fwrite(&f, 4, 1, out); }} }}\n\
         static void puti(FILE* out, const int* d, long n){{ std::vector<int> h(n); CK(cudaMemcpy(h.data(), d, n * 4, cudaMemcpyDeviceToHost)); for(auto c : h){{ float f = c; fwrite(&f, 4, 1, out); }} }}\n\
         template<typename T> static void putm(FILE* out, const T* d, long n){{ std::vector<T> h(n); CK(cudaMemcpy(h.data(), d, n * sizeof(T), cudaMemcpyDeviceToHost)); for(auto c : h){{ float f = (float)c; fwrite(&f, 4, 1, out); }} }}\n\
         int main(int argc, char** argv){{\n  std::string dir = argv[1];\n  FILE* out = fopen(argv[2], \"wb\");\n{calls}  fclose(out);\n  return 0;\n}}\n",
        Kernel::DeltaStep.source(),
        Kernel::DeltaNorm.source(),
        Kernel::DeltaStepNorm.source()
    );
    let (cu, bin, outp) = (dir.join("delta_norm.cu"), dir.join("delta_norm"), dir.join("out.bin"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "the step, the norm and the step with its norm build:
{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).arg(&dir).arg(&outp).output().unwrap();
    assert!(ran.status.success(), "they run: {}", String::from_utf8_lossy(&ran.stderr));
    let all: Vec<f32> = std::fs::read(&outp).unwrap().chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
    let mut at = 0;
    let mut take = |n: usize| {
        at += n;
        all[at - n..at].to_vec()
    };
    let same = |what: &str, name: &str, a: &[f32], b: &[f32]| {
        let parted = a.iter().zip(b).position(|(x, y)| x.to_bits() != y.to_bits());
        assert!(parted.is_none(), "{what}: {name} {parted:?} parts: {:?} apart, {:?} in one kernel", parted.map(|i| a[i]), parted.map(|i| b[i]));
    };
    for (what, words, memory, heads, q, outf) in &cases {
        for step in 0..2 {
            let what = format!("{what}, step {}", step + 1);
            let (oa, ob, ya, yb) = (take(*words), take(*words), take(*words), take(*words));
            let (wa, wb, la, lb, xa, xb) = (take(*words), take(*words), take(*words), take(*words), take(words / 32), take(words / 32));
            let (sa, sb, cnt) = (take(*memory), take(*memory), take(*heads));
            assert!(oa.iter().chain(&sa).all(|f| f.is_finite()), "{what}: every output and memory value written, and finite");
            same(&what, "unnormed output", &oa, &ob);
            same(&what, "memory value", &sa, &sb);
            if *outf {
                assert!(ya.iter().all(|f| f.is_finite()), "{what}: every norm written");
                same(&what, "norm", &ya, &yb);
            }
            if *q > 0 {
                same(&what, "word", &wa, &wb);
                same(&what, "scale", &xa, &xb);
                if *q == 2 {
                    same(&what, "low word", &la, &lb);
                }
            }
            assert!(cnt.iter().all(|c| *c == 0.0), "{what}: every head's count is back at zero");
        }
    }
    assert_eq!(at, all.len(), "every result read");
}

/// A block-format projection's last-arriving block norms the row it wrote as the norm's own launch does
/// (PARITY-GEMV-NORM-EPILOGUE): Llama and DeepSeek-V2 at 2048 wide in Q4_K — four rows a warp, each warp of the block's
/// four playing eight of the norm's 32 — and Llama wide in Q6_K, Q5_0 and MXFP4, under the quantized and the precise
/// numerics, with their projections folding the add and norm and with `gemv_norm_epilogue` held, generate the same
/// tokens and the same logits bit for bit through a prompt of chunks and decode steps.
#[test]
fn the_block_projections_last_block_norms_as_the_norm_launch_does() {
    use logicaffeine_compile::model::quant::GgmlType;
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let models = [
        ("llama-medium-q4k", Tiny::Llama.medium(CTX), GgmlType::Q4K),
        ("deepseek2-medium-q4k", Tiny::DeepSeek2.medium(CTX), GgmlType::Q4K),
        ("llama-wide-q6k", Tiny::Llama.wide(CTX), GgmlType::Q6K),
        ("llama-wide-q5_0", Tiny::Llama.wide(CTX), GgmlType::Q5_0),
        ("llama-wide-mxfp4", Tiny::Llama.wide(CTX), GgmlType::Mxfp4),
    ];
    let mut lanes = Vec::new();
    for (name, spec, ty) in &models {
        let ckpt = dir().join(format!("{name}-blk-norm.gguf"));
        let matrices: Vec<String> = tiny_models::tensors(spec)
            .into_iter()
            .map(|(n, _)| n)
            .filter(|n| n.ends_with(".weight") && !n.contains("norm") && !n.contains("ffn_gate_inp"))
            .collect();
        tiny_models::write_checkpoint_as(spec, &ckpt, &|t| if matrices.iter().any(|m| m == t) { *ty } else { GgmlType::F32 });
        for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8, Q8MatKind::Dequant] {
            let what = format!("{name} {numerics:?}");
            let opts = LaneOptions { ctx: CTX, knobs: CudaKnobs { activations: numerics, ..CudaKnobs::default() }, prefill: 4 };
            let folded = compile(spec, &ckpt, &opts).unwrap_or_else(|e| panic!("{what}: {e}"));
            assert!(folded.program.cu.contains("lk_gemv_blk_norm<"), "{what}: the decode step folds");
            let apart = compile_holding(spec, &ckpt, &opts, &["gemv_norm_epilogue"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
            lanes.push((what, spec.vocab, ckpt.clone(), folded, apart));
        }
    }
    held_alike(&lanes, "blk-norm");
}

/// A gated attention's store gates its output as the product with the gate's activation does (`attn_output_gate`):
/// Qwen3.5 tiny and wide, under the quantized and the precise numerics, gating in the store and with `attn_output_gate`
/// held, generate the same tokens and the same logits bit for bit through a prompt of chunks and decode steps.
#[test]
fn the_gated_attention_store_computes_what_the_gated_product_computes() {
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let tiny = Tiny::Qwen35.spec(CTX);
    let tiny_ckpt = dir().join("qwen35-attn-gate.gguf");
    tiny_models::write_checkpoint(&tiny, &tiny_ckpt);
    let wide = Tiny::Qwen35.wide(CTX);
    let wide_ckpt = dir().join("qwen35-wide-attn-gate.gguf");
    tiny_models::write_checkpoint(&wide, &wide_ckpt);
    let mut lanes = Vec::new();
    for (name, spec, ckpt) in [("qwen35", &tiny, &tiny_ckpt), ("qwen35-wide", &wide, &wide_ckpt)] {
        for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8, Q8MatKind::Dequant] {
            let what = format!("{name} {numerics:?}");
            let opts = LaneOptions { ctx: CTX, knobs: CudaKnobs { activations: numerics, ..CudaKnobs::default() }, prefill: 4 };
            let gated = compile(spec, ckpt, &opts).unwrap_or_else(|e| panic!("{what}: {e}"));
            assert!(gated.decode().schedule.fired("attn_output_gate") > 0, "{what}: the decode step gates in its store");
            let apart = compile_holding(spec, ckpt, &opts, &["attn_output_gate"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
            lanes.push((what, spec.vocab, ckpt.clone(), gated, apart));
        }
    }
    held_alike(&lanes, "attn-gate");
}

/// Each `(what, vocab, checkpoint, fused, held)` program pair, built on the pool, generates the same tokens and the same
/// logits bit for bit through a prompt of chunks and decode steps.
fn held_alike(lanes: &[(String, usize, PathBuf, Lane, Lane)], tag: &str) {
    let prompt = [5, 17, 2, 30, 9, 11, 3];
    let names: Vec<(String, String)> = lanes.iter().map(|(what, ..)| (format!("{tag}-{what}"), format!("{tag}-held-{what}"))).collect();
    let jobs: Vec<(&Lane, &str)> = lanes.iter().zip(&names).flat_map(|((_, _, _, fused, held), (f, h))| [(fused, f.as_str()), (held, h.as_str())]).collect();
    let built = build_all(&jobs);
    for ((what, vocab, ckpt, ..), pair) in lanes.iter().zip(built.chunks(2)) {
        let (a, b) = (exec(&pair[0], ckpt, &prompt, GENERATE, *vocab, true), exec(&pair[1], ckpt, &prompt, GENERATE, *vocab, true));
        assert_eq!(a.tokens, b.tokens, "{what}: fused, the step generates the tokens it generates held");
        for (i, (x, y)) in a.logits.iter().zip(&b.logits).enumerate() {
            let parted = x.iter().zip(y).position(|(p, q)| p.to_bits() != q.to_bits());
            assert!(parted.is_none(), "{what}: position {i}'s logit {parted:?} parts: {:?} fused, {:?} held", parted.map(|v| x[v]), parted.map(|v| y[v]));
        }
    }
}

/// The heads a projection turns in its epilogue are the heads its launch of rotations turns (PARITY-QKV-HEAD-EPILOGUE):
/// with the query, key and value projections turning their heads in the blocks that finish them and with
/// `qkv_head_epilogue` held, generate the same tokens and the same logits bit for bit through a prompt of chunks and
/// decode steps, under the quantized and the precise numerics — Llama's unnormed heads; Gemma-3's and Qwen3's normed
/// heads of 16, several to a block; GPT-OSS's biased projections; Qwen3 at 2048 wide in Q4_K with 32 query heads of 64,
/// its query projection four rows a warp turning its heads alone and its key's turned beside the values; and Qwen3 with
/// heads of 512, whose norm takes more threads than the GEMV's eight warps.
#[test]
fn the_head_epilogue_computes_what_the_rope_launches_compute() {
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    use logicaffeine_compile::model::quant::GgmlType;
    let mut wide_heads = Tiny::Qwen3.spec(CTX);
    for layer in &mut wide_heads.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            (a.head_dim, a.value_dim, a.scale) = (512, 512, 1.0 / (512f64).sqrt());
            if let Some(r) = &mut a.rope {
                r.dims = 512;
            }
        }
    }
    let mut medium = Tiny::Qwen3.medium(CTX);
    for layer in &mut medium.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            (a.heads, a.kv_heads) = (32, 8);
        }
    }
    let models = [
        ("llama", Tiny::Llama.spec(CTX), false),
        ("gemma3", Tiny::Gemma3.spec(CTX), false),
        ("qwen3", Tiny::Qwen3.spec(CTX), false),
        ("gpt-oss", Tiny::GptOss.spec(CTX), false),
        ("qwen3-medium", medium, true),
        ("qwen3-heads512", wide_heads, false),
    ];
    let mut lanes = Vec::new();
    for (name, spec, q4k) in &models {
        let ckpt = dir().join(format!("{name}-heads.gguf"));
        if *q4k {
            let matrices: Vec<String> = tiny_models::tensors(spec)
                .into_iter()
                .map(|(n, _)| n)
                .filter(|n| n.ends_with(".weight") && !n.contains("norm") && !n.contains("ffn_gate_inp"))
                .collect();
            tiny_models::write_checkpoint_as(spec, &ckpt, &|t| if matrices.iter().any(|m| m == t) { GgmlType::Q4K } else { GgmlType::F32 });
        } else {
            tiny_models::write_checkpoint(spec, &ckpt);
        }
        for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8, Q8MatKind::Dequant] {
            let what = format!("{name} {numerics:?}");
            let opts = LaneOptions { ctx: CTX, knobs: CudaKnobs { activations: numerics, ..CudaKnobs::default() }, prefill: 4 };
            let turned = compile(spec, &ckpt, &opts).unwrap_or_else(|e| panic!("{what}: {e}"));
            assert!(turned.decode().schedule.launches.iter().any(|l| kind(l) == "gemv_rope"), "{what}: the decode step turns its heads in its projections");
            if *q4k {
                let wide = turned.decode().schedule.launches.iter().any(|l| matches!(l, Launch::Fold { first, .. } if matches!(first.as_ref(), Launch::GemmParts { per_warp: 4, .. })));
                assert!(wide, "{what}: the query projection turns its heads at four rows a warp");
            }
            let apart = compile_holding(spec, &ckpt, &opts, &["qkv_head_epilogue"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
            lanes.push((what, spec.vocab, ckpt.clone(), turned, apart));
        }
    }
    held_alike(&lanes, "heads");
}

/// Every fusion of the hybrid step computes what its launches compute apart at the medium width: Qwen3.5 at 2048 wide,
/// its matrices of whole blocks in Q8_0 as the real checkpoint's are — its gates' projections quantized projections, or
/// kept in `f32` and cut over warps beside their fused gates — its convolutions, rotations and gated pairs at a real
/// layer's widths, with every rule held back and every activation quantized in its own launch, generates the fused
/// program's tokens and logits bit for bit, under the quantized and the precise numerics, through a prompt that runs
/// chunks and decode steps.
#[test]
fn every_fusion_of_the_hybrid_step_computes_what_its_launches_compute_apart_at_width() {
    use logicaffeine_compile::model::quant::GgmlType;
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let prompt = [5, 17, 2, 30, 9, 11, 3];
    let spec = Tiny::Qwen35.medium(CTX);
    let blocked: Vec<String> = tiny_models::tensors(&spec)
        .into_iter()
        .filter(|(n, dims)| dims.len() == 2 && dims[0] % 32 == 0 && !n.contains("norm"))
        .map(|(n, _)| n)
        .collect();
    for gates in [GgmlType::Q8_0, GgmlType::F32] {
        let ckpt = dir().join(format!("qwen35-medium-apart-{}.gguf", gates.name()));
        tiny_models::write_checkpoint_as(&spec, &ckpt, &|t| {
            if t.contains("ssm_alpha") || t.contains("ssm_beta") {
                gates
            } else if blocked.iter().any(|b| b == t) {
                GgmlType::Q8_0
            } else {
                GgmlType::F32
            }
        });
        for numerics in [Q8MatKind::Int8Act, Q8MatKind::Dequant] {
            let what = format!("qwen35-medium-{}-gates-{numerics:?}", gates.name());
            let opts = LaneOptions { ctx: CTX, knobs: CudaKnobs { activations: numerics, ..CudaKnobs::default() }, prefill: 4 };
            let fused = compile(&spec, &ckpt, &opts).unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_eq!(fused.decode().schedule.fired("gate_projections") > 0, gates == GgmlType::F32, "{what}: the gates fuse with f32 projections only");
            let apart_opts = LaneOptions { knobs: CudaKnobs { fuse_quant: false, ..opts.knobs }, ..opts };
            let apart = compile_holding(&spec, &ckpt, &apart_opts, &RULES).unwrap_or_else(|e| panic!("{what} apart: {e}"));
            let built = build_all(&[(&fused, &format!("fused-{what}")), (&apart, &format!("apart-{what}"))]);
            let (a, b) = (exec(&built[0], &ckpt, &prompt, GENERATE, spec.vocab, true), exec(&built[1], &ckpt, &prompt, GENERATE, spec.vocab, true));
            assert_eq!(b.tokens, a.tokens, "{what}: the program apart generates the fused one's tokens");
            for (i, (x, y)) in a.logits.iter().zip(&b.logits).enumerate() {
                let parted = x.iter().zip(y).position(|(p, q)| p.to_bits() != q.to_bits());
                assert!(parted.is_none(), "{what}: position {i}'s logit {parted:?} parts: {:?} fused, {:?} apart", parted.map(|v| x[v]), parted.map(|v| y[v]));
            }
        }
    }
}

/// The medium widths generate what the reference computes: Qwen3, Qwen3-MoE and DeepSeek-V2 at 2048 wide in Q4_K with
/// an `f32` router — each projection, gated pair and mixture of 2048 rows taking four rows a warp, the latent query
/// projection quantizing in its epilogue, every norm holding its row across 1024 threads, the router reading each
/// expert's row with four warps — a prompt run in chunks of four, held to the reference as the tiny architectures are,
/// under the precise and the two-word numerics. One int8 word at this width rounds about a hundred values a position
/// within `NEAR_ROUNDING` of a half-way point, more decisions than the near-call search takes the lane's way, so at this
/// width it is held to its program apart bit for bit (`every_fusion_computes_what_its_launches_compute_apart`) and to
/// llama.cpp's runs of the real checkpoints.
#[test]
fn the_medium_widths_generate_what_the_reference_computes() {
    use logicaffeine_compile::model::quant::GgmlType;
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let mut report = Vec::new();
    for t in [Tiny::Qwen3, Tiny::Qwen3Moe, Tiny::DeepSeek2] {
        let spec = t.medium(CTX);
        let ckpt = dir().join(format!("{}-medium.gguf", t.name()));
        let matrices: Vec<String> = tiny_models::tensors(&spec)
            .into_iter()
            .map(|(n, _)| n)
            .filter(|n| n.ends_with(".weight") && !n.contains("norm") && !n.contains("ffn_gate_inp"))
            .collect();
        tiny_models::write_checkpoint_as(&spec, &ckpt, &|n| if matrices.iter().any(|m| m == n) { GgmlType::Q4K } else { GgmlType::F32 });
        for numerics in [Q8MatKind::Dequant, Q8MatKind::TwoPassInt8] {
            let what = format!("{}-medium {numerics:?}", t.name());
            let lane = compile(&spec, &ckpt, &LaneOptions { ctx: CTX, knobs: CudaKnobs { activations: numerics, ..CudaKnobs::default() }, prefill: 4 })
                .unwrap_or_else(|e| panic!("{what}: {e}"));
            assert!(lane.decode().schedule.fired("rows_a_warp") > 0, "{what}: a shape takes four rows a warp");
            let r = run(&lane, &ckpt, PROMPT, GENERATE, &format!("{}-medium-{numerics:?}", t.name()), spec.vocab, true);
            let (worst, flips) = held_to_reference(&spec, &ckpt, PROMPT, &r, numerics, &what);
            report.push(format!("{what}: logits within {worst:.3e} of their scale, {flips} near calls decided the other way"));
        }
    }
    eprintln!("{}", report.join("\n"));
}

/// The real Qwen3-8B, compiled from its spec, continues llama.cpp's golden prompt with llama.cpp's golden tokens.
#[test]
fn the_real_qwen3_8b_continues_llama_cpps_golden() {
    if !(common::require_tool("qwen3-8b-checkpoint") && common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let golden: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/qwen_golden.json"))
            .unwrap(),
    )
    .unwrap();
    let ids = |k: &str| -> Vec<u32> { golden[k].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect() };
    let (prompt, want) = (ids("prompt_tokens"), ids("generated_tokens"));
    let ckpt = common::qwen3_8b_checkpoint();
    let spec = ModelSpec::from_gguf(&Gguf::open(&ckpt).unwrap()).unwrap();
    for numerics in [Q8MatKind::Int8Act, Q8MatKind::Dequant] {
        let lane = compile(&spec, &ckpt, &LaneOptions { ctx: 256, knobs: CudaKnobs { activations: numerics, ..CudaKnobs::default() }, prefill: 16 })
            .unwrap_or_else(|e| panic!("Qwen3-8B {numerics:?}: {e}"));
        let r = run(&lane, &ckpt, &prompt, want.len(), &format!("qwen3-8b-{numerics:?}"), spec.vocab, false);
        assert_eq!(r.tokens, want, "Qwen3-8B {numerics:?} continues the golden prompt as llama.cpp does");
        eprintln!("Qwen3-8B {numerics:?}: {:.2} tok/s", r.tok_per_s.unwrap_or(0.0));
    }
}

/// One prompt of a golden: its tokens, its greedy continuation, and the top logits after the prompt.
struct Prompt {
    tokens: Vec<u32>,
    continuation: Vec<u32>,
    top: Vec<(u32, f64)>,
}

fn golden(model: &str, which: &str) -> Vec<Prompt> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../benchmarks/models/{model}/golden-{which}.json"));
    let g: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let ids = |v: &serde_json::Value| -> Vec<u32> { v.as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as u32).collect() };
    g["prompts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| Prompt {
            tokens: ids(&p["tokens"]),
            continuation: ids(&p["continuation"]),
            top: p["top"].as_array().unwrap().iter().map(|e| (e[0].as_u64().unwrap() as u32, e[1].as_f64().unwrap())).collect(),
        })
        .collect()
}

fn agreed(a: &[u32], b: &[u32]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

/// The mean of `xs` and the variance of that mean.
fn mean_and_variance(xs: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    let var = xs.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (n - 1.0);
    (mean, var / n)
}

/// Every point of the `activations` knob, on a real checkpoint the lane compiles, held to the bar it earns against
/// llama.cpp's runs of the 32-prompt corpus (prompts run as prefill chunks):
///
///   • the precise and the two-pass lanes are more exact than llama.cpp ships: their greedy continuations follow
///     llama.cpp's `f32` run, summed over the corpus, at least as far as llama.cpp's shipped run does;
///   • the int8 lane is llama.cpp's own activation precision, so it is held to llama.cpp's closeness rather than to a
///     greedy walk a near-tie decides: over the corpus, its logits after the prompt sit no further from the `f32` run's
///     top five, on average, than the shipped run's do, beyond two standard errors of the difference of the means.
///
/// One test a checkpoint, each within its own time budget.
fn meets_every_activation_precisions_bar(model: &str) {
    meets_every_activation_precisions_bar_over(model, "", 16, 128);
}

/// The same bars over the long-prompt corpus (`benchmarks/models/prompts-long.txt`, prompts of several hundred tokens),
/// its prompts prefilled 64 rows a chunk, so a prompt runs on the wide chunks' paths — the projections on the tensor
/// cores, the attention in tiles.
fn meets_every_activation_precisions_bar_on_long_prompts(model: &str) {
    meets_every_activation_precisions_bar_over(model, "long-", 64, 1024);
}

/// The bars over the corpus whose goldens are `golden-{corpus}f32.json` and `golden-{corpus}cpu.json`, prompts run in
/// chunks of up to `prefill` rows within a cache of `ctx`.
fn meets_every_activation_precisions_bar_over(model: &str, corpus: &str, prefill: usize, ctx: usize) {
    if !(common::require_tool("nvcc") && common::require_gpu() && common::require_tool(&format!("checkpoint:{model}"))) {
        return;
    }
    let ckpt = common::checkpoint(model).unwrap();
    let spec = ModelSpec::from_gguf(&Gguf::open(&ckpt).unwrap()).unwrap();
    let (f32g, cpu) = (golden(model, &format!("{corpus}f32")), golden(model, &format!("{corpus}cpu")));
    let theirs: usize = f32g.iter().zip(&cpu).map(|(f, c)| agreed(&c.continuation, &f.continuation)).sum();
    let shipped_gap: Vec<f64> = f32g
        .iter()
        .zip(&cpu)
        .map(|(f, c)| {
            let at: std::collections::HashMap<u32, f64> = c.top.iter().copied().collect();
            f.top.iter().filter_map(|(id, v)| at.get(id).map(|s| (s - v).abs())).fold(0f64, f64::max)
        })
        .collect();
    for numerics in [Q8MatKind::Dequant, Q8MatKind::TwoPassInt8, Q8MatKind::Int8Act] {
        let what = format!("{model} {corpus}{numerics:?}");
        let opts = LaneOptions { ctx, knobs: CudaKnobs { activations: numerics, ..CudaKnobs::default() }, prefill };
        let lane = compile(&spec, &ckpt, &opts).unwrap_or_else(|e| panic!("{what}: {e}"));
        let built = build(&lane, &format!("{model}-{corpus}{numerics:?}"));
        let (mut ours, mut gap) = (0, Vec::new());
        let jobs: Vec<(Vec<u32>, usize)> = f32g.iter().map(|f| (f.tokens.clone(), f.continuation.len())).collect();
        for (f, r) in f32g.iter().zip(exec_many(&built, &ckpt, &jobs, spec.vocab, true)) {
            ours += agreed(&r.tokens, &f.continuation);
            gap.push(f.top.iter().map(|&(id, v)| (r.logits[0][id as usize] as f64 - v).abs()).fold(0f64, f64::max));
        }
        let ((m_ours, v_ours), (m_theirs, v_theirs)) = (mean_and_variance(&gap), mean_and_variance(&shipped_gap));
        eprintln!(
            "{what}: follows the f32 run {ours} steps (llama.cpp shipped {theirs}); mean top-logit gap {m_ours:.4} (shipped {m_theirs:.4}); \
             {} bytes on the device, {} in host memory ({:?})",
            lane.residence.device_bytes,
            lane.residence.host_bytes,
            lane.residence.host
        );
        match numerics {
            Q8MatKind::Int8Act => assert!(
                m_ours <= m_theirs + 2.0 * (v_ours + v_theirs).sqrt(),
                "{what}: the mean top-logit gap {m_ours:.4} is past llama.cpp's {m_theirs:.4} by more than two standard errors"
            ),
            _ => assert!(ours >= theirs, "{what}: follows the f32 run {ours} steps, llama.cpp's shipped run {theirs}"),
        }
    }
}

#[test]
fn qwen3_5_0_8b_meets_every_activation_precisions_bar() {
    meets_every_activation_precisions_bar("qwen3.5-0.8b");
}

#[test]
fn llama3_2_1b_meets_every_activation_precisions_bar() {
    meets_every_activation_precisions_bar("llama3.2-1b");
}

/// Llama 3.2 1B as bartowski's IQ4_XS quantization: its projections in IQ4_XS, its values in Q5_K, its tied table in
/// Q6_K.
#[test]
fn llama3_2_1b_iq4xs_meets_every_activation_precisions_bar() {
    meets_every_activation_precisions_bar("llama3.2-1b-iq4xs");
}

#[test]
fn gemma3_1b_meets_every_activation_precisions_bar() {
    meets_every_activation_precisions_bar("gemma3-1b");
}

#[test]
fn deepseek_v2_lite_meets_every_activation_precisions_bar() {
    meets_every_activation_precisions_bar("deepseek-v2-lite");
}

/// gpt-oss-20b: 12.1 GB of weights on the RTX 3060's 12 GiB, what the device cannot keep beside its context read from
/// mapped host memory.
#[test]
fn gpt_oss_20b_meets_every_activation_precisions_bar() {
    meets_every_activation_precisions_bar("gpt-oss-20b");
}

#[test]
fn qwen3_5_0_8b_meets_every_activation_precisions_bar_on_long_prompts() {
    meets_every_activation_precisions_bar_on_long_prompts("qwen3.5-0.8b");
}

#[test]
fn llama3_2_1b_meets_every_activation_precisions_bar_on_long_prompts() {
    meets_every_activation_precisions_bar_on_long_prompts("llama3.2-1b");
}

#[test]
fn gemma3_1b_meets_every_activation_precisions_bar_on_long_prompts() {
    meets_every_activation_precisions_bar_on_long_prompts("gemma3-1b");
}

#[test]
fn deepseek_v2_lite_meets_every_activation_precisions_bar_on_long_prompts() {
    meets_every_activation_precisions_bar_on_long_prompts("deepseek-v2-lite");
}

/// The lane's activation quantizer is the reference's, bit for bit: for activations spanning every magnitude a model's
/// rows take (and blocks of zeros, and blocks where one value dominates), the int8 words and the scale `lk_quant` writes
/// rebuild exactly the values `reference::quantize_activations` reads — under the one-word numerics and the two-word.
#[test]
fn the_activation_quantizer_is_the_references_bit_for_bit() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{Kernel, PRELUDE};
    use logicaffeine_compile::model::reference::quantize_activations;
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let n = 32 * 512;
    let x: Vec<f32> = (0..n)
        .map(|i| {
            let block = i / 32;
            let h = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(17) ^ 0x5851_F42D_4C95_7F2D;
            let unit = (h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0;
            match block % 8 {
                0 => 0.0,
                1 if i % 32 == 7 => 3.0e4,
                _ => (unit * 10f64.powi(block as i32 % 13 - 6)) as f32,
            }
        })
        .collect();
    let dir = dir().join("quantizer");
    std::fs::create_dir_all(&dir).unwrap();
    let main = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n{PRELUDE}{}\n\
         int main(int argc, char** argv){{\n  int n = {n}; int q = atoi(argv[1]);\n\
           float* hx = (float*)malloc(n * 4); FILE* f = fopen(argv[2], \"rb\"); fread(hx, 4, n, f); fclose(f);\n\
           float *x, *s; signed char *h, *l;\n\
           cudaMalloc(&x, n * 4); cudaMalloc(&s, n / 32 * 4); cudaMalloc(&h, n); cudaMalloc(&l, n);\n\
           cudaMemcpy(x, hx, n * 4, cudaMemcpyHostToDevice);\n\
           if(q == 1) lk_quant<1><<<(n + 255) / 256, 256>>>(x, h, nullptr, s, n);\n\
           else lk_quant<2><<<(n + 255) / 256, 256>>>(x, h, l, s, n);\n\
           if(cudaDeviceSynchronize() != cudaSuccess) return 1;\n\
           signed char* hh = (signed char*)malloc(n); signed char* hl = (signed char*)malloc(n); float* hs = (float*)malloc(n / 32 * 4);\n\
           cudaMemcpy(hh, h, n, cudaMemcpyDeviceToHost); cudaMemcpy(hs, s, n / 32 * 4, cudaMemcpyDeviceToHost);\n\
           if(q == 2) cudaMemcpy(hl, l, n, cudaMemcpyDeviceToHost);\n\
           FILE* o = fopen(argv[3], \"wb\"); fwrite(hh, 1, n, o); fwrite(q == 2 ? hl : hh, 1, n, o); fwrite(hs, 4, n / 32, o); fclose(o);\n\
           return 0;\n}}\n",
        Kernel::Quant.source()
    );
    let (cu, bin, input) = (dir.join("quant.cu"), dir.join("quant"), dir.join("x.bin"));
    std::fs::write(&cu, main).unwrap();
    std::fs::write(&input, x.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "the quantizer builds:\n{}", String::from_utf8_lossy(&built.stderr));
    let x64: Vec<f64> = x.iter().map(|&v| f64::from(v)).collect();
    for (q, mode) in [(1, Activations::Int8), (2, Activations::Int8TwoPass)] {
        let out = dir.join(format!("words{q}.bin"));
        let ran = Command::new(&bin).arg(q.to_string()).arg(&input).arg(&out).output().unwrap();
        assert!(ran.status.success(), "the quantizer runs: {}", String::from_utf8_lossy(&ran.stderr));
        let bytes = std::fs::read(&out).unwrap();
        let (hi, rest) = bytes.split_at(n);
        let (lo, scales) = rest.split_at(n);
        let s: Vec<f32> = scales.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
        let want = quantize_activations(&x64, mode);
        for i in 0..n {
            let sc = s[i / 32];
            let got = match q {
                1 => f64::from(hi[i] as i8) * f64::from(sc),
                _ => f64::from(hi[i] as i8) * f64::from(sc) + f64::from(lo[i] as i8) * f64::from(sc * 0.0078125),
            };
            assert_eq!(got.to_bits(), want[i].to_bits(), "{mode:?}: element {i} (block {}) is {got}, the reference reads {}", i / 32, want[i]);
        }
    }
}

/// The precise numerics' norm sums in `f64`: an RMSNorm of rows 64 to 8192 wide, their values spread over six orders of
/// magnitude, comes out of `lk_rmsnorm` within half an `f32` step of the `f64` norm — the output's own rounding, and
/// nothing an `f32` sum of the squares would add (at 2048 wide, a few steps).
#[test]
fn the_precise_norms_sum_in_f64() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{norm_threads, Kernel, PRELUDE};
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let dir = dir().join("norms");
    std::fs::create_dir_all(&dir).unwrap();
    for cols in [64usize, 2048, 8192] {
        let threads = norm_threads(cols);
        let per = cols.div_ceil(threads);
        let value = |i: usize, salt: u64| {
            let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(23) ^ 0x2545_F491_4F6C_DD1D;
            let unit = (h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0;
            (unit * 10f64.powi((i % 7) as i32 - 3)) as f32
        };
        let x: Vec<f32> = (0..cols).map(|i| value(i, 1)).collect();
        let w: Vec<f32> = (0..cols).map(|i| 1.0 + value(i, 2)).collect();
        let main = format!(
            "#include <cstdio>\n#include <cstdlib>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n{PRELUDE}{}\n\
             int main(int argc, char** argv){{\n  int n = {cols};\n\
               float* hx = (float*)malloc(n * 8); FILE* f = fopen(argv[1], \"rb\"); fread(hx, 4, 2 * n, f); fclose(f);\n\
               float *x, *w, *y;\n  cudaMalloc(&x, n * 4); cudaMalloc(&w, n * 4); cudaMalloc(&y, n * 4);\n\
               cudaMemcpy(x, hx, n * 4, cudaMemcpyHostToDevice); cudaMemcpy(w, hx + n, n * 4, cudaMemcpyHostToDevice);\n\
               lk_rmsnorm<false, false, true, 0, {per}, double><<<1, {threads}>>>(x, nullptr, nullptr, w, y, nullptr, nullptr, nullptr, n, 1e-6f, n, 0, nullptr, 0, n, 0);\n\
               if(cudaDeviceSynchronize() != cudaSuccess) return 1;\n\
               cudaMemcpy(hx, y, n * 4, cudaMemcpyDeviceToHost);\n\
               FILE* o = fopen(argv[2], \"wb\"); fwrite(hx, 4, n, o); fclose(o);\n  return 0;\n}}\n",
            Kernel::RmsNorm.source()
        );
        let (cu, bin, input, out) = (dir.join(format!("norm{cols}.cu")), dir.join(format!("norm{cols}")), dir.join(format!("x{cols}.bin")), dir.join(format!("y{cols}.bin")));
        std::fs::write(&cu, main).unwrap();
        std::fs::write(&input, x.iter().chain(&w).flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
        let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
        assert!(built.status.success(), "the norm builds:\n{}", String::from_utf8_lossy(&built.stderr));
        let ran = Command::new(&bin).arg(&input).arg(&out).output().unwrap();
        assert!(ran.status.success(), "the norm runs: {}", String::from_utf8_lossy(&ran.stderr));
        let got: Vec<f32> = std::fs::read(&out).unwrap().chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
        let ms = x.iter().map(|&v| f64::from(v) * f64::from(v)).sum::<f64>() / cols as f64;
        let scale = 1.0 / (ms + 1e-6f32 as f64).sqrt();
        for i in 0..cols {
            let want = f64::from(x[i]) * scale * f64::from(w[i]);
            let step = f64::from(f32::EPSILON) * want.abs().max(f64::from(f32::MIN_POSITIVE));
            assert!(
                (f64::from(got[i]) - want).abs() <= 0.5 * step * (1.0 + 1e-6),
                "{cols} wide, element {i}: the norm gives {}, the f64 norm {want} ({} steps off)",
                got[i],
                (f64::from(got[i]) - want).abs() / step
            );
        }
    }
}

/// The staged per-head matmul's own shared memory is `kernels::AXPY_SUMS`, what the emitter leaves beside a tile of its
/// rows in a block's default 48 KB: `lk_head_axpy_staged`'s static shared memory, as the driver reports it, in every
/// block format it stages, over one activation row and sixteen.
#[test]
fn the_staged_per_head_matmul_takes_the_shared_memory_its_tile_leaves() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{axpy_staged, head_mode, Kernel, AXPY_R, AXPY_SUMS, AXPY_W, PRELUDE};
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let q = head_mode(Q8MatKind::Int8Act);
    let staged: Vec<u32> = BLOCK_FORMATS.iter().filter(|&&ty| axpy_staged(ty, q)).map(|ty| ty.id()).collect();
    assert!(!staged.is_empty());
    let queries: String = staged
        .iter()
        .flat_map(|f| [1, 16].map(move |m| (*f, m)))
        .map(|(f, m)| {
            format!(
                "  CK(cudaFuncGetAttributes(&a, lk_head_axpy_staged<{f}, {q}, {m}, {AXPY_W}, {AXPY_R}>)); printf(\"{f} {m} %zu\\n\", a.sharedSizeBytes);\n"
            )
        })
        .collect();
    let cu = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ printf(\"cuda %s\\n\", cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         {PRELUDE}{}{}\nint main(){{\n  cudaFuncAttributes a;\n{queries}}}\n",
        Kernel::Blocks.source(),
        Kernel::Heads.source()
    );
    let src = dir().join("axpy_sums.cu");
    let bin = dir().join("axpy_sums");
    std::fs::write(&src, cu).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&src).output().unwrap();
    assert!(built.status.success(), "the probe builds:\n{}", String::from_utf8_lossy(&built.stderr));
    let out = Command::new(&bin).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "the probe runs:\n{stdout}");
    let sizes: Vec<(String, usize)> = stdout
        .lines()
        .map(|l| {
            let mut it = l.split_whitespace();
            let what = format!("format {} over {} rows", it.next().unwrap(), it.next().unwrap());
            (what, it.next().unwrap().parse().unwrap())
        })
        .collect();
    assert_eq!(sizes.len(), 2 * staged.len());
    for (what, bytes) in sizes {
        assert_eq!(bytes, AXPY_SUMS, "{what}: the kernel's static shared memory");
    }
}

/// Every block decoder agrees with `lk_sub`, the sub-block decoder: `lk_level`, the per-element decoder the
/// row-accumulating kernels read, decodes each element to the same level, scale and offset, and `lk_sub4`, the word-wise
/// decoder the row dots read, packs each sub-block's levels four to an int exactly as `lk_sub` lays them out — bit for
/// bit, for every block format, over seeded bytes (scales of every magnitude, NaNs and infinities among them).
#[test]
fn every_block_decoder_agrees_with_the_sub_block_decoder() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{Kernel, PRELUDE};
    use logicaffeine_compile::model::quant::GgmlType;
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    const K: usize = 1024;
    let formats = BLOCK_FORMATS;
    let checks: String = formats.iter().map(|ty| format!("  bad += run<{}>({}L);\n", ty.id(), ty.bytes_for(K))).collect();
    let main = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n{PRELUDE}{}\n\
         template<int F> __global__ void lk_check(const unsigned char* row, int nsb, int* bad){{\n\
           int sb = blockIdx.x, l = threadIdx.x; if(sb >= nsb) return;\n\
           signed char v[32]; float d[2], c[2]; lk_sub<F>(row, sb, v, d, c);\n\
           float dl, cl; int lv = lk_level<F>(row, sb, l, dl, cl);\n\
           if(lv != v[l] || __float_as_uint(dl) != __float_as_uint(d[l >> 4]) || __float_as_uint(cl) != __float_as_uint(c[l >> 4])) atomicAdd(bad, 1);\n\
           int w[8]; float d4[2], c4[2]; lk_sub4<F>(row, sb, w, d4, c4);\n\
           if(l < 8 && w[l] != lk_pack4(v + 4 * l)) atomicAdd(bad, 1);\n\
           if(l < 2 && (__float_as_uint(d4[l]) != __float_as_uint(d[l]) || __float_as_uint(c4[l]) != __float_as_uint(c[l]))) atomicAdd(bad, 1);\n\
         }}\n\
         template<int F> int run(long bytes){{\n\
           unsigned char* h = (unsigned char*)malloc(bytes);\n\
           for(long i = 0; i < bytes; ++i){{ unsigned long long x = (unsigned long long)i * 0x9E3779B97F4A7C15ULL ^ (unsigned long long)F * 0x5851F42D4C957F2DULL; h[i] = (unsigned char)(x >> 29); }}\n\
           unsigned char* r; int* bad; cudaMalloc(&r, bytes); cudaMalloc(&bad, 4); cudaMemset(bad, 0, 4);\n\
           cudaMemcpy(r, h, bytes, cudaMemcpyHostToDevice);\n\
           lk_check<F><<<{}, 32>>>(r, {}, bad);\n\
           int hb = -1; if(cudaMemcpy(&hb, bad, 4, cudaMemcpyDeviceToHost) != cudaSuccess) return 1 << 20;\n\
           printf(\"format %d: %d elements decoded otherwise\\n\", F, hb); return hb;\n\
         }}\n\
         int main(){{\n  int bad = 0;\n{checks}  return bad != 0;\n}}\n",
        Kernel::Blocks.source(),
        K / 32,
        K / 32,
    );
    let dir = dir().join("levels");
    std::fs::create_dir_all(&dir).unwrap();
    let (cu, bin) = (dir.join("levels.cu"), dir.join("levels"));
    std::fs::write(&cu, main).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "the decoders build:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).output().unwrap();
    let report = String::from_utf8_lossy(&ran.stdout);
    assert!(ran.status.success(), "every element decodes alike:\n{report}{}", String::from_utf8_lossy(&ran.stderr));
    assert_eq!(report.lines().count(), formats.len(), "every format checked:\n{report}");
}

/// The tiled per-head products compute what the GEMV's slabs compute, on the same inputs: DeepSeek's latent shapes —
/// each head's 128 query values (the first 128 of each 192-wide unit) through a 128 × 512 matrix whose rows are its
/// inputs into the first 512 of each 576-wide unit, and each head's 512 latent values through a 128 × 512 matrix whose
/// rows are its outputs — over 4 heads and 40 rows (the slabs 16, 16 and 8), in Q8_0, Q4_0, Q2_K, Q4_K, Q6_K, MXFP4,
/// IQ4_NL and IQ4_XS, the inputs quantized under both quantized numerics and as computed with `f32` sums (`Q` 3, the lane's own
/// numerics for a per-head product). Each output within `TOLERANCE` of its head's largest, every output written (they
/// start as NaN).
#[test]
fn the_tiled_per_head_products_compute_what_the_slabs_compute() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{Kernel, PRELUDE};
    use logicaffeine_compile::model::quant::GgmlType;
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    const TOLERANCE: f64 = 1e-5;
    const H: usize = 4;
    const ROWS: usize = 40;
    let formats = [GgmlType::Q8_0, GgmlType::Q4_0, GgmlType::Q2K, GgmlType::Q4K, GgmlType::Q6K, GgmlType::Mxfp4, GgmlType::Iq4Nl, GgmlType::Iq4Xs];
    // (orientation, a, b, xp, xo, yp, yo): rows the inputs — 128 of each 192-wide unit into 512 of each 576-wide one —
    // and rows the outputs — 512 latent values into 128.
    let shapes = [(false, 128usize, 512usize, 192usize, 0usize, 576usize, 0usize), (true, 512, 128, 512, 0, 128, 0)];
    let dir = dir().join("heads");
    std::fs::create_dir_all(&dir).unwrap();
    let unit = |i: usize, salt: u64| {
        let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(27) ^ 0x2545_F491_4F6C_DD1D;
        ((h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32
    };
    let mut calls = String::new();
    let mut cases = Vec::new();
    for (si, &(out, a, b, xp, xo, yp, yo)) in shapes.iter().enumerate() {
        let x: Vec<f32> = (0..ROWS * H * xp).map(|i| unit(i, 50 + si as u64) * 2.0).collect();
        std::fs::write(dir.join(format!("x{si}.bin")), x.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
        let (rows, cols) = if out { (H * b, a) } else { (H * a, b) };
        for &ty in &formats {
            let name = format!("w{si}{}", ty.name());
            let bytes = if ty == GgmlType::Q8_0 {
                let levels = (0..rows * cols).map(|i| ((unit(i, 60 + si as u64) * 127.0) as i8) as u8);
                levels.chain((0..rows * cols / 32).flat_map(|_| 0x2000u16.to_le_bytes())).collect()
            } else {
                tiny_models::seeded_blocks(&name, ty, rows * cols)
            };
            std::fs::write(dir.join(format!("{name}.bin")), bytes).unwrap();
            for q in [1, 2, 3] {
                let f = ty.id();
                let rb = ty.bytes_for(cols);
                let slabs: String = [(0usize, 16usize), (16, 16), (32, 8)]
                    .iter()
                    .map(|&(r0, r)| {
                        let (at, yat) = (r0 * H * xp, r0 * H * yp);
                        let xin = match q {
                            3 => format!("x + {at}, nullptr, nullptr, nullptr"),
                            _ => format!("nullptr, xq + {at}, {}, xs + {}", if q == 2 { format!("xl + {at}") } else { "nullptr".into() }, at / 32),
                        };
                        let shape = format!("{H}, {a}, {b}, {xp}, {xo}, nullptr, 0, {yp}, {yo}");
                        if out {
                            format!("    lk_head_gemv<{f}, {q}, {r}><<<{}, 256>>>(w, {xin}, ya + {yat}, {shape}); CK(cudaGetLastError());\n", (H * b).div_ceil(8))
                        } else {
                            format!("    lk_head_axpy<{f}, {q}, {r}><<<dim3({}, {H}), 256>>>(w, {xin}, ya + {yat}, {shape}); CK(cudaGetLastError());\n", b / 32)
                        }
                    })
                    .collect();
                // Under Q 3 the inputs are read as computed; otherwise quantized first.
                let quantized = if q == 3 {
                    "    signed char *xq = nullptr, *xl = nullptr; float* xs = nullptr;\n".to_string()
                } else {
                    format!(
                        "    long n = {ROWS}L * {H} * {xp}; signed char* xq = bytes(n); signed char* xl = {q} == 2 ? bytes(n) : nullptr; float* xs = nan(n / 32);\n\
                         \x20   lk_quant<{q}><<<(n + 255) / 256, 256>>>(x, xq, xl, xs, n); CK(cudaGetLastError());\n"
                    )
                };
                let _ = write!(
                    calls,
                    "  {{ // {} {name} Q{q}\n    lk_w w = load<{f}>(dir + \"/{name}.bin\", {rows}L, {cols}, {rb}L);\n    float* x = dev(dir + \"/x{si}.bin\");\n{quantized}\
                     \x20   float *ya = nan({ROWS}L * {H} * {yp}), *yb = nan({ROWS}L * {H} * {yp});\n{slabs}\
                     \x20   lk_head_tiled<{f}, {q}, {out}><<<dim3({}, {}, {H}), 256>>>(w, xq, xl, xs, yb, {H}, {a}, {b}, {xp}, {xo}, nullptr, 0, {yp}, {yo}, {ROWS}, x); CK(cudaGetLastError());\n\
                     \x20   CK(cudaDeviceSynchronize()); put(out, ya, {ROWS}L * {H} * {yp}); put(out, yb, {ROWS}L * {H} * {yp});\n  }}\n",
                    if out { "rows the outputs" } else { "rows the inputs" },
                    b.div_ceil(64),
                    ROWS.div_ceil(64)
                );
                cases.push((format!("{} {} Q{q}", if out { "rows out" } else { "rows in" }, ty.name()), b, yp, yo));
            }
        }
    }
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <string>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {PRELUDE}{}{}{}{}\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ fprintf(stderr, \"cuda %d %s\\n\", __LINE__, cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         static std::vector<char> slurp(const std::string& p){{ FILE* f = fopen(p.c_str(), \"rb\"); if(!f) exit(2); std::vector<char> b; char c[65536]; size_t n;\n\
           while((n = fread(c, 1, sizeof c, f)) > 0) b.insert(b.end(), c, c + n); fclose(f); return b; }}\n\
         static float* dev(const std::string& p){{ auto b = slurp(p); float* d; CK(cudaMalloc(&d, b.size())); CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice)); return d; }}\n\
         template<int F> static lk_w load(const std::string& p, long rows, int cols, long row_bytes){{\n\
           auto b = slurp(p);\n\
           if(F == 8){{ signed char* q; __half* s; long n = rows * cols; CK(cudaMalloc(&q, n)); CK(cudaMalloc(&s, n / 32 * 2));\n\
             CK(cudaMemcpy(q, b.data(), n, cudaMemcpyHostToDevice)); CK(cudaMemcpy(s, b.data() + n, n / 32 * 2, cudaMemcpyHostToDevice)); return lk_w{{q, s, nullptr, 0}}; }}\n\
           unsigned char* d; CK(cudaMalloc(&d, b.size() + 32)); CK(cudaMemset(d, 0, b.size() + 32)); CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice));\n\
           return lk_w{{nullptr, nullptr, d, row_bytes}};\n\
         }}\n\
         static float* nan(long n){{ float* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0xff, n * 4)); return p; }}\n\
         static signed char* bytes(long n){{ signed char* p; CK(cudaMalloc(&p, n)); return p; }}\n\
         static void put(FILE* out, const float* d, long n){{ std::vector<float> h(n); CK(cudaMemcpy(h.data(), d, n * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, n, out); }}\n\
         int main(int argc, char** argv){{\n  std::string dir = argv[1];\n  FILE* out = fopen(argv[2], \"wb\");\n{calls}  fclose(out);\n  return 0;\n}}\n",
        Kernel::Blocks.source(),
        Kernel::Quant.source(),
        Kernel::Heads.source(),
        Kernel::HeadTiled.source()
    );
    let (cu, bin, outp) = (dir.join("heads.cu"), dir.join("heads"), dir.join("out.bin"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "the slab and tiled products build:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).arg(&dir).arg(&outp).output().unwrap();
    assert!(ran.status.success(), "they run: {}", String::from_utf8_lossy(&ran.stderr));
    let all: Vec<f32> = std::fs::read(&outp).unwrap().chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
    assert_eq!(all.len(), cases.iter().map(|&(_, _, yp, _)| 2 * ROWS * H * yp).sum::<usize>(), "every case's outputs");
    let mut at = 0;
    let mut report = Vec::new();
    for (what, b, yp, yo) in &cases {
        let n = ROWS * H * yp;
        let (slab, tiled) = (&all[at..at + n], &all[at + n..at + 2 * n]);
        at += 2 * n;
        let mut worst = 0f64;
        for unit in 0..ROWS * H {
            let (u, v) = (&slab[unit * yp + yo..unit * yp + yo + b], &tiled[unit * yp + yo..unit * yp + yo + b]);
            assert!(u.iter().chain(v).all(|f| f.is_finite()), "{what}: row {} head {}: every output written, and finite", unit / H, unit % H);
            let scale = u.iter().fold(0f64, |m, f| m.max(f64::from(f.abs())));
            assert!(scale > 0.0, "{what}: row {} head {} is not all zero", unit / H, unit % H);
            for (o, (p, q)) in u.iter().zip(v).enumerate() {
                let off = f64::from((p - q).abs()) / scale;
                worst = worst.max(off);
                assert!(off <= TOLERANCE, "{what}: row {} head {} output {o}: the slabs give {p}, the tiles {q} ({off:.2e} of the head's largest)", unit / H, unit % H);
            }
        }
        report.push(format!("{what}: within {worst:.2e}"));
    }
    eprintln!("{}", report.join("\n"));
}

/// The tiled attention computes what the attention a block a row computes, on the same queries, caches and positions:
/// grouped heads, grouped heads within a window and with sinks, DeepSeek's latent attention (576-wide keys, its values
/// the first 512 of each cache row, read in place), and a head a group — 40 rows after 70 cached positions, so every
/// row's span crosses tiles and the block's last rows run past the chunk. Each output within `TOLERANCE` of its
/// head's largest, the difference of two summation orders, and every output written (they start as NaN).
#[test]
fn the_tiled_attention_computes_what_the_attention_computes() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{
        attn_smem_bytes, attn_tile_heads, attn_tile_smem_bytes, attn_tile_values, attn_values_per_thread, Kernel, ATTN_MIN_SPLIT, ATTN_TILE_Q,
        PRELUDE,
    };
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    const TOLERANCE: f64 = 1e-5;
    const ROWS: usize = 40;
    const POS: usize = 70;
    // (name, heads, kv heads, hd, vd, window, sinks, values in the key cache)
    let shapes: [(&str, usize, usize, usize, usize, usize, bool, bool); 4] = [
        ("grouped", 8, 2, 64, 64, 0, false, false),
        ("windowed with sinks", 8, 2, 64, 64, 32, true, false),
        ("latent", 16, 1, 576, 512, 0, false, true),
        ("a head a group", 4, 4, 128, 128, 0, false, false),
    ];
    let dir = dir().join("tiles");
    std::fs::create_dir_all(&dir).unwrap();
    let unit = |i: usize, salt: u64| {
        let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(31) ^ 0x2545_F491_4F6C_DD1D;
        ((h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32
    };
    let mut calls = String::new();
    let mut sizes = Vec::new();
    for (k, &(name, heads, kvh, hd, vd, win, sinks, latent)) in shapes.iter().enumerate() {
        let (group, len) = (heads / kvh, POS + ROWS);
        let kvw = kvh * hd;
        let (vw, voff) = if latent { (kvw, 0) } else { (kvh * vd, 0) };
        let q: Vec<f32> = (0..ROWS * heads * hd).map(|i| unit(i, 10 + k as u64)).collect();
        let kc: Vec<f32> = (0..len * kvw).map(|i| unit(i, 20 + k as u64)).collect();
        let vc: Vec<f32> = (0..len * vw).map(|i| unit(i, 30 + k as u64)).collect();
        let sk: Vec<f32> = (0..heads).map(|i| unit(i, 40 + k as u64) * 2.0).collect();
        for (part, v) in [("q", &q), ("k", &kc), ("v", &vc), ("s", &sk)] {
            std::fs::write(dir.join(format!("{part}{k}.bin")), v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
        }
        let scale = 1.0 / (hd as f64).sqrt();
        let (hg, vi) = (attn_tile_heads(group), attn_tile_values(vd));
        let _ = write!(
            calls,
            "  {{ // {name}\n    float* q = dev(dir + \"/q{k}.bin\"); float* kc = dev(dir + \"/k{k}.bin\"); float* vc = {}; float* sk = {};\n\
             \x20   float *a = nan({ROWS} * {heads} * {vd}), *b = nan({ROWS} * {heads} * {vd});\n\
             \x20   lk_attention<1, {}, float><<<dim3({heads}, {ROWS}, 1), 256, {}>>>(q, kc, vc, pos, sk, a, nullptr, nullptr, {len}, {win}, {hd}, {vd}, {group}, {heads}, {kvw}, {vw}, {voff}, {scale:e}f, {ATTN_MIN_SPLIT}, 1);\n\
             \x20   CK(cudaGetLastError());\n\
             \x20   CK(cudaFuncSetAttribute(lk_attention_tiled<{hg}, {vi}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {}));\n\
             \x20   lk_attention_tiled<{hg}, {vi}><<<dim3({}, {}), 256, {}>>>(q, kc, vc, pos, sk, b, {len}, {win}, {hd}, {vd}, {group}, {heads}, {kvw}, {vw}, {voff}, {scale:e}f, {ROWS});\n\
             \x20   CK(cudaGetLastError()); CK(cudaDeviceSynchronize());\n\
             \x20   put(out, a, {ROWS} * {heads} * {vd}); put(out, b, {ROWS} * {heads} * {vd});\n  }}\n",
            if latent { "kc".to_string() } else { format!("dev(dir + \"/v{k}.bin\")") },
            if sinks { format!("dev(dir + \"/s{k}.bin\")") } else { "nullptr".into() },
            attn_values_per_thread(vd),
            attn_smem_bytes(1, hd, 1, 4),
            attn_tile_smem_bytes(vd),
            ROWS.div_ceil(ATTN_TILE_Q / hg),
            heads / hg,
            attn_tile_smem_bytes(vd)
        );
        sizes.push((name, heads, vd));
    }
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <string>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {PRELUDE}{}{}\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ fprintf(stderr, \"cuda %d %s\\n\", __LINE__, cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         static float* dev(const std::string& p){{ FILE* f = fopen(p.c_str(), \"rb\"); if(!f) exit(2); std::vector<char> b; char c[65536]; size_t n;\n\
           while((n = fread(c, 1, sizeof c, f)) > 0) b.insert(b.end(), c, c + n); fclose(f);\n\
           float* d; CK(cudaMalloc(&d, b.size())); CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice)); return d; }}\n\
         static float* nan(long n){{ float* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0xff, n * 4)); return p; }}\n\
         static void put(FILE* out, const float* d, long n){{ std::vector<float> h(n); CK(cudaMemcpy(h.data(), d, n * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, n, out); }}\n\
         int main(int argc, char** argv){{\n  std::string dir = argv[1];\n  int* pos; CK(cudaMalloc(&pos, 4)); int p0 = {POS}; CK(cudaMemcpy(pos, &p0, 4, cudaMemcpyHostToDevice));\n\
           FILE* out = fopen(argv[2], \"wb\");\n{calls}  fclose(out);\n  return 0;\n}}\n",
        Kernel::Attention.source(),
        Kernel::AttentionTiled.source()
    );
    let (cu, bin, out) = (dir.join("tiles.cu"), dir.join("tiles"), dir.join("out.bin"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "both attentions build:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).arg(&dir).arg(&out).output().unwrap();
    assert!(ran.status.success(), "they run: {}", String::from_utf8_lossy(&ran.stderr));
    let all: Vec<f32> = std::fs::read(&out).unwrap().chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
    assert_eq!(all.len(), sizes.iter().map(|&(_, h, vd)| 2 * ROWS * h * vd).sum::<usize>(), "every shape's outputs");
    let mut at = 0;
    let mut report = Vec::new();
    for &(name, heads, vd) in &sizes {
        let n = ROWS * heads * vd;
        let (a, b) = (&all[at..at + n], &all[at + n..at + 2 * n]);
        at += 2 * n;
        let mut worst = 0f64;
        for v in 0..ROWS * heads {
            let (x, y) = (&a[v * vd..(v + 1) * vd], &b[v * vd..(v + 1) * vd]);
            assert!(x.iter().chain(y).all(|f| f.is_finite()), "{name}: row {} head {}: every output written, and finite", v / heads, v % heads);
            let scale = x.iter().fold(0f64, |m, f| m.max(f64::from(f.abs())));
            assert!(scale > 0.0, "{name}: row {} head {} attends something", v / heads, v % heads);
            for (e, (u, w)) in x.iter().zip(y).enumerate() {
                let off = f64::from((u - w).abs()) / scale;
                worst = worst.max(off);
                assert!(off <= TOLERANCE, "{name}: row {} head {} value {e}: a block a row gives {u}, the tiles {w} ({off:.2e} of the head's largest)", v / heads, v % heads);
            }
        }
        report.push(format!("{name}: within {worst:.2e}"));
    }
    eprintln!("{}", report.join("\n"));
}

/// The flash attention (`lk_attention_flash`) computes what the attention a block a row computes (`lk_attention`), on
/// the same queries, caches and positions: grouped heads; grouped heads within a window and with sinks; a head a group;
/// Qwen3's 32 heads over 8 groups; Gemma's 256-wide heads within a window (32 queries a block); values narrower than the
/// keys; a 72-wide latent whose values are the first 64 of each key row, read in place — 40 rows after 70 cached
/// positions, and 96 rows after 400 so every row's span crosses tiles; and split, each query's and key's last part read
/// apart as an expanded latent attention reads them (its own 128 in place in the query projection and its rotated 64
/// apart, each head's expanded key and the rotated key every head shares in place in the cache), against the tiles'
/// split. Each output within `TOLERANCE` of its head's largest, the difference of two summation orders, and every output
/// written (they start as NaN).
#[test]
fn the_flash_attention_computes_what_the_attention_computes() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{
        attn_flash_queries, attn_flash_smem_bytes, attn_smem_bytes, attn_tile_heads, attn_tile_smem_bytes, attn_tile_values, attn_values_per_thread,
        Kernel, ATTN_MIN_SPLIT, ATTN_TILE_Q, PRELUDE,
    };
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    const TOLERANCE: f64 = 1e-5;
    // (name, heads, kv heads, hd, vd, window, sinks, values in the key cache, rows, cached positions)
    let shapes: [(&str, usize, usize, usize, usize, usize, bool, bool, usize, usize); 9] = [
        ("grouped", 8, 2, 64, 64, 0, false, false, 40, 70),
        ("windowed with sinks", 8, 2, 64, 64, 32, true, false, 40, 70),
        ("a head a group", 4, 4, 128, 128, 0, false, false, 40, 70),
        ("qwen3", 32, 8, 128, 128, 0, false, false, 40, 70),
        ("gemma", 8, 1, 256, 256, 48, false, false, 40, 70),
        ("narrow values", 8, 2, 128, 64, 0, false, false, 40, 70),
        ("tiny latent", 4, 1, 72, 64, 0, false, true, 40, 70),
        ("grouped, long", 8, 2, 64, 64, 0, false, false, 96, 400),
        ("qwen3, long", 32, 8, 128, 128, 0, true, false, 96, 400),
    ];
    let dir = dir().join("flash");
    std::fs::create_dir_all(&dir).unwrap();
    let unit = |i: usize, salt: u64| {
        let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(31) ^ 0x2545_F491_4F6C_DD1D;
        ((h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32
    };
    let write = |name: String, v: &[f32]| std::fs::write(dir.join(name), v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    let mut calls = String::new();
    let mut sizes = Vec::new();
    for (k, &(name, heads, kvh, hd, vd, win, sinks, latent, rows, cached)) in shapes.iter().enumerate() {
        let (group, len) = (heads / kvh, cached + rows);
        let kvw = kvh * hd;
        let (vw, voff) = if latent { (kvw, 0) } else { (kvh * vd, 0) };
        write(format!("q{k}.bin"), &(0..rows * heads * hd).map(|i| unit(i, 10 + k as u64)).collect::<Vec<f32>>());
        write(format!("k{k}.bin"), &(0..len * kvw).map(|i| unit(i, 20 + k as u64)).collect::<Vec<f32>>());
        write(format!("v{k}.bin"), &(0..len * vw).map(|i| unit(i, 30 + k as u64)).collect::<Vec<f32>>());
        write(format!("s{k}.bin"), &(0..heads).map(|i| unit(i, 40 + k as u64) * 2.0).collect::<Vec<f32>>());
        let scale = 1.0 / (hd as f64).sqrt();
        let (hg, bq) = (attn_tile_heads(group), attn_flash_queries(hd, vd));
        let smem = attn_flash_smem_bytes(hd, vd);
        let _ = write!(
            calls,
            "  {{ // {name}\n    int p0 = {cached}; CK(cudaMemcpy(pos, &p0, 4, cudaMemcpyHostToDevice));\n\
             \x20   float* q = dev(dir + \"/q{k}.bin\"); float* kc = dev(dir + \"/k{k}.bin\"); float* vc = {}; float* sk = {};\n\
             \x20   float *a = nan({rows} * {heads} * {vd}), *b = nan({rows} * {heads} * {vd});\n\
             \x20   lk_attention<1, {}, float><<<dim3({heads}, {rows}, 1), 256, {}>>>(q, kc, vc, pos, sk, a, nullptr, nullptr, {len}, {win}, {hd}, {vd}, {group}, {heads}, {kvw}, {vw}, {voff}, {scale:e}f, {ATTN_MIN_SPLIT}, 1);\n\
             \x20   CK(cudaGetLastError());\n\
             \x20   CK(cudaFuncSetAttribute(lk_attention_flash<{hg}, {hd}, {vd}, {bq}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {smem}));\n\
             \x20   lk_attention_flash<{hg}, {hd}, {vd}, {bq}><<<dim3({}, {}), 128, {smem}>>>(q, kc, vc, pos, sk, b, {len}, {win}, {group}, {heads}, {kvw}, {vw}, {voff}, {scale:e}f, {rows});\n\
             \x20   CK(cudaGetLastError()); CK(cudaDeviceSynchronize());\n\
             \x20   put(out, a, {rows} * {heads} * {vd}); put(out, b, {rows} * {heads} * {vd});\n  }}\n",
            if latent { "kc".to_string() } else { format!("dev(dir + \"/v{k}.bin\")") },
            if sinks { format!("dev(dir + \"/s{k}.bin\")") } else { "nullptr".into() },
            attn_values_per_thread(vd),
            attn_smem_bytes(1, hd, 1, 4),
            rows.div_ceil(bq / hg),
            heads / hg
        );
        sizes.push((name.to_string(), rows, heads, vd));
    }
    // Split: 8 heads a group each, each query its own 128 in the projection (192 a head) and its rotated 64 apart; each
    // key its head's own 128 (expanded) and the rotated 64 every head shares at 512 in a 576-wide cache row.
    let (h, own, rope, vd, rows, cached, kw, lora) = (8usize, 128usize, 64usize, 128usize, 40usize, 70usize, 576usize, 512usize);
    let (qk, len) = (own + rope, cached + rows);
    write("sq.bin".into(), &(0..rows * h * qk).map(|i| unit(i, 60)).collect::<Vec<f32>>());
    write("sqe.bin".into(), &(0..rows * h * rope).map(|i| unit(i, 61)).collect::<Vec<f32>>());
    write("sk.bin".into(), &(0..len * h * own).map(|i| unit(i, 62)).collect::<Vec<f32>>());
    write("skc.bin".into(), &(0..len * kw).map(|i| unit(i, 63)).collect::<Vec<f32>>());
    write("sv.bin".into(), &(0..len * h * vd).map(|i| unit(i, 64)).collect::<Vec<f32>>());
    let scale = 1.0 / (qk as f64).sqrt();
    let (bq, smem) = (attn_flash_queries(qk, vd), attn_flash_smem_bytes(qk, vd));
    let _ = write!(
        calls,
        "  {{ // split\n    int p0 = {cached}; CK(cudaMemcpy(pos, &p0, 4, cudaMemcpyHostToDevice));\n\
         \x20   float* q = dev(dir + \"/sq.bin\"); float* qe = dev(dir + \"/sqe.bin\"); float* k = dev(dir + \"/sk.bin\"); float* kc = dev(dir + \"/skc.bin\"); float* v = dev(dir + \"/sv.bin\");\n\
         \x20   float *a = nan({rows} * {h} * {vd}), *b = nan({rows} * {h} * {vd});\n\
         \x20   CK(cudaFuncSetAttribute(lk_attention_tiled<1, {}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {}));\n\
         \x20   lk_attention_tiled<1, {}><<<dim3({}, {h}), 256, {}>>>(q, k, v, pos, nullptr, a, {len}, 0, {qk}, {vd}, 1, {h}, {}, {}, 0, {scale:e}f, {rows}, qe, {qk}, {rope}, kc, {kw}, {lora}, {own});\n\
         \x20   CK(cudaGetLastError());\n\
         \x20   CK(cudaFuncSetAttribute(lk_attention_flash<1, {qk}, {vd}, {bq}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {smem}));\n\
         \x20   lk_attention_flash<1, {qk}, {vd}, {bq}><<<dim3({}, {h}), 128, {smem}>>>(q, k, v, pos, nullptr, b, {len}, 0, 1, {h}, {}, {}, 0, {scale:e}f, {rows}, qe, {qk}, {rope}, kc, {kw}, {lora}, {own});\n\
         \x20   CK(cudaGetLastError()); CK(cudaDeviceSynchronize());\n\
         \x20   put(out, a, {rows} * {h} * {vd}); put(out, b, {rows} * {h} * {vd});\n  }}\n",
        attn_tile_values(vd),
        attn_tile_smem_bytes(vd),
        attn_tile_values(vd),
        rows.div_ceil(ATTN_TILE_Q),
        attn_tile_smem_bytes(vd),
        h * own,
        h * vd,
        rows.div_ceil(bq),
        h * own,
        h * vd
    );
    sizes.push(("split".to_string(), rows, h, vd));
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <string>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {PRELUDE}{}{}{}{}\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ fprintf(stderr, \"cuda %d %s\\n\", __LINE__, cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         static float* dev(const std::string& p){{ FILE* f = fopen(p.c_str(), \"rb\"); if(!f) exit(2); std::vector<char> b; char c[65536]; size_t n;\n\
           while((n = fread(c, 1, sizeof c, f)) > 0) b.insert(b.end(), c, c + n); fclose(f);\n\
           float* d; CK(cudaMalloc(&d, b.size())); CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice)); return d; }}\n\
         static float* nan(long n){{ float* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0xff, n * 4)); return p; }}\n\
         static void put(FILE* out, const float* d, long n){{ std::vector<float> h(n); CK(cudaMemcpy(h.data(), d, n * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, n, out); }}\n\
         int main(int argc, char** argv){{\n  std::string dir = argv[1];\n  int* pos; CK(cudaMalloc(&pos, 4));\n\
           FILE* out = fopen(argv[2], \"wb\");\n{calls}  fclose(out);\n  return 0;\n}}\n",
        Kernel::Mma.source(),
        Kernel::Attention.source(),
        Kernel::AttentionTiled.source(),
        Kernel::AttentionFlash.source()
    );
    let (cu, bin, out) = (dir.join("flash.cu"), dir.join("flash"), dir.join("out.bin"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "the attentions build:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).arg(&dir).arg(&out).output().unwrap();
    assert!(ran.status.success(), "they run: {}", String::from_utf8_lossy(&ran.stderr));
    let all: Vec<f32> = std::fs::read(&out).unwrap().chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
    assert_eq!(all.len(), sizes.iter().map(|&(_, r, h, vd)| 2 * r * h * vd).sum::<usize>(), "every shape's outputs");
    let mut at = 0;
    let mut report = Vec::new();
    for (name, rows, heads, vd) in &sizes {
        let n = rows * heads * vd;
        let (a, b) = (&all[at..at + n], &all[at + n..at + 2 * n]);
        at += 2 * n;
        let mut worst = 0f64;
        for v in 0..rows * heads {
            let (x, y) = (&a[v * vd..(v + 1) * vd], &b[v * vd..(v + 1) * vd]);
            assert!(x.iter().chain(y).all(|f| f.is_finite()), "{name}: row {} head {}: every output written, and finite", v / heads, v % heads);
            let scale = x.iter().fold(0f64, |m, f| m.max(f64::from(f.abs())));
            assert!(scale > 0.0, "{name}: row {} head {} attends something", v / heads, v % heads);
            for (e, (u, w)) in x.iter().zip(y).enumerate() {
                let off = f64::from((u - w).abs()) / scale;
                worst = worst.max(off);
                assert!(off <= TOLERANCE, "{name}: row {} head {} value {e}: {u} against the flash attention's {w} ({off:.2e} of the head's largest)", v / heads, v % heads);
            }
        }
        report.push(format!("{name}: within {worst:.2e}"));
    }
    eprintln!("{}", report.join("\n"));
}

/// The f16 tensor-core attention (`lk_attention_mma`, the one-word numerics') computes what the attention a block a row
/// computes (`lk_attention`) at llama.cpp's precision, on the same queries, caches and positions: grouped heads; grouped
/// heads within a window and with sinks; a head a group; Qwen3's 32 heads over 8 groups; values narrower than the keys;
/// 16-wide heads; a 64-wide latent whose values are the first 48 of each key row, read in place; Gemma-3's 256-wide
/// heads, four over one group within a window, and Qwen3.5's, eight over two (tiles of 32 keys) — 40 rows after 70
/// cached positions, and 96 rows after 400 so every row's span crosses tiles; and split, each query's and key's last
/// part read apart as an expanded latent attention reads them, against the tiles' split. Each output within `TOLERANCE`
/// of its head's largest — what rounding the queries, keys, values and weights to f16 moves — and every output written
/// (they start as NaN).
#[test]
fn the_tensor_core_attention_computes_what_the_attention_computes() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{
        attn_mma_smem_bytes, attn_smem_bytes, attn_tile_heads, attn_tile_smem_bytes, attn_tile_values, attn_values_per_thread, Kernel, ATTN_MIN_SPLIT,
        ATTN_MMA_QUERIES, ATTN_TILE_Q, PRELUDE,
    };
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    const TOLERANCE: f64 = 1e-3;
    // (name, heads, kv heads, hd, vd, window, sinks, values in the key cache, rows, cached positions)
    let shapes: [(&str, usize, usize, usize, usize, usize, bool, bool, usize, usize); 12] = [
        ("grouped", 8, 2, 64, 64, 0, false, false, 40, 70),
        ("windowed with sinks", 8, 2, 64, 64, 32, true, false, 40, 70),
        ("a head a group", 4, 4, 128, 128, 0, false, false, 40, 70),
        ("qwen3", 32, 8, 128, 128, 0, false, false, 40, 70),
        ("narrow values", 8, 2, 128, 64, 0, false, false, 40, 70),
        ("16-wide heads", 4, 2, 16, 16, 0, false, false, 40, 70),
        ("small latent", 4, 1, 64, 48, 0, false, true, 40, 70),
        ("grouped, long", 8, 2, 64, 64, 0, false, false, 96, 400),
        ("qwen3, long", 32, 8, 128, 128, 0, true, false, 96, 400),
        ("gemma3, windowed", 4, 1, 256, 256, 64, false, false, 40, 70),
        ("qwen3.5", 8, 2, 256, 256, 0, false, false, 40, 70),
        ("gemma3, windowed, long", 4, 1, 256, 256, 128, false, false, 96, 400),
    ];
    let dir = dir().join("mma");
    std::fs::create_dir_all(&dir).unwrap();
    let unit = |i: usize, salt: u64| {
        let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(31) ^ 0x2545_F491_4F6C_DD1D;
        ((h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32
    };
    let write = |name: String, v: &[f32]| std::fs::write(dir.join(name), v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    let mut calls = String::new();
    let mut sizes = Vec::new();
    for (k, &(name, heads, kvh, hd, vd, win, sinks, latent, rows, cached)) in shapes.iter().enumerate() {
        let (group, len) = (heads / kvh, cached + rows);
        let kvw = kvh * hd;
        let (vw, voff) = if latent { (kvw, 0) } else { (kvh * vd, 0) };
        write(format!("q{k}.bin"), &(0..rows * heads * hd).map(|i| unit(i, 10 + k as u64)).collect::<Vec<f32>>());
        write(format!("k{k}.bin"), &(0..len * kvw).map(|i| unit(i, 20 + k as u64)).collect::<Vec<f32>>());
        write(format!("v{k}.bin"), &(0..len * vw).map(|i| unit(i, 30 + k as u64)).collect::<Vec<f32>>());
        write(format!("s{k}.bin"), &(0..heads).map(|i| unit(i, 40 + k as u64) * 2.0).collect::<Vec<f32>>());
        let scale = 1.0 / (hd as f64).sqrt();
        let (hg, bq) = (attn_tile_heads(group), ATTN_MMA_QUERIES);
        let smem = attn_mma_smem_bytes(hd, vd);
        let _ = write!(
            calls,
            "  {{ // {name}\n    int p0 = {cached}; CK(cudaMemcpy(pos, &p0, 4, cudaMemcpyHostToDevice));\n\
             \x20   float* q = dev(dir + \"/q{k}.bin\"); float* kc = dev(dir + \"/k{k}.bin\"); float* vc = {}; float* sk = {};\n\
             \x20   float *a = nan({rows} * {heads} * {vd}), *b = nan({rows} * {heads} * {vd});\n\
             \x20   lk_attention<1, {}, float><<<dim3({heads}, {rows}, 1), 256, {}>>>(q, kc, vc, pos, sk, a, nullptr, nullptr, {len}, {win}, {hd}, {vd}, {group}, {heads}, {kvw}, {vw}, {voff}, {scale:e}f, {ATTN_MIN_SPLIT}, 1);\n\
             \x20   CK(cudaGetLastError());\n\
             \x20   CK(cudaFuncSetAttribute(lk_attention_mma<{hg}, {hd}, {vd}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {smem}));\n\
             \x20   lk_attention_mma<{hg}, {hd}, {vd}><<<dim3({}, {}), 128, {smem}>>>(q, kc, vc, pos, sk, b, {len}, {win}, {group}, {heads}, {kvw}, {vw}, {voff}, {scale:e}f, {rows});\n\
             \x20   CK(cudaGetLastError()); CK(cudaDeviceSynchronize());\n\
             \x20   put(out, a, {rows} * {heads} * {vd}); put(out, b, {rows} * {heads} * {vd});\n  }}\n",
            if latent { "kc".to_string() } else { format!("dev(dir + \"/v{k}.bin\")") },
            if sinks { format!("dev(dir + \"/s{k}.bin\")") } else { "nullptr".into() },
            attn_values_per_thread(vd),
            attn_smem_bytes(1, hd, 1, 4),
            rows.div_ceil(bq / hg),
            heads / hg
        );
        sizes.push((name.to_string(), rows, heads, vd));
    }
    // Split: 8 heads a group each, each query its own 128 in the projection (192 a head) and its rotated 64 apart; each
    // key its head's own 128 (expanded) and the rotated 64 every head shares at 512 in a 576-wide cache row.
    let (h, own, rope, vd, rows, cached, kw, lora) = (8usize, 128usize, 64usize, 128usize, 40usize, 70usize, 576usize, 512usize);
    let (qk, len) = (own + rope, cached + rows);
    write("sq.bin".into(), &(0..rows * h * qk).map(|i| unit(i, 60)).collect::<Vec<f32>>());
    write("sqe.bin".into(), &(0..rows * h * rope).map(|i| unit(i, 61)).collect::<Vec<f32>>());
    write("sk.bin".into(), &(0..len * h * own).map(|i| unit(i, 62)).collect::<Vec<f32>>());
    write("skc.bin".into(), &(0..len * kw).map(|i| unit(i, 63)).collect::<Vec<f32>>());
    write("sv.bin".into(), &(0..len * h * vd).map(|i| unit(i, 64)).collect::<Vec<f32>>());
    let scale = 1.0 / (qk as f64).sqrt();
    let (bq, smem) = (ATTN_MMA_QUERIES, attn_mma_smem_bytes(qk, vd));
    let _ = write!(
        calls,
        "  {{ // split\n    int p0 = {cached}; CK(cudaMemcpy(pos, &p0, 4, cudaMemcpyHostToDevice));\n\
         \x20   float* q = dev(dir + \"/sq.bin\"); float* qe = dev(dir + \"/sqe.bin\"); float* k = dev(dir + \"/sk.bin\"); float* kc = dev(dir + \"/skc.bin\"); float* v = dev(dir + \"/sv.bin\");\n\
         \x20   float *a = nan({rows} * {h} * {vd}), *b = nan({rows} * {h} * {vd});\n\
         \x20   CK(cudaFuncSetAttribute(lk_attention_tiled<1, {}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {}));\n\
         \x20   lk_attention_tiled<1, {}><<<dim3({}, {h}), 256, {}>>>(q, k, v, pos, nullptr, a, {len}, 0, {qk}, {vd}, 1, {h}, {}, {}, 0, {scale:e}f, {rows}, qe, {qk}, {rope}, kc, {kw}, {lora}, {own});\n\
         \x20   CK(cudaGetLastError());\n\
         \x20   CK(cudaFuncSetAttribute(lk_attention_mma<1, {qk}, {vd}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {smem}));\n\
         \x20   lk_attention_mma<1, {qk}, {vd}><<<dim3({}, {h}), 128, {smem}>>>(q, k, v, pos, nullptr, b, {len}, 0, 1, {h}, {}, {}, 0, {scale:e}f, {rows}, qe, {qk}, {rope}, kc, {kw}, {lora}, {own});\n\
         \x20   CK(cudaGetLastError()); CK(cudaDeviceSynchronize());\n\
         \x20   put(out, a, {rows} * {h} * {vd}); put(out, b, {rows} * {h} * {vd});\n  }}\n",
        attn_tile_values(vd),
        attn_tile_smem_bytes(vd),
        attn_tile_values(vd),
        rows.div_ceil(ATTN_TILE_Q),
        attn_tile_smem_bytes(vd),
        h * own,
        h * vd,
        rows.div_ceil(bq),
        h * own,
        h * vd
    );
    sizes.push(("split".to_string(), rows, h, vd));
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <string>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {PRELUDE}{}{}{}{}\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ fprintf(stderr, \"cuda %d %s\\n\", __LINE__, cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         static float* dev(const std::string& p){{ FILE* f = fopen(p.c_str(), \"rb\"); if(!f) exit(2); std::vector<char> b; char c[65536]; size_t n;\n\
           while((n = fread(c, 1, sizeof c, f)) > 0) b.insert(b.end(), c, c + n); fclose(f);\n\
           float* d; CK(cudaMalloc(&d, b.size())); CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice)); return d; }}\n\
         static float* nan(long n){{ float* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0xff, n * 4)); return p; }}\n\
         static void put(FILE* out, const float* d, long n){{ std::vector<float> h(n); CK(cudaMemcpy(h.data(), d, n * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, n, out); }}\n\
         int main(int argc, char** argv){{\n  std::string dir = argv[1];\n  int* pos; CK(cudaMalloc(&pos, 4));\n\
           FILE* out = fopen(argv[2], \"wb\");\n{calls}  fclose(out);\n  return 0;\n}}\n",
        Kernel::Mma.source(),
        Kernel::Attention.source(),
        Kernel::AttentionTiled.source(),
        Kernel::AttentionMma.source()
    );
    let (cu, bin, out) = (dir.join("mma.cu"), dir.join("mma"), dir.join("out.bin"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "the attentions build:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).arg(&dir).arg(&out).output().unwrap();
    assert!(ran.status.success(), "they run: {}", String::from_utf8_lossy(&ran.stderr));
    let all: Vec<f32> = std::fs::read(&out).unwrap().chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
    assert_eq!(all.len(), sizes.iter().map(|&(_, r, h, vd)| 2 * r * h * vd).sum::<usize>(), "every shape's outputs");
    let mut at = 0;
    let mut report = Vec::new();
    for (name, rows, heads, vd) in &sizes {
        let n = rows * heads * vd;
        let (a, b) = (&all[at..at + n], &all[at + n..at + 2 * n]);
        at += 2 * n;
        let mut worst = 0f64;
        for v in 0..rows * heads {
            let (x, y) = (&a[v * vd..(v + 1) * vd], &b[v * vd..(v + 1) * vd]);
            assert!(x.iter().chain(y).all(|f| f.is_finite()), "{name}: row {} head {}: every output written, and finite", v / heads, v % heads);
            let scale = x.iter().fold(0f64, |m, f| m.max(f64::from(f.abs())));
            assert!(scale > 0.0, "{name}: row {} head {} attends something", v / heads, v % heads);
            for (e, (u, w)) in x.iter().zip(y).enumerate() {
                let off = f64::from((u - w).abs()) / scale;
                worst = worst.max(off);
                assert!(off <= TOLERANCE, "{name}: row {} head {} value {e}: {u} against the tensor cores' {w} ({off:.2e} of the head's largest)", v / heads, v % heads);
            }
        }
        report.push(format!("{name}: within {worst:.2e}"));
    }
    eprintln!("{}", report.join("\n"));
}

/// The two-word attention (`lk_attention_i8` after `lk_kv_two_words`, the two-word numerics') computes what the
/// attention a block a row computes (`lk_attention`), on the same queries, caches and positions: grouped heads; grouped
/// heads within a window and with sinks; a head a group; Qwen3's 32 heads over 8 groups, and with one key channel in 37
/// twenty times the rest; values narrower than the keys; 32-wide heads; a 64-wide latent whose values are the first 32 of
/// each key row, read in place; Gemma-3's 256-wide heads, four over one group within a window, and Qwen3.5's, eight over
/// two (tiles of 32 keys) — 40 rows after 70 cached positions, and 96 rows after 400 so every row's span crosses tiles;
/// and split, each query's and key's last part read apart as an expanded latent attention reads them, against the tiles'
/// split. Each output within `TOLERANCE` of its head's largest — what taking the queries, keys and values into two words
/// (and the weights into three) moves, a quarter of what rounding them to f16 moves (`lk_attention_mma`'s 1e-3) — and
/// within `EXACT` of the `f64` attention of its operands taken into two words by the lane's quantizer, replicated bit for
/// bit: every product of the stored words taken, the weights finer than the operands. Every output written (they start
/// as NaN, and the two-word scratch as bytes no key or scale has). Under the pinned
/// compute-sanitizer the program shows no shared-memory race — every `cp.async` tile waited for before it is read, every
/// write after the reads it follows — no misplaced barrier and no stray access, as a warning or an error.
#[test]
fn the_two_word_attention_computes_what_the_attention_computes() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{
        attn_i8_rows, attn_i8_smem_bytes, attn_smem_bytes, attn_tile_heads, attn_tile_smem_bytes, attn_tile_values, attn_values_per_thread, Kernel,
        ATTN_I8_QUERIES, ATTN_MIN_SPLIT, ATTN_TILE_Q, PRELUDE,
    };
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    const TOLERANCE: f64 = 3e-4;
    // From the `f64` attention of the operands in two words: what the kernel's `f32` sums and exponentials move.
    const EXACT: f64 = 1e-5;
    // (name, heads, kv heads, hd, vd, window, sinks, values in the key cache, rows, cached positions, outlier key channel)
    let shapes: [(&str, usize, usize, usize, usize, usize, bool, bool, usize, usize, f32); 13] = [
        ("grouped", 8, 2, 64, 64, 0, false, false, 40, 70, 1.0),
        ("windowed with sinks", 8, 2, 64, 64, 32, true, false, 40, 70, 1.0),
        ("a head a group", 4, 4, 128, 128, 0, false, false, 40, 70, 1.0),
        ("qwen3", 32, 8, 128, 128, 0, false, false, 40, 70, 1.0),
        ("qwen3, outlier keys", 32, 8, 128, 128, 0, false, false, 40, 70, 20.0),
        ("narrow values", 8, 2, 128, 64, 0, false, false, 40, 70, 1.0),
        ("32-wide heads", 4, 2, 32, 32, 0, false, false, 40, 70, 1.0),
        ("small latent", 4, 1, 64, 32, 0, false, true, 40, 70, 1.0),
        ("grouped, long", 8, 2, 64, 64, 0, false, false, 96, 400, 1.0),
        ("qwen3, long", 32, 8, 128, 128, 0, true, false, 96, 400, 1.0),
        ("gemma3, windowed", 4, 1, 256, 256, 64, false, false, 40, 70, 1.0),
        ("qwen3.5", 8, 2, 256, 256, 0, false, false, 40, 70, 1.0),
        ("gemma3, windowed, long", 4, 1, 256, 256, 128, false, false, 96, 400, 1.0),
    ];
    let dir = dir().join("two-words");
    std::fs::create_dir_all(&dir).unwrap();
    let unit = |i: usize, salt: u64| {
        let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(31) ^ 0x2545_F491_4F6C_DD1D;
        ((h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32
    };
    let write = |name: String, v: &[f32]| std::fs::write(dir.join(name), v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    // The scratch of a cache of `len` positions over `kvh` groups: the keys' words (high, then low), the values', the
    // scales (keys', then values').
    let scratch = |len: usize, kvh: usize, hd: usize, vd: usize| {
        let rows = kvh * attn_i8_rows(len);
        format!(
            "signed char* k8 = junk({}); signed char* w8 = junk({}); float* s8 = (float*)junk({});\n\
             \x20   signed char *k8l = k8 + {}, *w8l = w8 + {}; float* s8v = s8 + {rows};",
            2 * rows * hd,
            2 * rows * vd,
            2 * rows * 4,
            rows * hd,
            rows * vd
        )
    };
    let mut calls = String::new();
    let mut sizes = Vec::new();
    for (k, &(name, heads, kvh, hd, vd, win, sinks, latent, rows, cached, outlier)) in shapes.iter().enumerate() {
        let (group, len) = (heads / kvh, cached + rows);
        let kvw = kvh * hd;
        let (vw, voff) = if latent { (kvw, 0) } else { (kvh * vd, 0) };
        write(format!("q{k}.bin"), &(0..rows * heads * hd).map(|i| unit(i, 10 + k as u64)).collect::<Vec<f32>>());
        write(format!("k{k}.bin"), &(0..len * kvw).map(|i| unit(i, 20 + k as u64) * if (i % hd) % 37 == 3 { outlier } else { 1.0 }).collect::<Vec<f32>>());
        write(format!("v{k}.bin"), &(0..len * vw).map(|i| unit(i, 30 + k as u64)).collect::<Vec<f32>>());
        write(format!("s{k}.bin"), &(0..heads).map(|i| unit(i, 40 + k as u64) * 2.0).collect::<Vec<f32>>());
        let scale = 1.0 / (hd as f64).sqrt();
        let hg = attn_tile_heads(group);
        let lpad = attn_i8_rows(len);
        let _ = write!(
            calls,
            "  {{ // {name}\n    int p0 = {cached}; CK(cudaMemcpy(pos, &p0, 4, cudaMemcpyHostToDevice));\n\
             \x20   float* q = dev(dir + \"/q{k}.bin\"); float* kc = dev(dir + \"/k{k}.bin\"); float* vc = {}; float* sk = {};\n\
             \x20   float *a = nan({rows} * {heads} * {vd}), *b = nan({rows} * {heads} * {vd});\n\
             \x20   {}\n\
             \x20   lk_attention<1, {}, float><<<dim3({heads}, {rows}, 1), 256, {}>>>(q, kc, vc, pos, sk, a, nullptr, nullptr, {len}, {win}, {hd}, {vd}, {group}, {heads}, {kvw}, {vw}, {voff}, {scale:e}f, {ATTN_MIN_SPLIT}, 1);\n\
             \x20   CK(cudaGetLastError());\n\
             \x20   lk_kv_two_words<{hd}, {vd}><<<dim3({}, {kvh}), 256>>>(kc, vc, pos, {rows}, {len}, {kvw}, {vw}, {voff}, {lpad}, k8, k8l, s8, w8, w8l, s8v);\n\
             \x20   CK(cudaGetLastError());\n\
             \x20   lk_attention_i8<{hg}, {hd}, {vd}><<<dim3({}, {}), 128, {}>>>(q, k8, k8l, s8, w8, w8l, s8v, {lpad}, pos, sk, b, {len}, {win}, {group}, {heads}, {scale:e}f, {rows});\n\
             \x20   CK(cudaGetLastError()); CK(cudaDeviceSynchronize());\n\
             \x20   put(out, a, {rows} * {heads} * {vd}); put(out, b, {rows} * {heads} * {vd});\n  }}\n",
            if latent { "kc".to_string() } else { format!("dev(dir + \"/v{k}.bin\")") },
            if sinks { format!("dev(dir + \"/s{k}.bin\")") } else { "nullptr".into() },
            scratch(len, kvh, hd, vd),
            attn_values_per_thread(vd),
            attn_smem_bytes(1, hd, 1, 4),
            lpad / 32,
            rows.div_ceil(ATTN_I8_QUERIES / hg),
            heads / hg,
            attn_i8_smem_bytes(hd, vd)
        );
        sizes.push((name.to_string(), rows, heads, vd));
    }
    // Split: 8 heads a group each, each query its own 128 in the projection (192 a head) and its rotated 64 apart; each
    // key its head's own 128 (expanded) and the rotated 64 every head shares at 512 in a 576-wide cache row.
    let (h, own, rope, vd, rows, cached, kw, lora) = (8usize, 128usize, 64usize, 128usize, 40usize, 70usize, 576usize, 512usize);
    let (qk, len) = (own + rope, cached + rows);
    write("sq.bin".into(), &(0..rows * h * qk).map(|i| unit(i, 60)).collect::<Vec<f32>>());
    write("sqe.bin".into(), &(0..rows * h * rope).map(|i| unit(i, 61)).collect::<Vec<f32>>());
    write("sk.bin".into(), &(0..len * h * own).map(|i| unit(i, 62)).collect::<Vec<f32>>());
    write("skc.bin".into(), &(0..len * kw).map(|i| unit(i, 63)).collect::<Vec<f32>>());
    write("sv.bin".into(), &(0..len * h * vd).map(|i| unit(i, 64)).collect::<Vec<f32>>());
    let scale = 1.0 / (qk as f64).sqrt();
    let lpad = attn_i8_rows(len);
    let _ = write!(
        calls,
        "  {{ // split\n    int p0 = {cached}; CK(cudaMemcpy(pos, &p0, 4, cudaMemcpyHostToDevice));\n\
         \x20   float* q = dev(dir + \"/sq.bin\"); float* qe = dev(dir + \"/sqe.bin\"); float* k = dev(dir + \"/sk.bin\"); float* kc = dev(dir + \"/skc.bin\"); float* v = dev(dir + \"/sv.bin\");\n\
         \x20   float *a = nan({rows} * {h} * {vd}), *b = nan({rows} * {h} * {vd});\n\
         \x20   {}\n\
         \x20   CK(cudaFuncSetAttribute(lk_attention_tiled<1, {}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {}));\n\
         \x20   lk_attention_tiled<1, {}><<<dim3({}, {h}), 256, {}>>>(q, k, v, pos, nullptr, a, {len}, 0, {qk}, {vd}, 1, {h}, {}, {}, 0, {scale:e}f, {rows}, qe, {qk}, {rope}, kc, {kw}, {lora}, {own});\n\
         \x20   CK(cudaGetLastError());\n\
         \x20   lk_kv_two_words<{qk}, {vd}><<<dim3({}, {h}), 256>>>(k, v, pos, {rows}, {len}, {}, {}, 0, {lpad}, k8, k8l, s8, w8, w8l, s8v, kc, {kw}, {lora}, {own});\n\
         \x20   CK(cudaGetLastError());\n\
         \x20   lk_attention_i8<1, {qk}, {vd}><<<dim3({}, {h}), 128, {}>>>(q, k8, k8l, s8, w8, w8l, s8v, {lpad}, pos, nullptr, b, {len}, 0, 1, {h}, {scale:e}f, {rows}, qe, {qk}, {rope}, {own});\n\
         \x20   CK(cudaGetLastError()); CK(cudaDeviceSynchronize());\n\
         \x20   put(out, a, {rows} * {h} * {vd}); put(out, b, {rows} * {h} * {vd});\n  }}\n",
        scratch(len, h, qk, vd),
        attn_tile_values(vd),
        attn_tile_smem_bytes(vd),
        attn_tile_values(vd),
        rows.div_ceil(ATTN_TILE_Q),
        attn_tile_smem_bytes(vd),
        h * own,
        h * vd,
        lpad / 32,
        h * own,
        h * vd,
        rows.div_ceil(ATTN_I8_QUERIES),
        attn_i8_smem_bytes(qk, vd)
    );
    sizes.push(("split".to_string(), rows, h, vd));
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <string>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {PRELUDE}{}{}{}{}\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ fprintf(stderr, \"cuda %d %s\\n\", __LINE__, cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         static float* dev(const std::string& p){{ FILE* f = fopen(p.c_str(), \"rb\"); if(!f) exit(2); std::vector<char> b; char c[65536]; size_t n;\n\
           while((n = fread(c, 1, sizeof c, f)) > 0) b.insert(b.end(), c, c + n); fclose(f);\n\
           float* d; CK(cudaMalloc(&d, b.size())); CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice)); return d; }}\n\
         static float* nan(long n){{ float* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0xff, n * 4)); return p; }}\n\
         static signed char* junk(long n){{ signed char* p; CK(cudaMalloc(&p, n)); CK(cudaMemset(p, 0xff, n)); return p; }}\n\
         static void put(FILE* out, const float* d, long n){{ std::vector<float> h(n); CK(cudaMemcpy(h.data(), d, n * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, n, out); }}\n\
         int main(int argc, char** argv){{\n  std::string dir = argv[1];\n  int* pos; CK(cudaMalloc(&pos, 4));\n\
           FILE* out = fopen(argv[2], \"wb\");\n{calls}  fclose(out);\n  return 0;\n}}\n",
        Kernel::Mma.source(),
        Kernel::Attention.source(),
        Kernel::AttentionTiled.source(),
        Kernel::AttentionI8.source()
    );
    let (cu, bin, out) = (dir.join("two.cu"), dir.join("two"), dir.join("out.bin"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-lineinfo", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "the attentions build:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).arg(&dir).arg(&out).output().unwrap();
    assert!(ran.status.success(), "they run: {}", String::from_utf8_lossy(&ran.stderr));
    let all: Vec<f32> = std::fs::read(&out).unwrap().chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
    assert_eq!(all.len(), sizes.iter().map(|&(_, r, h, vd)| 2 * r * h * vd).sum::<usize>(), "every shape's outputs");
    let mut at = 0;
    let mut report = Vec::new();
    for (name, rows, heads, vd) in &sizes {
        let n = rows * heads * vd;
        let (a, b) = (&all[at..at + n], &all[at + n..at + 2 * n]);
        at += 2 * n;
        let mut worst = 0f64;
        for v in 0..rows * heads {
            let (x, y) = (&a[v * vd..(v + 1) * vd], &b[v * vd..(v + 1) * vd]);
            assert!(x.iter().chain(y).all(|f| f.is_finite()), "{name}: row {} head {}: every output written, and finite", v / heads, v % heads);
            let scale = x.iter().fold(0f64, |m, f| m.max(f64::from(f.abs())));
            assert!(scale > 0.0, "{name}: row {} head {} attends something", v / heads, v % heads);
            for (e, (u, w)) in x.iter().zip(y).enumerate() {
                let off = f64::from((u - w).abs()) / scale;
                worst = worst.max(off);
                assert!(off <= TOLERANCE, "{name}: row {} head {} value {e}: {u} against the two-word attention's {w} ({off:.2e} of the head's largest)", v / heads, v % heads);
            }
        }
        report.push(format!("{name}: within {worst:.2e}"));
    }
    eprintln!("{}", report.join("\n"));
    // Against the attention of the operands taken into two words, in `f64`: the inputs as the program read them, each
    // query row by the queries' quantizer, each key and value row by the pre-pass's.
    let rowed = |data: &[f32], at: usize, n: usize, products: bool| two_words_row(&data[at..at + n], products);
    let mut models: Vec<Vec<f64>> = Vec::new();
    for (k, &(_, heads, kvh, hd, vd, win, sinks, latent, rows, cached, outlier)) in shapes.iter().enumerate() {
        let (group, len, kvw) = (heads / kvh, cached + rows, kvh * hd);
        let vw = if latent { kvw } else { kvh * vd };
        let q: Vec<f32> = (0..rows * heads * hd).map(|i| unit(i, 10 + k as u64)).collect();
        let kc: Vec<f32> = (0..len * kvw).map(|i| unit(i, 20 + k as u64) * if (i % hd) % 37 == 3 { outlier } else { 1.0 }).collect();
        let vc: Vec<f32> = if latent { kc.clone() } else { (0..len * vw).map(|i| unit(i, 30 + k as u64)).collect() };
        let sk: Vec<f32> = (0..heads).map(|i| unit(i, 40 + k as u64) * 2.0).collect();
        let keys: Vec<Vec<Vec<f64>>> = (0..kvh).map(|g| (0..len).map(|j| rowed(&kc, j * kvw + g * hd, hd, true)).collect()).collect();
        let values: Vec<Vec<Vec<f64>>> = (0..kvh).map(|g| (0..len).map(|j| rowed(&vc, j * vw + g * vd, vd, true)).collect()).collect();
        let queries = |r: usize, h: usize| rowed(&q, (r * heads + h) * hd, hd, false);
        models.push(attention_f64(rows, heads, group, cached, win, if sinks { Some(&sk) } else { None }, 1.0 / (hd as f64).sqrt(), &queries, &keys, &values));
    }
    {
        let (h, own, rope, vd, rows, cached, kw, lora) = (8usize, 128usize, 64usize, 128usize, 40usize, 70usize, 576usize, 512usize);
        let (qk, len) = (own + rope, cached + rows);
        let sq: Vec<f32> = (0..rows * h * qk).map(|i| unit(i, 60)).collect();
        let sqe: Vec<f32> = (0..rows * h * rope).map(|i| unit(i, 61)).collect();
        let sk: Vec<f32> = (0..len * h * own).map(|i| unit(i, 62)).collect();
        let skc: Vec<f32> = (0..len * kw).map(|i| unit(i, 63)).collect();
        let sv: Vec<f32> = (0..len * h * vd).map(|i| unit(i, 64)).collect();
        let joined = |a: &[f32], b: &[f32]| a.iter().chain(b).copied().collect::<Vec<f32>>();
        let keys: Vec<Vec<Vec<f64>>> =
            (0..h).map(|g| (0..len).map(|j| two_words_row(&joined(&sk[j * h * own + g * own..][..own], &skc[j * kw + lora..][..rope]), true)).collect()).collect();
        let values: Vec<Vec<Vec<f64>>> = (0..h).map(|g| (0..len).map(|j| rowed(&sv, j * h * vd + g * vd, vd, true)).collect()).collect();
        let queries = |r: usize, hh: usize| {
            let u = r * h + hh;
            two_words_row(&joined(&sq[u * qk..][..own], &sqe[u * rope..][..rope]), false)
        };
        models.push(attention_f64(rows, h, 1, cached, 0, None, 1.0 / (qk as f64).sqrt(), &queries, &keys, &values));
    }
    let mut at = 0;
    let mut exact = Vec::new();
    for ((name, rows, heads, vd), model) in sizes.iter().zip(&models) {
        let n = rows * heads * vd;
        let b = &all[at + n..at + 2 * n];
        at += 2 * n;
        let mut worst = 0f64;
        for u in 0..rows * heads {
            let (got, want) = (&b[u * vd..(u + 1) * vd], &model[u * vd..(u + 1) * vd]);
            let scale = want.iter().fold(0f64, |m, w| m.max(w.abs()));
            for (e, (g, w)) in got.iter().zip(want).enumerate() {
                let off = (f64::from(*g) - w).abs() / scale;
                worst = worst.max(off);
                assert!(
                    off <= EXACT,
                    "{name}: row {} head {} value {e}: {g} against the attention of its two-word operands' {w} ({off:.2e} of the head's largest)",
                    u / heads,
                    u % heads
                );
            }
        }
        exact.push(format!("{name}: within {worst:.2e} of its two-word operands' attention"));
    }
    eprintln!("{}", exact.join("\n"));
    if common::require_tool("cuda-sanitizer") {
        for (tool, clean) in [("racecheck", "RACECHECK SUMMARY: 0 hazards displayed (0 errors, 0 warnings)"), ("synccheck", "ERROR SUMMARY: 0 errors"), ("memcheck", "ERROR SUMMARY: 0 errors")] {
            let checked = Command::new(common::cuda_sanitizer())
                .args(["--tool", tool])
                .args(if tool == "racecheck" { &["--racecheck-report", "all"][..] } else { &[][..] })
                .arg(&bin)
                .arg(&dir)
                .arg(dir.join(format!("{tool}.bin")))
                .output()
                .unwrap();
            let said = String::from_utf8_lossy(&checked.stdout).into_owned() + &String::from_utf8_lossy(&checked.stderr);
            assert!(checked.status.success() && said.contains(clean), "the attentions under {tool}:\n{}", said.lines().take(40).collect::<Vec<_>>().join("\n"));
        }
    }
}

/// The lane's two-word quantizer replicated in `f32`, bit for bit: a row's scale from its largest magnitude
/// (`lk_two_scale`), each value's two words by quotients (`lk_two_words`, the queries') or by products with the scale's
/// reciprocal (`lk_two_words_r`, the keys' and values') — the value the two words hold, in `f64`.
fn two_words_row(row: &[f32], by_products: bool) -> Vec<f64> {
    let a = row.iter().fold(0f32, |m, v| m.max(v.abs()));
    let s = (if a > 1e-30 { a } else { 1e-30 }) / 127.0;
    let r = 1.0 / s;
    let word = |x: f32| x.round_ties_even().clamp(-128.0, 127.0);
    row.iter()
        .map(|&v| {
            let h = word(if by_products { v * r } else { v / s });
            let rest = (-h).mul_add(s, v);
            let l = word(if by_products { rest * r * 128.0 } else { rest / (s * 0.0078125) });
            f64::from(h) * f64::from(s) + f64::from(l) * f64::from(s * 0.0078125)
        })
        .collect()
}

/// An attention in `f64`: row `r` of `rows`, head `h` (of `heads`, `group` a key/value group), query `query(r, h)`,
/// attends keys `[lo, end)` of its group — `end = cached + r`, `lo` the window's start — `keys[g][j]` and
/// `values[g][j]`; a sink joins each head's denominator. Each head's values, a row's heads one after another.
fn attention_f64(
    rows: usize,
    heads: usize,
    group: usize,
    cached: usize,
    win: usize,
    sinks: Option<&[f32]>,
    scale: f64,
    query: &dyn Fn(usize, usize) -> Vec<f64>,
    keys: &[Vec<Vec<f64>>],
    values: &[Vec<Vec<f64>>],
) -> Vec<f64> {
    let vd = values[0][0].len();
    let mut out = Vec::with_capacity(rows * heads * vd);
    for r in 0..rows {
        for h in 0..heads {
            let (q, g, end) = (query(r, h), h / group, cached + r);
            let lo = if win > 0 && end > win { end - win } else { 0 };
            let scores: Vec<f64> = (lo..end).map(|j| q.iter().zip(&keys[g][j]).map(|(a, b)| a * b).sum::<f64>() * scale).collect();
            let sink = sinks.map(|s| f64::from(s[h]));
            let m = scores.iter().copied().chain(sink).fold(f64::NEG_INFINITY, f64::max);
            let mut o = vec![0f64; vd];
            let mut l = sink.map_or(0.0, |s| (s - m).exp());
            for (j, sc) in (lo..end).zip(&scores) {
                let p = (sc - m).exp();
                l += p;
                for (x, v) in o.iter_mut().zip(&values[g][j]) {
                    *x += p * v;
                }
            }
            out.extend(o.into_iter().map(|x| x / l));
        }
    }
    out
}

/// The shared-row attention (`lk_attention_shared`: every head of one key/value group in a block, each cached row
/// read once for every head's score and value, its chunks folded by `lk_attention_fold`) computes what the attention
/// computes (`lk_attention`, its heads a block as the planner gives them), on the same queries, caches and positions:
/// DeepSeek-V2-Lite's latent attention (16 heads over 576-wide rows, values the first 512), the tiny latent attention
/// (4 heads over 72, values the first 64) and 8 heads over 64-wide rows, values the first 32, with sinks within a window
/// of 24 (one chunk) and of 200 (folded) — one row (the decode step) and four rows (a narrow chunk), after 70 cached
/// positions and after 1100, the span cut into at most 1, 7, 17 and 40 chunks (one chunk written directly, the fold's
/// first sixteen chunks, more, and more than a warp's lanes). Each output within `TOLERANCE` of its head's largest, the
/// difference of two summation orders, and every output written (they start as NaN); and the values read from rows of
/// their own, sliced out of the key rows as a program holding back `strided_values` slices them, give the same outputs
/// bit for bit.
#[test]
fn the_shared_row_attention_computes_what_the_attention_computes() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{
        attn_heads_per_block, attn_shared_smem_bytes, attn_smem_bytes, attn_splits, attn_values_per_thread, Kernel, ATTN_FOLD_PAIRS, ATTN_MIN_SPLIT,
        PRELUDE,
    };
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    const TOLERANCE: f64 = 1e-5;
    // (name, heads, hd, vd, window, sinks)
    let shapes: [(&str, usize, usize, usize, usize, bool); 4] = [
        ("latent", 16, 576, 512, 0, false),
        ("tiny latent", 4, 72, 64, 0, false),
        ("windowed with sinks", 8, 64, 32, 24, true),
        ("widely windowed with sinks", 8, 64, 32, 200, true),
    ];
    let dir = dir().join("shared");
    std::fs::create_dir_all(&dir).unwrap();
    let unit = |i: usize, salt: u64| {
        let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(17) ^ 0x2545_F491_4F6C_DD1D;
        ((h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32
    };
    let mut calls = String::new();
    let mut cases = Vec::new();
    for (k, &(name, heads, hd, vd, win, sinks)) in shapes.iter().enumerate() {
        let vpt = attn_values_per_thread(vd);
        let smem = attn_shared_smem_bytes(heads, hd);
        for vk in ["true", "false"] {
            let _ = writeln!(
                calls,
                "  CK(cudaFuncSetAttribute(lk_attention_shared<{heads}, {vpt}, {}, {vk}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {smem}));",
                hd / 4
            );
        }
        for cached in [70usize, 1100] {
            for rows in [1usize, 4] {
                let len = cached + rows;
                let tag = format!("{k}-{cached}-{rows}");
                let q: Vec<f32> = (0..rows * heads * hd).map(|i| unit(i, 10 + k as u64)).collect();
                let kc: Vec<f32> = (0..len * hd).map(|i| unit(i, 20 + k as u64)).collect();
                let sk: Vec<f32> = (0..heads).map(|i| unit(i, 40 + k as u64) * 2.0).collect();
                let vs: Vec<f32> = kc.chunks_exact(hd).flat_map(|row| row[..vd].to_vec()).collect();
                for (part, v) in [("q", &q), ("k", &kc), ("s", &sk), ("v", &vs)] {
                    std::fs::write(dir.join(format!("{part}{tag}.bin")), v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
                }
                let scale = 1.0 / (hd as f64).sqrt();
                let g = attn_heads_per_block(heads, hd, vd, len, 28, 99 * 1024, 4).expect("the attention's heads fit a block");
                let splits_a = attn_splits(heads / g, len, 28);
                let sinks_arg = if sinks { format!("dev(dir + \"/s{tag}.bin\")") } else { "nullptr".into() };
                for splits in [1usize, 7, 17, 40] {
                    let _ = write!(
                        calls,
                        "  {{ // {name}, {rows} rows after {cached}, {splits} chunks\n\
                         \x20   int p0 = {cached} + 1; CK(cudaMemcpy(pos, &p0, 4, cudaMemcpyHostToDevice));\n\
                         \x20   float* q = dev(dir + \"/q{tag}.bin\"); float* kc = dev(dir + \"/k{tag}.bin\"); float* sk = {sinks_arg};\n\
                         \x20   float* vs = dev(dir + \"/v{tag}.bin\");\n\
                         \x20   float *a = nan({rows} * {heads} * {vd}), *b = nan({rows} * {heads} * {vd}), *c = nan({rows} * {heads} * {vd});\n\
                         \x20   float* pa = nan({rows}L * {heads} * {splits_a} * ({vd} + 2)); int* ca = zeros({rows} * {heads});\n\
                         \x20   CK(cudaFuncSetAttribute(lk_attention<{g}, {vpt}, float>, cudaFuncAttributeMaxDynamicSharedMemorySize, {sa}));\n\
                         \x20   lk_attention<{g}, {vpt}, float><<<dim3({}, {rows}, {splits_a}), 256, {sa}>>>(q, kc, kc, pos, sk, a, pa, ca, {len}, {win}, {hd}, {vd}, {heads}, {heads}, {hd}, {hd}, 0, {scale:e}f, {ATTN_MIN_SPLIT}, {splits_a});\n\
                         \x20   CK(cudaGetLastError());\n\
                         \x20   float* pb = nan({rows}L * {heads} * {splits} * ({vd} + 2));\n\
                         \x20   lk_attention_shared<{heads}, {vpt}, {h4}, true><<<dim3(1, {rows}, {splits}), 256, {smem}>>>(q, kc, kc, pos, sk, b, pb, {len}, {win}, {vd}, {heads}, {hd}, {hd}, 0, {scale:e}f, {ATTN_MIN_SPLIT}, {splits});\n\
                         \x20   CK(cudaGetLastError());\n\
                         \x20   lk_attention_fold<<<dim3({heads}, {rows}, {fb}), {ATTN_FOLD_PAIRS}, {fs}>>>(pb, pos, sk, b, {len}, {win}, {vd}, {heads}, {ATTN_MIN_SPLIT}, {splits});\n\
                         \x20   CK(cudaGetLastError());\n\
                         \x20   float* pc = nan({rows}L * {heads} * {splits} * ({vd} + 2));\n\
                         \x20   lk_attention_shared<{heads}, {vpt}, {h4}, false><<<dim3(1, {rows}, {splits}), 256, {smem}>>>(q, kc, vs, pos, sk, c, pc, {len}, {win}, {vd}, {heads}, {hd}, {vd}, 0, {scale:e}f, {ATTN_MIN_SPLIT}, {splits});\n\
                         \x20   CK(cudaGetLastError());\n\
                         \x20   lk_attention_fold<<<dim3({heads}, {rows}, {fb}), {ATTN_FOLD_PAIRS}, {fs}>>>(pc, pos, sk, c, {len}, {win}, {vd}, {heads}, {ATTN_MIN_SPLIT}, {splits});\n\
                         \x20   CK(cudaGetLastError()); CK(cudaDeviceSynchronize());\n\
                         \x20   put(out, a, {rows} * {heads} * {vd}); put(out, b, {rows} * {heads} * {vd}); put(out, c, {rows} * {heads} * {vd});\n  }}\n",
                        heads / g,
                        sa = attn_smem_bytes(g, hd, splits_a, 4),
                        h4 = hd / 4,
                        fb = vd.div_ceil(2 * ATTN_FOLD_PAIRS),
                        fs = splits * 4
                    );
                    cases.push((format!("{name}, {rows} rows after {cached}, {splits} chunks"), rows, heads, vd));
                }
            }
        }
    }
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <string>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {PRELUDE}{}{}{}\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ fprintf(stderr, \"cuda %d %s\\n\", __LINE__, cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         static float* dev(const std::string& p){{ FILE* f = fopen(p.c_str(), \"rb\"); if(!f) exit(2); std::vector<char> b; char c[65536]; size_t n;\n\
           while((n = fread(c, 1, sizeof c, f)) > 0) b.insert(b.end(), c, c + n); fclose(f);\n\
           float* d; CK(cudaMalloc(&d, b.size())); CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice)); return d; }}\n\
         static float* nan(long n){{ float* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0xff, n * 4)); return p; }}\n\
         static int* zeros(long n){{ int* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0, n * 4)); return p; }}\n\
         static void put(FILE* out, const float* d, long n){{ std::vector<float> h(n); CK(cudaMemcpy(h.data(), d, n * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, n, out); }}\n\
         int main(int argc, char** argv){{\n  std::string dir = argv[1];\n  int* pos; CK(cudaMalloc(&pos, 4));\n\
           FILE* out = fopen(argv[2], \"wb\");\n{calls}  fclose(out);\n  return 0;\n}}\n",
        Kernel::Mma.source(),
        Kernel::Attention.source(),
        Kernel::AttentionShared.source()
    );
    let (cu, bin, outp) = (dir.join("shared.cu"), dir.join("shared"), dir.join("out.bin"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "both attentions build:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).arg(&dir).arg(&outp).output().unwrap();
    assert!(ran.status.success(), "they run: {}", String::from_utf8_lossy(&ran.stderr));
    let all: Vec<f32> = std::fs::read(&outp).unwrap().chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
    assert_eq!(all.len(), cases.iter().map(|&(_, r, h, vd)| 3 * r * h * vd).sum::<usize>(), "every case's outputs");
    let mut at = 0;
    let mut report = Vec::new();
    for (what, rows, heads, vd) in &cases {
        let n = rows * heads * vd;
        let (a, b, c) = (&all[at..at + n], &all[at + n..at + 2 * n], &all[at + 2 * n..at + 3 * n]);
        at += 3 * n;
        let parted = b.iter().zip(c).position(|(x, y)| x.to_bits() != y.to_bits());
        assert!(parted.is_none(), "{what}: the values in rows of their own part from the key rows' at {parted:?}: {:?}", parted.map(|i| (b[i], c[i])));
        let mut worst = 0f64;
        for v in 0..rows * heads {
            let (x, y) = (&a[v * vd..(v + 1) * vd], &b[v * vd..(v + 1) * vd]);
            assert!(x.iter().chain(y).all(|f| f.is_finite()), "{what}: row {} head {}: every output written, and finite", v / heads, v % heads);
            let largest = x.iter().fold(0f64, |m, f| m.max(f64::from(f.abs())));
            assert!(largest > 0.0, "{what}: row {} head {} attends something", v / heads, v % heads);
            for (e, (u, w)) in x.iter().zip(y).enumerate() {
                let off = f64::from((u - w).abs()) / largest;
                worst = worst.max(off);
                assert!(off <= TOLERANCE, "{what}: row {} head {} value {e}: the attention gives {u}, the shared rows {w} ({off:.2e} of the head's largest)", v / heads, v % heads);
            }
        }
        report.push(format!("{what}: within {worst:.2e}"));
    }
    eprintln!("{}", report.join("\n"));
}

/// The expanded latent attention computes what the absorbed one computes, on the same cache, queries and weights, at
/// DeepSeek-V2-Lite's latent widths over 4 heads: 40 rows after 70 cached positions. Absorbed, as a wide chunk computed
/// it before: each head's unrotated query, read in place in the query projection, through the key half (`lk_head_tiled`,
/// rows its inputs in one expansion, its outputs with the halves split), joined with its rotated part, attending every
/// cache row with the rows' latents as its values, and the attended latent through the value half. Expanded: each
/// cached latent row before the chunk's last position through both halves, every head reading the row's one latent,
/// then each head's query — its unrotated part in place, its rotated part apart — over its own keys — its expansion,
/// then the rotated key in place in the cache — and its own values. Every product reads its activation `f32` and sums
/// in `f32` (`Q` 3), with the halves in Q8_0, Q4_K, Q6_K and Q5_0 — a split key half, its rows 128 wide, in the formats
/// of 32-value blocks alone. Each output within `TOLERANCE` of its head's largest, the difference of two summation
/// orders, and every output written (they start as NaN).
#[test]
fn the_expanded_latent_attention_computes_what_the_absorbed_one_computes() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{attn_tile_heads, attn_tile_smem_bytes, attn_tile_values, Kernel, ATTN_TILE_Q, PRELUDE};
    use logicaffeine_compile::model::quant::GgmlType;
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    const TOLERANCE: f64 = 1e-4;
    const ROWS: usize = 40;
    const POS: usize = 71;
    const H: usize = 4;
    let (nope, rope, lora, vd) = (128usize, 64usize, 512usize, 128usize);
    let (qk, kw, len) = (nope + rope, lora + rope, POS - 1 + ROWS);
    let scale = 1.0 / (qk as f64).sqrt();
    let dir = dir().join("expanded");
    std::fs::create_dir_all(&dir).unwrap();
    let unit = |i: usize, salt: u64| {
        let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(23) ^ 0x2545_F491_4F6C_DD1D;
        ((h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32
    };
    let floats = |name: &str, v: Vec<f32>| std::fs::write(dir.join(name), v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    floats("kc.bin", (0..len * kw).map(|i| unit(i, 1)).collect());
    floats("qr.bin", (0..ROWS * H * qk).map(|i| unit(i, 2)).collect());
    floats("qe.bin", (0..ROWS * H * rope).map(|i| unit(i, 3)).collect());
    // A matrix of `rows` rows of `cols` in `ty`: a Q8_0 one as its levels then its scales, a block format's as its blocks.
    let matrix = |name: &str, ty: GgmlType, rows: usize, cols: usize| -> Vec<u8> {
        if ty == GgmlType::Q8_0 {
            let levels = (0..rows * cols).map(|i| ((unit(i, name.len() as u64 * 131) * 127.0) as i8) as u8);
            levels.chain((0..rows * cols / 32).flat_map(|_| 0x2000u16.to_le_bytes())).collect()
        } else {
            tiny_models::seeded_blocks(name, ty, rows * cols)
        }
    };
    let mut calls = String::new();
    let mut cases = Vec::new();
    for ty in [GgmlType::Q8_0, GgmlType::Q4K, GgmlType::Q6K, GgmlType::Q5_0] {
        let f = ty.id();
        std::fs::write(dir.join(format!("wv-{}.bin", ty.name())), matrix(&format!("wv{}", ty.name()), ty, H * vd, lora)).unwrap();
        // The key half in one expansion: each head's `nope` rows of the latent's width; split: each head's latent-wide
        // block of rows `nope` wide.
        for split in [false, true] {
            if split && ty.block_len() != 32 {
                continue;
            }
            let (rows, cols) = if split { (H * lora, nope) } else { (H * nope, lora) };
            let wk = format!("wk{}-{}", u8::from(split), ty.name());
            std::fs::write(dir.join(format!("{wk}.bin")), matrix(&wk, ty, rows, cols)).unwrap();
            // The absorption's rows are its outputs where the expansion's are its inputs, and the other way about.
            let (absorb_out, expand_out) = (split, !split);
            let (hg, vi, hl, vl) = (attn_tile_heads(H), attn_tile_values(lora), attn_tile_heads(1), attn_tile_values(vd));
            let _ = write!(
                calls,
                "  {{ // {wk}\n\
                 \x20   lk_w wk = load<{f}>(dir + \"/{wk}.bin\", {rows}L, {cols}, {}L), wv = load<{f}>(dir + \"/wv-{}.bin\", {}L, {lora}, {}L);\n\
                 \x20   float *ql = nan({ROWS}L * {H} * {lora}), *qa = nan({ROWS}L * {H} * {kw}), *ol = nan({ROWS}L * {H} * {lora}), *ca = nan({ROWS}L * {H} * {vd});\n\
                 \x20   lk_head_tiled<{f}, 3, {absorb_out}><<<dim3({}, 1, {H}), 256>>>(wk, nullptr, nullptr, nullptr, ql, {H}, {nope}, {lora}, {qk}, 0, nullptr, 0, {lora}, 0, {ROWS}, qr);\n\
                 \x20   lk_join<<<({ROWS} * {H} * {kw} + 255) / 256, 256>>>(ql, qe, qa, nullptr, 0, {ROWS}L * {H} * {kw}, {lora}, {rope});\n\
                 \x20   CK(cudaFuncSetAttribute(lk_attention_tiled<{hg}, {vi}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {}));\n\
                 \x20   lk_attention_tiled<{hg}, {vi}><<<dim3({}, {}), 256, {}>>>(qa, kc, kc, pos, nullptr, ol, {len}, 0, {kw}, {lora}, {H}, {H}, {kw}, {kw}, 0, {scale:e}f, {ROWS});\n\
                 \x20   lk_head_tiled<{f}, 3, true><<<dim3({}, 1, {H}), 256>>>(wv, nullptr, nullptr, nullptr, ca, {H}, {lora}, {vd}, {lora}, 0, nullptr, 0, {vd}, 0, {ROWS}, ol);\n\
                 \x20   float *k = nan({len}L * {H} * {nope}), *v = nan({len}L * {H} * {vd}), *cb = nan({ROWS}L * {H} * {vd});\n\
                 \x20   lk_head_tiled<{f}, 3, {expand_out}><<<dim3({}, {}, {H}), 256>>>(wk, nullptr, nullptr, nullptr, k, {H}, {lora}, {nope}, 0, 0, nullptr, 0, {nope}, 0, {len}, kc, {kw}, pos, {});\n\
                 \x20   lk_head_tiled<{f}, 3, true><<<dim3({}, {}, {H}), 256>>>(wv, nullptr, nullptr, nullptr, v, {H}, {lora}, {vd}, 0, 0, nullptr, 0, {vd}, 0, {len}, kc, {kw}, pos, {});\n\
                 \x20   CK(cudaFuncSetAttribute(lk_attention_tiled<{hl}, {vl}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {}));\n\
                 \x20   lk_attention_tiled<{hl}, {vl}><<<dim3({}, {}), 256, {}>>>(qr, k, v, pos, nullptr, cb, {len}, 0, {qk}, {vd}, 1, {H}, {}, {}, 0, {scale:e}f, {ROWS}, qe, {qk}, {rope}, kc, {kw}, {lora}, {nope});\n\
                 \x20   CK(cudaGetLastError()); CK(cudaDeviceSynchronize());\n\
                 \x20   put(out, ca, {ROWS}L * {H} * {vd}); put(out, cb, {ROWS}L * {H} * {vd});\n  }}\n",
                ty.bytes_for(cols),
                ty.name(),
                H * vd,
                ty.bytes_for(lora),
                lora.div_ceil(64),
                attn_tile_smem_bytes(lora),
                ROWS.div_ceil(ATTN_TILE_Q / hg),
                H / hg,
                attn_tile_smem_bytes(lora),
                vd.div_ceil(64),
                nope.div_ceil(64),
                len.div_ceil(64),
                ROWS - 1,
                vd.div_ceil(64),
                len.div_ceil(64),
                ROWS - 1,
                attn_tile_smem_bytes(vd),
                ROWS.div_ceil(ATTN_TILE_Q / hl),
                H / hl,
                attn_tile_smem_bytes(vd),
                H * nope,
                H * vd
            );
            cases.push(format!("{} key half {}", ty.name(), if split { "split" } else { "in one expansion" }));
        }
    }
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <string>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {PRELUDE}{}{}{}{}\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ fprintf(stderr, \"cuda %d %s\\n\", __LINE__, cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         static std::vector<char> slurp(const std::string& p){{ FILE* f = fopen(p.c_str(), \"rb\"); if(!f) exit(2); std::vector<char> b; char c[65536]; size_t n;\n\
           while((n = fread(c, 1, sizeof c, f)) > 0) b.insert(b.end(), c, c + n); fclose(f); return b; }}\n\
         static float* dev(const std::string& p){{ auto b = slurp(p); float* d; CK(cudaMalloc(&d, b.size())); CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice)); return d; }}\n\
         template<int F> static lk_w load(const std::string& p, long rows, int cols, long row_bytes){{\n\
           auto b = slurp(p);\n\
           if(F == 8){{ signed char* q; __half* s; long n = rows * cols; CK(cudaMalloc(&q, n)); CK(cudaMalloc(&s, n / 32 * 2));\n\
             CK(cudaMemcpy(q, b.data(), n, cudaMemcpyHostToDevice)); CK(cudaMemcpy(s, b.data() + n, n / 32 * 2, cudaMemcpyHostToDevice)); return lk_w{{q, s, nullptr, 0}}; }}\n\
           unsigned char* d; CK(cudaMalloc(&d, b.size() + 32)); CK(cudaMemset(d, 0, b.size() + 32)); CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice));\n\
           return lk_w{{nullptr, nullptr, d, row_bytes}};\n\
         }}\n\
         static float* nan(long n){{ float* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0xff, n * 4)); return p; }}\n\
         static void put(FILE* out, const float* d, long n){{ std::vector<float> h(n); CK(cudaMemcpy(h.data(), d, n * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, n, out); }}\n\
         int main(int argc, char** argv){{\n  std::string dir = argv[1];\n  int* pos; CK(cudaMalloc(&pos, 4)); int p0 = {POS}; CK(cudaMemcpy(pos, &p0, 4, cudaMemcpyHostToDevice));\n\
           float *kc = dev(dir + \"/kc.bin\"), *qr = dev(dir + \"/qr.bin\"), *qe = dev(dir + \"/qe.bin\");\n\
           FILE* out = fopen(argv[2], \"wb\");\n{calls}  fclose(out);\n  return 0;\n}}\n",
        Kernel::Blocks.source(),
        Kernel::Join.source(),
        Kernel::HeadTiled.source(),
        Kernel::AttentionTiled.source()
    );
    let (cu, bin, outp) = (dir.join("expanded.cu"), dir.join("expanded"), dir.join("out.bin"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "both latent attentions build:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).arg(&dir).arg(&outp).output().unwrap();
    assert!(ran.status.success(), "they run: {}", String::from_utf8_lossy(&ran.stderr));
    let all: Vec<f32> = std::fs::read(&outp).unwrap().chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
    assert_eq!(all.len(), cases.len() * 2 * ROWS * H * vd, "every case's outputs");
    let mut report = Vec::new();
    for (c, what) in cases.iter().enumerate() {
        let n = ROWS * H * vd;
        let (a, b) = (&all[2 * c * n..(2 * c + 1) * n], &all[(2 * c + 1) * n..(2 * c + 2) * n]);
        let mut worst = 0f64;
        for u in 0..ROWS * H {
            let (x, y) = (&a[u * vd..(u + 1) * vd], &b[u * vd..(u + 1) * vd]);
            assert!(x.iter().chain(y).all(|f| f.is_finite()), "{what}: row {} head {}: every output written, and finite", u / H, u % H);
            let largest = x.iter().fold(0f64, |m, f| m.max(f64::from(f.abs())));
            assert!(largest > 0.0, "{what}: row {} head {} attends something", u / H, u % H);
            for (e, (p, q)) in x.iter().zip(y).enumerate() {
                let off = f64::from((p - q).abs()) / largest;
                worst = worst.max(off);
                assert!(off <= TOLERANCE, "{what}: row {} head {} value {e}: absorbed {p}, expanded {q} ({off:.2e} of the head's largest)", u / H, u % H);
            }
        }
        report.push(format!("{what}: within {worst:.2e}"));
    }
    eprintln!("{}", report.join("\n"));
}

/// A dense block GEMM whose last wave splits its remaining tiles' `k` (`kernels::gemm_blk_tail`: the tiles past the
/// last whole wave of `workers` blocks, each cut into as many parts as fill the wave, the last part of a tile to arrive
/// summing the parts in order) computes what the GEMM without the split computes: Q8_0 (its split arrays, every block's
/// scale its own), Q4_K, Q6_K, Q5_0 and MXFP4 under both quantized numerics, over DeepSeek-V2-Lite's shapes at 512 rows — tiles in no whole wave (2816, 2048 and 576
/// outputs), in whole waves (3584, so nothing splits), fewer than a wave (64 outputs at 160 rows) — with and without a
/// bias, and splits forced to 2 and 3. Each output within `TOLERANCE` of its row's largest, the difference of two
/// summation orders; every output written (they start as NaN); a GEMM with nothing to split bit for bit the same.
#[test]
fn a_dense_gemms_last_wave_split_computes_what_the_whole_tiles_compute() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{gemm_blk_rows, gemm_blk_smem_bytes, gemm_blk_tail, gemm_blk_threads, Kernel, GEMM_TILE_N, PRELUDE};
    use logicaffeine_compile::model::quant::GgmlType;
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    const TOLERANCE: f64 = 1e-5;
    const WORKERS: usize = 28;
    let unit = |i: usize, salt: u64| {
        let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(19) ^ 0x2545_F491_4F6C_DD1D;
        ((h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32
    };
    let dir = dir().join("tail");
    std::fs::create_dir_all(&dir).unwrap();
    // (rows, outputs, inputs, a bias, a forced split)
    let shapes: [(usize, usize, usize, bool, Option<usize>); 7] = [
        (512, 2816, 2048, false, None),
        (512, 2048, 2048, true, None),
        (512, 576, 2048, false, None),
        (512, 3584, 2048, false, None),
        (160, 64, 512, true, None),
        (512, 2816, 2048, false, Some(2)),
        (512, 2048, 2048, false, Some(3)),
    ];
    let mut calls = String::new();
    let mut cases = Vec::new();
    for ty in [GgmlType::Q8_0, GgmlType::Q4K, GgmlType::Q6K, GgmlType::Q5_0, GgmlType::Mxfp4] {
        let f = ty.id();
        for (si, &(m, n, k, bias, forced)) in shapes.iter().enumerate() {
            let name = format!("w{si}-{}", ty.name());
            // A Q8_0 matrix as the lane uploads it: its levels, then a scale a block, every scale its own.
            let matrix = if ty == GgmlType::Q8_0 {
                let levels = (0..n * k).map(|i| ((unit(i, 90 + si as u64) * 127.0) as i8) as u8);
                levels.chain((0..n * k / 32).flat_map(|b| (0x2000u16 | ((b * 97) & 0x3FF) as u16).to_le_bytes())).collect()
            } else {
                tiny_models::seeded_blocks(&name, ty, n * k)
            };
            std::fs::write(dir.join(format!("{name}.bin")), matrix).unwrap();
            for q in [1, 2] {
                // The lane's tiles for the format and numerics.
                let bm = gemm_blk_rows(f, q == 2);
                let tiles = n.div_ceil(GEMM_TILE_N) * m.div_ceil(bm);
                let stages = k.div_ceil(128);
                let (full, split) = match forced {
                    Some(sp) => (tiles - tiles % WORKERS, sp),
                    None => gemm_blk_tail(tiles, WORKERS, stages),
                };
                let parts = (tiles - full) * split;
                let smem = gemm_blk_smem_bytes(f, q == 2, bm);
                let _ = write!(
                    calls,
                    "  {{ // {name} Q{q}\n\
                     \x20   lk_w w = {f} == 8 ? load8(dir + \"/{name}.bin\", {n}L * {k}) : load(dir + \"/{name}.bin\", {}L);\n\
                     \x20   float* x = dev(dir + \"/x{si}.bin\"); float* b = {};\n\
                     \x20   long nx = {m}L * {k}; signed char* xq = bytes(nx); signed char* xl = {q} == 2 ? bytes(nx) : nullptr; float* xs = nan(nx / 32);\n\
                     \x20   lk_quant<{q}><<<(nx + 255) / 256, 256>>>(x, xq, xl, xs, nx); CK(cudaGetLastError());\n\
                     \x20   float *ya = nan({m}L * {n}), *yb = nan({m}L * {n}), *part = nan({}L); int* cnt = zeros({});\n\
                     \x20   CK(cudaFuncSetAttribute(lk_gemm_blk<{f}, {q}, 0, {bm}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {smem}));\n\
                     \x20   lk_gemm_blk<{f}, {q}, 0, {bm}><<<dim3({}, {}), {}, {smem}>>>(w, xq, xl, xs, b, ya, nullptr, 0, {k}, {n}, {m}); CK(cudaGetLastError());\n\
                     \x20   lk_gemm_blk<{f}, {q}, 0, {bm}><<<{}, {}, {smem}>>>(w, xq, xl, xs, b, yb, nullptr, 0, {k}, {n}, {m}, nullptr, nullptr, nullptr, nullptr,\n\
                     \x20                                         {{}}, nullptr, nullptr, nullptr, nullptr, 0.f, 0.f, {full}, {split}, part, cnt); CK(cudaGetLastError());\n\
                     \x20   CK(cudaDeviceSynchronize()); put(out, ya, {m}L * {n}); put(out, yb, {m}L * {n});\n  }}\n",
                    if ty == GgmlType::Q8_0 { 0 } else { ty.bytes_for(k) },
                    if bias { format!("dev(dir + \"/b{si}.bin\")") } else { "nullptr".into() },
                    (parts * bm * GEMM_TILE_N).max(1),
                    (tiles - full).max(1),
                    n.div_ceil(GEMM_TILE_N),
                    m.div_ceil(bm),
                    gemm_blk_threads(bm),
                    full + parts,
                    gemm_blk_threads(bm)
                );
                cases.push((format!("{} {m}x{n}x{k}{} split {split} of {} tiles past {full} Q{q}", ty.name(), if bias { " biased" } else { "" }, tiles - full), m, n, parts == 0));
            }
        }
    }
    for (si, &(m, n, k, _, _)) in shapes.iter().enumerate() {
        let floats = |name: String, v: Vec<f32>| std::fs::write(dir.join(name), v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
        floats(format!("x{si}.bin"), (0..m * k).map(|i| unit(i, 1 + si as u64) * 2.0).collect());
        floats(format!("b{si}.bin"), (0..n).map(|i| unit(i, 50 + si as u64) * 0.1).collect());
    }
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <string>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {PRELUDE}{}{}{}{}\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ fprintf(stderr, \"cuda %d %s\\n\", __LINE__, cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         static std::vector<char> slurp(const std::string& p){{ FILE* f = fopen(p.c_str(), \"rb\"); if(!f) exit(2); std::vector<char> b; char c[65536]; size_t n;\n\
           while((n = fread(c, 1, sizeof c, f)) > 0) b.insert(b.end(), c, c + n); fclose(f); return b; }}\n\
         static float* dev(const std::string& p){{ auto b = slurp(p); float* d; CK(cudaMalloc(&d, b.size())); CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice)); return d; }}\n\
         static lk_w load(const std::string& p, long row_bytes){{ auto b = slurp(p); unsigned char* d; CK(cudaMalloc(&d, b.size() + 32)); CK(cudaMemset(d, 0, b.size() + 32));\n\
           CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice)); return lk_w{{nullptr, nullptr, d, row_bytes}}; }}\n\
         static lk_w load8(const std::string& p, long n){{ auto b = slurp(p); signed char* q; __half* s; CK(cudaMalloc(&q, n)); CK(cudaMalloc(&s, n / 16));\n\
           CK(cudaMemcpy(q, b.data(), n, cudaMemcpyHostToDevice)); CK(cudaMemcpy(s, b.data() + n, n / 16, cudaMemcpyHostToDevice)); return lk_w{{q, s, nullptr, 0}}; }}\n\
         static float* nan(long n){{ float* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0xff, n * 4)); return p; }}\n\
         static int* zeros(long n){{ int* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0, n * 4)); return p; }}\n\
         static signed char* bytes(long n){{ signed char* p; CK(cudaMalloc(&p, n)); return p; }}\n\
         static void put(FILE* out, const float* d, long n){{ std::vector<float> h(n); CK(cudaMemcpy(h.data(), d, n * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, n, out); }}\n\
         int main(int argc, char** argv){{\n  std::string dir = argv[1];\n  FILE* out = fopen(argv[2], \"wb\");\n{calls}  fclose(out);\n  return 0;\n}}\n",
        Kernel::Blocks.source(),
        Kernel::Quant.source(),
        Kernel::Mma.source(),
        Kernel::GemmBlk.source()
    );
    let (cu, bin, outp) = (dir.join("tail.cu"), dir.join("tail"), dir.join("out.bin"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "both GEMMs build:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).arg(&dir).arg(&outp).output().unwrap();
    assert!(ran.status.success(), "they run: {}", String::from_utf8_lossy(&ran.stderr));
    let all: Vec<f32> = std::fs::read(&outp).unwrap().chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
    assert_eq!(all.len(), cases.iter().map(|&(_, m, n, _)| 2 * m * n).sum::<usize>(), "every case's outputs");
    let mut at = 0;
    let mut report = Vec::new();
    for (what, m, n, whole) in &cases {
        let (a, b) = (&all[at..at + m * n], &all[at + m * n..at + 2 * m * n]);
        at += 2 * m * n;
        assert!(a.iter().chain(b).all(|v| v.is_finite()), "{what}: every output written, and finite");
        if *whole {
            assert!(a.iter().zip(b).all(|(u, v)| u.to_bits() == v.to_bits()), "{what}: nothing splits, so nothing changes");
        }
        let mut worst = 0f64;
        for r in 0..*m {
            let (x, y) = (&a[r * n..(r + 1) * n], &b[r * n..(r + 1) * n]);
            let largest = x.iter().fold(0f64, |acc, v| acc.max(f64::from(v.abs())));
            for (o, (u, v)) in x.iter().zip(y).enumerate() {
                let off = f64::from((u - v).abs()) / largest;
                worst = worst.max(off);
                assert!(off <= TOLERANCE, "{what}: row {r} output {o}: whole {u}, split {v} ({off:.2e} of the row's largest)");
            }
        }
        report.push(format!("{what}: within {worst:.2e}"));
    }
    eprintln!("{}", report.join("\n"));
}

/// A GEMM in parts (`lk_gemm_blk`'s `G` 4: one activation by up to `GEMM_PARTS` matrices side by side, each part's
/// outputs into its own buffer, or into a table's rows from the position its row names, with its own bias) computes
/// each part's outputs bit for bit as that part's GEMM alone does when neither cuts tiles along `k` — in Q8_0 (its split
/// arrays), Q4_K and Q6_K (halves scaled apart), under both quantized numerics, over Qwen3.5's four projections of a
/// 512-row chunk (2048 each over 1024), a Llama-like query, key and value (2048, 512, 512 over 2048; the value into a
/// cache's rows) of 100 rows (a partial row tile), and the narrowest wide chunk (17 rows) of parts 128 (into a table)
/// and 256 wide — with and without biases, a table's rows beside a part's left as they were. Its last wave cut along `k`
/// as the lane cuts it on this chip (or
/// cut in two where the tiles fill whole waves), each output within `TOLERANCE` of its row's largest; the cut GEMM sums
/// its parts in one order whichever arrives last, so a second run computes the same bits and leaves its counts at zero.
#[test]
fn a_gemm_in_parts_computes_each_parts_gemm() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{gemm_blk_rows, gemm_blk_smem_bytes, gemm_blk_tail, gemm_blk_threads, Kernel, GEMM_PARTS, GEMM_TILE_N, PRELUDE};
    use logicaffeine_compile::model::quant::GgmlType;
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    const TOLERANCE: f64 = 1e-5;
    let workers = CudaKnobs::default().chip.sms;
    let unit = |i: usize, salt: u64| {
        let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(23) ^ 0x2545_F491_4F6C_DD1D;
        ((h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32
    };
    let dir = dir().join("parts");
    std::fs::create_dir_all(&dir).unwrap();
    // (rows, inputs, the parts' (outputs, a bias, into the rows of a table from row `AT`))
    const AT: usize = 37;
    let sets: [(usize, usize, &[(usize, bool, bool)]); 3] = [
        (512, 1024, &[(2048, false, false), (2048, true, false), (2048, false, false), (2048, false, false)]),
        (100, 2048, &[(2048, true, false), (512, false, false), (512, true, true)]),
        (17, 512, &[(128, false, true), (256, true, false)]),
    ];
    let floats = |name: String, v: Vec<f32>| std::fs::write(dir.join(name), v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    let mut calls = String::new();
    let mut cases = Vec::new();
    for (si, &(m, k, parts)) in sets.iter().enumerate() {
        assert!(parts.len() <= GEMM_PARTS);
        floats(format!("x{si}.bin"), (0..m * k).map(|i| unit(i, 1 + si as u64) * 2.0).collect());
        for (p, &(n, _, _)) in parts.iter().enumerate() {
            floats(format!("b{si}-{p}.bin"), (0..n).map(|i| unit(i, 50 + (10 * si + p) as u64) * 0.1).collect());
        }
        let cols: usize = parts.iter().map(|&(n, _, _)| n).sum();
        for ty in [GgmlType::Q8_0, GgmlType::Q4K, GgmlType::Q6K] {
            let f = ty.id();
            for (p, &(n, _, _)) in parts.iter().enumerate() {
                let name = format!("w{si}-{p}-{}", ty.name());
                let matrix = if ty == GgmlType::Q8_0 {
                    let levels = (0..n * k).map(|i| ((unit(i, 90 + (10 * si + p) as u64) * 127.0) as i8) as u8);
                    levels.chain((0..n * k / 32).flat_map(|b| (0x2000u16 | ((b * 97) & 0x3FF) as u16).to_le_bytes())).collect()
                } else {
                    tiny_models::seeded_blocks(&name, ty, n * k)
                };
                std::fs::write(dir.join(format!("{name}.bin")), matrix).unwrap();
            }
            for q in [1, 2] {
                // The lane's tiles for the format and numerics.
                let bm = gemm_blk_rows(f, q == 2);
                let tiles = cols.div_ceil(GEMM_TILE_N) * m.div_ceil(bm);
                let (full, split) = match gemm_blk_tail(tiles, workers, k.div_ceil(128)) {
                    (_, 1) => (tiles - tiles.min(workers).min(2), 2),
                    cut => cut,
                };
                let cut = (tiles - full) * split;
                let smem = gemm_blk_smem_bytes(f, q == 2, bm);
                let threads = gemm_blk_threads(bm);
                let mut alone = String::new();
                let (mut ws, mut ys, mut bs, mut ends, mut zs, mut cs) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
                let (mut rows_at, mut pitches) = (Vec::new(), Vec::new());
                let mut end = 0;
                for (p, &(n, bias, rowed)) in parts.iter().enumerate() {
                    end += n;
                    // A part into a table: its rows from row `AT` (the one-based position `AT + 1`), a row on each side
                    // of them left as it was.
                    let tables = if rowed {
                        format!(
                            "    float *tb{p} = nan({0}L * {n}), *tc{p} = nan({0}L * {n}), *td{p} = nan({0}L * {n}); int* at{p} = rowat({1});\n\
                             \x20   float *yb{p} = tb{p} + {AT}L * {n}, *yc{p} = tc{p} + {AT}L * {n}, *yd{p} = td{p} + {AT}L * {n};\n",
                            AT + m + 1,
                            AT + 1
                        )
                    } else {
                        format!("    float *yb{p} = nan({m}L * {n}), *yc{p} = nan({m}L * {n}), *yd{p} = nan({m}L * {n});\n")
                    };
                    let _ = write!(
                        alone,
                        "    lk_w w{p} = {f} == 8 ? load8(dir + \"/w{si}-{p}-{}.bin\", {n}L * {k}) : load(dir + \"/w{si}-{p}-{}.bin\", {}L); float* b{p} = {};\n\
                         \x20   float *ya{p} = nan({m}L * {n});\n{tables}\
                         \x20   lk_gemm_blk<{f}, {q}, 0, {bm}><<<dim3({}, {}), {threads}, {smem}>>>(w{p}, xq, xl, xs, b{p}, ya{p}, nullptr, 0, {k}, {n}, {m}); CK(cudaGetLastError());\n",
                        ty.name(),
                        ty.name(),
                        if ty == GgmlType::Q8_0 { 0 } else { ty.bytes_for(k) },
                        if bias { format!("dev(dir + \"/b{si}-{p}.bin\")") } else { "nullptr".into() },
                        n.div_ceil(GEMM_TILE_N),
                        m.div_ceil(bm)
                    );
                    ws.push(format!("w{p}"));
                    ys.push(format!("{}{p}", if rowed { "tb" } else { "yb" }));
                    zs.push(format!("{}{p}", if rowed { "tc" } else { "yc" }));
                    cs.push(format!("{}{p}", if rowed { "td" } else { "yd" }));
                    bs.push(format!("b{p}"));
                    ends.push(end.to_string());
                    rows_at.push(if rowed { format!("at{p}") } else { "nullptr".into() });
                    pitches.push(if rowed { n.to_string() } else { "0".into() });
                }
                let pad = |v: &[String], with: &str| (0..GEMM_PARTS).map(|i| v.get(i).cloned().unwrap_or_else(|| with.to_string())).collect::<Vec<_>>().join(", ");
                let (w4, b4, e4) = (pad(&ws, "lk_w{}"), pad(&bs, "nullptr"), pad(&ends, &cols.to_string()));
                let (r4, p4) = (pad(&rows_at, "nullptr"), pad(&pitches, "0"));
                let merged = |grid: String, ys: &[String], line: String| {
                    format!(
                        "    lk_gemm_blk<{f}, {q}, 4, {bm}><<<{grid}, {threads}, {smem}>>>(lk_w{{}}, xq, xl, xs, nullptr, nullptr, nullptr, 0, {k}, {cols}, {m}, nullptr, nullptr, nullptr, nullptr,\n\
                         \x20     {{}}, nullptr, nullptr, nullptr, nullptr, 0.f, 0.f, {line}, lk_parts{{{{{w4}}}, {{{}}}, {{{b4}}}, {{{e4}}}, {{{r4}}}, {{{p4}}}}}); CK(cudaGetLastError());\n",
                        pad(ys, "nullptr")
                    )
                };
                let _ = write!(
                    calls,
                    "  {{ // set {si} {} Q{q}\n\
                     \x20   float* x = dev(dir + \"/x{si}.bin\"); long nx = {m}L * {k}; signed char* xq = bytes(nx); signed char* xl = {q} == 2 ? bytes(nx) : nullptr; float* xs = nan(nx / 32);\n\
                     \x20   lk_quant<{q}><<<(nx + 255) / 256, 256>>>(x, xq, xl, xs, nx); CK(cudaGetLastError());\n\
                     \x20   CK(cudaFuncSetAttribute(lk_gemm_blk<{f}, {q}, 0, {bm}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {smem}));\n\
                     \x20   CK(cudaFuncSetAttribute(lk_gemm_blk<{f}, {q}, 4, {bm}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {smem}));\n\
                     {alone}\
                     \x20   float* part = nan({}L); int* cnt = zeros({});\n\
                     {}{}{}\
                     \x20   CK(cudaDeviceSynchronize());\n",
                    ty.name(),
                    (cut * bm * GEMM_TILE_N).max(1),
                    (tiles - full).max(1),
                    merged(format!("dim3({}, {})", cols.div_ceil(GEMM_TILE_N), m.div_ceil(bm)), &ys, "0, 1, nullptr, nullptr".into()),
                    merged(format!("{}", full + cut), &zs, format!("{full}, {split}, part, cnt")),
                    merged(format!("{}", full + cut), &cs, format!("{full}, {split}, part, cnt"))
                );
                for (p, &(n, _, rowed)) in parts.iter().enumerate() {
                    let _ = writeln!(calls, "    put(out, ya{p}, {m}L * {n}); put(out, yb{p}, {m}L * {n}); put(out, yc{p}, {m}L * {n}); put(out, yd{p}, {m}L * {n});");
                    if rowed {
                        for tb in ["tb", "tc", "td"] {
                            let _ = writeln!(calls, "    put(out, {tb}{p} + {}L * {n}, {n}); put(out, {tb}{p} + {}L * {n}, {n});", AT - 1, AT + m);
                        }
                    }
                }
                let _ = writeln!(calls, "    std::vector<int> h({0}); CK(cudaMemcpy(h.data(), cnt, {0} * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, {0}, out);\n  }}", (tiles - full).max(1));
                cases.push((format!("{} Q{q}, {m} rows over {k}, parts {:?}, {} tiles past {full} cut in {split}", ty.name(), parts.iter().map(|p| (p.0, p.2)).collect::<Vec<_>>(), tiles - full), m, parts, (tiles - full).max(1)));
            }
        }
    }
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <string>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {PRELUDE}{}{}{}{}\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ fprintf(stderr, \"cuda %d %s\\n\", __LINE__, cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         static std::vector<char> slurp(const std::string& p){{ FILE* f = fopen(p.c_str(), \"rb\"); if(!f) exit(2); std::vector<char> b; char c[65536]; size_t n;\n\
           while((n = fread(c, 1, sizeof c, f)) > 0) b.insert(b.end(), c, c + n); fclose(f); return b; }}\n\
         static float* dev(const std::string& p){{ auto b = slurp(p); float* d; CK(cudaMalloc(&d, b.size())); CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice)); return d; }}\n\
         static lk_w load(const std::string& p, long row_bytes){{ auto b = slurp(p); unsigned char* d; CK(cudaMalloc(&d, b.size() + 32)); CK(cudaMemset(d, 0, b.size() + 32));\n\
           CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice)); return lk_w{{nullptr, nullptr, d, row_bytes}}; }}\n\
         static lk_w load8(const std::string& p, long n){{ auto b = slurp(p); signed char* q; __half* s; CK(cudaMalloc(&q, n)); CK(cudaMalloc(&s, n / 16));\n\
           CK(cudaMemcpy(q, b.data(), n, cudaMemcpyHostToDevice)); CK(cudaMemcpy(s, b.data() + n, n / 16, cudaMemcpyHostToDevice)); return lk_w{{q, s, nullptr, 0}}; }}\n\
         static float* nan(long n){{ float* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0xff, n * 4)); return p; }}\n\
         static int* zeros(long n){{ int* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0, n * 4)); return p; }}\n\
         static int* rowat(int at){{ int* p; CK(cudaMalloc(&p, 4)); CK(cudaMemcpy(p, &at, 4, cudaMemcpyHostToDevice)); return p; }}\n\
         static signed char* bytes(long n){{ signed char* p; CK(cudaMalloc(&p, n)); return p; }}\n\
         static void put(FILE* out, const float* d, long n){{ std::vector<float> h(n); CK(cudaMemcpy(h.data(), d, n * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, n, out); }}\n\
         int main(int argc, char** argv){{\n  std::string dir = argv[1];\n  FILE* out = fopen(argv[2], \"wb\");\n{calls}  fclose(out);\n  return 0;\n}}\n",
        Kernel::Blocks.source(),
        Kernel::Quant.source(),
        Kernel::Mma.source(),
        Kernel::GemmBlk.source()
    );
    let (cu, bin, outp) = (dir.join("parts.cu"), dir.join("parts"), dir.join("out.bin"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "the GEMMs build:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).arg(&dir).arg(&outp).output().unwrap();
    assert!(ran.status.success(), "they run: {}", String::from_utf8_lossy(&ran.stderr));
    let all: Vec<u32> = std::fs::read(&outp).unwrap().chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    let mut at = 0;
    let mut take = |n: usize| {
        at += n;
        all[at - n..at].to_vec()
    };
    let mut report = Vec::new();
    for (what, m, parts, counts) in &cases {
        let mut worst = 0f64;
        for (p, &(n, _, rowed)) in parts.iter().enumerate() {
            let (alone, merged, cut, again) = (take(m * n), take(m * n), take(m * n), take(m * n));
            if rowed {
                let untouched = take(6 * n);
                assert!(untouched.iter().all(|&b| b == u32::MAX), "{what}: part {p}: the table's rows beside the part's left as they were");
            }
            assert!(alone.iter().chain(&merged).chain(&cut).all(|b| f32::from_bits(*b).is_finite()), "{what}: part {p}: every output written, and finite");
            let first = alone.iter().zip(&merged).position(|(a, b)| a != b);
            assert_eq!(first, None, "{what}: part {p}: the GEMM in parts computes the part's GEMM bit for bit, apart first at {first:?}");
            assert_eq!(again, cut, "{what}: part {p}: the cut GEMM sums its parts in one order whichever arrives last");
            for r in 0..*m {
                let (x, y) = (&alone[r * n..(r + 1) * n], &cut[r * n..(r + 1) * n]);
                let largest = x.iter().fold(0f64, |acc, v| acc.max(f64::from(f32::from_bits(*v).abs())));
                for (o, (u, v)) in x.iter().zip(y).enumerate() {
                    let off = f64::from((f32::from_bits(*u) - f32::from_bits(*v)).abs()) / largest;
                    worst = worst.max(off);
                    assert!(off <= TOLERANCE, "{what}: part {p} row {r} output {o}: alone {}, cut {} ({off:.2e} of the row's largest)", f32::from_bits(*u), f32::from_bits(*v));
                }
            }
        }
        assert!(take(*counts).iter().all(|&c| c == 0), "{what}: the counts of parts arrived left at zero");
        report.push(format!("{what}: cut within {worst:.2e}"));
    }
    assert_eq!(at, all.len(), "every result read");
    eprintln!("{}", report.join("\n"));
}

/// The Q8_0 block GEMM (`lk_gemm_blk<8, Q>`, in the lane's tiles: `gemm_blk_rows`) sums each output exactly as its
/// replica on the CPU does: each 32-value block's dot of int8 levels exact, accumulated `fma(row scale · column scale,
/// dot, sum)` block after block in `k`'s order — under two words the block's high and low dots folded into one integer,
/// `fma(row scale / 128 · column scale, 128 · dot + low dot, sum)` — a tile cut along `k` summing each part from zero over
/// its stages and the parts in order, the bias added last. Bit for bit, under both quantized numerics, whole tiles and the
/// last wave cut as the lane cuts it on this chip, over a `k` of whole 128-value stages (the streamed path) and one
/// ending inside a stage (1056), rows and outputs past a tile's edge, with and without a bias, scales varied block to
/// block.
#[test]
fn the_q8_0_block_gemm_sums_each_output_as_its_replica_does() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{gemm_blk_rows, gemm_blk_smem_bytes, gemm_blk_tail, gemm_blk_threads, Kernel, GEMM_TILE_N, PRELUDE};
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let workers = CudaKnobs::default().chip.sms;
    let unit = |i: usize, salt: u64| {
        let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(31) ^ 0x2545_F491_4F6C_DD1D;
        ((h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32
    };
    let dir = dir().join("q8-replica");
    std::fs::create_dir_all(&dir).unwrap();
    // (rows, outputs, inputs, a bias)
    let shapes = [(100usize, 384usize, 1024usize, true), (512, 2048, 1024, false), (40, 256, 1056, true), (512, 640, 3584, false)];
    let mut calls = String::new();
    let mut cases = Vec::new();
    for (si, &(m, n, k, biased)) in shapes.iter().enumerate() {
        let levels: Vec<i8> = (0..n * k).map(|i| (unit(i, 10 + si as u64) * 127.0) as i8).collect();
        // Normal f16 scales of 2⁻¹⁰ to 2⁻⁶, exponent and mantissa varied block to block.
        let scales: Vec<u16> = (0..n * k / 32).map(|b| (((5 + b % 4) << 10) | (((unit(b, 20 + si as u64) + 1.0) * 511.0) as usize & 0x3FF)) as u16).collect();
        let mut matrix: Vec<u8> = levels.iter().map(|&l| l as u8).collect();
        matrix.extend(scales.iter().flat_map(|s| s.to_le_bytes()));
        std::fs::write(dir.join(format!("w{si}.bin")), &matrix).unwrap();
        let floats = |name: String, v: Vec<f32>| std::fs::write(dir.join(name), v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
        floats(format!("x{si}.bin"), (0..m * k).map(|i| unit(i, 30 + si as u64) * (1.0 + 3.0 * ((i / 32) % 5) as f32)).collect());
        floats(format!("b{si}.bin"), (0..n).map(|i| unit(i, 40 + si as u64) * 0.2).collect());
        for q in [1, 2] {
            // The lane's tiles under the numerics.
            let bm = gemm_blk_rows(8, q == 2);
            let tiles = n.div_ceil(GEMM_TILE_N) * m.div_ceil(bm);
            let (whole, split) = gemm_blk_tail(tiles, workers, k.div_ceil(128));
            let smem = gemm_blk_smem_bytes(8, q == 2, bm);
            let threads = gemm_blk_threads(bm);
            let bias = if biased { "b" } else { "nullptr" };
            let cut = if split > 1 {
                format!(
                    "lk_gemm_blk<8, {q}, 0, {bm}><<<{}, {threads}, {smem}>>>(w, xq, xl, xs, {bias}, yc, nullptr, 0, {k}, {n}, {m}, nullptr, nullptr, nullptr, nullptr, {{}}, nullptr, nullptr, nullptr, nullptr, 0.f, 0.f, {whole}, {split}, part, cnt); CK(cudaGetLastError());",
                    whole + (tiles - whole) * split
                )
            } else {
                format!("lk_gemm_blk<8, {q}, 0, {bm}><<<dim3({}, {}), {threads}, {smem}>>>(w, xq, xl, xs, {bias}, yc, nullptr, 0, {k}, {n}, {m}); CK(cudaGetLastError());", n.div_ceil(GEMM_TILE_N), m.div_ceil(bm))
            };
            let _ = write!(
                calls,
                "  {{ // shape {si} Q{q}\n\
                 \x20   lk_w w = load8(dir + \"/w{si}.bin\", {n}L * {k}); float* x = dev(dir + \"/x{si}.bin\"); float* b = dev(dir + \"/b{si}.bin\");\n\
                 \x20   long nx = {m}L * {k}; signed char* xq = bytes(nx); signed char* xl = {q} == 2 ? bytes(nx) : nullptr; float* xs = nan(nx / 32);\n\
                 \x20   lk_quant<{q}><<<(nx + 255) / 256, 256>>>(x, xq, xl, xs, nx); CK(cudaGetLastError());\n\
                 \x20   CK(cudaFuncSetAttribute(lk_gemm_blk<8, {q}, 0, {bm}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {smem}));\n\
                 \x20   float *yw = nan({m}L * {n}), *yc = nan({m}L * {n}), *part = nan({}L); int* cnt = zeros({});\n\
                 \x20   lk_gemm_blk<8, {q}, 0, {bm}><<<dim3({}, {}), {threads}, {smem}>>>(w, xq, xl, xs, {bias}, yw, nullptr, 0, {k}, {n}, {m}); CK(cudaGetLastError());\n\
                 \x20   {cut}\n\
                 \x20   CK(cudaDeviceSynchronize()); putb(out, xq, nx); if({q} == 2) putb(out, xl, nx); put(out, xs, nx / 32); put(out, yw, {m}L * {n}); put(out, yc, {m}L * {n});\n  }}\n",
                ((tiles - whole) * split * bm * GEMM_TILE_N).max(1),
                (tiles - whole).max(1),
                n.div_ceil(GEMM_TILE_N),
                m.div_ceil(bm)
            );
            cases.push((si, q, m, n, k, biased, bm, whole, split, levels.clone(), scales.clone()));
        }
    }
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <string>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {PRELUDE}{}{}{}{}\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ fprintf(stderr, \"cuda %d %s\\n\", __LINE__, cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         static std::vector<char> slurp(const std::string& p){{ FILE* f = fopen(p.c_str(), \"rb\"); if(!f) exit(2); std::vector<char> b; char c[65536]; size_t n;\n\
           while((n = fread(c, 1, sizeof c, f)) > 0) b.insert(b.end(), c, c + n); fclose(f); return b; }}\n\
         static float* dev(const std::string& p){{ auto b = slurp(p); float* d; CK(cudaMalloc(&d, b.size())); CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice)); return d; }}\n\
         static lk_w load8(const std::string& p, long n){{ auto b = slurp(p); signed char* q; __half* s; CK(cudaMalloc(&q, n)); CK(cudaMalloc(&s, n / 16));\n\
           CK(cudaMemcpy(q, b.data(), n, cudaMemcpyHostToDevice)); CK(cudaMemcpy(s, b.data() + n, n / 16, cudaMemcpyHostToDevice)); return lk_w{{q, s, nullptr, 0}}; }}\n\
         static float* nan(long n){{ float* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0xff, n * 4)); return p; }}\n\
         static int* zeros(long n){{ int* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0, n * 4)); return p; }}\n\
         static signed char* bytes(long n){{ signed char* p; CK(cudaMalloc(&p, n)); return p; }}\n\
         static void put(FILE* out, const float* d, long n){{ std::vector<float> h(n); CK(cudaMemcpy(h.data(), d, n * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, n, out); }}\n\
         static void putb(FILE* out, const signed char* d, long n){{ std::vector<signed char> h(n); CK(cudaMemcpy(h.data(), d, n, cudaMemcpyDeviceToHost)); fwrite(h.data(), 1, n, out); }}\n\
         int main(int argc, char** argv){{\n  std::string dir = argv[1];\n  FILE* out = fopen(argv[2], \"wb\");\n{calls}  fclose(out);\n  return 0;\n}}\n",
        Kernel::Blocks.source(),
        Kernel::Quant.source(),
        Kernel::Mma.source(),
        Kernel::GemmBlk.source()
    );
    let (cu, bin, outp) = (dir.join("replica.cu"), dir.join("replica"), dir.join("out.bin"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "the GEMM builds:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).arg(&dir).arg(&outp).output().unwrap();
    assert!(ran.status.success(), "it runs: {}", String::from_utf8_lossy(&ran.stderr));
    let all = std::fs::read(&outp).unwrap();
    let mut at = 0;
    let mut take = |n: usize| {
        at += n;
        all[at - n..at].to_vec()
    };
    let f32s = |v: Vec<u8>| v.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect::<Vec<f32>>();
    for (si, q, m, n, k, biased, bm, whole, split, levels, scales) in cases {
        let what = format!("{m} × {k} by {n} Q{q}{}", if biased { " with a bias" } else { "" });
        let xq: Vec<i8> = take(m * k).into_iter().map(|b| b as i8).collect();
        let xl: Vec<i8> = if q == 2 { take(m * k).into_iter().map(|b| b as i8).collect() } else { Vec::new() };
        let xs = f32s(take(m * k / 32 * 4));
        let (yw, yc) = (f32s(take(m * n * 4)), f32s(take(m * n * 4)));
        let bias: Vec<f32> = std::fs::read(dir.join(format!("b{si}.bin"))).unwrap().chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
        let nb = k / 32;
        let stages = k.div_ceil(128);
        let nbx = n.div_ceil(GEMM_TILE_N);
        let scale = |h: u16| f32::from_bits(((h as u32 & 0x8000) << 16) | ((((h as u32 >> 10) & 0x1F) + 112) << 23) | ((h as u32 & 0x3FF) << 13));
        for r in 0..m {
            for c in 0..n {
                // Each block's dot, the low words' folded in under two words, and its scale.
                let blocks: Vec<(i32, f32)> = (0..nb)
                    .map(|j| {
                        let dot = |a: &[i8]| (0..32).map(|i| a[r * k + 32 * j + i] as i32 * levels[c * k + 32 * j + i] as i32).sum::<i32>();
                        let (s0, d) = (xs[r * nb + j], scale(scales[c * nb + j]));
                        if q == 2 { (dot(&xq) * 128 + dot(&xl), s0 * 0.0078125 * d) } else { (dot(&xq), s0 * d) }
                    })
                    .collect();
                // The sum over blocks `[lo, hi)`, from zero.
                let run = |lo: usize, hi: usize| blocks[lo..hi.min(nb)].iter().fold(0f32, |acc, &(dot, sd)| sd.mul_add(dot as f32, acc));
                let tail = |v: f32| if biased { v + bias[c] } else { v };
                let plain = tail(run(0, nb));
                assert_eq!(yw[r * n + c].to_bits(), plain.to_bits(), "{what}: whole tiles, row {r} output {c}: {} against the replica's {plain}", yw[r * n + c]);
                let tile = (r / bm) * nbx + c / GEMM_TILE_N;
                let cut = if split > 1 && tile >= whole {
                    (0..split).fold(0f32, |acc, piece| acc + run(piece * stages / split * 4, (piece + 1) * stages / split * 4))
                } else {
                    run(0, nb)
                };
                let cut = tail(cut);
                assert_eq!(yc[r * n + c].to_bits(), cut.to_bits(), "{what}: the lane's cut (split {split} past {whole}), row {r} output {c}: {} against the replica's {cut}", yc[r * n + c]);
            }
        }
    }
    assert_eq!(at, all.len(), "every result read");
}

/// The block GEMM's streamed Q8_0 path (`lk_gemm_blk<8>` over a `k` of whole 128-value scale groups) computes, bit for
/// bit, what its block path computes of the same matrix and activation with a zero 32-value block appended to every row
/// (`k + 32`, no whole number of groups: the block path, whose sums
/// `the_q8_0_block_gemm_sums_each_output_as_its_replica_does` holds to their replica; a zero block adds an exact zero to
/// every sum, and no sum from zero is ever −0) — launched as the lane launches it (`gemm_blk_launch_threads`: over 128
/// rows, eight warps of 64) and, over 128 rows, as a warp each 32 × 32 too — in every variant the lane launches it in: dense, into rows of a table, in
/// parts, paired through the SwiGLU and through GELU into `f32` and quantized outputs, and grouped by expert — gate and
/// up apart, down weighed by each choice, gate and up paired through the clamped activation and quantized, in tiles of
/// 64 and of 128 rows — under both quantized numerics, over rows and outputs past a tile's edge, scales varied block to
/// block. Its last wave cut along `k` sums each cut tile's parts in order as their replica does — the tiles past the
/// first `whole` of the tiles row by row cut, as the block path cuts them, the line taking its whole tiles a column tile
/// at a time — and leaves its counts of parts at zero.
#[test]
fn the_streamed_q8_0_path_computes_what_the_block_path_computes() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{
        gemm_blk_launch_smem, gemm_blk_launch_threads, gemm_blk_rows, gemm_blk_smem_bytes, gemm_blk_tail, gemm_blk_threads, Kernel, GEMM_TILE_N, PRELUDE,
    };
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let workers = CudaKnobs::default().chip.sms;
    let unit = |i: usize, salt: u64| {
        let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(27) ^ 0x2545_F491_4F6C_DD1D;
        ((h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32
    };
    let dir = dir().join("q8-stream");
    std::fs::create_dir_all(&dir).unwrap();
    let floats = |name: &str, v: Vec<f32>| std::fs::write(dir.join(name), v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    let ints = |name: &str, v: Vec<i32>| std::fs::write(dir.join(name), v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    // An activation of `rows` rows of `cols`, as it is (`<name>.bin`) and with 32 zeros appended to every row
    // (`<name>+.bin`).
    let acts = |name: &str, rows: usize, cols: usize, v: Vec<f32>| {
        floats(&format!("{name}+.bin"), (0..rows).flat_map(|r| v[r * cols..(r + 1) * cols].iter().copied().chain([0.0; 32])).collect());
        floats(&format!("{name}.bin"), v);
    };
    // A Q8_0 matrix of `rows` rows of `cols`: its int8 levels, then normal f16 scales of 2⁻¹⁰ to 2⁻⁶, exponent and
    // mantissa varied block to block — as it is, and with a zero block (levels and scale) appended to every row. Kept
    // for the replica.
    let mut matrices: std::collections::BTreeMap<String, (Vec<i8>, Vec<u16>)> = std::collections::BTreeMap::new();
    let mut matrix = |name: &str, rows: usize, cols: usize, salt: u64| {
        let levels: Vec<i8> = (0..rows * cols).map(|i| (unit(i, salt) * 127.0) as i8).collect();
        let scales: Vec<u16> = (0..rows * cols / 32).map(|b| (((5 + b % 4) << 10) | (((unit(b, salt + 1) + 1.0) * 511.0) as usize & 0x3FF)) as u16).collect();
        for (file, pad) in [(format!("{name}.bin"), 0), (format!("{name}+.bin"), 32)] {
            let mut bytes: Vec<u8> = (0..rows).flat_map(|r| levels[r * cols..(r + 1) * cols].iter().map(|&l| l as u8).chain(std::iter::repeat(0).take(pad))).collect();
            bytes.extend((0..rows).flat_map(|r| scales[r * cols / 32..(r + 1) * cols / 32].iter().copied().chain(std::iter::repeat(0).take(pad / 32))).flat_map(u16::to_le_bytes));
            std::fs::write(dir.join(file), bytes).unwrap();
        }
        matrices.insert(name.to_string(), (levels, scales));
    };
    // A variant's kernel — one for both paths — and the shared memory it is allowed.
    let kernel = |q: usize, g: usize, bm: usize, act: i32| (format!("lk_gemm_blk<8, {q}, {g}, {bm}, {act}>"), gemm_blk_smem_bytes(8, q == 2, bm));
    // A variant's runs, (k, the files' suffix, threads, shared memory): the block path over the padded matrix and
    // activation (`k + 32`, the files' `+`), then the streamed path over `k` as the lane launches it, and as a warp each
    // 32 × 32 where the lane launches it otherwise.
    let paths = |k: usize, q: usize, bm: usize| {
        let two = q == 2;
        let mut runs = vec![
            (k + 32, "+", gemm_blk_launch_threads(8, bm, k + 32), gemm_blk_launch_smem(8, two, bm, k + 32)),
            (k, "", gemm_blk_launch_threads(8, bm, k), gemm_blk_launch_smem(8, two, bm, k)),
        ];
        if gemm_blk_launch_threads(8, bm, k) != gemm_blk_threads(bm) {
            runs.push((k, "", gemm_blk_threads(bm), gemm_blk_launch_smem(8, two, bm, k)));
        }
        runs
    };
    let attrs = |(name, smem): &(String, usize)| -> String {
        format!("    CK(cudaFuncSetAttribute({name}, cudaFuncAttributeMaxDynamicSharedMemorySize, {smem}));\n")
    };
    const TOLERANCE: f64 = 1e-5;
    let mut calls = String::new();
    // What each case wrote, in order: the f32 and byte arrays each of its runs wrote (the block path's first), to be
    // equal; or a cut launch's outputs and counts beside the whole tiles' (`rows` × `cols`), to be within `TOLERANCE`.
    enum Check {
        Equal(String, Vec<(usize, bool)>, usize),
        Close(String, usize, usize, usize),
    }
    let mut cases: Vec<Check> = Vec::new();
    // Dense, into rows of a table where `AT` names them.
    const AT: usize = 3;
    for (si, &(m, n, k, biased, rowed)) in [(100usize, 384usize, 1024usize, true, false), (512, 640, 3584, false, false), (40, 256, 2048, true, true)].iter().enumerate() {
        assert_eq!(k % 128, 0, "a whole number of scale groups");
        matrix(&format!("d{si}"), n, k, 10 + si as u64);
        acts(&format!("dx{si}"), m, k, (0..m * k).map(|i| unit(i, 30 + si as u64) * (1.0 + 3.0 * ((i / 32) % 5) as f32)).collect());
        floats(&format!("db{si}.bin"), (0..n).map(|i| unit(i, 40 + si as u64) * 0.2).collect());
        for q in [1, 2] {
            let bm = gemm_blk_rows(8, q == 2);
            let ks = kernel(q, 0, bm, 0);
            let name = &ks.0;
            let runs = paths(k, q, bm);
            let (rows, at) = if rowed { (AT + m + 1, format!("rowat({})", AT + 1)) } else { (m, "nullptr".to_string()) };
            let _ = write!(calls, "  {{ // dense {si} Q{q}\n    float* b = {};\n{}", if biased { format!("dev(dir + \"/db{si}.bin\")") } else { "nullptr".into() }, attrs(&ks));
            for &(kk, pad, threads, smem) in &runs {
                let _ = writeln!(
                    calls,
                    "    {{ lk_w w = load8(dir + \"/d{si}{pad}.bin\", {n}L * {kk}); ACTS(\"dx{si}{pad}\", {m}L * {kk}, {q});\n\
                     \x20     float* y = nan({rows}L * {n}); {name}<<<dim3({}, {}), {threads}, {smem}>>>(w, xq, xl, xs, b, y, {at}, {n}, {kk}, {n}, {m}); CK(cudaGetLastError()); CK(cudaDeviceSynchronize()); put(out, y, {rows}L * {n}); }}",
                    n.div_ceil(GEMM_TILE_N),
                    m.div_ceil(bm)
                );
            }
            calls.push_str("  }\n");
            cases.push(Check::Equal(format!("dense {m} × {k} by {n}{} Q{q}", if rowed { " into a table's rows" } else { "" }), vec![(rows * n, true)], runs.len()));
        }
    }
    // In parts: three matrices of one activation, the second into a table's rows.
    let parts = [(256usize, true, false), (128, false, true), (384, true, false)];
    let (pm, pk) = (100usize, 1024usize);
    let pn: usize = parts.iter().map(|p| p.0).sum();
    for (p, &(n, _, _)) in parts.iter().enumerate() {
        matrix(&format!("p{p}"), n, pk, 50 + p as u64);
        floats(&format!("pb{p}.bin"), (0..n).map(|i| unit(i, 60 + p as u64) * 0.2).collect());
    }
    acts("px", pm, pk, (0..pm * pk).map(|i| unit(i, 70) * 2.0).collect());
    for q in [1, 2] {
        let bm = gemm_blk_rows(8, q == 2);
        let ks = kernel(q, 4, bm, 0);
        let name = &ks.0;
        let runs = paths(pk, q, bm);
        let mut ends = Vec::new();
        for p in &parts {
            ends.push(ends.last().copied().unwrap_or(0) + p.0);
        }
        let rows = |&(_, _, rowed): &(usize, bool, bool)| if rowed { AT + pm + 1 } else { pm };
        let _ = write!(calls, "  {{ // parts Q{q}\n{}", attrs(&ks));
        for &(kk, pad, threads, smem) in &runs {
            let ys: Vec<String> = (0..3).map(|p| format!("y{p}")).collect();
            let alloc: String = parts.iter().enumerate().map(|(p, part)| format!("float* y{p} = nan({}L * {});", rows(part), part.0)).collect();
            let field = |v: Vec<String>, pad: &str| v.into_iter().chain(std::iter::repeat(pad.to_string())).take(4).collect::<Vec<_>>().join(", ");
            let launch = format!(
                "{name}<<<dim3({}, {}), {threads}, {smem}>>>(lk_w{{}}, xq, xl, xs, nullptr, nullptr, nullptr, 0, {kk}, {pn}, {pm}, nullptr, nullptr, nullptr, nullptr, {{}}, nullptr, nullptr, nullptr, nullptr, 0.f, 0.f, 0, 1, nullptr, nullptr, \
                 lk_parts{{{{{}}}, {{{}}}, {{{}}}, {{{}}}, {{{}}}, {{{}}}}})",
                pn.div_ceil(GEMM_TILE_N),
                pm.div_ceil(bm),
                field((0..3).map(|p| format!("load8(dir + \"/p{p}{pad}.bin\", {}L * {kk})", parts[p].0)).collect(), "lk_w{}"),
                field(ys.clone(), "nullptr"),
                field(parts.iter().enumerate().map(|(p, part)| if part.1 { format!("dev(dir + \"/pb{p}.bin\")") } else { "nullptr".into() }).collect(), "nullptr"),
                field(ends.iter().map(usize::to_string).collect(), &pn.to_string()),
                field(parts.iter().map(|part| if part.2 { format!("rowat({})", AT + 1) } else { "nullptr".into() }).collect(), "nullptr"),
                field(parts.iter().map(|part| if part.2 { part.0.to_string() } else { "0".into() }).collect(), "0"),
            );
            let puts: String = parts.iter().enumerate().map(|(p, part)| format!(" put(out, y{p}, {}L * {});", rows(part), part.0)).collect();
            let _ = writeln!(calls, "    {{ ACTS(\"px{pad}\", {pm}L * {kk}, {q}); {alloc} {launch}; CK(cudaGetLastError()); CK(cudaDeviceSynchronize());{puts} }}");
        }
        calls.push_str("  }\n");
        cases.push(Check::Equal(format!("parts {parts:?} Q{q}"), parts.iter().map(|part| (rows(part) * part.0, true)).collect(), runs.len()));
    }
    // Paired, through the SwiGLU and through GELU, into f32 and quantized hidden values.
    let (am, ak, an) = (100usize, 1024usize, 320usize);
    matrix("ag", an, ak, 80);
    matrix("au", an, ak, 90);
    floats("abg.bin", (0..an).map(|i| unit(i, 100) * 0.1).collect());
    floats("abu.bin", (0..an).map(|i| unit(i, 110) * 0.1).collect());
    acts("ax", am, ak, (0..am * ak).map(|i| unit(i, 120) * 2.0).collect());
    for act in [-1i32, 2] {
        for q in [1, 2] {
            let bm = gemm_blk_rows(8, q == 2);
            let ks = kernel(q, 5, bm, act);
            let name = &ks.0;
            let runs = paths(ak, q, bm);
            let nh = am * an;
            let _ = write!(calls, "  {{ // paired {act} Q{q}\n    float* bg = dev(dir + \"/abg.bin\"); float* bu = dev(dir + \"/abu.bin\");\n{}", attrs(&ks));
            for &(kk, pad, threads, smem) in &runs {
                let _ = writeln!(
                    calls,
                    "    {{ lk_w wg = load8(dir + \"/ag{pad}.bin\", {an}L * {kk}); lk_w wu = load8(dir + \"/au{pad}.bin\", {an}L * {kk}); ACTS(\"ax{pad}\", {am}L * {kk}, {q});\n\
                     \x20     float* y = nan({nh}); signed char* hq = bytes({nh}); signed char* hl = {q} == 2 ? bytes({nh}) : nullptr; float* hs = nan({nh} / 32);\n\
                     \x20     {name}<<<dim3({}, {}), {threads}, {smem}>>>(wg, xq, xl, xs, bg, y, nullptr, 0, {kk}, {an}, {am}, nullptr, nullptr, nullptr, nullptr, wu, bu, hq, hl, hs); CK(cudaGetLastError()); CK(cudaDeviceSynchronize());\n\
                     \x20     put(out, y, {nh}); put(out, hs, {nh} / 32); putb(out, hq, {nh}); if({q} == 2) putb(out, hl, {nh}); }}",
                    an.div_ceil(GEMM_TILE_N / 2),
                    am.div_ceil(bm)
                );
            }
            // The streamed path's last two pair tiles cut in two along `k`, beside its whole tiles, launched as the lane
            // launches it.
            let (threads, smem) = (gemm_blk_launch_threads(8, bm, ak), gemm_blk_launch_smem(8, q == 2, bm, ak));
            let tiles = an.div_ceil(GEMM_TILE_N / 2) * am.div_ceil(bm);
            let whole = tiles - tiles.min(2);
            let _ = writeln!(
                calls,
                "    {{ lk_w wg = load8(dir + \"/ag.bin\", {an}L * {ak}); lk_w wu = load8(dir + \"/au.bin\", {an}L * {ak}); ACTS(\"ax\", {am}L * {ak}, {q});
                       float* y = nan({nh}); float* part = nan({}L); int* cnt = zeros({});
                       {name}<<<{}, {threads}, {smem}>>>(wg, xq, xl, xs, bg, y, nullptr, 0, {ak}, {an}, {am}, nullptr, nullptr, nullptr, nullptr, wu, bu, nullptr, nullptr, nullptr, 0.f, 0.f, {whole}, 2, part, cnt);
                       CK(cudaGetLastError()); CK(cudaDeviceSynchronize()); put(out, y, {nh});
                       std::vector<int> h({}); CK(cudaMemcpy(h.data(), cnt, h.size() * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, h.size(), out); }}",
                (tiles - whole) * 2 * bm * GEMM_TILE_N,
                tiles - whole,
                whole + (tiles - whole) * 2,
                tiles - whole
            );
            calls.push_str("  }\n");
            let mut arrays = vec![(nh, true), (nh / 32, true), (nh, false)];
            if q == 2 {
                arrays.push((nh, false));
            }
            cases.push(Check::Equal(format!("paired act {act} Q{q}"), arrays, runs.len()));
            cases.push(Check::Close(format!("paired act {act} Q{q}, its last two tiles cut in two"), am, an, tiles - whole));
        }
    }
    // Grouped by expert: three experts of 70, 50 and 30 gathered rows.
    const E: usize = 3;
    let (gd, gff, counts) = (512usize, 768usize, [70usize, 50, 30]);
    let gn: usize = counts.iter().sum();
    let offs: Vec<i32> = std::iter::once(0).chain(counts.iter().scan(0, |a, c| { *a += c; Some(*a as i32) })).collect();
    ints("ec.bin", counts.iter().map(|&c| c as i32).collect());
    ints("eo.bin", offs.clone());
    // Each gathered row's choice: a fixed shuffle of the choices, and each choice's weight.
    ints("el.bin", (0..gn).map(|i| ((i * 97) % gn) as i32).collect());
    floats("ewt.bin", (0..gn).map(|i| 0.25 + 0.5 * unit(i, 130).abs()).collect());
    matrix("eg", E * gff, gd, 140);
    matrix("eu", E * gff, gd, 150);
    matrix("ed", E * gd, gff, 160);
    floats("ebg.bin", (0..E * gff).map(|i| unit(i, 170) * 0.1).collect());
    floats("ebu.bin", (0..E * gff).map(|i| unit(i, 180) * 0.1).collect());
    floats("ebd.bin", (0..E * gd).map(|i| unit(i, 190) * 0.1).collect());
    acts("ex", gn, gd, (0..gn * gd).map(|i| unit(i, 200) * 2.0).collect());
    acts("eh", gn, gff, (0..gn * gff).map(|i| unit(i, 210) * 1.5).collect());
    for bm in [64usize, 128] {
        for q in [1, 2] {
            let common = "int* ec = idev(dir + \"/ec.bin\"); int* eo = idev(dir + \"/eo.bin\"); int* el = idev(dir + \"/el.bin\"); float* wt = dev(dir + \"/ewt.bin\");";
            // Gate apart (G 1), down weighed (G 2), gate and up paired through the clamped activation (G 3).
            let (k1, k2, k3) = (kernel(q, 1, bm, 0), kernel(q, 2, bm, 0), kernel(q, 3, bm, 1));
            let _ = write!(
                calls,
                "  {{ // grouped bm {bm} Q{q}\n    {common}\n    float* bg = dev(dir + \"/ebg.bin\"); float* bu = dev(dir + \"/ebu.bin\"); float* bd = dev(dir + \"/ebd.bin\");\n{}{}{}",
                attrs(&k1),
                attrs(&k2),
                attrs(&k3)
            );
            let nh = gn * gff;
            let (runs_d, runs_ff) = (paths(gd, q, bm), paths(gff, q, bm));
            for &(kk, pad, threads, smem) in &runs_d {
                let name = &k1.0;
                let _ = writeln!(
                    calls,
                    "    {{ lk_w wg = load8(dir + \"/eg{pad}.bin\", {}L * {kk}); ACTS(\"ex{pad}\", {gn}L * {kk}, {q});\n\
                     \x20     float* y = nan({gn}L * {gff}); {name}<<<dim3({}, {}, {E}), {threads}, {smem}>>>(wg, xq, xl, xs, nullptr, y, nullptr, 0, {kk}, {gff}, 0, ec, eo); CK(cudaGetLastError()); CK(cudaDeviceSynchronize()); put(out, y, {gn}L * {gff}); }}",
                    E * gff,
                    gff.div_ceil(GEMM_TILE_N),
                    70usize.div_ceil(bm)
                );
            }
            for &(kk, pad, threads, smem) in &runs_d {
                let name = &k3.0;
                let _ = writeln!(
                    calls,
                    "    {{ lk_w wg = load8(dir + \"/eg{pad}.bin\", {}L * {kk}); lk_w wu = load8(dir + \"/eu{pad}.bin\", {}L * {kk}); ACTS(\"ex{pad}\", {gn}L * {kk}, {q});\n\
                     \x20     signed char* hq = bytes({nh}); signed char* hl = {q} == 2 ? bytes({nh}) : nullptr; float* hs = nan({nh} / 32);\n\
                     \x20     {name}<<<dim3({}, {}, {E}), {threads}, {smem}>>>(wg, xq, xl, xs, bg, nullptr, nullptr, 0, {kk}, {gff}, 0, ec, eo, nullptr, nullptr, wu, bu, hq, hl, hs, 7.0f, 1.702f);\n\
                     \x20     CK(cudaGetLastError()); CK(cudaDeviceSynchronize()); put(out, hs, {nh} / 32); putb(out, hq, {nh}); if({q} == 2) putb(out, hl, {nh}); }}",
                    E * gff,
                    E * gff,
                    gff.div_ceil(GEMM_TILE_N / 2),
                    70usize.div_ceil(bm)
                );
            }
            for &(kk, pad, threads, smem) in &runs_ff {
                let name = &k2.0;
                let _ = writeln!(
                    calls,
                    "    {{ lk_w wd = load8(dir + \"/ed{pad}.bin\", {}L * {kk}); ACTS(\"eh{pad}\", {gn}L * {kk}, {q});\n\
                     \x20     float* y = nan({gn}L * {gd}); {name}<<<dim3({}, {}, {E}), {threads}, {smem}>>>(wd, xq, xl, xs, bd, y, nullptr, 0, {kk}, {gd}, 0, ec, eo, el, wt); CK(cudaGetLastError()); CK(cudaDeviceSynchronize()); put(out, y, {gn}L * {gd}); }}",
                    E * gd,
                    gd.div_ceil(GEMM_TILE_N),
                    70usize.div_ceil(bm)
                );
            }
            calls.push_str("  }\n");
            cases.push(Check::Equal(format!("grouped gate bm {bm} Q{q}"), vec![(gn * gff, true)], runs_d.len()));
            let mut paired = vec![(nh / 32, true), (nh, false)];
            if q == 2 {
                paired.push((nh, false));
            }
            cases.push(Check::Equal(format!("grouped pair bm {bm} Q{q}"), paired, runs_d.len()));
            cases.push(Check::Equal(format!("grouped down bm {bm} Q{q}"), vec![(gn * gd, true)], runs_ff.len()));
        }
    }
    // The last wave cut along `k`, against its replica: (rows, outputs, inputs), the lane's cut or, where it cuts
    // nothing, the last two tiles in two.
    let mut cuts = Vec::new();
    for (ci, &(m, n, k)) in [(512usize, 2048usize, 1024usize), (512, 640, 3584), (100, 384, 2048)].iter().enumerate() {
        matrix(&format!("c{ci}"), n, k, 220 + ci as u64);
        acts(&format!("cx{ci}"), m, k, (0..m * k).map(|i| unit(i, 230 + ci as u64) * (1.0 + 2.0 * ((i / 64) % 3) as f32)).collect());
        for q in [1, 2] {
            let bm = gemm_blk_rows(8, q == 2);
            let tiles = n.div_ceil(GEMM_TILE_N) * m.div_ceil(bm);
            let (whole, split) = match gemm_blk_tail(tiles, workers, k.div_ceil(128)) {
                (_, 1) => (tiles - tiles.min(2), 2),
                cut => cut,
            };
            let (name, allowed) = &kernel(q, 0, bm, 0);
            let (threads, smem) = (gemm_blk_launch_threads(8, bm, k), gemm_blk_launch_smem(8, q == 2, bm, k));
            let blocks = whole + (tiles - whole) * split;
            let _ = writeln!(
                calls,
                "  {{ // cut {ci} Q{q}\n    lk_w w = load8(dir + \"/c{ci}.bin\", {n}L * {k});\n    CK(cudaFuncSetAttribute({name}, cudaFuncAttributeMaxDynamicSharedMemorySize, {allowed}));\n\
                 \x20   ACTS(\"cx{ci}\", {m}L * {k}, {q});\n\
                 \x20   float* y = nan({m}L * {n}); float* part = nan({}L); int* cnt = zeros({});\n\
                 \x20   {name}<<<{blocks}, {threads}, {smem}>>>(w, xq, xl, xs, nullptr, y, nullptr, 0, {k}, {n}, {m}, nullptr, nullptr, nullptr, nullptr, {{}}, nullptr, nullptr, nullptr, nullptr, 0.f, 0.f, {whole}, {split}, part, cnt);\n\
                 \x20   CK(cudaGetLastError()); CK(cudaDeviceSynchronize());\n\
                 \x20   putb(out, xq, {m}L * {k}); if({q} == 2) putb(out, xl, {m}L * {k}); put(out, xs, {m}L * {k} / 32); put(out, y, {m}L * {n});\n\
                 \x20   std::vector<int> h({}); CK(cudaMemcpy(h.data(), cnt, h.size() * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, h.size(), out);\n  }}",
                ((tiles - whole) * split * bm * GEMM_TILE_N).max(1),
                (tiles - whole).max(1),
                (tiles - whole).max(1)
            );
            cuts.push((ci, q, m, n, k, bm, whole, split));
        }
    }
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <string>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {PRELUDE}{}{}{}{}\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ fprintf(stderr, \"cuda %d %s\\n\", __LINE__, cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         static std::vector<char> slurp(const std::string& p){{ FILE* f = fopen(p.c_str(), \"rb\"); if(!f) exit(2); std::vector<char> b; char c[65536]; size_t n;\n\
           while((n = fread(c, 1, sizeof c, f)) > 0) b.insert(b.end(), c, c + n); fclose(f); return b; }}\n\
         static float* dev(const std::string& p){{ auto b = slurp(p); float* d; CK(cudaMalloc(&d, b.size())); CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice)); return d; }}\n\
         static int* idev(const std::string& p){{ return (int*)dev(p); }}\n\
         static lk_w load8(const std::string& p, long n){{ auto b = slurp(p); signed char* q; __half* s; CK(cudaMalloc(&q, n)); CK(cudaMalloc(&s, n / 16));\n\
           CK(cudaMemcpy(q, b.data(), n, cudaMemcpyHostToDevice)); CK(cudaMemcpy(s, b.data() + n, n / 16, cudaMemcpyHostToDevice)); return lk_w{{q, s, nullptr, 0}}; }}\n\
         static float* nan(long n){{ float* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0xff, n * 4)); return p; }}\n\
         static int* zeros(long n){{ int* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0, n * 4)); return p; }}\n\
         static int* rowat(int at){{ int* p; CK(cudaMalloc(&p, 4)); CK(cudaMemcpy(p, &at, 4, cudaMemcpyHostToDevice)); return p; }}\n\
         static signed char* bytes(long n){{ signed char* p; CK(cudaMalloc(&p, n)); CK(cudaMemset(p, 0x5a, n)); return p; }}\n\
         static void put(FILE* out, const float* d, long n){{ std::vector<float> h(n); CK(cudaMemcpy(h.data(), d, n * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, n, out); }}\n\
         static void putb(FILE* out, const signed char* d, long n){{ std::vector<signed char> h(n); CK(cudaMemcpy(h.data(), d, n, cudaMemcpyDeviceToHost)); fwrite(h.data(), 1, n, out); }}\n\
         // An activation's words under numerics Q, from its f32 file.\n\
         #define ACTS(file, count, Q) long nx = (count); signed char* xq = bytes(nx); signed char* xl = (Q) == 2 ? bytes(nx) : nullptr; float* xs = nan(nx / 32); \\\n\
           {{ float* x = dev(dir + \"/\" file \".bin\"); lk_quant<(Q)><<<(nx + 255) / 256, 256>>>(x, xq, xl, xs, nx); CK(cudaGetLastError()); }}\n\
         int main(int argc, char** argv){{\n  std::string dir = argv[1];\n  FILE* out = fopen(argv[2], \"wb\");\n{calls}  fclose(out);\n  return 0;\n}}\n",
        Kernel::Blocks.source(),
        Kernel::Quant.source(),
        Kernel::Mma.source(),
        Kernel::GemmBlk.source()
    );
    let (cu, bin, outp) = (dir.join("stream.cu"), dir.join("stream"), dir.join("out.bin"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "the GEMMs build:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).arg(&dir).arg(&outp).output().unwrap();
    assert!(ran.status.success(), "they run: {}", String::from_utf8_lossy(&ran.stderr));
    let all = std::fs::read(&outp).unwrap();
    let mut at = 0;
    let mut take = |n: usize| {
        at += n;
        all[at - n..at].to_vec()
    };
    let f32s = |v: Vec<u8>| v.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect::<Vec<f32>>();
    // The streamed path's whole-tile outputs of the case before, for a cut launch to be held to.
    let mut last: Vec<Vec<u8>> = Vec::new();
    for case in &cases {
        match case {
            Check::Equal(what, arrays, runs) => {
                let block: Vec<Vec<u8>> = arrays.iter().map(|&(n, f)| take(if f { 4 * n } else { n })).collect();
                for run in 1..*runs {
                    let streamed: Vec<Vec<u8>> = arrays.iter().map(|&(n, f)| take(if f { 4 * n } else { n })).collect();
                    for (i, ((a, b), &(_, is_f32))) in block.iter().zip(&streamed).zip(arrays).enumerate() {
                        if is_f32 {
                            let written = b.chunks_exact(4).filter(|c| u32::from_le_bytes((*c).try_into().unwrap()) != u32::MAX).count();
                            assert!(written > 0, "{what}: array {i}: the streamed path's run {run} wrote it");
                        }
                        let first = a.iter().zip(b).position(|(x, y)| x != y);
                        assert_eq!(first, None, "{what}: array {i}: the streamed path's run {run} computes the block path's result bit for bit, apart first at byte {first:?}");
                    }
                    last = streamed;
                }
            }
            Check::Close(what, rows, cols, cut) => {
                let (whole, y) = (f32s(last[0].clone()), f32s(take(4 * rows * cols)));
                let counts: Vec<i32> = take(4 * cut).chunks_exact(4).map(|c| i32::from_le_bytes(c.try_into().unwrap())).collect();
                assert!(counts.iter().all(|&c| c == 0), "{what}: the counts of parts arrived left at zero");
                for r in 0..*rows {
                    let (a, b) = (&whole[r * cols..(r + 1) * cols], &y[r * cols..(r + 1) * cols]);
                    let largest = a.iter().fold(0f64, |acc, v| acc.max(f64::from(v.abs())));
                    for (o, (u, v)) in a.iter().zip(b).enumerate() {
                        assert!(v.is_finite(), "{what}: row {r} output {o} written");
                        let off = f64::from((u - v).abs()) / largest;
                        assert!(off <= TOLERANCE, "{what}: row {r} output {o}: whole {u}, cut {v} ({off:.2e} of the row's largest)");
                    }
                }
            }
        }
    }
    for (ci, q, m, n, k, bm, whole, split) in cuts {
        let what = format!("cut {m} × {k} by {n} Q{q} (split {split} past {whole})");
        let (levels, scales) = &matrices[&format!("c{ci}")];
        let xq: Vec<i8> = take(m * k).into_iter().map(|b| b as i8).collect();
        let xl: Vec<i8> = if q == 2 { take(m * k).into_iter().map(|b| b as i8).collect() } else { Vec::new() };
        let xs = f32s(take(m * k / 32 * 4));
        let y = f32s(take(m * n * 4));
        let tiles = n.div_ceil(GEMM_TILE_N) * m.div_ceil(bm);
        let counts: Vec<i32> = take((tiles - whole).max(1) * 4).chunks_exact(4).map(|c| i32::from_le_bytes(c.try_into().unwrap())).collect();
        assert!(counts.iter().all(|&c| c == 0), "{what}: the counts of parts arrived left at zero");
        let nb = k / 32;
        let groups = nb / 4;
        let nbx = n.div_ceil(GEMM_TILE_N);
        let scale = |h: u16| f32::from_bits(((h as u32 & 0x8000) << 16) | ((((h as u32 >> 10) & 0x1F) + 112) << 23) | ((h as u32 & 0x3FF) << 13));
        for r in 0..m {
            for c in 0..n {
                let blocks: Vec<(i32, f32)> = (0..nb)
                    .map(|j| {
                        let dot = |a: &[i8]| (0..32).map(|i| a[r * k + 32 * j + i] as i32 * levels[c * k + 32 * j + i] as i32).sum::<i32>();
                        let (s0, d) = (xs[r * nb + j], scale(scales[c * nb + j]));
                        if q == 2 { (dot(&xq) * 128 + dot(&xl), s0 * 0.0078125 * d) } else { (dot(&xq), s0 * d) }
                    })
                    .collect();
                let run = |lo: usize, hi: usize| blocks[lo..hi].iter().fold(0f32, |acc, &(dot, sd)| sd.mul_add(dot as f32, acc));
                // The tiles row by row: those past the first `whole` cut.
                let tile = (r / bm) * nbx + c / GEMM_TILE_N;
                let want = if tile >= whole {
                    (0..split).fold(0f32, |acc, piece| acc + run(4 * (piece * groups / split), 4 * ((piece + 1) * groups / split)))
                } else {
                    run(0, nb)
                };
                assert_eq!(y[r * n + c].to_bits(), want.to_bits(), "{what}: row {r} output {c}: {} against the replica's {want}", y[r * n + c]);
            }
        }
    }
    assert_eq!(at, all.len(), "every result read");
}

/// A gated pair as one paired GEMM (`lk_gemm_blk`'s `G` 5: the gate's and up's tiles side by side, the gated activation
/// and its quantizer in the epilogue) computes, bit for bit, what the gate's GEMM, the up's GEMM and the gated launch
/// compute apart (`lk_swiglu` for the SwiGLU, `lk_binary` multiplying the up by the gate's GELU) — its `f32` hidden
/// values and their quantized words and scales — in Q8_0, Q4_K and Q6_K under both quantized numerics, over Qwen3.5's
/// feed-forward (3584 of 1024, 512 rows: whole waves), 2816 of 2048 at 512 rows (its last wave cut along `k`, the pair's
/// as the gate's own GEMM cuts it) and 256 of 512 at the narrowest wide chunk (17 rows, every tile cut), with and
/// without biases.
#[test]
fn a_paired_gemm_computes_what_its_gate_and_up_compute_apart() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{gemm_blk_rows, gemm_blk_smem_bytes, gemm_blk_tail, gemm_blk_threads, Kernel, GEMM_TILE_N, PRELUDE};
    use logicaffeine_compile::model::quant::GgmlType;
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let workers = CudaKnobs::default().chip.sms;
    let unit = |i: usize, salt: u64| {
        let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29) ^ 0x2545_F491_4F6C_DD1D;
        ((h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32
    };
    let dir = dir().join("pair");
    std::fs::create_dir_all(&dir).unwrap();
    // (rows, inputs, hidden values, biases, the gate's activation: −1 the SwiGLU, 2 GELU)
    let shapes = [(512usize, 1024usize, 3584usize, false, -1i32), (512, 2048, 2816, true, 2), (17, 512, 256, true, -1)];
    let floats = |name: String, v: Vec<f32>| std::fs::write(dir.join(name), v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    let mut calls = String::new();
    let mut cases = Vec::new();
    for (si, &(m, k, n, biased, act)) in shapes.iter().enumerate() {
        floats(format!("x{si}.bin"), (0..m * k).map(|i| unit(i, 1 + si as u64) * 2.0).collect());
        floats(format!("bg{si}.bin"), (0..n).map(|i| unit(i, 40 + si as u64) * 0.1).collect());
        floats(format!("bu{si}.bin"), (0..n).map(|i| unit(i, 60 + si as u64) * 0.1).collect());
        for ty in [GgmlType::Q8_0, GgmlType::Q4K, GgmlType::Q6K] {
            let f = ty.id();
            for (which, salt) in [("g", 90u64), ("u", 120)] {
                let name = format!("{which}{si}-{}", ty.name());
                let matrix = if ty == GgmlType::Q8_0 {
                    let levels = (0..n * k).map(|i| ((unit(i, salt + si as u64) * 127.0) as i8) as u8);
                    levels.chain((0..n * k / 32).flat_map(|b| (0x2000u16 | ((b * 89) & 0x3FF) as u16).to_le_bytes())).collect()
                } else {
                    tiny_models::seeded_blocks(&name, ty, n * k)
                };
                std::fs::write(dir.join(format!("{name}.bin")), matrix).unwrap();
            }
            for q in [1, 2] {
                // The lane's tiles for the format and numerics.
                let bm = gemm_blk_rows(f, q == 2);
                let tiles = n.div_ceil(GEMM_TILE_N) * m.div_ceil(bm);
                let (whole, split) = gemm_blk_tail(tiles, workers, k.div_ceil(128));
                let cut = (tiles - whole) * split;
                let smem = gemm_blk_smem_bytes(f, q == 2, bm);
                let threads = gemm_blk_threads(bm);
                let plain = |w: &str, b: &str, y: &str, part: &str, cnt: &str| {
                    if split > 1 {
                        format!(
                            "lk_gemm_blk<{f}, {q}, 0, {bm}><<<{}, {threads}, {smem}>>>({w}, xq, xl, xs, {b}, {y}, nullptr, 0, {k}, {n}, {m}, nullptr, nullptr, nullptr, nullptr, {{}}, nullptr, nullptr, nullptr, nullptr, 0.f, 0.f, {whole}, {split}, {part}, {cnt}); CK(cudaGetLastError());",
                            whole + cut
                        )
                    } else {
                        format!("lk_gemm_blk<{f}, {q}, 0, {bm}><<<dim3({}, {}), {threads}, {smem}>>>({w}, xq, xl, xs, {b}, {y}, nullptr, 0, {k}, {n}, {m}); CK(cudaGetLastError());", n.div_ceil(GEMM_TILE_N), m.div_ceil(bm))
                    }
                };
                let gated = if act < 0 {
                    format!("lk_swiglu<true, {q}><<<({m}L * {n} + 255) / 256, 256>>>(g, u, ya, aq, al, as, {m}L * {n}); CK(cudaGetLastError());")
                } else {
                    format!("lk_binary<1, {act}, true, {q}><<<({m}L * {n} + 255) / 256, 256>>>(u, g, ya, aq, al, as, {m}L * {n}, {m}L * {n}); CK(cudaGetLastError());")
                };
                let pair = if split > 1 {
                    format!("{}, {threads}, {smem}>>>(wg, xq, xl, xs, bg, yb, nullptr, 0, {k}, {n}, {m}, nullptr, nullptr, nullptr, nullptr, wu, bu, bq, bl, bs, 0.f, 0.f, {}, {split}, pp, pc);", 2 * whole + 2 * cut, 2 * whole)
                } else {
                    format!("dim3({}, {}), {threads}, {smem}>>>(wg, xq, xl, xs, bg, yb, nullptr, 0, {k}, {n}, {m}, nullptr, nullptr, nullptr, nullptr, wu, bu, bq, bl, bs);", n.div_ceil(GEMM_TILE_N / 2), m.div_ceil(bm))
                };
                let _ = write!(
                    calls,
                    "  {{ // shape {si} {ty_name} Q{q}\n\
                     \x20   lk_w wg = {f} == 8 ? load8(dir + \"/g{si}-{ty_name}.bin\", {n}L * {k}) : load(dir + \"/g{si}-{ty_name}.bin\", {row_bytes}L);\n\
                     \x20   lk_w wu = {f} == 8 ? load8(dir + \"/u{si}-{ty_name}.bin\", {n}L * {k}) : load(dir + \"/u{si}-{ty_name}.bin\", {row_bytes}L);\n\
                     \x20   float* bg = {bgs}; float* bu = {bus};\n\
                     \x20   float* x = dev(dir + \"/x{si}.bin\"); long nx = {m}L * {k}, nh = {m}L * {n}; signed char* xq = bytes(nx); signed char* xl = {q} == 2 ? bytes(nx) : nullptr; float* xs = nan(nx / 32);\n\
                     \x20   lk_quant<{q}><<<(nx + 255) / 256, 256>>>(x, xq, xl, xs, nx); CK(cudaGetLastError());\n\
                     \x20   CK(cudaFuncSetAttribute(lk_gemm_blk<{f}, {q}, 0, {bm}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {smem}));\n\
                     \x20   CK(cudaFuncSetAttribute(lk_gemm_blk<{f}, {q}, 5, {bm}, {act}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {smem}));\n\
                     \x20   float *g = nan(nh), *u = nan(nh), *ya = nan(nh), *yb = nan(nh), *as = nan(nh / 32), *bs = nan(nh / 32);\n\
                     \x20   signed char *aq = bytes(nh), *bq = bytes(nh), *al = {q} == 2 ? bytes(nh) : nullptr, *bl = {q} == 2 ? bytes(nh) : nullptr;\n\
                     \x20   float *gp = nan({parts}L), *up = nan({parts}L), *pp = nan(2L * {parts}); int *gc = zeros({counts}), *uc = zeros({counts}), *pc = zeros(2 * {counts});\n\
                     \x20   {gplain}\n    {uplain}\n    {gated}\n\
                     \x20   lk_gemm_blk<{f}, {q}, 5, {bm}, {act}><<<{pair} CK(cudaGetLastError());\n\
                     \x20   CK(cudaDeviceSynchronize()); put(out, ya, nh); put(out, yb, nh); put(out, as, nh / 32); put(out, bs, nh / 32);\n\
                     \x20   putb(out, aq, nh); putb(out, bq, nh); if({q} == 2){{ putb(out, al, nh); putb(out, bl, nh); }}\n  }}\n",
                    ty_name = ty.name(),
                    row_bytes = if ty == GgmlType::Q8_0 { 0 } else { ty.bytes_for(k) },
                    parts = (cut * bm * GEMM_TILE_N).max(1),
                    counts = (tiles - whole).max(1),
                    bgs = if biased { format!("dev(dir + \"/bg{si}.bin\")") } else { "nullptr".into() },
                    bus = if biased { format!("dev(dir + \"/bu{si}.bin\")") } else { "nullptr".into() },
                    gplain = plain("wg", "bg", "g", "gp", "gc"),
                    uplain = plain("wu", "bu", "u", "up", "uc")
                );
                cases.push((format!("{} Q{q}, {n} of {k} over {m} rows, act {act}{}, {} tiles past {whole} cut in {split}", ty.name(), if biased { ", biased" } else { "" }, tiles - whole), m * n, q));
            }
        }
    }
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <string>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {PRELUDE}{}{}{}{}{}{}\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ fprintf(stderr, \"cuda %d %s\\n\", __LINE__, cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         static std::vector<char> slurp(const std::string& p){{ FILE* f = fopen(p.c_str(), \"rb\"); if(!f) exit(2); std::vector<char> b; char c[65536]; size_t n;\n\
           while((n = fread(c, 1, sizeof c, f)) > 0) b.insert(b.end(), c, c + n); fclose(f); return b; }}\n\
         static float* dev(const std::string& p){{ auto b = slurp(p); float* d; CK(cudaMalloc(&d, b.size())); CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice)); return d; }}\n\
         static lk_w load(const std::string& p, long row_bytes){{ auto b = slurp(p); unsigned char* d; CK(cudaMalloc(&d, b.size() + 32)); CK(cudaMemset(d, 0, b.size() + 32));\n\
           CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice)); return lk_w{{nullptr, nullptr, d, row_bytes}}; }}\n\
         static lk_w load8(const std::string& p, long n){{ auto b = slurp(p); signed char* q; __half* s; CK(cudaMalloc(&q, n)); CK(cudaMalloc(&s, n / 16));\n\
           CK(cudaMemcpy(q, b.data(), n, cudaMemcpyHostToDevice)); CK(cudaMemcpy(s, b.data() + n, n / 16, cudaMemcpyHostToDevice)); return lk_w{{q, s, nullptr, 0}}; }}\n\
         static float* nan(long n){{ float* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0xff, n * 4)); return p; }}\n\
         static int* zeros(long n){{ int* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0, n * 4)); return p; }}\n\
         static signed char* bytes(long n){{ signed char* p; CK(cudaMalloc(&p, n)); CK(cudaMemset(p, 0x5a, n)); return p; }}\n\
         static void put(FILE* out, const float* d, long n){{ std::vector<float> h(n); CK(cudaMemcpy(h.data(), d, n * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, n, out); }}\n\
         static void putb(FILE* out, const signed char* d, long n){{ std::vector<signed char> h(n); CK(cudaMemcpy(h.data(), d, n, cudaMemcpyDeviceToHost)); fwrite(h.data(), 1, n, out); }}\n\
         int main(int argc, char** argv){{\n  std::string dir = argv[1];\n  FILE* out = fopen(argv[2], \"wb\");\n{calls}  fclose(out);\n  return 0;\n}}\n",
        Kernel::Blocks.source(),
        Kernel::Quant.source(),
        Kernel::Mma.source(),
        Kernel::GemmBlk.source(),
        Kernel::SwiGlu.source(),
        Kernel::Binary.source()
    );
    let (cu, bin, outp) = (dir.join("pair.cu"), dir.join("pair"), dir.join("out.bin"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "the GEMMs build:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).arg(&dir).arg(&outp).output().unwrap();
    assert!(ran.status.success(), "they run: {}", String::from_utf8_lossy(&ran.stderr));
    let all = std::fs::read(&outp).unwrap();
    let mut at = 0;
    let mut take = |n: usize| {
        at += n;
        all[at - n..at].to_vec()
    };
    for (what, nh, q) in &cases {
        let (ya, yb, sa, sb) = (take(4 * nh), take(4 * nh), take(nh / 8), take(nh / 8));
        assert!(ya.chunks_exact(4).all(|c| f32::from_le_bytes(c.try_into().unwrap()).is_finite()), "{what}: every hidden value written apart, and finite");
        let first = ya.chunks_exact(4).zip(yb.chunks_exact(4)).position(|(a, b)| a != b);
        assert_eq!(first, None, "{what}: the paired GEMM's hidden values are the ones apart, bit for bit, first apart at {first:?}");
        assert_eq!(sb, sa, "{what}: and their scales");
        for word in 0..*q {
            let (wa, wb) = (take(*nh), take(*nh));
            assert_eq!(wb, wa, "{what}: and their words ({})", ["high", "low"][word]);
        }
    }
    assert_eq!(at, all.len(), "every result read");
}

/// A wide chunk's router — one GEMM of logits and a pick a row — routes as the router a block a row does: 160 rows of 512
/// choosing 6 of 64 experts through an `f32` router with a bias (`lk_gemm_f32`, each tile's `k` cut into the parts the
/// lane cuts it into on this chip) and through routers in Q4_K and Q8_0 (`lk_gemm_blk`), renormalized and not, under
/// both quantized numerics. Every logit within `TOLERANCE` of its row's
/// largest; the pick over the router's own logits (`lk_moe_pick`) gives the router's choices and weights bit for bit;
/// and over the GEMM's logits it gives the same choices, with weights within `TOLERANCE`, wherever no two of a row's
/// seventh and sixth largest logits lie within `NEAR` of each other.
#[test]
fn the_router_as_a_gemm_routes_as_the_router_does() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{gemm_blk_smem_bytes, gemm_blk_threads, gemm_f32_splits, route_warps, Kernel, GEMM_BLK_TILE_M, PRELUDE};
    use logicaffeine_compile::model::quant::GgmlType;
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    const TOLERANCE: f64 = 1e-5;
    const NEAR: f64 = 1e-4;
    const ROWS: usize = 160;
    const D: usize = 512;
    const E: usize = 64;
    const K: usize = 6;
    let unit = |i: usize, salt: u64| {
        let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(21) ^ 0x2545_F491_4F6C_DD1D;
        ((h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32
    };
    let dir = dir().join("router");
    std::fs::create_dir_all(&dir).unwrap();
    let floats = |name: &str, v: Vec<f32>| std::fs::write(dir.join(name), v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    floats("x.bin", (0..ROWS * D).map(|i| unit(i, 1) * 2.0).collect());
    floats("wf.bin", (0..E * D).map(|i| unit(i, 2) * 0.1).collect());
    floats("rb.bin", (0..E).map(|i| unit(i, 3) * 0.1).collect());
    let q8: Vec<u8> = (0..E * D).map(|i| ((unit(i, 4) * 127.0) as i8) as u8).chain((0..E * D / 32).flat_map(|_| 0x2000u16.to_le_bytes())).collect();
    std::fs::write(dir.join("w8.bin"), q8).unwrap();
    std::fs::write(dir.join("w12.bin"), tiny_models::seeded_blocks("router", GgmlType::Q4K, E * D)).unwrap();
    let mut calls = String::new();
    let mut cases = Vec::new();
    for q in [1, 2] {
        for (router, f, rf, fr) in [("f32", 0, 1, 8), ("q8_0", 8, 0, 8), ("q4_K", 12, 0, 12)] {
            for norm in [0, 1] {
                let wpe = route_warps(rf == 1, D);
                let rows_w = if rf == 1 { "lk_w{nullptr, nullptr, nullptr, 0}, wf".to_string() } else { format!("load<{f}>(dir + \"/w{f}.bin\", {E}L, {D}, {}L), nullptr", if f == 8 { 0 } else { GgmlType::Q4K.bytes_for(D) }) };
                let gemm = if rf == 1 {
                    // Each tile's `k` cut into the parts the lane cuts it into on this chip.
                    let tiles = E.div_ceil(64) * ROWS.div_ceil(64);
                    let split = gemm_f32_splits(tiles, CudaKnobs::default().chip.sms, D / 32);
                    format!(
                        "{{ float* rp = nan({}L); int* rq = ints({tiles}); CK(cudaMemset(rq, 0, {tiles} * 4)); \
                         lk_gemm_f32<<<dim3({}, {}, {split}), 256>>>(x, wf, rb, lb, {ROWS}, {E}, {D}, {split}, rp, rq); }}",
                        tiles * split * 256 * 16,
                        E.div_ceil(64),
                        ROWS.div_ceil(64)
                    )
                } else {
                    format!(
                        "{{ lk_w w = load<{f}>(dir + \"/w{f}.bin\", {E}L, {D}, {}L); CK(cudaFuncSetAttribute(lk_gemm_blk<{f}, {q}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {})); \
                         lk_gemm_blk<{f}, {q}><<<dim3({}, {}), {}, {}>>>(w, xq, xl, xs, rb, lb, nullptr, 0, {D}, {E}, {ROWS}); }}",
                        if f == 8 { 0 } else { GgmlType::Q4K.bytes_for(D) },
                        gemm_blk_smem_bytes(f, q == 2, GEMM_BLK_TILE_M),
                        E.div_ceil(128),
                        ROWS.div_ceil(GEMM_BLK_TILE_M),
                        gemm_blk_threads(GEMM_BLK_TILE_M),
                        gemm_blk_smem_bytes(f, q == 2, GEMM_BLK_TILE_M)
                    )
                };
                let _ = write!(
                    calls,
                    "  {{ // {router} Q{q} norm {norm}\n\
                     \x20   signed char* xq = bytes({ROWS} * {D}); signed char* xl = {q} == 2 ? bytes({ROWS} * {D}) : nullptr; float* xs = nan({ROWS} * {D} / 32);\n\
                     \x20   lk_quant<{q}><<<({ROWS} * {D} + 255) / 256, 256>>>(x, xq, xl, xs, {ROWS} * {D}); CK(cudaGetLastError());\n\
                     \x20   int *ia = ints({ROWS} * {K}), *ib = ints({ROWS} * {K}), *ic = ints({ROWS} * {K}), *cnt = ints({ROWS}); CK(cudaMemset(cnt, 0, {ROWS} * 4));\n\
                     \x20   float *wa = nan({ROWS} * {K}), *wb = nan({ROWS} * {K}), *wc = nan({ROWS} * {K}), *la = nan({ROWS} * {E}), *lb = nan({ROWS} * {E});\n\
                     \x20   lk_moe_route<{q}, {rf}, {fr}{}><<<dim3({}, {ROWS}), 256, {E} * 4>>>(x, xq, xl, xs, {rows_w}, rb, {D}, {E}, {K}, {norm}, ia, wa, la, cnt); CK(cudaGetLastError());\n\
                     \x20   {gemm} CK(cudaGetLastError());\n\
                     \x20   lk_moe_pick<{q}><<<{ROWS}, 256, {E} * 4>>>(lb, {E}, {K}, {norm}, ib, wb); CK(cudaGetLastError());\n\
                     \x20   lk_moe_pick<{q}><<<{ROWS}, 256, {E} * 4>>>(la, {E}, {K}, {norm}, ic, wc); CK(cudaGetLastError());\n\
                     \x20   CK(cudaDeviceSynchronize());\n\
                     \x20   put(out, la, {ROWS} * {E}); put(out, lb, {ROWS} * {E}); puti(out, ia, {ROWS} * {K}); puti(out, ib, {ROWS} * {K}); puti(out, ic, {ROWS} * {K});\n\
                     \x20   put(out, wa, {ROWS} * {K}); put(out, wb, {ROWS} * {K}); put(out, wc, {ROWS} * {K});\n  }}\n",
                    if wpe > 1 { format!(", {wpe}") } else { String::new() },
                    (E * wpe).div_ceil(8)
                );
                cases.push(format!("{router} Q{q} norm {norm}"));
            }
        }
    }
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <string>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {PRELUDE}{}{}{}{}{}{}\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ fprintf(stderr, \"cuda %d %s\\n\", __LINE__, cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         static std::vector<char> slurp(const std::string& p){{ FILE* f = fopen(p.c_str(), \"rb\"); if(!f) exit(2); std::vector<char> b; char c[65536]; size_t n;\n\
           while((n = fread(c, 1, sizeof c, f)) > 0) b.insert(b.end(), c, c + n); fclose(f); return b; }}\n\
         static float* dev(const std::string& p){{ auto b = slurp(p); float* d; CK(cudaMalloc(&d, b.size())); CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice)); return d; }}\n\
         template<int F> static lk_w load(const std::string& p, long rows, int cols, long row_bytes){{\n\
           auto b = slurp(p);\n\
           if(F == 8){{ signed char* q; __half* s; long n = rows * cols; CK(cudaMalloc(&q, n)); CK(cudaMalloc(&s, n / 32 * 2));\n\
             CK(cudaMemcpy(q, b.data(), n, cudaMemcpyHostToDevice)); CK(cudaMemcpy(s, b.data() + n, n / 32 * 2, cudaMemcpyHostToDevice)); return lk_w{{q, s, nullptr, 0}}; }}\n\
           unsigned char* d; CK(cudaMalloc(&d, b.size() + 32)); CK(cudaMemset(d, 0, b.size() + 32)); CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice));\n\
           return lk_w{{nullptr, nullptr, d, row_bytes}};\n\
         }}\n\
         static float* nan(long n){{ float* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0xff, n * 4)); return p; }}\n\
         static int* ints(long n){{ int* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0xff, n * 4)); return p; }}\n\
         static signed char* bytes(long n){{ signed char* p; CK(cudaMalloc(&p, n)); return p; }}\n\
         static void put(FILE* out, const float* d, long n){{ std::vector<float> h(n); CK(cudaMemcpy(h.data(), d, n * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, n, out); }}\n\
         static void puti(FILE* out, const int* d, long n){{ std::vector<int> h(n); CK(cudaMemcpy(h.data(), d, n * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, n, out); }}\n\
         int main(int argc, char** argv){{\n  std::string dir = argv[1];\n  float *x = dev(dir + \"/x.bin\"), *wf = dev(dir + \"/wf.bin\"), *rb = dev(dir + \"/rb.bin\");\n\
           FILE* out = fopen(argv[2], \"wb\");\n{calls}  fclose(out);\n  return 0;\n}}\n",
        Kernel::Blocks.source(),
        Kernel::Quant.source(),
        Kernel::Mma.source(),
        Kernel::GemmBlk.source(),
        Kernel::GemmF32.source(),
        Kernel::Moe.source()
    );
    let (cu, bin, outp) = (dir.join("router.cu"), dir.join("router"), dir.join("out.bin"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "both routers build:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).arg(&dir).arg(&outp).output().unwrap();
    assert!(ran.status.success(), "they run: {}", String::from_utf8_lossy(&ran.stderr));
    let bytes = std::fs::read(&outp).unwrap();
    let words: Vec<[u8; 4]> = bytes.chunks_exact(4).map(|c| c.try_into().unwrap()).collect();
    assert_eq!(words.len(), cases.len() * (2 * ROWS * E + 6 * ROWS * K), "every case's results");
    let mut at = 0;
    let mut take = |n: usize| {
        at += n;
        words[at - n..at].to_vec()
    };
    let mut report = Vec::new();
    for what in &cases {
        let f = |v: Vec<[u8; 4]>| v.into_iter().map(f32::from_le_bytes).collect::<Vec<f32>>();
        let i = |v: Vec<[u8; 4]>| v.into_iter().map(i32::from_le_bytes).collect::<Vec<i32>>();
        let (la, lb) = (f(take(ROWS * E)), f(take(ROWS * E)));
        let (ia, ib, ic) = (i(take(ROWS * K)), i(take(ROWS * K)), i(take(ROWS * K)));
        let (wa, wb, wc) = (f(take(ROWS * K)), f(take(ROWS * K)), f(take(ROWS * K)));
        assert_eq!((&ic, wc.iter().map(|v| v.to_bits()).collect::<Vec<_>>()), (&ia, wa.iter().map(|v| v.to_bits()).collect::<Vec<_>>()), "{what}: the pick over the router's logits is the router's, bit for bit");
        let (mut worst, mut near) = (0f64, 0);
        for r in 0..ROWS {
            let (a, b) = (&la[r * E..(r + 1) * E], &lb[r * E..(r + 1) * E]);
            let scale = a.iter().fold(0f64, |m, v| m.max(f64::from(v.abs())));
            for (e, (u, v)) in a.iter().zip(b).enumerate() {
                let off = f64::from((u - v).abs()) / scale;
                worst = worst.max(off);
                assert!(off <= TOLERANCE, "{what}: row {r} expert {e}: the router's logit {u}, the GEMM's {v} ({off:.2e} of the row's largest)");
            }
            let mut sorted: Vec<f32> = a.to_vec();
            sorted.sort_by(|x, y| y.total_cmp(x));
            if f64::from(sorted[K - 1] - sorted[K]) / scale < NEAR {
                near += 1;
                continue;
            }
            assert_eq!(&ib[r * K..(r + 1) * K], &ia[r * K..(r + 1) * K], "{what}: row {r} chooses alike");
            for k in 0..K {
                let off = f64::from((wa[r * K + k] - wb[r * K + k]).abs());
                assert!(off <= TOLERANCE, "{what}: row {r} choice {k}: weights {} and {} ({off:.2e} apart)", wa[r * K + k], wb[r * K + k]);
            }
        }
        report.push(format!("{what}: logits within {worst:.2e}, {near} rows near a tie"));
    }
    eprintln!("{}", report.join("\n"));
}

/// The grouped expert GEMMs compute what the slab kernels compute, on the same inputs. Each expert's gate and up rows go
/// through the gated activation (`lk_gemm_blk<F, Q, 1>` twice, then `lk_moe_glu`, against `lk_moe_up_rows`). Its down
/// rows are weighed by each choice's weight (`lk_gemm_blk<F, Q, 2>` against `lk_moe_down_rows`), both reading the same
/// quantized hidden rows, so no rounding near a half-way point can part them. 160 rows choose two of four experts, 150,
/// 90, 50 and 30 rows an expert, so a GEMM's 64-row tiles fill, spill over and stay part full. Every weight format runs
/// under both quantized numerics with SwiGLU, and MXFP4 and Q8_0 also with gpt-oss's clamped gate and the experts'
/// biases. Each result agrees with the slabs' to within `TOLERANCE` of its row's largest — the difference of two
/// summation orders — and every output is written (they start as NaN).
#[test]
fn the_grouped_expert_gemms_compute_what_the_slabs_compute() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{
        gemm_blk_smem_bytes, gemm_blk_threads, moe_group_smem_bytes, moe_group_threads, Kernel, PRELUDE,
    };
    use logicaffeine_compile::model::quant::GgmlType;
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    const E: usize = 4;
    let (group_threads, group_smem) = (moe_group_threads(E), moe_group_smem_bytes(E, moe_group_threads(E)));
    const ROWS: usize = 160;
    const D: usize = 512;
    const DFF: usize = 768;
    const TOLERANCE: f64 = 1e-5;
    let choices = ROWS * 2;
    let formats: Vec<GgmlType> = std::iter::once(GgmlType::Q8_0).chain(BLOCK_FORMATS).collect();
    // Each row's two experts: 0 for the first 150 rows and 1 after; then 1, 2 or 3 — 150, 90, 50 and 30 rows.
    let idx: Vec<i32> = (0..ROWS).flat_map(|r| [i32::from(r >= 150), if r < 80 { 1 } else if r < 130 { 2 } else { 3 }]).collect();
    let unit = |i: usize, salt: u64| {
        let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29) ^ 0x2545_F491_4F6C_DD1D;
        (h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    };
    let x: Vec<f32> = (0..choices * D).map(|i| (unit(i, 1) * 2.0) as f32).collect();
    let wt: Vec<f32> = (0..choices).map(|i| (0.25 + 0.5 * unit(i, 2).abs()) as f32).collect();
    let bias = |n: usize, salt: u64| -> Vec<f32> { (0..n).map(|i| (unit(i, salt) * 0.1) as f32).collect() };
    let (bg, bu, bd) = (bias(E * DFF, 3), bias(E * DFF, 4), bias(E * D, 5));
    let dir = dir().join("grouped");
    std::fs::create_dir_all(&dir).unwrap();
    let floats = |name: &str, v: &[f32]| std::fs::write(dir.join(name), v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    floats("x.bin", &x);
    floats("wt.bin", &wt);
    floats("bg.bin", &bg);
    floats("bu.bin", &bu);
    floats("bd.bin", &bd);
    std::fs::write(dir.join("idx.bin"), idx.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    // A stacked matrix of `rows` rows of `cols` in `ty`: a Q8_0 one as its int8 levels then its f16 scales, the layout
    // the kernels read; a block format's as its blocks.
    let matrix = |name: &str, ty: GgmlType, rows: usize, cols: usize| -> Vec<u8> {
        if ty == GgmlType::Q8_0 {
            let levels = (0..rows * cols).map(|i| ((unit(i, name.len() as u64 * 7919) * 127.0) as i8) as u8);
            levels.chain((0..rows * cols / 32).flat_map(|_| 0x2000u16.to_le_bytes())).collect()
        } else {
            tiny_models::seeded_blocks(name, ty, rows * cols)
        }
    };
    let mut cases = Vec::new();
    for &ty in &formats {
        for (w, rows, cols) in [("wg", E * DFF, D), ("wu", E * DFF, D), ("wd", E * D, DFF)] {
            std::fs::write(dir.join(format!("{w}-{}.bin", ty.name())), matrix(&format!("{w}{}", ty.name()), ty, rows, cols)).unwrap();
        }
        for q in [1, 2] {
            for bm in [64, 128] {
                cases.push((ty, q, 0, bm));
                if matches!(ty, GgmlType::Mxfp4 | GgmlType::Q8_0) {
                    cases.push((ty, q, 1, bm));
                }
            }
        }
    }
    let calls: String = cases
        .iter()
        .map(|&(ty, q, act, bm)| {
            let (f, name) = (ty.id(), ty.name());
            let load = |w: &str, rows: usize, cols: usize| format!("load<{f}>(dir + \"/{w}-{name}.bin\", {rows}L, {cols}, {}L)", ty.bytes_for(cols));
            assert_eq!(gemm_blk_threads(bm), bm * 4, "a warp each 32 × 32");
            format!(
                "  run_case<{f}, {q}, {act}, {bm}>({}, {}, {}, {}, out);\n",
                load("wg", E * DFF, D),
                load("wu", E * DFF, D),
                load("wd", E * D, DFF),
                gemm_blk_smem_bytes(f, q == 2, bm)
            )
        })
        .collect();
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <string>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {PRELUDE}{}{}{}{}{}\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ fprintf(stderr, \"cuda %d %s\\n\", __LINE__, cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         static const int E = {E}, ROWS = {ROWS}, D = {D}, DFF = {DFF}, N = {choices};\n\
         static float *x, *wt, *bg, *bu, *bd; static int* idx;\n\
         static std::vector<char> slurp(const std::string& p){{ FILE* f = fopen(p.c_str(), \"rb\"); if(!f) exit(2); std::vector<char> b; char c[65536]; size_t n; \
           while((n = fread(c, 1, sizeof c, f)) > 0) b.insert(b.end(), c, c + n); fclose(f); return b; }}\n\
         template<typename T> static T* dev(const std::string& p){{ auto b = slurp(p); T* d; CK(cudaMalloc(&d, b.size() + 32)); \
           CK(cudaMemset(d, 0, b.size() + 32)); CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice)); return d; }}\n\
         template<int F> static lk_w load(const std::string& p, long rows, int cols, long row_bytes){{\n\
           if(F == 8){{ auto b = slurp(p); signed char* q; __half* s; long n = rows * cols;\n\
             CK(cudaMalloc(&q, n)); CK(cudaMalloc(&s, n / 32 * 2)); CK(cudaMemcpy(q, b.data(), n, cudaMemcpyHostToDevice));\n\
             CK(cudaMemcpy(s, b.data() + n, n / 32 * 2, cudaMemcpyHostToDevice)); return lk_w{{q, s, nullptr, 0}}; }}\n\
           return lk_w{{nullptr, nullptr, dev<unsigned char>(p), row_bytes}};\n\
         }}\n\
         template<typename T> static T* nan(long n){{ T* p; CK(cudaMalloc(&p, n * sizeof(T))); CK(cudaMemset(p, 0xff, n * sizeof(T))); return p; }}\n\
         static void put(FILE* out, const float* d, long n){{ std::vector<float> h(n); CK(cudaMemcpy(h.data(), d, n * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, n, out); }}\n\
         template<int F, int Q, int ACT, int BM>\n\
         static void run_case(lk_w wg, lk_w wu, lk_w wd, int smem, FILE* out){{\n\
           const float *g_b = ACT ? bg : nullptr, *u_b = ACT ? bu : nullptr, *d_b = ACT ? bd : nullptr;\n\
           float limit = ACT ? 7.0f : 0.f, slope = ACT ? 1.702f : 0.f;\n\
           signed char* gq = nan<signed char>((long)N * D); signed char* gl = Q == 2 ? nan<signed char>((long)N * D) : nullptr; float* gs = nan<float>((long)N * D / 32);\n\
           lk_quant<Q><<<(N * D + 255) / 256, 256>>>(x, gq, gl, gs, N * D); CK(cudaGetLastError());\n\
           int *ec = nan<int>(E), *eo = nan<int>(E + 1), *el = nan<int>(N), *ep = nan<int>(N);\n\
           lk_moe_group<<<1, {group_threads}, {group_smem}>>>(idx, N, E, ec, eo, el, ep); CK(cudaGetLastError());\n\
           float* ha = nan<float>((long)N * DFF);\n\
           lk_moe_up_rows<Q, ACT, F, F><<<dim3((DFF + 7) / 8, E), 256>>>(nullptr, gq, gl, gs, wg, wu, g_b, u_b, ec, eo, ha, D, DFF, limit, slope); CK(cudaGetLastError());\n\
           float *hg = nan<float>((long)N * DFF), *hu = nan<float>((long)N * DFF), *hb = nan<float>((long)N * DFF);\n\
           CK(cudaFuncSetAttribute(lk_gemm_blk<F, Q, 1, BM>, cudaFuncAttributeMaxDynamicSharedMemorySize, smem));\n\
           CK(cudaFuncSetAttribute(lk_gemm_blk<F, Q, 2, BM>, cudaFuncAttributeMaxDynamicSharedMemorySize, smem));\n\
           dim3 up((DFF + 127) / 128, (ROWS + BM - 1) / BM, E), down((D + 127) / 128, (ROWS + BM - 1) / BM, E);\n\
           lk_gemm_blk<F, Q, 1, BM><<<up, BM * 4, smem>>>(wg, gq, gl, gs, nullptr, hg, nullptr, 0, D, DFF, 0, ec, eo); CK(cudaGetLastError());\n\
           lk_gemm_blk<F, Q, 1, BM><<<up, BM * 4, smem>>>(wu, gq, gl, gs, nullptr, hu, nullptr, 0, D, DFF, 0, ec, eo); CK(cudaGetLastError());\n\
           lk_moe_glu<ACT><<<(N * DFF + 255) / 256, 256>>>(hg, hu, g_b, u_b, idx, el, hb, (long)N * DFF, DFF, limit, slope); CK(cudaGetLastError());\n\
           signed char* hq = nan<signed char>((long)N * DFF); signed char* hl = Q == 2 ? nan<signed char>((long)N * DFF) : nullptr; float* hs = nan<float>((long)N * DFF / 32);\n\
           lk_quant<Q><<<(N * DFF + 255) / 256, 256>>>(ha, hq, hl, hs, N * DFF); CK(cudaGetLastError());\n\
           float *pa = nan<float>((long)N * D), *pb = nan<float>((long)N * D);\n\
           lk_moe_down_rows<Q, F><<<dim3((D + 7) / 8, E), 256>>>(nullptr, hq, hl, hs, wd, d_b, ec, eo, el, wt, pa, D, DFF); CK(cudaGetLastError());\n\
           lk_gemm_blk<F, Q, 2, BM><<<down, BM * 4, smem>>>(wd, hq, hl, hs, d_b, pb, nullptr, 0, DFF, D, 0, ec, eo, el, wt); CK(cudaGetLastError());\n\
           CK(cudaDeviceSynchronize());\n\
           put(out, ha, (long)N * DFF); put(out, hb, (long)N * DFF); put(out, pa, (long)N * D); put(out, pb, (long)N * D);\n\
         }}\n\
         int main(int argc, char** argv){{\n  std::string dir = argv[1];\n\
           x = dev<float>(dir + \"/x.bin\"); wt = dev<float>(dir + \"/wt.bin\"); bg = dev<float>(dir + \"/bg.bin\"); bu = dev<float>(dir + \"/bu.bin\");\n\
           bd = dev<float>(dir + \"/bd.bin\"); idx = dev<int>(dir + \"/idx.bin\");\n\
           FILE* out = fopen(argv[2], \"wb\");\n{calls}  fclose(out);\n  return 0;\n}}\n",
        Kernel::Blocks.source(),
        Kernel::Quant.source(),
        Kernel::Mma.source(),
        Kernel::GemmBlk.source(),
        Kernel::Moe.source()
    );
    let (cu, bin, out) = (dir.join("grouped.cu"), dir.join("grouped"), dir.join("out.bin"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "the grouped and slab kernels build:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).arg(&dir).arg(&out).output().unwrap();
    assert!(ran.status.success(), "they run: {}", String::from_utf8_lossy(&ran.stderr));
    let all: Vec<f32> = std::fs::read(&out).unwrap().chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
    assert_eq!(all.len(), cases.len() * 2 * choices * (DFF + D), "every case's results");
    let mut at = 0;
    let mut take = |n: usize| {
        at += n;
        all[at - n..at].to_vec()
    };
    let mut report = Vec::new();
    for &(ty, q, act, bm) in &cases {
        let what = format!("{} Q{q} {bm} rows{}", ty.name(), if act == 1 { " clamped, biased" } else { "" });
        let (ha, hb, pa, pb) = (take(choices * DFF), take(choices * DFF), take(choices * D), take(choices * D));
        let mut worst = [0f64; 2];
        for (k, (slab, grouped, width, part)) in [(&ha, &hb, DFF, "hidden row"), (&pa, &pb, D, "weighed down row")].into_iter().enumerate() {
            for row in 0..choices {
                let (a, b) = (&slab[row * width..(row + 1) * width], &grouped[row * width..(row + 1) * width]);
                assert!(a.iter().chain(b).all(|v| v.is_finite()), "{what}: choice {row}'s {part} is written, and finite");
                let scale = a.iter().fold(0f64, |m, v| m.max(f64::from(v.abs())));
                assert!(scale > 0.0, "{what}: choice {row}'s {part} is not all zero");
                for (j, (u, v)) in a.iter().zip(b).enumerate() {
                    let off = f64::from((u - v).abs()) / scale;
                    worst[k] = worst[k].max(off);
                    assert!(off <= TOLERANCE, "{what}: choice {row}'s {part}, element {j}: the slabs give {u}, the grouped GEMM {v} ({off:.2e} of the row's largest)");
                }
            }
        }
        report.push(format!("{what}: hidden rows within {:.2e}, down rows within {:.2e}", worst[0], worst[1]));
    }
    eprintln!("{}", report.join("\n"));
}

/// The paired expert GEMM (`lk_gemm_blk<F, Q, 3>`: each block's weight rows an expert's 64 gate rows and the same 64 up
/// rows, the gated activation and the activation quantizer in its epilogue) quantizes each choice's hidden row exactly
/// as the unpaired launches do — the gate GEMM, the up GEMM, `lk_moe_glu` and `lk_quant` — byte for byte, words and
/// scales: every weight format under both quantized numerics with SwiGLU, MXFP4 and Q8_0 also with gpt-oss's clamped
/// gate and the experts' biases, at 64 and 128 rows a block, over the grouped lock's choices (150, 90, 50 and 30 rows an
/// expert, so a tile fills, spills over and stays part full).
#[test]
fn the_paired_gate_and_up_gemm_quantizes_the_hidden_rows_as_the_unpaired_launches_do() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{gemm_blk_smem_bytes, moe_group_smem_bytes, moe_group_threads, Kernel, PRELUDE};
    use logicaffeine_compile::model::quant::GgmlType;
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    const E: usize = 4;
    const ROWS: usize = 160;
    const D: usize = 512;
    const DFF: usize = 768;
    let (group_threads, group_smem) = (moe_group_threads(E), moe_group_smem_bytes(E, moe_group_threads(E)));
    let choices = ROWS * 2;
    let formats: Vec<GgmlType> = std::iter::once(GgmlType::Q8_0).chain(BLOCK_FORMATS).collect();
    let idx: Vec<i32> = (0..ROWS).flat_map(|r| [i32::from(r >= 150), if r < 80 { 1 } else if r < 130 { 2 } else { 3 }]).collect();
    let unit = |i: usize, salt: u64| {
        let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29) ^ 0x2545_F491_4F6C_DD1D;
        (h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    };
    let dir = dir().join("paired");
    std::fs::create_dir_all(&dir).unwrap();
    let floats = |name: &str, v: &[f32]| std::fs::write(dir.join(name), v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    floats("x.bin", &(0..choices * D).map(|i| (unit(i, 1) * 2.0) as f32).collect::<Vec<f32>>());
    floats("bg.bin", &(0..E * DFF).map(|i| (unit(i, 3) * 0.1) as f32).collect::<Vec<f32>>());
    floats("bu.bin", &(0..E * DFF).map(|i| (unit(i, 4) * 0.1) as f32).collect::<Vec<f32>>());
    std::fs::write(dir.join("idx.bin"), idx.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    let matrix = |name: &str, ty: GgmlType, rows: usize, cols: usize| -> Vec<u8> {
        if ty == GgmlType::Q8_0 {
            let levels = (0..rows * cols).map(|i| ((unit(i, name.len() as u64 * 7919) * 127.0) as i8) as u8);
            levels.chain((0..rows * cols / 32).flat_map(|_| 0x2000u16.to_le_bytes())).collect()
        } else {
            tiny_models::seeded_blocks(name, ty, rows * cols)
        }
    };
    let mut cases = Vec::new();
    for &ty in &formats {
        for w in ["wg", "wu"] {
            std::fs::write(dir.join(format!("{w}-{}.bin", ty.name())), matrix(&format!("{w}{}", ty.name()), ty, E * DFF, D)).unwrap();
        }
        for q in [1, 2] {
            for bm in [64, 128] {
                cases.push((ty, q, 0, bm));
                if matches!(ty, GgmlType::Mxfp4 | GgmlType::Q8_0) {
                    cases.push((ty, q, 1, bm));
                }
            }
        }
    }
    let calls: String = cases
        .iter()
        .map(|&(ty, q, act, bm)| {
            let (f, name) = (ty.id(), ty.name());
            let load = |w: &str| format!("load<{f}>(dir + \"/{w}-{name}.bin\", {}L, {D}, {}L)", E * DFF, ty.bytes_for(D));
            format!("  run_case<{f}, {q}, {act}, {bm}>({}, {}, {}, out);\n", load("wg"), load("wu"), gemm_blk_smem_bytes(f, q == 2, bm))
        })
        .collect();
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <string>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {PRELUDE}{}{}{}{}{}\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ fprintf(stderr, \"cuda %d %s\\n\", __LINE__, cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         static const int E = {E}, D = {D}, DFF = {DFF}, N = {choices}, ROWS = {ROWS};\n\
         static float *x, *bg, *bu; static int* idx;\n\
         static std::vector<char> slurp(const std::string& p){{ FILE* f = fopen(p.c_str(), \"rb\"); if(!f) exit(2); std::vector<char> b; char c[65536]; size_t n; \
           while((n = fread(c, 1, sizeof c, f)) > 0) b.insert(b.end(), c, c + n); fclose(f); return b; }}\n\
         template<typename T> static T* dev(const std::string& p){{ auto b = slurp(p); T* d; CK(cudaMalloc(&d, b.size() + 32)); \
           CK(cudaMemset(d, 0, b.size() + 32)); CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice)); return d; }}\n\
         template<int F> static lk_w load(const std::string& p, long rows, int cols, long row_bytes){{\n\
           if(F == 8){{ auto b = slurp(p); signed char* q; __half* s; long n = rows * cols;\n\
             CK(cudaMalloc(&q, n)); CK(cudaMalloc(&s, n / 32 * 2)); CK(cudaMemcpy(q, b.data(), n, cudaMemcpyHostToDevice));\n\
             CK(cudaMemcpy(s, b.data() + n, n / 32 * 2, cudaMemcpyHostToDevice)); return lk_w{{q, s, nullptr, 0}}; }}\n\
           return lk_w{{nullptr, nullptr, dev<unsigned char>(p), row_bytes}};\n\
         }}\n\
         template<typename T> static T* nan(long n){{ T* p; CK(cudaMalloc(&p, n * sizeof(T))); CK(cudaMemset(p, 0xff, n * sizeof(T))); return p; }}\n\
         template<typename T> static void put(FILE* out, const T* d, long n){{ std::vector<T> h(n); CK(cudaMemcpy(h.data(), d, n * sizeof(T), cudaMemcpyDeviceToHost)); fwrite(h.data(), sizeof(T), n, out); }}\n\
         template<int F, int Q, int ACT, int BM>\n\
         static void run_case(lk_w wg, lk_w wu, int smem, FILE* out){{\n\
           const float *g_b = ACT ? bg : nullptr, *u_b = ACT ? bu : nullptr;\n\
           float limit = ACT ? 7.0f : 0.f, slope = ACT ? 1.702f : 0.f;\n\
           long nh = (long)N * DFF;\n\
           signed char* gq = nan<signed char>((long)N * D); signed char* gl = Q == 2 ? nan<signed char>((long)N * D) : nullptr; float* gs = nan<float>((long)N * D / 32);\n\
           lk_quant<Q><<<(N * D + 255) / 256, 256>>>(x, gq, gl, gs, N * D); CK(cudaGetLastError());\n\
           int *ec = nan<int>(E), *eo = nan<int>(E + 1), *el = nan<int>(N), *ep = nan<int>(N);\n\
           lk_moe_group<<<1, {group_threads}, {group_smem}>>>(idx, N, E, ec, eo, el, ep); CK(cudaGetLastError());\n\
           float *hg = nan<float>(nh), *hu = nan<float>(nh), *hb = nan<float>(nh);\n\
           CK(cudaFuncSetAttribute(lk_gemm_blk<F, Q, 1, BM>, cudaFuncAttributeMaxDynamicSharedMemorySize, smem));\n\
           CK(cudaFuncSetAttribute(lk_gemm_blk<F, Q, 3, BM, ACT>, cudaFuncAttributeMaxDynamicSharedMemorySize, smem));\n\
           dim3 up((DFF + 127) / 128, (ROWS + BM - 1) / BM, E), paired((DFF + 63) / 64, (ROWS + BM - 1) / BM, E);\n\
           lk_gemm_blk<F, Q, 1, BM><<<up, BM * 4, smem>>>(wg, gq, gl, gs, nullptr, hg, nullptr, 0, D, DFF, 0, ec, eo); CK(cudaGetLastError());\n\
           lk_gemm_blk<F, Q, 1, BM><<<up, BM * 4, smem>>>(wu, gq, gl, gs, nullptr, hu, nullptr, 0, D, DFF, 0, ec, eo); CK(cudaGetLastError());\n\
           lk_moe_glu<ACT><<<(nh + 255) / 256, 256>>>(hg, hu, g_b, u_b, idx, el, hb, nh, DFF, limit, slope); CK(cudaGetLastError());\n\
           signed char* aq = nan<signed char>(nh); signed char* al = Q == 2 ? nan<signed char>(nh) : nullptr; float* as = nan<float>(nh / 32);\n\
           lk_quant<Q><<<(nh + 255) / 256, 256>>>(hb, aq, al, as, nh); CK(cudaGetLastError());\n\
           signed char* bq = nan<signed char>(nh); signed char* bl = Q == 2 ? nan<signed char>(nh) : nullptr; float* bs = nan<float>(nh / 32);\n\
           lk_gemm_blk<F, Q, 3, BM, ACT><<<paired, BM * 4, smem>>>(wg, gq, gl, gs, g_b, nullptr, nullptr, 0, D, DFF, 0, ec, eo, nullptr, nullptr,\n\
                                                                     wu, u_b, bq, bl, bs, limit, slope); CK(cudaGetLastError());\n\
           CK(cudaDeviceSynchronize());\n\
           put(out, aq, nh); put(out, bq, nh); put(out, as, nh / 32); put(out, bs, nh / 32);\n\
           if(Q == 2){{ put(out, al, nh); put(out, bl, nh); }}\n\
         }}\n\
         int main(int argc, char** argv){{\n  std::string dir = argv[1];\n\
           x = dev<float>(dir + \"/x.bin\"); bg = dev<float>(dir + \"/bg.bin\"); bu = dev<float>(dir + \"/bu.bin\"); idx = dev<int>(dir + \"/idx.bin\");\n\
           FILE* out = fopen(argv[2], \"wb\");\n{calls}  fclose(out);\n  return 0;\n}}\n",
        Kernel::Blocks.source(),
        Kernel::Quant.source(),
        Kernel::Mma.source(),
        Kernel::GemmBlk.source(),
        Kernel::Moe.source()
    );
    let (cu, bin, out) = (dir.join("paired.cu"), dir.join("paired"), dir.join("out.bin"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "the paired and unpaired launches build:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).arg(&dir).arg(&out).output().unwrap();
    assert!(ran.status.success(), "they run: {}", String::from_utf8_lossy(&ran.stderr));
    let bytes = std::fs::read(&out).unwrap();
    let nh = choices * DFF;
    let mut at = 0;
    let mut take = |n: usize| {
        at += n;
        bytes[at - n..at].to_vec()
    };
    for &(ty, q, act, bm) in &cases {
        let what = format!("{} Q{q} {bm} rows{}", ty.name(), if act == 1 { " clamped, biased" } else { "" });
        let (aq, bq, scales_a, scales_b) = (take(nh), take(nh), take(nh / 32 * 4), take(nh / 32 * 4));
        assert_eq!(scales_b, scales_a, "{what}: every block's scale, bit for bit");
        let first = aq.iter().zip(&bq).position(|(a, b)| a != b);
        assert_eq!(first, None, "{what}: every hidden value's word, the first apart at {first:?}");
        if q == 2 {
            let (al, bl) = (take(nh), take(nh));
            let first = al.iter().zip(&bl).position(|(a, b)| a != b);
            assert_eq!(first, None, "{what}: every hidden value's low word, the first apart at {first:?}");
        }
    }
    assert_eq!(at, bytes.len(), "every case's results read");
}

/// The causal conv's ring keeps position `p` (from 0) in slot `p mod T` — a row written once, where the row `T`
/// positions before it was, and read in its window oldest to newest — so it computes, bit for bit, what the ring that
/// shifts up a row a step computes (written out here as that conv was): over 2 and 4 taps, plain, through silu and
/// through silu with each head's L2 norm, in `f32` and `f64` sums, from position 1 through single steps (`lk_conv`), runs
/// of rows one after another, and chunks at once (`lk_conv_rows`, then `lk_conv_carry`) longer and shorter than the
/// window; and a batch's step (`lk_conv_parts` over three sequences at positions of their own, a plain part and a normed
/// one). After every launch the last `T` positions each sit in their own slot as the shifting ring's rows, oldest first.
#[test]
fn a_convs_ring_keeps_each_position_in_its_own_slot() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{Kernel, PRELUDE};
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    const C: usize = 256;
    // (rows, at once): single steps, rows one after another, chunks at once longer and shorter than the window.
    let plan: [(usize, bool); 15] = [(1, false), (1, false), (1, false), (3, false), (5, true), (1, true), (2, true), (1, false), (6, true), (3, false), (1, false), (4, true), (1, false), (2, true), (1, false)];
    // Each sequence's steps before the batch's step.
    let history = [4usize, 9, 1];
    let plan_list = plan.iter().map(|(r, c)| format!("{{{r}, {}}}", u8::from(*c))).collect::<Vec<_>>().join(", ");
    let history_list = history.iter().map(usize::to_string).collect::<Vec<_>>().join(", ");
    let mut calls = String::new();
    let mut expected = 0usize;
    for taps in [2usize, 4] {
        for (silu, l2) in [(false, false), (true, false), (true, true)] {
            for ty in ["float", "double"] {
                let _ = writeln!(calls, "  steps<{silu}, {l2}, {ty}>(\"{taps} taps, silu {silu}, l2 {l2}, {ty}\", {taps});");
                expected += plan.iter().map(|(r, _)| r * C + taps * C).sum::<usize>();
            }
        }
        for ty in ["float", "double"] {
            let _ = writeln!(calls, "  batch<{ty}>(\"{taps} taps, a batch, {ty}\", {taps});");
            expected += 2 * history.iter().map(|h| (h + 1) * (C + taps * C)).sum::<usize>();
        }
    }
    let harness = r#"
#define CK(x) do { cudaError_t e_ = (x); if(e_ != cudaSuccess){ fprintf(stderr, "cuda %d %s\n", __LINE__, cudaGetErrorString(e_)); exit(1); } } while(0)
// The ring shifted up a row a step, its rows the window oldest to newest: the conv as it was.
template<bool SILU, bool L2, typename A>
__global__ void shifting(const float* __restrict__ x, const float* __restrict__ wc, float* __restrict__ cs,
                         float* __restrict__ y, int C, int T, int R, int nl2, float eps){
  __shared__ A red[32];
  int c = blockIdx.x * blockDim.x + threadIdx.x;
  for(int r = 0; r < R; ++r){
    float v = 0.f;
    if(c < C){
      for(int t = 0; t < T - 1; ++t) cs[(long)t * C + c] = cs[(long)(t + 1) * C + c];
      cs[(long)(T - 1) * C + c] = x[(long)r * C + c];
      A sum = 0;
      for(int t = 0; t < T; ++t) sum += (A)cs[(long)t * C + c] * (A)wc[(long)t * C + c];
      v = (float)sum;
      if(SILU) v = v / (1.0f + expf(-v));
    }
    if(L2 && blockIdx.x < nl2){
      A den = sqrt(lk_block_sum(c < C ? (A)v * (A)v : (A)0, red) + (A)eps);
      v = (float)((A)v / den);
    }
    if(c < C) y[(long)r * C + c] = v;
  }
}
static const int C = @C@;
static const int PLAN[][2] = {@PLAN@};
static const int HISTORY[] = {@HISTORY@};
static long checked = 0, apart = 0;
static std::vector<float> host(const float* d, long n){ std::vector<float> h(n); CK(cudaMemcpy(h.data(), d, n * 4, cudaMemcpyDeviceToHost)); return h; }
static void same(const char* what, int at, const float* got, const float* want, long n){
  std::vector<float> g = host(got, n), w = host(want, n);
  for(long i = 0; i < n; ++i){ ++checked; if(memcmp(&g[i], &w[i], 4) && apart++ < 10) printf("APART %s through %d: %ld: %.9g against %.9g\n", what, at, i, g[i], w[i]); }
}
// After position `last` (from 0) the last `T` positions, each in slot `p mod T`, are the shifting ring's rows.
static void rings(const char* what, int last, int T, const float* rot, const float* sh){
  std::vector<float> r = host(rot, (long)T * C), s = host(sh, (long)T * C);
  for(int i = 0; i < T; ++i){
    int p = last - (T - 1) + i, slot = ((p % T) + T) % T;
    for(int c = 0; c < C; ++c){ ++checked; if(memcmp(&r[(long)slot * C + c], &s[(long)i * C + c], 4) && apart++ < 10) printf("RING %s after %d: position %d channel %d\n", what, last, p, c); }
  }
}
static float* seeded(long n, unsigned salt){
  std::vector<float> h(n); for(long i = 0; i < n; ++i) h[i] = (float)((((unsigned)i * 2654435761u + salt * 40503u) >> 9) % 2001) / 1000.f - 1.f;
  float* d; CK(cudaMalloc(&d, n * 4)); CK(cudaMemcpy(d, h.data(), n * 4, cudaMemcpyHostToDevice)); return d;
}
static float* zeros(long n){ float* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0, n * 4)); return p; }
static int* positions(const std::vector<int>& v){ int* p; CK(cudaMalloc(&p, v.size() * 4)); CK(cudaMemcpy(p, v.data(), v.size() * 4, cudaMemcpyHostToDevice)); return p; }
template<bool S, bool L, typename A>
static void steps(const char* what, int T){
  const int block = L ? 128 : 256, grid = C / block, nl2 = L ? C / 128 : 0;
  int total = 0; for(auto& p : PLAN) total += p[0];
  float *x = seeded((long)total * C, 7 + T), *wc = seeded((long)T * C, 11 + T);
  float *rot = zeros((long)T * C), *sh = zeros((long)T * C), *ya = zeros((long)total * C), *yb = zeros((long)total * C);
  int done = 0;
  for(auto& p : PLAN){
    const int r = p[0];
    int* pos = positions({done + 1});
    const float* xr = x + (long)done * C;
    shifting<S, L, A><<<grid, block>>>(xr, wc, sh, ya + (long)done * C, C, T, r, nl2, 1e-6f);
    if(p[1]){
      lk_conv_rows<S, L, A><<<dim3(grid, r), block>>>(xr, wc, rot, yb + (long)done * C, C, T, r, nl2, 1e-6f, pos);
      lk_conv_carry<<<(C + 255) / 256, 256>>>(xr, rot, C, T, r, pos);
    } else {
      lk_conv<S, L, A><<<grid, block>>>(xr, wc, rot, yb + (long)done * C, C, T, r, nl2, 1e-6f, pos);
    }
    CK(cudaGetLastError()); CK(cudaDeviceSynchronize());
    same(what, done + r, yb + (long)done * C, ya + (long)done * C, (long)r * C);
    done += r;
    rings(what, done - 1, T, rot, sh);
    CK(cudaFree(pos));
  }
  CK(cudaFree(x)); CK(cudaFree(wc)); CK(cudaFree(rot)); CK(cudaFree(sh)); CK(cudaFree(ya)); CK(cudaFree(yb));
}
template<typename A>
static void batch(const char* what, int T){
  const int B = sizeof(HISTORY) / sizeof(HISTORY[0]), block = 128, nb = C / block;
  int longest = 0; for(int h : HISTORY) longest = longest > h ? longest : h;
  float *wc0 = seeded((long)T * C, 21 + T), *wc1 = seeded((long)T * C, 23 + T);
  float *x0 = seeded((long)B * (longest + 1) * C, 25 + T), *x1 = seeded((long)B * (longest + 1) * C, 27 + T);
  float *rot0 = zeros((long)B * T * C), *rot1 = zeros((long)B * T * C), *sh0 = zeros((long)B * T * C), *sh1 = zeros((long)B * T * C);
  float *ya0 = zeros((long)B * C), *ya1 = zeros((long)B * C), *yb0 = zeros((long)B * C), *yb1 = zeros((long)B * C);
  // Each sequence's history a step at a time, then the batch's step: sequence `z`'s rows at row `z·(longest + 1) + j`.
  for(int z = 0; z < B; ++z)
    for(int j = 0; j < HISTORY[z]; ++j){
      int* pos = positions({j + 1});
      const long xr = ((long)z * (longest + 1) + j) * C;
      lk_conv<true, false, A><<<nb, block>>>(x0 + xr, wc0, rot0 + (long)z * T * C, yb0 + (long)z * C, C, T, 1, 0, 1e-6f, pos);
      lk_conv<true, true, A><<<nb, block>>>(x1 + xr, wc1, rot1 + (long)z * T * C, yb1 + (long)z * C, C, T, 1, nb, 1e-6f, pos);
      shifting<true, false, A><<<nb, block>>>(x0 + xr, wc0, sh0 + (long)z * T * C, ya0 + (long)z * C, C, T, 1, 0, 1e-6f);
      shifting<true, true, A><<<nb, block>>>(x1 + xr, wc1, sh1 + (long)z * T * C, ya1 + (long)z * C, C, T, 1, nb, 1e-6f);
      CK(cudaGetLastError()); CK(cudaDeviceSynchronize());
      same(what, j + 1, yb0 + (long)z * C, ya0 + (long)z * C, C); same(what, j + 1, yb1 + (long)z * C, ya1 + (long)z * C, C);
      rings(what, j, T, rot0 + (long)z * T * C, sh0 + (long)z * T * C); rings(what, j, T, rot1 + (long)z * T * C, sh1 + (long)z * T * C);
      CK(cudaFree(pos));
    }
  // The batch's step: each sequence's row and ring its part of one array, at its own position.
  std::vector<int> at(B);
  for(int z = 0; z < B; ++z) at[z] = HISTORY[z] + 1;
  int* pos = positions(at);
  float *xs0 = zeros((long)B * C), *xs1 = zeros((long)B * C);
  for(int z = 0; z < B; ++z){
    const long xr = ((long)z * (longest + 1) + HISTORY[z]) * C;
    CK(cudaMemcpy(xs0 + (long)z * C, x0 + xr, C * 4, cudaMemcpyDeviceToDevice)); CK(cudaMemcpy(xs1 + (long)z * C, x1 + xr, C * 4, cudaMemcpyDeviceToDevice));
    shifting<true, false, A><<<nb, block>>>(x0 + xr, wc0, sh0 + (long)z * T * C, ya0 + (long)z * C, C, T, 1, 0, 1e-6f);
    shifting<true, true, A><<<nb, block>>>(x1 + xr, wc1, sh1 + (long)z * T * C, ya1 + (long)z * C, C, T, 1, nb, 1e-6f);
  }
  lk_conv_parts<A><<<dim3(2 * nb, B), block>>>(lk_convs{{{xs0, wc0, rot0, yb0, C, nb, 0}, {xs1, wc1, rot1, yb1, C, nb, 1}}}, 2, T, 1e-6f, pos, 1);
  CK(cudaGetLastError()); CK(cudaDeviceSynchronize());
  for(int z = 0; z < B; ++z){
    same(what, at[z], yb0 + (long)z * C, ya0 + (long)z * C, C); same(what, at[z], yb1 + (long)z * C, ya1 + (long)z * C, C);
    rings(what, at[z] - 1, T, rot0 + (long)z * T * C, sh0 + (long)z * T * C); rings(what, at[z] - 1, T, rot1 + (long)z * T * C, sh1 + (long)z * T * C);
  }
  CK(cudaFree(pos));
}
"#
    .replace("@C@", &C.to_string())
    .replace("@PLAN@", &plan_list)
    .replace("@HISTORY@", &history_list);
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <cstring>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n{PRELUDE}{}\n{harness}\
         int main(){{\n{calls}  printf(\"CHECKED %ld APART %ld\\n\", checked, apart);\n  return 0;\n}}\n",
        Kernel::Conv.source()
    );
    let dir = dir().join("conv-ring");
    std::fs::create_dir_all(&dir).unwrap();
    let (cu, bin) = (dir.join("ring.cu"), dir.join("ring"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "the rings build:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).output().unwrap();
    let stdout = String::from_utf8_lossy(&ran.stdout);
    assert!(ran.status.success(), "they run:\n{stdout}\n{}", String::from_utf8_lossy(&ran.stderr));
    let line = stdout.lines().find(|l| l.starts_with("CHECKED ")).unwrap_or_else(|| panic!("a summary:\n{stdout}"));
    let field = |key: &str| -> usize {
        let words: Vec<&str> = line.split_whitespace().collect();
        let at = words.iter().position(|w| *w == key).unwrap_or_else(|| panic!("{key} in {line}"));
        words[at + 1].parse().unwrap()
    };
    assert_eq!(field("CHECKED"), expected, "every output and every slot of every case checked");
    assert_eq!(field("APART"), 0, "the rotating ring computes the shifting ring's bits and keeps its rows:\n{stdout}");
}

/// A chunk's causal conv over all its rows at once (`lk_conv_rows`, then `lk_conv_carry` advancing the ring) computes,
/// bit for bit, what its rows fed one after another compute (`lk_conv`), and leaves the ring as they leave it: a run of
/// chunks of 1, 2, 3, 16, 5 and 64 rows from position 1, each launch given its first row's position — shorter than the
/// window too, so rows read and the ring keeps rows of the chunk before — over 2 and 4 taps, plain, through silu and through silu with each head's L2 norm, in `f32` and `f64` sums.
/// And the `f32` GEMV over its rows four a block (`lk_gemv_f32`'s row groups) computes what one block of every row does,
/// a warp a row and with each row's `k` cut over the warps the lane gives it (`gemv_f32_parts`).
#[test]
fn a_chunks_conv_and_f32_gemv_over_its_rows_at_once_compute_what_its_rows_one_after_another_compute() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{gemv_f32_parts, Kernel, GEMV_F32_ROWS, PRELUDE};
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    const C: usize = 512;
    const HEAD: usize = 128;
    let chunks = [1usize, 2, 3, 16, 5, 64];
    let rows: usize = chunks.iter().sum();
    // Each chunk's first row's position, from 1.
    let start_list = chunks.iter().scan(1usize, |at, r| { let first = *at; *at += r; Some(first.to_string()) }).collect::<Vec<_>>().join(", ");
    let mut calls = String::new();
    let mut cases = Vec::new();
    for taps in [2usize, 4] {
        for (silu, l2) in [(false, false), (true, false), (true, true)] {
            for ty in ["float", "double"] {
                let (grid, block, nl2) = if l2 { (C / HEAD, HEAD, C / HEAD) } else { (C / 256, 256, 0) };
                let mut runs = String::new();
                let mut at = 0;
                for (i, &r) in chunks.iter().enumerate() {
                    let _ = write!(
                        runs,
                        "    lk_conv<{silu}, {l2}, {ty}><<<{grid}, {block}>>>(x + {}, wc, ra, ya + {}, {C}, {taps}, {r}, {nl2}, 1e-6f, starts + {i}); CK(cudaGetLastError());\n\
                         \x20   lk_conv_rows<{silu}, {l2}, {ty}><<<dim3({grid}, {r}), {block}>>>(x + {}, wc, rb, yb + {}, {C}, {taps}, {r}, {nl2}, 1e-6f, starts + {i}); CK(cudaGetLastError());\n\
                         \x20   lk_conv_carry<<<{}, 256>>>(x + {}, rb, {C}, {taps}, {r}, starts + {i}); CK(cudaGetLastError());\n",
                        at * C,
                        at * C,
                        at * C,
                        at * C,
                        C.div_ceil(256),
                        at * C
                    );
                    at += r;
                }
                let _ = write!(
                    calls,
                    "  {{ // {taps} taps, silu {silu}, l2 {l2}, {ty}\n\
                     \x20   float *ra = ring(), *rb = ring(), *ya = nan({rows} * {C}), *yb = nan({rows} * {C});\n\
                     {runs}\
                     \x20   CK(cudaDeviceSynchronize()); put(out, ya, {rows} * {C}); put(out, yb, {rows} * {C}); put(out, ra, {taps} * {C}); put(out, rb, {taps} * {C});\n  }}\n"
                );
                cases.push((format!("{taps} taps, silu {silu}, l2 {l2}, {ty} sums"), taps));
            }
        }
    }
    let parts = gemv_f32_parts(16, 1024, CudaKnobs::default().chip.sms);
    assert!(parts > 1, "Qwen3.5's 16 gates over 1024 values cut each row over warps");
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {PRELUDE}{}{}\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ fprintf(stderr, \"cuda %d %s\\n\", __LINE__, cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         static float* seeded(long n, unsigned salt){{ std::vector<float> h(n); for(long i = 0; i < n; ++i) h[i] = (float)((((unsigned)i * 2654435761u + salt * 40503u) >> 9) % 2001) / 1000.f - 1.f;\n\
           float* d; CK(cudaMalloc(&d, n * 4)); CK(cudaMemcpy(d, h.data(), n * 4, cudaMemcpyHostToDevice)); return d; }}\n\
         static float* nan(long n){{ float* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0xff, n * 4)); return p; }}\n\
         static float* x; static float* wc; static float* ring0; static int* starts;\n\
         static float* ring(){{ float* p; CK(cudaMalloc(&p, 4L * {C} * 4)); CK(cudaMemcpy(p, ring0, 4L * {C} * 4, cudaMemcpyDeviceToDevice)); return p; }}\n\
         static void put(FILE* out, const float* d, long n){{ std::vector<float> h(n); CK(cudaMemcpy(h.data(), d, n * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, n, out); }}\n\
         int main(int argc, char** argv){{\n\
           x = seeded({rows}L * {C}, 1); wc = seeded(4L * {C}, 2); ring0 = seeded(4L * {C}, 3);\n\
           {{ const int s0[] = {{{start_list}}}; CK(cudaMalloc(&starts, sizeof s0)); CK(cudaMemcpy(starts, s0, sizeof s0, cudaMemcpyHostToDevice)); }}\n\
           FILE* out = fopen(argv[1], \"wb\");\n{calls}\
           float* w = seeded(16L * 1024, 4); float* xg = seeded({rows}L * 1024, 5); float *ga = nan({rows} * 16), *gb = nan({rows} * 16);\n\
           lk_gemv_f32<<<2, 256>>>(w, xg, nullptr, ga, nullptr, 0, 1024, 16, {rows}); CK(cudaGetLastError());\n\
           lk_gemv_f32<<<dim3(2, {}), 256>>>(w, xg, nullptr, gb, nullptr, 0, 1024, 16, {rows}); CK(cudaGetLastError());\n\
           CK(cudaDeviceSynchronize()); put(out, ga, {rows} * 16); put(out, gb, {rows} * 16);\n\
           lk_gemv_f32<<<{}, 256>>>(w, xg, nullptr, ga, nullptr, 0, 1024, 16, {rows}, {parts}); CK(cudaGetLastError());\n\
           lk_gemv_f32<<<dim3({}, {}), 256>>>(w, xg, nullptr, gb, nullptr, 0, 1024, 16, {rows}, {parts}); CK(cudaGetLastError());\n\
           CK(cudaDeviceSynchronize()); put(out, ga, {rows} * 16); put(out, gb, {rows} * 16);\n\
           fclose(out);\n  return 0;\n}}\n",
        Kernel::Conv.source(),
        Kernel::GemvF32.source(),
        rows.div_ceil(GEMV_F32_ROWS),
        16usize.div_ceil(8 / parts),
        16usize.div_ceil(8 / parts),
        rows.div_ceil(GEMV_F32_ROWS)
    );
    let dir = dir().join("conv-rows");
    std::fs::create_dir_all(&dir).unwrap();
    let (cu, bin, outp) = (dir.join("conv.cu"), dir.join("conv"), dir.join("out.bin"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "the conv and the f32 GEMV build:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).arg(&outp).output().unwrap();
    assert!(ran.status.success(), "they run: {}", String::from_utf8_lossy(&ran.stderr));
    let all: Vec<u32> = std::fs::read(&outp).unwrap().chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    let mut at = 0;
    let mut take = |n: usize| {
        at += n;
        all[at - n..at].to_vec()
    };
    for (what, taps) in &cases {
        let (ya, yb, ra, rb) = (take(rows * C), take(rows * C), take(taps * C), take(taps * C));
        assert!(ya.iter().all(|b| f32::from_bits(*b).is_finite()), "{what}: every row's outputs written, and finite");
        let first = ya.iter().zip(&yb).position(|(a, b)| a != b);
        assert_eq!(first, None, "{what}: every output as the rows one after another compute it, the first apart at {first:?} (row {:?})", first.map(|i| i / C));
        assert_eq!(rb, ra, "{what}: the ring the rows leave");
    }
    let (ga, gb) = (take(rows * 16), take(rows * 16));
    assert!(ga.iter().all(|b| f32::from_bits(*b).is_finite()), "the f32 GEMV writes every row");
    assert_eq!(gb, ga, "the f32 GEMV's row groups compute what one block of every row does");
    let (pa, pb) = (take(rows * 16), take(rows * 16));
    assert!(pa.iter().all(|b| f32::from_bits(*b).is_finite()), "the f32 GEMV cut over {parts} warps a row writes every row");
    assert_eq!(pb, pa, "the f32 GEMV cut over {parts} warps a row: its row groups compute what one block of every row does");
    assert_eq!(at, all.len(), "every result read");
}

/// A wide chunk's `f32` projection in tiles (`lk_gemm_f32`) computes the `f64` product of its rows and weights to within
/// `√k·ε` of each output's `Σ|x·w|`, a sum of `k` terms each rounding to `f32` — the GEMV (`lk_gemv_f32`, its sum in
/// `f64`) to within its sum's rounding and its bias's, a warp a row and each row's `k` cut over the warps the lane gives
/// it (`gemv_f32_parts`) — whole and cut into the
/// parts its shape takes on the chip (`gemm_f32_splits`): Qwen3.5's 16 gates over 1024 and 2048 values from the narrowest
/// wide chunk (17 rows) to a prompt's (512), outputs and rows past a tile's edge (48 × 100, 130 × 130), a `k` ending
/// inside a staged chunk (100), with and without a bias. The cut tiles sum their parts in order whichever arrives last,
/// so a second run computes the same bits, its arrival counts left at zero by the first.
#[test]
fn an_f32_projection_in_tiles_computes_what_its_f64_product_computes() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{gemm_f32_splits, gemv_f32_parts, Kernel, GEMV_F32_ROWS, PRELUDE};
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let shapes = [(17usize, 16usize, 1024usize, false), (64, 16, 1024, false), (100, 48, 1024, true), (512, 16, 1024, false), (512, 16, 2048, true), (130, 130, 100, true)];
    let sms = CudaKnobs::default().chip.sms;
    let mut calls = String::new();
    let mut cases = Vec::new();
    for (i, &(m, n, k, biased)) in shapes.iter().enumerate() {
        let tiles = n.div_ceil(64) * m.div_ceil(64);
        let cut = gemm_f32_splits(tiles, sms, k.div_ceil(32));
        assert!(cut > 1, "{m} × {k} by {n}: the chip cuts its tiles");
        let parts = gemv_f32_parts(n, k, sms);
        let bias = if biased { "b" } else { "nullptr" };
        let mut runs = String::new();
        for split in [1, cut, cut] {
            let _ = write!(
                runs,
                "    {{ float* y = nan({m}L * {n}); lk_gemm_f32<<<dim3({}, {}, {split}), 256>>>(x, w, {bias}, y, {m}, {n}, {k}, {split}, part, cnt); CK(cudaGetLastError());\n\
                 \x20     CK(cudaDeviceSynchronize()); put(out, y, {m}L * {n}); }}\n",
                n.div_ceil(64),
                m.div_ceil(64)
            );
        }
        let _ = write!(
            calls,
            "  {{ // {m} × {k} by {n}\n\
             \x20   float *x = seeded({m}L * {k}, {}), *w = seeded({n}L * {k}, {}), *b = seeded({n}, {}), *g = nan({m}L * {n});\n\
             \x20   float* part = nan({tiles}L * {cut} * 256 * 16); int* cnt; CK(cudaMalloc(&cnt, {tiles} * 4)); CK(cudaMemset(cnt, 0, {tiles} * 4));\n\
             \x20   put(out, x, {m}L * {k}); put(out, w, {n}L * {k}); put(out, b, {n});\n\
             \x20   lk_gemv_f32<<<dim3({}, {}), 256>>>(w, x, {bias}, g, nullptr, 0, {k}, {n}, {m}); CK(cudaGetLastError());\n\
             \x20   CK(cudaDeviceSynchronize()); put(out, g, {m}L * {n});\n\
             \x20   lk_gemv_f32<<<dim3({}, {}), 256>>>(w, x, {bias}, g, nullptr, 0, {k}, {n}, {m}, {parts}); CK(cudaGetLastError());\n\
             \x20   CK(cudaDeviceSynchronize()); put(out, g, {m}L * {n});\n\
             {runs}\
             \x20   std::vector<int> h({tiles}); CK(cudaMemcpy(h.data(), cnt, {tiles} * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, {tiles}, out);\n  }}\n",
            3 * i + 1,
            3 * i + 2,
            3 * i + 3,
            n.div_ceil(8),
            m.div_ceil(GEMV_F32_ROWS),
            n.div_ceil(8 / parts),
            m.div_ceil(GEMV_F32_ROWS)
        );
        cases.push((m, n, k, biased, tiles));
    }
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {PRELUDE}{}{}\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ fprintf(stderr, \"cuda %d %s\\n\", __LINE__, cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         static float* seeded(long n, unsigned salt){{ std::vector<float> h(n); for(long i = 0; i < n; ++i) h[i] = (float)((((unsigned)i * 2654435761u + salt * 40503u) >> 7) % 20011) / 10000.f - 1.f;\n\
           float* d; CK(cudaMalloc(&d, n * 4)); CK(cudaMemcpy(d, h.data(), n * 4, cudaMemcpyHostToDevice)); return d; }}\n\
         static float* nan(long n){{ float* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0xff, n * 4)); return p; }}\n\
         static void put(FILE* out, const float* d, long n){{ std::vector<float> h(n); CK(cudaMemcpy(h.data(), d, n * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, n, out); }}\n\
         int main(int argc, char** argv){{\n\
           FILE* out = fopen(argv[1], \"wb\");\n{calls}\
           fclose(out);\n  return 0;\n}}\n",
        Kernel::GemvF32.source(),
        Kernel::GemmF32.source()
    );
    let dir = dir().join("gemm-f32");
    std::fs::create_dir_all(&dir).unwrap();
    let (cu, bin, outp) = (dir.join("gemm.cu"), dir.join("gemm"), dir.join("out.bin"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "the f32 GEMM builds:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).arg(&outp).output().unwrap();
    assert!(ran.status.success(), "it runs: {}", String::from_utf8_lossy(&ran.stderr));
    let all: Vec<u32> = std::fs::read(&outp).unwrap().chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    let mut at = 0;
    let mut take = |n: usize| {
        at += n;
        all[at - n..at].to_vec()
    };
    let f = |v: Vec<u32>| v.into_iter().map(f32::from_bits).collect::<Vec<f32>>();
    for (m, n, k, biased, tiles) in cases {
        let what = format!("{m} × {k} by {n}{}", if biased { " with a bias" } else { "" });
        let (x, w, b) = (f(take(m * k)), f(take(n * k)), f(take(n)));
        let gemv = f(take(m * n));
        let parted = f(take(m * n));
        let (whole, cut, again) = (take(m * n), take(m * n), take(m * n));
        let counts = take(tiles);
        let mut worst = [0f64; 3];
        for r in 0..m {
            for o in 0..n {
                let (mut sum, mut mag) = (0f64, 0f64);
                for j in 0..k {
                    let p = x[r * k + j] as f64 * w[o * k + j] as f64;
                    sum += p;
                    mag += p.abs();
                }
                let products = sum;
                if biased {
                    sum += b[o] as f64;
                    mag += (b[o] as f64).abs();
                }
                let i = r * n + o;
                let rounded = f32::EPSILON as f64 * (products.abs() + if biased { sum.abs() } else { 0.0 });
                for (how, got) in [("a warp a row", &gemv), ("its rows cut over warps", &parted)] {
                    let g = got[i] as f64;
                    assert!((g - sum).abs() <= rounded, "{what}: the GEMV's ({r}, {o}), {how}, {g} is the f64 sum {sum} rounded");
                }
                for (slot, got) in [&whole, &cut].into_iter().enumerate() {
                    let y = f32::from_bits(got[i]) as f64;
                    assert!(y.is_finite(), "{what}: ({r}, {o}) written");
                    worst[slot] = worst[slot].max((y - sum).abs() / mag);
                }
            }
        }
        let tolerance = (k as f64).sqrt() * f32::EPSILON as f64;
        eprintln!("{what}: whole within {:.2e}, cut within {:.2e} of Σ|x·w| (√k·ε {tolerance:.2e})", worst[0], worst[1]);
        assert!(worst[0] <= tolerance && worst[1] <= tolerance, "{what}: within {tolerance:e} of Σ|x·w|: {worst:?}");
        assert_eq!(again, cut, "{what}: the cut tiles sum their parts in one order whichever arrives last");
        assert!(counts.iter().all(|&c| c == 0), "{what}: the arrival counts left at zero");
    }
    assert_eq!(at, all.len(), "every result read");
}

/// The chunked delta rule (`lk_delta_prep`, then `lk_delta_state`) computes what the delta rule a row at a time
/// computes (`lk_delta_step`): four value heads over two key heads, heads 64 and 128 wide, keys and queries of unit
/// length, write strengths in (0, 1) and decays in (0.5, 1), a memory left by earlier rows — 64 rows (one whole chunk),
/// 100 (a partial chunk after a whole one) and 512 (a prompt's chunk). Each output within `TOLERANCE` of its head's
/// largest and each row of the memory it leaves within `TOLERANCE` of that row's largest, the difference of two
/// summation orders; every output written (they start as NaN).
#[test]
fn the_chunked_delta_rule_computes_what_the_delta_rule_a_row_at_a_time_computes() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{delta_chunk_scratch, Kernel, DELTA_CHUNK, DELTA_PREP_SMEM, DELTA_STATE_ROWS, DELTA_STATE_THREADS, PRELUDE};
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    const TOLERANCE: f64 = 1e-5;
    const HV: usize = 4;
    const NK: usize = 2;
    let unit = |i: usize, salt: u64| {
        let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(27) ^ 0x2545_F491_4F6C_DD1D;
        (h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    };
    let dir = dir().join("delta");
    std::fs::create_dir_all(&dir).unwrap();
    let write = |name: String, v: &[f32]| std::fs::write(dir.join(name), v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    let mut calls = String::new();
    let mut cases = Vec::new();
    for d in [64usize, 128] {
        for rows in [64usize, 100, 512] {
            let tag = format!("{d}-{rows}");
            // Unit-length keys and queries, as the conv's L2 norm leaves them.
            let normed = |salt: u64| -> Vec<f32> {
                (0..rows * NK)
                    .flat_map(|u| {
                        let x: Vec<f64> = (0..d).map(|i| unit(u * d + i, salt)).collect();
                        let n = x.iter().map(|v| v * v).sum::<f64>().sqrt();
                        x.into_iter().map(move |v| (v / n) as f32)
                    })
                    .collect()
            };
            write(format!("q{tag}.bin"), &normed(1));
            write(format!("k{tag}.bin"), &normed(2));
            write(format!("v{tag}.bin"), &(0..rows * HV * d).map(|i| unit(i, 3) as f32).collect::<Vec<f32>>());
            write(format!("b{tag}.bin"), &(0..rows * HV).map(|i| (1.0 / (1.0 + (-2.0 * unit(i, 4)).exp())) as f32).collect::<Vec<f32>>());
            write(format!("a{tag}.bin"), &(0..rows * HV).map(|i| (0.75 + 0.25 * unit(i, 5)) as f32).collect::<Vec<f32>>());
            write(format!("s{tag}.bin"), &(0..HV * d * d).map(|i| (0.1 * unit(i, 6)) as f32).collect::<Vec<f32>>());
            let chunks = rows.div_ceil(DELTA_CHUNK);
            let [kn, qn, tn, pn, gn] = delta_chunk_scratch(chunks, HV, d);
            let _ = write!(
                calls,
                "  {{ // heads {d} wide, {rows} rows\n\
                 \x20   float *q = dev(dir + \"/q{tag}.bin\"), *k = dev(dir + \"/k{tag}.bin\"), *v = dev(dir + \"/v{tag}.bin\"), *b = dev(dir + \"/b{tag}.bin\"), *a = dev(dir + \"/a{tag}.bin\");\n\
                 \x20   float *sa = dev(dir + \"/s{tag}.bin\"), *sb = dev(dir + \"/s{tag}.bin\"), *oa = nan({rows} * {HV} * {d}), *ob = nan({rows} * {HV} * {d});\n\
                 \x20   lk_delta_step<float><<<dim3({HV}, {}), 128>>>(q, k, v, b, a, sa, oa, {d}, {NK}, {HV}, {rows}); CK(cudaGetLastError());\n\
                 \x20   float *kg = nan({kn}), *qg = nan({qn}), *tb = nan({tn}), *p = nan({pn}), *g = nan({gn});\n\
                 \x20   lk_delta_prep<{d}><<<dim3({HV}, {chunks}), 256, {DELTA_PREP_SMEM}>>>(q, k, b, a, kg, qg, tb, p, g, {NK}, {HV}, {rows}); CK(cudaGetLastError());\n\
                 \x20   lk_delta_state<{d}, {DELTA_STATE_ROWS}><<<dim3({HV}, {}), {DELTA_STATE_THREADS}>>>(k, v, kg, qg, tb, p, g, sb, ob, {NK}, {HV}, {rows}); CK(cudaGetLastError());\n\
                 \x20   CK(cudaDeviceSynchronize()); put(out, oa, {rows} * {HV} * {d}); put(out, ob, {rows} * {HV} * {d}); put(out, sa, {HV} * {d} * {d}); put(out, sb, {HV} * {d} * {d});\n  }}\n",
                d.div_ceil(4),
                d / DELTA_STATE_ROWS
            );
            cases.push((format!("heads {d} wide, {rows} rows"), rows, d));
        }
    }
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <string>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {PRELUDE}{}{}\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ fprintf(stderr, \"cuda %d %s\\n\", __LINE__, cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         static float* dev(const std::string& p){{ FILE* f = fopen(p.c_str(), \"rb\"); if(!f) exit(2); std::vector<char> b; char c[65536]; size_t n;\n\
           while((n = fread(c, 1, sizeof c, f)) > 0) b.insert(b.end(), c, c + n); fclose(f);\n\
           float* d; CK(cudaMalloc(&d, b.size())); CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice)); return d; }}\n\
         static float* nan(long n){{ float* p; CK(cudaMalloc(&p, n * 4)); CK(cudaMemset(p, 0xff, n * 4)); return p; }}\n\
         static void put(FILE* out, const float* d, long n){{ std::vector<float> h(n); CK(cudaMemcpy(h.data(), d, n * 4, cudaMemcpyDeviceToHost)); fwrite(h.data(), 4, n, out); }}\n\
         int main(int argc, char** argv){{\n  std::string dir = argv[1];\n  FILE* out = fopen(argv[2], \"wb\");\n{calls}  fclose(out);\n  return 0;\n}}\n",
        Kernel::DeltaStep.source(),
        Kernel::DeltaChunks.source()
    );
    let (cu, bin, outp) = (dir.join("delta.cu"), dir.join("delta"), dir.join("out.bin"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "both delta rules build:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).arg(&dir).arg(&outp).output().unwrap();
    assert!(ran.status.success(), "they run: {}", String::from_utf8_lossy(&ran.stderr));
    let all: Vec<f32> = std::fs::read(&outp).unwrap().chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
    let mut at = 0;
    let mut take = |n: usize| {
        at += n;
        all[at - n..at].to_vec()
    };
    let mut report = Vec::new();
    for (what, rows, d) in &cases {
        let (oa, ob, sa, sb) = (take(rows * HV * d), take(rows * HV * d), take(HV * d * d), take(HV * d * d));
        let mut worst = (0f64, 0f64);
        for (name, a, b, worst) in [("output", &oa, &ob, &mut worst.0), ("memory row", &sa, &sb, &mut worst.1)] {
            for (u, (x, y)) in a.chunks_exact(*d).zip(b.chunks_exact(*d)).enumerate() {
                assert!(x.iter().chain(y).all(|f| f.is_finite()), "{what}: {name} {u}: every value written, and finite");
                let largest = x.iter().fold(0f64, |m, f| m.max(f64::from(f.abs())));
                for (e, (p, q)) in x.iter().zip(y).enumerate() {
                    let off = f64::from((p - q).abs()) / largest.max(1e-30);
                    *worst = worst.max(off);
                    assert!(off <= TOLERANCE, "{what}: {name} {u} value {e}: a row at a time {p}, in chunks {q} ({off:.2e} of the largest)");
                }
            }
        }
        report.push(format!("{what}: outputs within {:.2e}, memory within {:.2e}", worst.0, worst.1));
    }
    assert_eq!(at, all.len(), "every result read");
    eprintln!("{}", report.join("\n"));
}

/// The decode step's expert down projection with several outputs a block (`lk_moe_down`'s `W`, its shared expert's rows
/// over `SW` warps) computes, bit for bit, what a block of one output computes, and that is the down projection: three
/// kept experts' rows 768 wide in every weight format with their biases, then Q8_0 and Q5_0 beside a shared expert as
/// wide as an expert's rows (Q4_K, one warp) and twice as wide (Q6_K, a warp a row or two), under the precise, one- and
/// two-word numerics — with two and four outputs a block, and the planner's choice (`kernels::moe_down_rows`) — every
/// block's scale its own (no power of two hides a product's order) and each output within `TOLERANCE` of the largest of
/// an `f64` sum over the dequantized weights and the activations as the numerics read them (the `f32` values, or the
/// int8 words the GPU quantized, the low word an eighth of a level's 1/16).
#[test]
fn the_expert_down_takes_several_outputs_a_block_as_a_block_of_one_does() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{format_arg, moe_down_rows, Kernel, PRELUDE};
    use logicaffeine_compile::model::quant::{f16_to_f32, GgmlType};
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    const TOLERANCE: f64 = 1e-5;
    const E: usize = 4;
    const K: usize = 3;
    const D: usize = 256;
    const DFF: usize = 768;
    const SDFF: usize = 1536;
    let formats: Vec<GgmlType> = std::iter::once(GgmlType::Q8_0).chain(BLOCK_FORMATS).collect();
    let unit = |i: usize, salt: u64| {
        let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(23) ^ 0x2545_F491_4F6C_DD1D;
        (h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    };
    let dir = dir().join("down-rows");
    std::fs::create_dir_all(&dir).unwrap();
    let floats = |name: &str, v: &[f32]| std::fs::write(dir.join(name), v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    let hidden = K * DFF + SDFF;
    let h: Vec<f32> = (0..hidden).map(|i| (unit(i, 1) * 3.0) as f32).collect();
    let bd: Vec<f32> = (0..E * D).map(|i| (unit(i, 2) * 0.1) as f32).collect();
    let (wt, idx) = ([0.5f32, 0.3, 0.2], [3usize, 0, 2]);
    floats("h.bin", &h);
    floats("bd.bin", &bd);
    floats("wt.bin", &wt);
    std::fs::write(dir.join("idx.bin"), idx.iter().flat_map(|&v| (v as i32).to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    // A block's scale as its seeded exponent with a mantissa of its own.
    let varied = |bits: u16, block: usize, salt: usize| (bits & 0xFC00) | ((block * 97 + salt * 31) as u16 & 0x3FF);
    let matrix = |name: &str, ty: GgmlType, rows: usize, cols: usize| -> Vec<u8> {
        if ty == GgmlType::Q8_0 {
            let levels = (0..rows * cols).map(|i| ((unit(i, name.len() as u64 * 7919) * 127.0) as i8) as u8);
            return levels.chain((0..rows * cols / 32).flat_map(|b| varied(0x2000, b, 1).to_le_bytes())).collect();
        }
        let mut bytes = tiny_models::seeded_blocks(name, ty, rows * cols);
        let size = ty.block_bytes();
        for (b, block) in bytes.chunks_exact_mut(size).enumerate() {
            let mut set = |at: usize, salt: usize| {
                let v = varied(u16::from_le_bytes([block[at], block[at + 1]]), b, salt);
                block[at..at + 2].copy_from_slice(&v.to_le_bytes());
            };
            match ty {
                GgmlType::Q4K | GgmlType::Q5K | GgmlType::Q4_1 | GgmlType::Q5_1 => {
                    set(0, 1);
                    set(2, 2);
                }
                GgmlType::Q6K => set(size - 2, 1),
                GgmlType::Q2K => {
                    set(80, 1);
                    set(82, 2);
                }
                GgmlType::Q3K => set(108, 1),
                GgmlType::Q4_0 | GgmlType::Q5_0 => set(0, 1),
                _ => {}
            }
        }
        bytes
    };
    // The weights as the CPU dequantizer decodes them (a Q8_0 matrix as its levels, then a scale a block).
    let weights = |ty: GgmlType, bytes: &[u8], n: usize| -> Vec<f64> {
        if ty == GgmlType::Q8_0 {
            let scales: Vec<f64> = bytes[n..].chunks_exact(2).map(|c| f64::from(f16_to_f32(u16::from_le_bytes([c[0], c[1]])))).collect();
            return (0..n).map(|i| f64::from(bytes[i] as i8) * scales[i / 32]).collect();
        }
        let mut out = Vec::new();
        ty.dequantize(bytes, &mut out);
        out.into_iter().map(f64::from).collect()
    };
    // (routed format, shared format, shared rows, biases, outputs a block, shared warps)
    let mut cases: Vec<(GgmlType, GgmlType, usize, bool, usize, usize)> = Vec::new();
    let mut routed = std::collections::HashMap::new();
    for &ty in &formats {
        let bytes = matrix(&format!("wd{}", ty.name()), ty, E * D, DFF);
        std::fs::write(dir.join(format!("wd-{}.bin", ty.name())), &bytes).unwrap();
        routed.insert(ty.name(), weights(ty, &bytes, E * D * DFF));
        for w in [1, 2, 4] {
            cases.push((ty, ty, 0, true, w, 1));
        }
    }
    let mut shared = std::collections::HashMap::new();
    for (sty, sdff) in [(GgmlType::Q4K, DFF), (GgmlType::Q6K, SDFF)] {
        let bytes = matrix(&format!("sd{}", sty.name()), sty, D, sdff);
        std::fs::write(dir.join(format!("sd-{}.bin", sty.name())), &bytes).unwrap();
        shared.insert(sty.name(), weights(sty, &bytes, D * sdff));
        for ty in [GgmlType::Q8_0, GgmlType::Q5_0] {
            let planned = moe_down_rows(D, DFF, sdff);
            let mut shapes = vec![(1, 1), (2, 1), (2, 2), (4, 1), (4, 2), (4, 4), planned];
            shapes.sort_unstable();
            shapes.dedup();
            cases.extend(shapes.into_iter().map(|(w, sw)| (ty, sty, sdff, false, w, sw)));
        }
    }
    assert!(cases.contains(&(GgmlType::Q8_0, GgmlType::Q6K, SDFF, false, 4, 4)), "the planner gives the wider shared expert a warp a row");
    let calls: String = [0, 1, 2]
        .iter()
        .flat_map(|&q| cases.iter().map(move |c| (q, *c)))
        .map(|(q, (ty, sty, sdff, bias, w, sw))| {
            let (f, sf) = (format_arg(ty), format_arg(sty));
            let threads = 32 * (K + if sdff > 0 { sw } else { 0 });
            format!(
                "  {{ lk_w wd = load<{f}>(dir + \"/wd-{}.bin\", {}L, {DFF}, {}L);\n\
                 \x20   lk_w sd = {};\n\
                 \x20   float* y = nan<float>({D});\n\
                 \x20   lk_moe_down<{q}, {f}, {sf}, {w}, {sw}><<<{}, {threads}>>>(h, hq{q}, hl{q}, hs{q}, wd, sd, {}, idx, wt, y, {D}, {DFF}, {K}, {sdff});\n\
                 \x20   CK(cudaGetLastError()); CK(cudaDeviceSynchronize()); put(out, y, {D}); }}\n",
                ty.name(),
                E * D,
                ty.bytes_for(DFF),
                if sdff > 0 { format!("load<{sf}>(dir + \"/sd-{}.bin\", {D}L, {sdff}, {}L)", sty.name(), sty.bytes_for(sdff)) } else { "lk_w{}".into() },
                D / w,
                if bias { "bd" } else { "nullptr" }
            )
        })
        .collect();
    let program = format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <string>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {PRELUDE}{}{}{}\n\
         #define CK(x) do {{ cudaError_t e_ = (x); if(e_ != cudaSuccess){{ fprintf(stderr, \"cuda %d %s\\n\", __LINE__, cudaGetErrorString(e_)); exit(1); }} }} while(0)\n\
         static std::vector<char> slurp(const std::string& p){{ FILE* f = fopen(p.c_str(), \"rb\"); if(!f) exit(2); std::vector<char> b; char c[65536]; size_t n; \
           while((n = fread(c, 1, sizeof c, f)) > 0) b.insert(b.end(), c, c + n); fclose(f); return b; }}\n\
         template<typename T> static T* dev(const std::string& p){{ auto b = slurp(p); T* d; CK(cudaMalloc(&d, b.size() + 32)); \
           CK(cudaMemset(d, 0, b.size() + 32)); CK(cudaMemcpy(d, b.data(), b.size(), cudaMemcpyHostToDevice)); return d; }}\n\
         template<int F> static lk_w load(const std::string& p, long rows, int cols, long row_bytes){{\n\
           if(F == 8){{ auto b = slurp(p); signed char* q; __half* s; long n = rows * cols;\n\
             CK(cudaMalloc(&q, n)); CK(cudaMalloc(&s, n / 32 * 2)); CK(cudaMemcpy(q, b.data(), n, cudaMemcpyHostToDevice));\n\
             CK(cudaMemcpy(s, b.data() + n, n / 32 * 2, cudaMemcpyHostToDevice)); return lk_w{{q, s, nullptr, 0}}; }}\n\
           return lk_w{{nullptr, nullptr, dev<unsigned char>(p), row_bytes}};\n\
         }}\n\
         template<typename T> static T* nan(long n){{ T* p; CK(cudaMalloc(&p, n * sizeof(T))); CK(cudaMemset(p, 0xff, n * sizeof(T))); return p; }}\n\
         template<typename T> static void put(FILE* out, const T* d, long n){{ std::vector<T> h(n); CK(cudaMemcpy(h.data(), d, n * sizeof(T), cudaMemcpyDeviceToHost)); fwrite(h.data(), sizeof(T), n, out); }}\n\
         int main(int argc, char** argv){{\n  std::string dir = argv[1];\n\
           float* h = dev<float>(dir + \"/h.bin\"); float* bd = dev<float>(dir + \"/bd.bin\"); float* wt = dev<float>(dir + \"/wt.bin\"); int* idx = dev<int>(dir + \"/idx.bin\");\n\
           signed char *hq0 = nullptr, *hl0 = nullptr; float* hs0 = nullptr;\n\
           signed char *hq1 = nan<signed char>({hidden}), *hl1 = nullptr; float* hs1 = nan<float>({hidden} / 32);\n\
           lk_quant<1><<<({hidden} + 255) / 256, 256>>>(h, hq1, hl1, hs1, {hidden}); CK(cudaGetLastError());\n\
           signed char *hq2 = nan<signed char>({hidden}), *hl2 = nan<signed char>({hidden}); float* hs2 = nan<float>({hidden} / 32);\n\
           lk_quant<2><<<({hidden} + 255) / 256, 256>>>(h, hq2, hl2, hs2, {hidden}); CK(cudaGetLastError());\n\
           FILE* out = fopen(argv[2], \"wb\");\n\
           put(out, hq1, {hidden}); put(out, hs1, {hidden} / 32); put(out, hq2, {hidden}); put(out, hl2, {hidden}); put(out, hs2, {hidden} / 32);\n\
         {calls}  fclose(out);\n  return 0;\n}}\n",
        Kernel::Blocks.source(),
        Kernel::Quant.source(),
        Kernel::Moe.source()
    );
    let (cu, bin, out) = (dir.join("down.cu"), dir.join("down"), dir.join("out.bin"));
    std::fs::write(&cu, program).unwrap();
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().unwrap();
    assert!(built.status.success(), "every shape of the down projection builds:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).arg(&dir).arg(&out).output().unwrap();
    assert!(ran.status.success(), "they run: {}", String::from_utf8_lossy(&ran.stderr));
    let bytes = std::fs::read(&out).unwrap();
    let mut at = 0;
    let mut take = |n: usize| {
        at += n;
        bytes[at - n..at].to_vec()
    };
    let f32s = |b: Vec<u8>| -> Vec<f32> { b.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect() };
    let (hq1, hs1, hq2, hl2, hs2) = (take(hidden), f32s(take(hidden / 32 * 4)), take(hidden), take(hidden), f32s(take(hidden / 32 * 4)));
    // The activations as each numerics reads them.
    let acts: [Vec<f64>; 3] = [
        h.iter().map(|&v| f64::from(v)).collect(),
        (0..hidden).map(|i| f64::from(hq1[i] as i8) * f64::from(hs1[i / 32])).collect(),
        (0..hidden).map(|i| (f64::from(hq2[i] as i8) + f64::from(hl2[i] as i8) * 0.0078125) * f64::from(hs2[i / 32])).collect(),
    ];
    for q in 0..3 {
        let x = &acts[q];
        let mut one: Vec<u32> = Vec::new();
        for &(ty, sty, sdff, bias, w, sw) in &cases {
            let y: Vec<u32> = take(D * 4).chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
            let what = format!("Q{q} {} beside {} of {sdff}{}, {w} outputs a block, {sw} shared warps", ty.name(), sty.name(), if bias { ", biased" } else { "" });
            assert!(y.iter().all(|b| f32::from_bits(*b).is_finite()), "{what}: every output written, and finite");
            if w > 1 {
                let first = one.iter().zip(&y).position(|(a, b)| a != b);
                assert_eq!(first, None, "{what}: every output as a block of one output computes it, the first apart at {first:?}");
                continue;
            }
            let wd = &routed[ty.name()];
            let want: Vec<f64> = (0..D)
                .map(|o| {
                    let experts: f64 = (0..K)
                        .map(|r| {
                            let row = idx[r] * D + o;
                            let dot: f64 = (0..DFF).map(|j| wd[row * DFF + j] * x[r * DFF + j]).sum();
                            f64::from(wt[r]) * (dot + if bias { f64::from(bd[row]) } else { 0.0 })
                        })
                        .sum();
                    experts + if sdff > 0 { (0..sdff).map(|j| shared[sty.name()][o * sdff + j] * x[K * DFF + j]).sum::<f64>() } else { 0.0 }
                })
                .collect();
            let largest = want.iter().fold(0f64, |m, v| m.max(v.abs()));
            assert!(largest > 0.0, "{what}: the projection moves something");
            for (o, (got, want)) in y.iter().map(|b| f64::from(f32::from_bits(*b))).zip(&want).enumerate() {
                let off = (got - want).abs() / largest;
                assert!(off <= TOLERANCE, "{what}: output {o} is {got}, the down projection {want} ({off:.2e} of the largest)");
            }
            one = y;
        }
    }
    assert_eq!(at, bytes.len(), "every case's outputs read");
}


/// Every format a matrix comes in: wide models (whole K-quant super-blocks) written with every quantized matrix in a
/// different format by turns — Q2_K through Q6_K, Q4_0 through Q5_1, MXFP4, IQ4_NL and IQ4_XS — generate what the reference decoding the
/// same blocks computes, under every numerics, the dense step's projections, the expert mixtures' rows and the
/// embedding table alike; and a prompt run in 32-row chunks continues as the same prompt fed a token at a time — bit
/// for bit under the precise numerics (the block GEMV's 16-row slabs), and under the two-word numerics, whose chunk
/// multiplies on the int8 tensor cores and attends in tiles, held to the reference as the token-at-a-time run is. Under
/// one word a 40-token prompt makes more roundings near a half-way point than the near calls the reference may take
/// the other way can account for, fed a token at a time or in chunks alike, so the one-word chunk's kernels are held to
/// the row kernels one by one (`the_grouped_expert_gemms_compute_what_the_slabs_compute`,
/// `the_tiled_attention_computes_what_the_attention_computes`,
/// `the_tiled_per_head_products_compute_what_the_slabs_compute`, `the_router_as_a_gemm_routes_as_the_router_does`).
#[test]
fn every_weight_format_generates_what_the_reference_computes() {
    use logicaffeine_compile::model::quant::GgmlType;
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let ctx = 64;
    let mut report = Vec::new();
    for t in [Tiny::Qwen3, Tiny::Qwen3Moe, Tiny::GptOss, Tiny::DeepSeek2] {
        let spec = t.wide(ctx);
        let ckpt = dir().join(format!("{}-formats.gguf", t.name()));
        let matrices: Vec<String> = tiny_models::tensors(&spec)
            .into_iter()
            .map(|(n, _)| n)
            .filter(|n| n.ends_with(".weight") && !n.contains("norm") && !n.contains("attn_sinks"))
            .filter(|n| !(spec.arch == "gpt-oss" && n.contains("ffn_gate_inp")))
            .collect();
        tiny_models::write_checkpoint_as(&spec, &ckpt, &|name| match matrices.iter().position(|m| m == name) {
            Some(k) => BLOCK_FORMATS[k % BLOCK_FORMATS.len()],
            None => GgmlType::F32,
        });
        for numerics in [Q8MatKind::Dequant, Q8MatKind::TwoPassInt8, Q8MatKind::Int8Act] {
            let what = format!("{} formats {numerics:?}", t.name());
            let knobs = CudaKnobs { activations: numerics, ..CudaKnobs::default() };
            let lane = compile(&spec, &ckpt, &LaneOptions { ctx, knobs, prefill: 1 }).unwrap_or_else(|e| panic!("{what}: {e}"));
            let r = run(&lane, &ckpt, PROMPT, GENERATE, &format!("{}-formats-{numerics:?}", t.name()), spec.vocab, true);
            let (worst, flips) = held_to_reference(&spec, &ckpt, PROMPT, &r, numerics, &what);
            report.push(format!("{what}: logits within {worst:.3e} of their scale, {flips} near calls decided the other way"));
            if numerics == Q8MatKind::Int8Act {
                continue;
            }
            let prompt: Vec<u32> = (0..40u32).map(|i| (i * 37 + 5) % tiny_models::VOCAB as u32).collect();
            let runs: Vec<Run> = [1, 32]
                .iter()
                .map(|&prefill| {
                    let lane = compile(&spec, &ckpt, &LaneOptions { ctx, knobs, prefill }).unwrap();
                    run(&lane, &ckpt, &prompt, 6, &format!("{}-formats-{numerics:?}-{prefill}", t.name()), spec.vocab, true)
                })
                .collect();
            if numerics == Q8MatKind::Dequant {
                assert_eq!(runs[1].tokens, runs[0].tokens, "{what}: the chunked prompt continues alike");
                assert_eq!(runs[1].logits, runs[0].logits, "{what}: bit for bit");
            } else {
                let (worst, flips) = held_to_reference(&spec, &ckpt, &prompt, &runs[1], numerics, &format!("{what} in chunks"));
                report.push(format!("{what} in 32-row chunks: logits within {worst:.3e} of their scale, {flips} near calls decided the other way"));
            }
        }
    }
    eprintln!("{}", report.join("\n"));
}

/// A wide chunk's expert mixtures, multiplied on the int8 tensor cores expert by expert (the grouped `lk_gemm_blk`),
/// generate what the reference computes: the wide expert models with their matrices in Q8_0, Q4_K, Q6_K and MXFP4 by
/// turns, a prompt of 230 tokens run in chunks of 128, 64 and 32 rows — 256 choices of four experts in the first, so an
/// expert's rows fill a tile of 64 and spill past it — and the rest in slabs, held to the reference as a prompt fed a
/// token at a time is. Under the two-word numerics: under one word, a prompt this long makes more roundings near a
/// half-way point than the near calls the reference may take the other way can account for — the run fed a token at a
/// time parts from the reference as far — so the one-word GEMMs are held to the slabs kernel by kernel
/// (`the_grouped_expert_gemms_compute_what_the_slabs_compute`) and to the reference over shorter prompts
/// (`every_weight_format_generates_what_the_reference_computes`).
#[test]
fn a_wide_chunks_experts_on_the_tensor_cores_generate_what_the_reference_computes() {
    use logicaffeine_compile::model::quant::GgmlType;
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    const FORMATS: [GgmlType; 4] = [GgmlType::Q8_0, GgmlType::Q4K, GgmlType::Q6K, GgmlType::Mxfp4];
    let ctx = 256;
    let prompt: Vec<u32> = (0..230u32).map(|i| (i * 53 + 7) % tiny_models::VOCAB as u32).collect();
    let mut report = Vec::new();
    for t in [Tiny::Qwen3Moe, Tiny::GptOss, Tiny::DeepSeek2] {
        let spec = t.wide(ctx);
        let ckpt = dir().join(format!("{}-grouped.gguf", t.name()));
        let matrices: Vec<String> = tiny_models::tensors(&spec)
            .into_iter()
            .map(|(n, _)| n)
            .filter(|n| n.ends_with(".weight") && !n.contains("norm") && !n.contains("attn_sinks"))
            .filter(|n| !(spec.arch == "gpt-oss" && n.contains("ffn_gate_inp")))
            .collect();
        tiny_models::write_checkpoint_as(&spec, &ckpt, &|name| match matrices.iter().position(|m| m == name) {
            Some(k) => FORMATS[k % FORMATS.len()],
            None => GgmlType::F32,
        });
        for numerics in [Q8MatKind::TwoPassInt8] {
            let what = format!("{} grouped {numerics:?}", t.name());
            let knobs = CudaKnobs { activations: numerics, ..CudaKnobs::default() };
            let lane = compile(&spec, &ckpt, &LaneOptions { ctx, knobs, prefill: 128 }).unwrap_or_else(|e| panic!("{what}: {e}"));
            let wide: Vec<usize> = lane.forms.iter().map(|f| f.rows).filter(|&r| r > 16).collect();
            assert_eq!(wide, [128, 64, 32], "{what}: the wide chunks");
            assert!(lane.program.cu.contains("lk_moe_glu<"), "{what}: the wide chunks' experts on the tensor cores");
            let r = run(&lane, &ckpt, &prompt, 6, &format!("{}-grouped-{numerics:?}", t.name()), spec.vocab, true);
            let (worst, flips) = held_to_reference(&spec, &ckpt, &prompt, &r, numerics, &what);
            report.push(format!("{what}: logits within {worst:.3e} of their scale, {flips} near calls decided the other way"));
        }
    }
    eprintln!("{}", report.join("\n"));
}
