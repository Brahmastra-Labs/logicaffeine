//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE CPU TIER (benchmarks/FUSIONS_WE_NEED.md, PARITY-CPU-SHIM).
//!
//! Each CPU instruction set answers the shared optimizer through the one trait (`codegen_cpu::shim::CpuShim`), and its
//! link emits a model's program for the host's cores: kernels in the instruction set the shim declares, called in
//! order, each one the CPU library's.
//!
//!   cargo nextest run -p logicaffeine-tests --test cpu_shim_lock
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

#[path = "common/lane_cuts.rs"]
mod lane_cuts;
#[path = "common/lane_corpus.rs"]
mod lane_corpus;
#[path = "common/fusion_corpus.rs"]
mod fusion_corpus;
#[path = "common/scratch.rs"]
mod scratch;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::process::Command;

use lane_corpus::every_lane_cut;
use tiny_models::Tiny;
use logicaffeine_compile::codegen_cuda::lane::weights::Placement;
use logicaffeine_compile::codegen_cpu::link::{link, BufKind, LinkError};
use logicaffeine_compile::model::ngram_hash::NgramHash;
use logicaffeine_compile::model::quant::{f16_to_f32, f32_to_f16, GgmlType};
use logicaffeine_compile::tensor_graph::eval::{eval, filled, Data};
use logicaffeine_compile::tensor_graph::fuse::{optimize, realizes};
use logicaffeine_compile::tensor_graph::graph::{Elem, Role, Storage, Sym, TensorGraph};

use logicaffeine_compile::codegen_cpu::kernels::{ATTN_SPLIT, LIBRARY_X86_AVX2_VNNI, LINKED_WHOLE};
use logicaffeine_compile::codegen_cpu::shim::{CpuActivations, CpuCache, CpuIsa, CpuShim};
use logicaffeine_compile::tensor_graph::numerics::{hadamard, Keep};
use logicaffeine_compile::tensor_graph::shim::{Shim, Support};
use logicaffeine_compile::tensor_graph::symbols::{ClassId, Exemplar, Kind, CLASSES};

/// Whether a refusal is a reason: a lowercase clause of several words that ends without a stop.
fn a_reason(why: &str) -> bool {
    why.split_whitespace().count() >= 4 && why.starts_with(|c: char| c.is_lowercase()) && !why.ends_with('.')
}

/// Every CPU instruction set under each of its numerics, its fused classes linked whole or apart, answers every
/// registered class — linked whole, run as its parts, or refused with its reason — under a name of its own
/// (`cpu:<isa>`, `cpu:<isa>:precise`, each `:f16-cache` with binary16 caches, `:int8-cache` with int8 rows and
/// `:int8-rotated-cache` with turned int8 rows, and `:apart` with fusion off). The classes it
/// links whole are exactly the ones the
/// CPU link takes whole (`codegen_cpu::kernels::LINKED_WHOLE`) — less every fused class where fusion is off, each then
/// its parts — so the shim never promises a kernel the library lacks and the library holds none the shim leaves unused. It declares its numerics: an activation a product by checkpoint rows reads in one word a
/// 32-value block, its dots exact in 32-bit integers, or as its values under the precise point; caches `f32`, binary16,
/// int8 rows or turned int8 rows as the point says, states `f32`. A CPU runs a whole step as its ops on one thread pool — no graph to capture, no layer to keep resident — and
/// refuses a reduction across hosts by name.
#[test]
fn every_class_has_a_cpu_answer() {
    let library: BTreeSet<ClassId> = LINKED_WHOLE.iter().copied().collect();
    let mut names = BTreeSet::new();
    for (isa, activations, cache, fuse) in CpuIsa::ALL
        .into_iter()
        .flat_map(|i| [(i, CpuActivations::OneWord), (i, CpuActivations::Values)])
        .flat_map(|(i, a)| CpuCache::ALL.map(|c| (i, a, c)))
        .flat_map(|(i, a, c)| [(i, a, c, true), (i, a, c, false)])
    {
        let shim = CpuShim { isa, activations, cache, fuse };
        let name = shim.name();
        assert!(name.starts_with("cpu:") && names.insert(name.clone()), "{name}: a CPU's name of its own");
        let mut native = BTreeSet::new();
        let mut apart = 0usize;
        for &class in ClassId::ALL {
            match shim.supports(class) {
                Support::Native => {
                    native.insert(class);
                }
                Support::Decompose => apart += 1,
                Support::Refuse(why) => assert!(a_reason(why), "{name} refuses `{class}` without its reason: `{why}`"),
            }
        }
        let whole: BTreeSet<ClassId> = library.iter().copied().filter(|c| fuse || c.class().kind != Kind::Fused).collect();
        assert_eq!(native, whole, "{name}: the classes it links whole are the CPU library's");
        assert!(apart > 0, "{name}: some fused class runs as its parts");
        for class in [ClassId::CudaGraphs, ClassId::PersistentLayerMegakernel, ClassId::LastLayerRows] {
            assert_eq!(shim.supports(class), Support::Decompose, "{name}: `{class}` runs as the step's ops on one thread pool");
        }
        assert!(matches!(shim.supports(ClassId::AllreduceNorm), Support::Refuse(_)), "{name}: one host reduces across no others");
        let c = shim.numerics().unwrap_or_else(|| panic!("{name}: the CPU declares its numerics"));
        assert_eq!(c.accumulator_bits, 32, "{name}: dots in 32-bit integers");
        let words = if activations == CpuActivations::OneWord { Some(1) } else { None };
        assert_eq!(c.activations.map(|s| s.words), words, "{name}: an activation in one word a block, or as its values");
        let kept = [Keep::F32, Keep::F16, Keep::Int8Rows, Keep::Int8RowsRotated][CpuCache::ALL.iter().position(|&x| x == cache).unwrap()];
        assert_eq!((c.cache, c.state), (kept, Keep::F32), "{name}: caches in their format, states in f32");
    }
    assert_eq!(names.len(), 2 * CpuCache::ALL.len() * 2 * CpuIsa::ALL.len(), "every instruction set answers under each of its numerics and cache formats, fused and apart");
}

/// A deterministic value in [-1, 1) for place `i` under `salt`.
fn unit(i: usize, salt: u64) -> f64 {
    let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(31) ^ 0x2545_F491_4F6C_DD1D;
    (h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
}

/// A tensor of the CPU program's outputs, read back from `<dir>/out/<name>.bin`.
fn read_back(dir: &std::path::Path, name: &str, kind: &BufKind) -> Data {
    let bytes = std::fs::read(dir.join("out").join(format!("{name}.bin"))).unwrap_or_default();
    match kind {
        BufKind::I64 => Data::Int(bytes.chunks_exact(8).map(|x| i64::from_le_bytes(x.try_into().unwrap())).collect()),
        BufKind::F16 => Data::Real(bytes.chunks_exact(2).map(|x| f64::from(f16_to_f32(u16::from_le_bytes(x.try_into().unwrap())))).collect()),
        BufKind::Int8Rows { unit, turned } => {
            let n = bytes.len() / (*unit + 2) * *unit;
            let (codes, scales) = bytes.split_at(n.min(bytes.len()));
            Data::Real(
                codes
                    .chunks_exact(*unit)
                    .zip(scales.chunks_exact(2))
                    .flat_map(|(c, s)| int8_values(&c.iter().map(|&b| b as i8).collect::<Vec<_>>(), u16::from_le_bytes([s[0], s[1]]), *turned))
                    .collect(),
            )
        }
        _ => Data::Real(bytes.chunks_exact(4).map(|x| f64::from(f32::from_le_bytes(x.try_into().unwrap()))).collect()),
    }
}

/// `program` built by the host's compiler with the library into `dir` and run there on four threads (with its trace,
/// every buffer each op writes after the op, where `trace`), its outputs in `<dir>/out/` — and a step still running five
/// minutes on, hung, killed and failed by name.
fn build_and_run(program: &logicaffeine_compile::codegen_cpu::link::CpuProgram, dir: &std::path::Path, what: &str, trace: bool) {
    std::fs::create_dir_all(dir.join("out")).unwrap();
    let (cpp, bin) = (dir.join("step.cpp"), dir.join("step"));
    std::fs::write(&cpp, program.source_with_file_driver()).unwrap();
    let built = Command::new("g++").args(["-O2", "-std=c++17", "-mavx2", "-mfma", "-mavxvnni", "-mf16c", "-pthread", "-o"]).arg(&bin).arg(&cpp).output().unwrap();
    assert!(built.status.success(), "{what}: the step builds:\n{}", String::from_utf8_lossy(&built.stderr));
    let mut run = Command::new(&bin);
    run.arg(dir).arg("4").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());
    if trace {
        run.arg("trace");
    }
    // A tiny cut's step takes milliseconds; one still running after its deadline is hung (a pool that never returns).
    let mut child = run.spawn().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            panic!("{what}: the step still runs after five minutes");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let mut err = String::new();
    if let Some(mut e) = child.stderr.take() {
        use std::io::Read as _;
        let _ = e.read_to_string(&mut err);
    }
    assert!(status.success(), "{what}: the step runs: {err}");
}

/// `f` over every item on a pool of the host's threads (at most 16: each item builds programs with the host's compiler,
/// one core each), items taken in turn; the first item to panic stops the pool from taking more, and its panic is the
/// call's.
fn each_on_threads<T: Sync>(items: &[T], f: impl Fn(usize, &T) + Sync) {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let (next, stop) = (AtomicUsize::new(0), AtomicBool::new(false));
    let first = std::sync::Mutex::new(None);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16);
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                while !stop.load(Ordering::Relaxed) {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(i, item))) {
                        stop.store(true, Ordering::Relaxed);
                        first.lock().unwrap().get_or_insert(e);
                    }
                }
            });
        }
    });
    if let Some(e) = first.into_inner().unwrap() {
        std::panic::resume_unwind(e);
    }
}

/// How far `got` lies from `want`: its worst difference over `want`'s largest magnitude for values; for integers 0
/// where every one is equal, else infinitely far.
fn distance(got: &Data, want: &Data) -> f64 {
    match (got, want) {
        (Data::Int(g), Data::Int(w)) => {
            if g.len() >= w.len() && g[..w.len()] == w[..] {
                0.0
            } else {
                f64::INFINITY
            }
        }
        (g, w) => {
            let (g, w) = (g.reals(), w.reals());
            let scale = w.iter().fold(0f64, |m, v| m.max(v.abs())).max(1e-30);
            if g.len() < w.len() {
                return f64::INFINITY;
            }
            g.iter().zip(&w).fold(0f64, |m, (g, e)| if g.is_finite() { m.max((g - e).abs() / scale) } else { f64::INFINITY })
        }
    }
}

/// The values the evaluator and the CPU program read for cut `c`, written into `dir` as the file driver reads them: the
/// checkpoint's rows in place on the CPU and dequantized on the evaluator's side (each head's block transposed where the
/// program declares the matrix so), the compiler's values, each declared tensor's fill, a position of `position`, a token
/// of 7, and every carried state a deterministic pattern.
fn stage(c: &lane_corpus::LaneCut, dir: &std::path::Path, position: i64) -> BTreeMap<String, Data> {
    std::fs::create_dir_all(dir.join("out")).unwrap();
    let ckpt = scratch::dir("logos_lane_corpus").join(format!("{}.gguf", c.what.split_whitespace().next().unwrap()));
    let file = std::fs::read(&ckpt).unwrap_or_default();
    let io = c.g.io.as_ref();
    let mut inputs: BTreeMap<String, Data> = BTreeMap::new();
    for (i, t) in c.g.tensors.values().enumerate() {
        let value: Option<Data> = match c.constants.get(&t.name) {
            Some(Placement::Rows(r)) => {
                let mut bytes = Vec::new();
                for run in &r.runs {
                    let at = run.offset as usize;
                    bytes.extend_from_slice(&file[at..at + run.rows * r.ty.bytes_for(r.cols)]);
                }
                std::fs::write(dir.join(format!("{}.bin", t.name)), &bytes).unwrap();
                let mut v = Vec::new();
                r.ty.dequantize(&bytes, &mut v);
                // A matrix declared the other way round is its heads' blocks each transposed — one block a product's,
                // a block a head for a per-head product's: program element (h·cc + e, c) is checkpoint row h·C + c's
                // value e, of C program columns and cc checkpoint columns.
                let meaning: Vec<f64> = if r.transposed {
                    let (cc, c_) = (r.cols, t.cols);
                    (0..t.rows * t.cols).map(|p| (p / c_, p % c_)).map(|(row, col)| f64::from(v[((row / cc) * c_ + col) * cc + row % cc])).collect()
                } else {
                    v.iter().map(|&x| f64::from(x)).collect()
                };
                Some(Data::Real(meaning))
            }
            Some(Placement::Values(v)) => Some(Data::Real(v.iter().map(|&x| f64::from(x)).collect())),
            None if io.is_some_and(|io| io.position == t.name) => Some(Data::Int(vec![position; t.len()])),
            None if io.is_some_and(|io| io.token == t.name) => Some(Data::Int(vec![7; t.len()])),
            None => match t.fill.as_ref().and_then(|f| filled(f, t.len())) {
                Some(d) => Some(d),
                None if t.carried => Some(if t.elem == Elem::Int {
                    Data::Int(vec![1; t.len()])
                } else {
                    Data::Real((0..t.len()).map(|p| unit(p, 31 + i as u64)).collect())
                }),
                None => None,
            },
        };
        if let Some(d) = value {
            if !matches!(c.constants.get(&t.name), Some(Placement::Rows(_))) {
                let bytes: Vec<u8> = match &d {
                    Data::Real(v) => v.iter().flat_map(|&x| (x as f32).to_le_bytes()).collect(),
                    Data::Int(v) => v.iter().flat_map(|&x| x.to_le_bytes()).collect(),
                };
                std::fs::write(dir.join(format!("{}.bin", t.name)), bytes).unwrap();
            }
            inputs.insert(t.name.clone(), d);
        }
    }
    inputs
}

/// A unit of values kept in int8 rows as the CPU library keeps it (`lk_keep_row`): its codes and its binary16 scale —
/// the values in `f32` and every step after in `f64`, turned first by the Walsh–Hadamard transform where `turned`, the
/// scale the largest magnitude as `f32` over 127 in `f32` rounded to binary16, each code the value over it to the
/// nearest within ±127.
fn int8_unit(values: &[f32], turned: bool) -> (Vec<i8>, u16) {
    let stored: Vec<f64> = values.iter().map(|&v| f64::from(v)).collect();
    let y = if turned { hadamard(&stored) } else { stored };
    let amax = y.iter().fold(0f64, |m, v| m.max(v.abs()));
    let scale = f32_to_f16(amax as f32 / 127.0);
    let s = f64::from(f16_to_f32(scale));
    (y.iter().map(|&v| if s == 0.0 { 0 } else { (v / s).round_ties_even().clamp(-127.0, 127.0) as i8 }).collect(), scale)
}

/// A unit's values as its int8 codes and binary16 scale give them back, turned back where `turned`.
fn int8_values(codes: &[i8], scale: u16, turned: bool) -> Vec<f64> {
    let s = f64::from(f16_to_f32(scale));
    let v: Vec<f64> = codes.iter().map(|&c| f64::from(c) * s).collect();
    if turned {
        let n = v.len() as f64;
        hadamard(&v).into_iter().map(|x| x / n).collect()
    } else {
        v
    }
}

/// Every cache of `program` staged in `dir` in its format, the file rewritten and `inputs` (what the evaluator reads)
/// holding the values the format keeps: a binary16 cache its values rounded to the nearest; a cache in int8 rows each
/// unit's codes then every unit's binary16 scale, its values the ones `Keep::keep_row` keeps (each unit checked to be
/// them, a zero code's sign aside).
fn keep_caches(program: &logicaffeine_compile::codegen_cpu::link::CpuProgram, dir: &std::path::Path, inputs: &mut BTreeMap<String, Data>) {
    for b in &program.buffers {
        let BufKind::Int8Rows { unit, turned } = b.kind else { continue };
        let path = dir.join(format!("{}.bin", b.name));
        let values: Vec<f32> = match inputs.get(&b.name) {
            Some(Data::Real(v)) => v.iter().map(|&x| x as f32).collect(),
            _ => std::fs::read(&path).unwrap_or_default().chunks_exact(4).map(|x| f32::from_le_bytes(x.try_into().unwrap())).collect(),
        };
        if values.is_empty() {
            continue;
        }
        let keep = if turned { Keep::Int8RowsRotated } else { Keep::Int8Rows };
        let (mut codes, mut scales, mut kept) = (Vec::new(), Vec::new(), Vec::new());
        for row in values.chunks(unit) {
            let (c, sc) = int8_unit(row, turned);
            let back = int8_values(&c, sc, turned);
            let meaning = keep.keep_row(&row.iter().map(|&v| f64::from(v)).collect::<Vec<_>>()).expect("a unit the format keeps");
            assert!(back == meaning, "`{}`: a unit's codes and scale give back what Keep::keep_row keeps", b.name);
            codes.extend(c.iter().map(|&x| x as u8));
            scales.extend(sc.to_le_bytes());
            kept.extend(back);
        }
        codes.extend(scales);
        std::fs::write(&path, codes).unwrap();
        inputs.insert(b.name.clone(), Data::Real(kept));
    }
    for b in program.buffers.iter().filter(|b| b.kind == BufKind::F16) {
        let path = dir.join(format!("{}.bin", b.name));
        let values: Vec<f32> = match inputs.get(&b.name) {
            Some(Data::Real(v)) => v.iter().map(|&x| x as f32).collect(),
            _ => std::fs::read(&path).unwrap_or_default().chunks_exact(4).map(|x| f32::from_le_bytes(x.try_into().unwrap())).collect(),
        };
        if values.is_empty() {
            continue;
        }
        let halves: Vec<u16> = values.iter().map(|&x| f32_to_f16(x)).collect();
        std::fs::write(&path, halves.iter().flat_map(|h| h.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
        inputs.insert(b.name.clone(), Data::Real(halves.iter().map(|&h| f64::from(f16_to_f32(h))).collect()));
    }
}

/// A cut linked under `shim`, built by the host's compiler with the library and run on four threads from the values the
/// evaluator reads, each tensor its optimized step lands compared with the evaluator — or the library's refusal, by
/// name, from the values `stage` writes.
///
/// Under the precise point the whole step is compared with the evaluator's whole step: the link computes the program's
/// meaning. Under one word a block each op is compared with the evaluator's op on the CPU's own values as they stood
/// before it (the step run with its trace: every buffer each op writes, after the op): what quantizing its own
/// activations moves, apart from what the ops before it moved — so a mixture choosing among close experts, or an arg-max
/// among close logits, chooses as the evaluator does on the same inputs, and every integer is exact.
fn run_cut(c: &lane_corpus::LaneCut, k: usize, shim: &CpuShim, root: &std::path::Path, position: i64) -> Result<Vec<(String, f64)>, String> {
    let program = match link(&c.g, shim) {
        Ok(p) => p,
        Err(LinkError::NotYet { what, .. }) => return Err(what),
        Err(e) => panic!("{} under {}: {e}", c.what, shim.name()),
    };
    let point = match (shim.activations, shim.cache) {
        (CpuActivations::Values, _) => "precise",
        (_, CpuCache::F32) => "word",
        (_, CpuCache::F16) => "word-f16",
        (_, CpuCache::Int8Rows) => "word-int8",
        (_, CpuCache::Int8RowsRotated) => "word-int8-rotated",
    };
    let dir = root.join(format!("cut{k}-{point}"));
    let mut inputs = stage(c, &dir, position);
    keep_caches(&program, &dir, &mut inputs);
    build_and_run(&program, &dir, &format!("{} under {}", c.what, shim.name()), true);
    let optimized = optimize(&c.g, shim, &[]).graph;
    let kinds: BTreeMap<&str, &BufKind> = program.buffers.iter().filter(|b| !matches!(b.kind, BufKind::Rows { .. })).map(|b| (b.name.as_str(), &b.kind)).collect();
    let mut out = Vec::new();
    if shim.activations == CpuActivations::Values {
        let want = eval(&c.g, &inputs).unwrap_or_else(|e| panic!("{}: the evaluator: {e}", c.what));
        for name in optimized.ops.iter().flat_map(|o| o.writes()).collect::<BTreeSet<_>>() {
            let Some(kind) = kinds.get(name) else { continue };
            let w = want.get(name).unwrap_or_else(|| panic!("{}: the evaluator computes `{name}`", c.what));
            out.push((name.to_string(), distance(&read_back(&dir, name, kind), w)));
        }
        return Ok(out);
    }
    // Each op on the CPU's values as they stood before it — the inputs, then every tensor as the last op before it
    // that wrote it left it (the step's trace) — against what the op leaves.
    let mut before = inputs.clone();
    for (at, op) in optimized.ops.iter().enumerate() {
        let mut local = optimized.clone();
        local.ops = vec![op.clone()];
        let want = eval(&local, &before).unwrap_or_else(|e| panic!("{}: the evaluator's `{}`: {e}", c.what, op.kind()));
        for name in &program.writes[at] {
            let got = read_back(&dir, &format!("after-{at}-{name}"), kinds[name.as_str()]);
            if let Some(w) = want.get(name) {
                out.push((format!("{name} ({})", op.kind()), distance(&got, w)));
            }
            before.insert(name.clone(), got);
        }
    }
    Ok(out)
}

/// Within `PRECISE` of the evaluator at the precise point (`run_cut`), the whole step against the evaluator's whole step.
const PRECISE: f64 = 1e-5;
/// Within `ONE_WORD` of the evaluator under one word a block (`run_cut`), each op against the evaluator's op on the CPU's
/// own values before it.
const ONE_WORD: f64 = 3e-2;

/// Cut `c` at `position` under x86 AVX2 and AVX-VNNI at the precise point, and under one word with each of its caches —
/// `f32`, binary16, int8 rows, turned int8 rows (`run_cut`) — each tensor within its point's bound of the evaluator:
/// its line of the worst distances, or the library's refusal, alike at every point. A turned cache is refused only by
/// name, where a head's row is not a power of two or something but an attention reads the cache.
fn judge(c: &lane_corpus::LaneCut, k: usize, root: &std::path::Path, position: i64) -> Result<String, String> {
    let at = |activations, cache| run_cut(c, k, &CpuShim { isa: CpuIsa::X86Avx2Vnni, activations, cache, fuse: true }, root, position);
    let (precise, word) = (at(CpuActivations::Values, CpuCache::F32), at(CpuActivations::OneWord, CpuCache::F32));
    let most = |v: &[(String, f64)]| v.iter().fold(0f64, |m, (_, x)| m.max(*x));
    let mut caches = String::new();
    for (cache, what) in [(CpuCache::F16, "binary16"), (CpuCache::Int8Rows, "int8"), (CpuCache::Int8RowsRotated, "turned int8")] {
        match at(CpuActivations::OneWord, cache) {
            Ok(h) => {
                assert!(word.is_ok(), "{}: links under the {what} cache, not under f32", c.what);
                for (name, worst) in &h {
                    assert!(*worst <= ONE_WORD, "{} one word, {what} cache: `{name}` {worst:.3e} of its scale from the evaluator", c.what);
                }
                let _ = write!(caches, ", {what} {:.1e}", most(&h));
            }
            Err(why) if cache == CpuCache::Int8RowsRotated && why.contains("turned") => {
                let _ = write!(caches, ", {what} refused: {why}");
            }
            Err(why) => assert!(word.is_err(), "{}: refused under the {what} cache alone: {why}", c.what),
        }
    }
    match (precise, word) {
        (Ok(p), Ok(w)) => {
            for (name, worst) in &p {
                assert!(*worst <= PRECISE, "{} precise: `{name}` {worst:.3e} of its scale from the evaluator", c.what);
            }
            for (name, worst) in &w {
                assert!(*worst <= ONE_WORD, "{} one word: `{name}` {worst:.3e} of its scale from the evaluator", c.what);
            }
            Ok(format!("{}: precise {:.1e}, one word {:.1e}{caches}", c.what, most(&p), most(&w)))
        }
        (Err(a), Err(b)) => {
            assert_eq!(a, b, "{}: refused alike at both points", c.what);
            Err(a)
        }
        (p, w) => panic!("{}: linked at one point only: precise {:?}, one word {:?}", c.what, p.err(), w.err()),
    }
}

/// The CPU link computes what the evaluator computes on every tiny cut: every tiny architecture's decode step, prefill
/// chunk of 4 and batch of 2 (`lane_corpus`), linked under x86 AVX2 and AVX-VNNI at both of its numerics points
/// (`judge`). Under the precise point every tensor the step writes is within `PRECISE` of its largest magnitude —
/// `f32` against the evaluator's `f64` (3.8e-6 the most, DeepSeek-V2's wide chunk of 64) — so the link computes the
/// program's meaning, every integer bit for bit, the next token among them; under one word a block every op is within
/// `ONE_WORD` of the evaluator's op on the same inputs (2.4e-2 the most, DeepSeek-V2's chunks), what quantizing its own
/// activations moves, and every integer it lands exact — and so again with binary16 caches, int8 rows and turned int8
/// rows, each cache row an op lands kept in its format and every attention reading the rows the step keeps. Every tiny architecture links: the expert mixtures, the
/// gated delta rule, the latent attention among them.
#[test]
fn the_cpu_link_computes_what_the_evaluator_computes_on_every_tiny_cut() {
    if !common_tool("g++") {
        return;
    }
    let root = scratch::dir("logos_cpu_link");
    let cuts = every_lane_cut();
    let (linked, refused) = (std::sync::Mutex::new(Vec::new()), std::sync::Mutex::new(Vec::new()));
    each_on_threads(&cuts, |k, c| match judge(c, k, &root, 6) {
        Ok(line) => linked.lock().unwrap().push(line),
        Err(why) => refused.lock().unwrap().push(format!("{}: {why}", c.what)),
    });
    let (mut linked, refused) = (linked.into_inner().unwrap(), refused.into_inner().unwrap());
    linked.sort();
    eprintln!("linked {}:\n{}\nrefused {}:\n{}", linked.len(), linked.join("\n"), refused.len(), refused.join("\n"));
    assert!(refused.is_empty(), "every tiny architecture links: refused {refused:?}");
    assert_eq!(linked.len(), cuts.len(), "every cut computed");
}

/// A row attending more positions than a split (`kernels::ATTN_SPLIT`) is attended a split at a time and its splits
/// merged, as the evaluator computes it: Llama's, gpt-oss's (sinks, windowed layers) and Gemma 2's (soft caps, windowed
/// layers) decode steps, prefill chunks of 2 to 32 rows and batches of 2 over a cache of 600 positions at position 560 —
/// each row of a global layer attending three splits — each within its point's bound of the evaluator (`judge`) under
/// every cache format. A decode step and a batch spread their splits over the threads (`lk_attention_part_units`); a
/// chunk with a unit for each core of a large host runs a row's splits in its unit; no spread phase is joined to
/// another row by row.
#[test]
fn a_long_cache_attends_a_split_at_a_time_as_the_evaluator_computes() {
    if !common_tool("g++") {
        return;
    }
    let (ctx, position, chunk) = (600usize, 560i64, 32usize);
    assert!(position as usize > 2 * ATTN_SPLIT && position as usize + chunk <= ctx, "every row three splits, every chunk within the cache");
    let root = scratch::dir("logos_cpu_long");
    let cuts = lane_corpus::long_cache_cuts(&[Tiny::Llama, Tiny::GptOss, Tiny::Gemma2], ctx, chunk);
    let shim = CpuShim { isa: CpuIsa::X86Avx2Vnni, activations: CpuActivations::OneWord, cache: CpuCache::F32, fuse: true };
    let mut in_units = 0;
    for c in &cuts {
        let program = link(&c.g, &shim).unwrap_or_else(|e| panic!("{}: {e}", c.what));
        let spread = program.step.contains("lk_attention_part_units(");
        if c.what.ends_with(" 1") || c.what.ends_with(" b2") {
            assert!(spread, "{}: a step of few rows spreads its splits over the threads", c.what);
        } else if !spread {
            in_units += 1;
        }
        // Each spread phase stands as a phase of its own: one joining another row by row runs each row's splits on one
        // thread.
        for (at, _) in program.step.match_indices("lk_attention_part_units(") {
            let phase = program.step[..at].rfind("case ").max(program.step[..at].rfind("lk_phases(")).unwrap();
            assert!(!program.step[phase..at].contains("const long tlo"), "{}: a spread phase joined row by row:\n{}", c.what, &program.step[phase..at]);
        }
    }
    assert!(in_units > 0, "some chunk of {chunk} runs each row's splits in its unit");
    let lines = std::sync::Mutex::new(Vec::new());
    each_on_threads(&cuts, |k, c| {
        let line = judge(c, k, &root, position).unwrap_or_else(|why| panic!("{}: refused: {why}", c.what));
        lines.lock().unwrap().push(line);
    });
    let mut lines = lines.into_inner().unwrap();
    lines.sort();
    eprintln!("{}", lines.join("\n"));
    assert_eq!(lines.len(), cuts.len(), "every cut computed");
}

/// The values an exemplar's program reads, written into `dir` as the file driver reads them: each matrix's checkpoint
/// rows in Q8_0 (codes a deterministic pattern, never −128, each block's scale 0.01 to 0.026), each declared tensor's
/// fill, each parameter 2 and up, and every other constant or state a deterministic pattern.
fn stage_graph(g: &TensorGraph, dir: &std::path::Path) {
    std::fs::create_dir_all(dir.join("out")).unwrap();
    for (i, t) in g.tensors.values().enumerate() {
        let bytes: Option<Vec<u8>> = match &t.storage {
            Storage::Rows { format: GgmlType::Q8_0, transposed } => {
                let (rows, cols) = if *transposed { (t.cols, t.rows) } else { (t.rows, t.cols) };
                let mut b = Vec::new();
                for blk in 0..rows * cols / 32 {
                    b.extend_from_slice(&f32_to_f16(0.01 + 0.001 * (blk % 17) as f32).to_le_bytes());
                    b.extend((0..32).map(|v| ((unit(32 * blk + v, 40 + i as u64) * 127.0).round() as i32).clamp(-127, 127) as i8 as u8));
                }
                Some(b)
            }
            Storage::Rows { .. } => None,
            _ => match t.fill.as_ref().and_then(|f| filled(f, t.len())) {
                Some(Data::Real(v)) => Some(v.iter().flat_map(|&x| (x as f32).to_le_bytes()).collect()),
                Some(Data::Int(v)) => Some(v.iter().flat_map(|&x| x.to_le_bytes()).collect()),
                None if t.role == Role::Param => Some((0..t.len() as i64).flat_map(|k| (2 + k).to_le_bytes()).collect()),
                None if t.role != Role::Activation && t.elem == Elem::Float => Some((0..t.len()).flat_map(|k| (fusion_corpus::value(&t.name, k) as f32).to_le_bytes()).collect()),
                None => None,
            },
        };
        if let Some(b) = bytes {
            std::fs::write(dir.join(format!("{}.bin", t.name)), b).unwrap();
        }
    }
}

/// The values `stage_graph` wrote into `dir`, as the evaluator reads them: each real tensor's `f32`s, each integer one's
/// `i64`s.
fn staged(g: &TensorGraph, dir: &std::path::Path) -> BTreeMap<String, Data> {
    g.tensors
        .values()
        .filter_map(|t| {
            let bytes = std::fs::read(dir.join(format!("{}.bin", t.name))).ok()?;
            let data = match t.elem {
                Elem::Float => Data::Real(bytes.chunks_exact(4).map(|x| f64::from(f32::from_le_bytes(x.try_into().unwrap()))).collect()),
                _ => Data::Int(bytes.chunks_exact(8).map(|x| i64::from_le_bytes(x.try_into().unwrap())).collect()),
            };
            Some((t.name.clone(), data))
        })
        .collect()
}

/// Every op the census locks name the CPU link a computer of, it computes as the evaluator does: the lane sums and the
/// signed root, bare and with its epsilon (`tensor_lane_gate_lock`); the mean over lanes and the spread over lanes, by
/// weights and as a copy (`tensor_lane_mix_lock`); a norm with a weight row a lane (`tensor_lane_norm_lock`); a capped
/// attention (`tensor_softcap_lock`); a gated delta rule gated by a sigmoid and by SiLU (`tensor_gated_delta_gate_lock`);
/// and the n-gram ids, ends of document among the prompt's rows (`qwen4_exp_ngram_lock`) — each program linked under
/// x86 AVX2 and AVX-VNNI at the precise point and under one word, built, and run, every tensor its step writes within
/// `PRECISE` of the evaluator's on the inputs it ran on. None is refused.
#[test]
fn every_op_the_census_locks_name_the_cpu_a_computer_of_it_computes_as_the_evaluator_does() {
    if !common_tool("g++") {
        return;
    }
    let delta = |gate: &str| {
        format!(
            "## Main\nLet Q be a 1 by 64 tensor of Real.\nLet K be a 1 by 64 tensor of Real.\nLet V be a 1 by 128 tensor of Real.\n\
             Let Z be a 1 by 128 tensor of Real.\nLet B be a 1 by 4 tensor of Real.\nLet A be a 1 by 4 tensor of Real.\n\
             Let S be a 128 by 32 tensor of Real.\nLet G be a 1 by 32 tensor of Real.\n\
             Let O be gated delta of Q over K and V with {gate} Z beta B decay A state S norm G with 4 heads 2 groups.\n"
        )
    };
    // A prompt of rows (a token's id plus one) with an end of document among them, from a fresh history.
    let ngram = {
        let h = NgramHash::new(1000, 4, 2, 50, 8, 1, 7, 999);
        let ints = |name: &str, values: &[u64]| {
            let mut s = format!("Let {name} be a 1 by {} tensor of Int.\n", values.len());
            for (k, v) in values.iter().enumerate() {
                s.push_str(&format!("Set item {} of {name} to {v}.\n", k + 1));
            }
            s
        };
        format!(
            "## Main\n{}{}{}{}{}Let Ids be the ngram ids of Toks after Hist by Mult modulo Mods from Offs ending at 1000.\n",
            ints("Toks", &[18, 1000, 8, 9, 10, 1000, 1000, 41]),
            ints("Hist", &[0, 0, 0]),
            ints("Mult", h.multipliers()),
            ints("Mods", h.head_primes()),
            ints("Offs", h.head_offsets()),
        )
    };
    let programs: Vec<(&str, String)> = vec![
        ("the lane sums", "## Main\nLet X be a 2 by 12 tensor of Real.\nLet Y be the lane sums of X over 3 lanes.\n".to_string()),
        ("the signed root", "## Main\nLet X be a 2 by 8 tensor of Real.\nLet Y be the signed root of X.\n".to_string()),
        ("the signed root with its epsilon", "## Main\nLet X be a 2 by 8 tensor of Real.\nLet Y be the signed root of X with epsilon 0.01.\n".to_string()),
        ("the mean over lanes", "## Main\nLet X be a 2 by 12 tensor of Real.\nLet Y be the mean of X over 3 lanes.\n".to_string()),
        ("a spread by weights", "## Main\nLet O be a 2 by 4 tensor of Real.\nLet S be a 2 by 3 tensor of Real.\nLet Y be O spread over 3 lanes by S.\n".to_string()),
        ("a spread", "## Main\nLet O be a 2 by 4 tensor of Real.\nLet Y be O spread over 3 lanes.\n".to_string()),
        ("a norm in lanes", "## Main\nLet H be a 2 by 12 tensor of Real.\nLet W be a 3 by 4 tensor of Real.\nLet Y be rmsnorm of H with weight W in 3 lanes.\n".to_string()),
        (
            "a capped attention",
            "## Main\nLet Q be a 1 by 8 tensor of Real.\nLet K be a 6 by 4 tensor of Real.\nLet V be a 6 by 4 tensor of Real.\nLet pos be 5.\n\
             Let C be attention of Q over K and V with 2 heads 1 groups up to pos capped at 0.5.\n"
                .to_string(),
        ),
        ("a delta rule gated by a sigmoid", delta("sigmoid gate")),
        ("a delta rule gated by SiLU", delta("gate")),
        ("the n-gram ids of an order of 4, two heads an order", ngram),
    ];
    let root = scratch::dir("logos_cpu_census_ops");
    each_on_threads(&programs, |k, (what, source)| {
        let g = fusion_corpus::lifted(source, false, None);
        for activations in [CpuActivations::Values, CpuActivations::OneWord] {
            let shim = CpuShim { isa: CpuIsa::X86Avx2Vnni, activations, cache: CpuCache::F32, fuse: true };
            let at = format!("{what} under {}", shim.name());
            let program = link(&g, &shim).unwrap_or_else(|e| panic!("{at}: {e}"));
            let dir = root.join(format!("p{k}-{activations:?}"));
            stage_graph(&g, &dir);
            build_and_run(&program, &dir, &at, false);
            let want = eval(&g, &staged(&g, &dir)).unwrap_or_else(|e| panic!("{at}: the evaluator: {e}"));
            let kinds: BTreeMap<&str, &BufKind> = program.buffers.iter().filter(|b| !matches!(b.kind, BufKind::Rows { .. })).map(|b| (b.name.as_str(), &b.kind)).collect();
            let optimized = optimize(&g, &shim, &[]).graph;
            let mut compared = 0usize;
            for name in optimized.ops.iter().flat_map(|o| o.writes()).collect::<BTreeSet<_>>() {
                let (Some(kind), Some(w)) = (kinds.get(name), want.get(name)) else { continue };
                let worst = distance(&read_back(&dir, name, kind), w);
                assert!(worst <= PRECISE, "{at}: `{name}` {worst:.3e} of its scale from the evaluator");
                compared += 1;
            }
            assert!(compared > 0, "{at}: the step writes what the evaluator computes");
        }
    });
}

/// Every fused op the CPU links whole is one run on the host's threads and its parts to the bit: on every class's
/// exemplar (its matrices checkpoint rows) and every tiny cut, linked under x86 AVX2 and AVX-VNNI at both numerics
/// points and under one word with binary16, int8-row and turned int8-row caches, each fused op's code is one run of
/// phases (`lk_phases`), and every
/// tensor its step lands is the bits the program linking every fused class as its parts (`fuse` off) computes from the
/// same inputs. The fused classes the CPU links whole that no witness realizes (`fuse::realizes`) are named; every one
/// an exemplar witnesses is realized by it or a tiny cut.
#[test]
fn every_fused_op_is_one_run_and_its_parts_to_the_bit() {
    if !common_tool("g++") {
        return;
    }
    let root = scratch::dir("logos_cpu_fused");
    let mut witnesses: Vec<(String, TensorGraph, Option<lane_corpus::LaneCut>)> = Vec::new();
    for c in CLASSES {
        if let Exemplar::Program { source, .. } = c.exemplar {
            witnesses.push((format!("{} exemplar", c.id), fusion_corpus::lifted(source, true, None), None));
        }
    }
    for cut in every_lane_cut() {
        witnesses.push((cut.what.clone(), cut.g.clone(), Some(cut)));
    }
    let (built, compared) = (std::sync::Mutex::new(BTreeSet::<ClassId>::new()), std::sync::atomic::AtomicUsize::new(0));
    each_on_threads(&witnesses, |k, (what, g, cut)| {
        let points = [(CpuActivations::Values, CpuCache::F32)].into_iter().chain(CpuCache::ALL.map(|c| (CpuActivations::OneWord, c)));
        for (activations, cache) in points {
            let shim = CpuShim { isa: CpuIsa::X86Avx2Vnni, activations, cache, fuse: true };
            let apart = CpuShim { fuse: false, ..shim };
            let (Ok(fused), Ok(parts)) = (link(g, &shim), link(g, &apart)) else { continue };
            let optimized = optimize(g, &shim, &[]).graph;
            for (at, op) in optimized.ops.iter().enumerate() {
                built.lock().unwrap().extend(realizes(op, &optimized));
                let Sym::Fused(_) = &op.sym else { continue };
                let from = fused.step.find(&format!("  // op {at}: ")).expect("the op's code");
                let to = fused.step[from..].find(&format!("  LK_AFTER({at});")).expect("the op's end") + from;
                let code = &fused.step[from..to];
                let runs = code.matches("lk_phases(").count();
                assert_eq!(runs, 1, "{what} under {}: `{}` is one run, not {runs}:\n{code}", shim.name(), op.kind());
                // A chunk's mixtures, independent rows of one table, run beside one another in one row's phases (its
                // views, then six under one word, four precise), the add, norm and quantize after them in at most two.
                fn sorts(o: &logicaffeine_compile::tensor_graph::graph::Op) -> bool {
                    matches!(&o.sym, Sym::Fused(f) if f.class == ClassId::MoeTokenSort || f.parts.iter().any(sorts))
                }
                if sorts(op) {
                    let phases = code[code.find("lk_phases(").unwrap()..].split("[&]").next().unwrap().matches("L, ").count();
                    let most = 1 + if activations == CpuActivations::OneWord { 6 } else { 4 } + 2;
                    assert!(phases <= most, "{what} under {}: a chunk's mixtures and what follows them take {phases} phases, not at most {most}", shim.name());
                }
            }
            let dir = root.join(format!("w{k}-{}", shim.name().replace(':', "-")));
            let (fdir, pdir) = (dir.join("fused"), dir.join("apart"));
            for (d, program) in [(&fdir, &fused), (&pdir, &parts)] {
                let mut inputs = match cut {
                    Some(c) => stage(c, d, 6),
                    None => {
                        stage_graph(g, d);
                        BTreeMap::new()
                    }
                };
                keep_caches(program, d, &mut inputs);
            }
            build_and_run(&fused, &fdir, &format!("{what} under {}", shim.name()), false);
            build_and_run(&parts, &pdir, &format!("{what} under {}", apart.name()), false);
            let theirs: BTreeSet<&str> = parts.buffers.iter().map(|b| b.name.as_str()).collect();
            for name in optimized.ops.iter().flat_map(|o| o.writes()).collect::<BTreeSet<_>>() {
                if !theirs.contains(name) || !fused.buffers.iter().any(|b| b.name == name && !matches!(b.kind, BufKind::Rows { .. })) {
                    continue;
                }
                let read = |d: &std::path::Path| std::fs::read(d.join("out").join(format!("{name}.bin"))).unwrap_or_default();
                assert!(read(&fdir) == read(&pdir), "{what} under {}: `{name}` fused is not its parts' bits", shim.name());
                compared.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    });
    let (built, compared) = (built.into_inner().unwrap(), compared.into_inner());
    let native: BTreeSet<ClassId> = LINKED_WHOLE.iter().copied().filter(|c| c.class().kind == Kind::Fused).collect();
    let built: BTreeSet<ClassId> = built.into_iter().filter(|c| native.contains(c)).collect();
    let unbuilt: Vec<ClassId> = native.difference(&built).copied().collect();
    let witnessed: Vec<ClassId> = unbuilt.iter().copied().filter(|c| matches!(c.class().exemplar, Exemplar::Program { .. })).collect();
    eprintln!("fused classes realized {}: {built:?}\nlinked whole and realized by no witness yet: {unbuilt:?}\n{compared} tensors compared", built.len());
    assert!(witnessed.is_empty(), "native fused classes whose exemplar realizes them under the CPU shim nowhere: {witnessed:?}");
    assert!(compared > 0, "some fused step is compared with its parts");
}

/// Whether `tool` is on the path (the CPU link's tests need the host's C++ compiler).
fn common_tool(tool: &str) -> bool {
    Command::new(tool).arg("--version").output().is_ok_and(|o| o.status.success())
}

/// The emitted kernels use the instruction set the shim declares: a program linked for x86 AVX2 and AVX-VNNI takes its
/// dots by `vpdpbusd` in its VEX form and its scales from binary16 by F16C, in 256-bit registers, with no AVX-512
/// instruction; and an instruction set the library has no kernels in yet is refused by name, never linked with
/// another's.
#[test]
fn the_emitted_kernels_use_the_isa_the_shim_declares() {
    let cut = every_lane_cut().into_iter().find(|c| c.what.starts_with("llama")).expect("a Llama cut");
    let program = link(&cut.g, &CpuShim { isa: CpuIsa::X86Avx2Vnni, activations: CpuActivations::OneWord, cache: CpuCache::F32, fuse: true }).unwrap_or_else(|e| panic!("{}: {e}", cut.what));
    let source = program.source_with_file_driver();
    for want in ["_mm256_dpbusd_avx_epi32", "_cvtsh_ss", "_mm256_fmadd_ps"] {
        assert!(source.contains(want), "x86-avx2-vnni takes `{want}`");
    }
    for never in ["_mm512_", "__m512", "vdotq_s32", "__riscv_v"] {
        assert!(!source.contains(never), "x86-avx2-vnni emits no `{never}`");
    }
    for isa in [CpuIsa::X86Avx512, CpuIsa::ArmNeonDot, CpuIsa::Rv64Rvv] {
        match link(&cut.g, &CpuShim { isa, activations: CpuActivations::OneWord, cache: CpuCache::F32, fuse: true }) {
            Err(LinkError::NotYet { what, .. }) => assert!(what.contains(isa.name()), "{}: refused by name: {what}", isa.name()),
            other => panic!("{}: linked with no library of its own: {:?}", isa.name(), other.map(|p| p.isa)),
        }
    }
}

/// The CPU library computes what its reference computes, the same on every thread count: activations quantized one word
/// a 32-value block — each code and scale bit for bit as the block's largest magnitude over 127 and each value times 127
/// over it rounded to the nearest code, ties to even, a zero block all zero, each block's code sum exact; products by
/// Q8_0, Q4_K and Q6_K rows read in place — each output within `EXACT` of the exact sum of the weights' values times the
/// activation's codes and scales, a −127 code beside a −127 activation code among them, over rows of 1, 3, 4, 5 and 13
/// of 512 values (two K-quant blocks) by 600 outputs (a thread's chunk past the outputs a panel stages at once, chunks
/// ending inside a vector's eight) — and a row of a chunk the bits it is alone (13 rows through the panel kernels, each
/// the row kernel's one row); a root mean square norm of a sum with a residual by a weight; an attention of three
/// query rows of four heads over two key/value heads within a window of 5 with sinks and a cap, written a pitch apart;
/// rows attending more positions than a
/// split (`ATTN_SPLIT`) beside rows within one — a prompt's, the same rows each alone as a step, rows within a window,
/// rows with no sink, a prompt over binary16 caches, heads 11 wide, a prompt over int8 rows plain and turned (each
/// unit's codes and scale the reference's bit for bit) — each within `EXACT` of the f64 reference of the same inputs, a
/// row alone the bits it is among the prompt's, its splits spread over the threads the bits of its splits run one after
/// another; and e^x within one and a half ulp of the exact value over [−104, 88.7] (1.26 the most). Run on 1, 2 and 7 threads, every
/// output is the same bits: each unit is one thread's, computed one way. And the pool returns from a run only once every
/// chunk of it is written, and takes no chunk of a phase before the last one's are: 100,000 two-phase runs back to back
/// on 48 threads (more than the host runs at once, so workers are descheduled mid-run), the second phase reading values
/// across the first's and the next run reading them all, miss none — a pool whose late worker takes a chunk of the next
/// run, or of the next phase early, misses some; and 2,000 runs of a chain of 40 phases — more than a run of the pool
/// holds, so they run as consecutive runs — each phase reading across the last's, miss none.
#[test]
fn the_cpu_library_computes_what_its_reference_computes() {
    const EXACT: f64 = 1e-5;
    if !common_tool("g++") {
        return;
    }
    let dir = scratch::dir("logos_cpu_library");
    std::fs::create_dir_all(&dir).unwrap();
    let harness = r#"
#include <cstdio>
#include <string>
static void put(FILE* f, const void* p, size_t n){ std::fwrite(p, 1, n, f); }
int main(int argc, char** argv){
  const int threads = std::atoi(argv[1]);
  FILE* in = std::fopen(argv[2], "rb"); FILE* out = std::fopen(argv[3], "wb");
  auto get = [&](void* p, size_t n){ if(std::fread(p, 1, n, in) != n) std::exit(3); };
  lk_pool pool(threads);
  const int K = 512, N = 600, R = 13;
  std::vector<float> x(R * K); get(x.data(), x.size() * 4);
  std::vector<int8_t> q(R * K); std::vector<float> d(R * K / 32); std::vector<int> s(R * K / 32);
  lk_phases(pool, {{R * K / 32, 16}}, [&](int, long lo, long hi){ lk_quant_blocks(x.data(), K, q.data(), d.data(), s.data(), K, lo, hi); });
  put(out, q.data(), q.size()); put(out, d.data(), d.size() * 4); put(out, s.data(), s.size() * 4);
  for(int f : {8, 12, 14}){
    const long pitch = f == 8 ? K / 32 * 34 : f == 12 ? K / 256 * 144 : K / 256 * 210;
    std::vector<uint8_t> w(N * pitch); get(w.data(), w.size());
    auto product = [&](int r0, int rows, float* y){
      if(f == 8){ lk_q8_job j{w.data(), pitch, q.data() + (long)r0 * K, d.data() + (long)r0 * (K / 32), y, rows, N, K, N}; lk_phases(pool, {{N, 4}}, [&](int, long lo, long hi){ lk_gemm_q8_0_units(j, lo, hi); }); }
      else { lk_kq_job j{f, w.data(), pitch, q.data() + (long)r0 * K, d.data() + (long)r0 * (K / 32), s.data() + (long)r0 * (K / 32), nullptr, 0, y, rows, N, K, N}; lk_phases(pool, {{N, 4}}, [&](int, long lo, long hi){ lk_gemm_kq_units(j, lo, hi); }); }
    };
    for(int rows : {1, 3, 4, 5, 13}){ std::vector<float> y(rows * N); product(0, rows, y.data()); put(out, y.data(), y.size() * 4); }
    for(int r = 0; r < R; ++r){ std::vector<float> y(N); product(r, 1, y.data()); put(out, y.data(), y.size() * 4); }
  }
  const int NR = 3, C = 64;
  std::vector<float> nx(NR * C), nr(NR * C), nw(C), ny(NR * C);
  get(nx.data(), nx.size() * 4); get(nr.data(), nr.size() * 4); get(nw.data(), nw.size() * 4);
  lk_phases(pool, {{NR, 1}}, [&](int, long lo, long hi){ lk_rmsnorm_rows(nx.data(), C, nr.data(), nw.data(), 1, ny.data(), C, C, 1e-5f, lo, hi); });
  put(out, ny.data(), ny.size() * 4);
  const int H = 4, KV = 2, HD = 16, L = 9, OP = H * HD + 8;
  std::vector<float> aq(NR * H * HD), ak(L * KV * HD), av(L * KV * HD), sk(H), ao(NR * OP);
  get(aq.data(), aq.size() * 4); get(ak.data(), ak.size() * 4); get(av.data(), av.size() * 4); get(sk.data(), sk.size() * 4);
  lk_attn_job aj{aq.data(), H * HD, ak.data(), av.data(), sk.data(), ao.data(), OP, NR, H, KV, HD, HD, 5, L, 5, KV * HD, KV * HD, 0.25f, 3.f};
  lk_phases(pool, {{NR * KV, 1}}, [&](int, long lo, long hi){ lk_attention_units(aj, lo, hi); });
  put(out, ao.data(), ao.size() * 4);
  const int SL = 3 * LK_ATTN_SPLIT - 68, SR = 10, SOP = H * HD + 4;
  std::vector<float> sq(SR * H * HD), skc(SL * KV * HD), svc(SL * KV * HD);
  get(sq.data(), sq.size() * 4); get(skc.data(), skc.size() * 4); get(svc.data(), svc.size() * 4);
  std::vector<uint16_t> hkc(skc.size()), hvc(svc.size());
  for(size_t i = 0; i < skc.size(); ++i){ hkc[i] = lk_to_f16(skc[i]); hvc[i] = lk_to_f16(svc[i]); }
  auto whole = [&](const auto& j){ lk_phases(pool, {{(long)j.rows * j.kvh, 1}}, [&](int, long lo, long hi){ lk_attention_units(j, lo, hi); }); };
  auto spread = [&](const auto& j, int splits){
    std::vector<float> part((long)j.rows * splits * j.heads * (j.vd + 2));
    lk_phases(pool, {{(long)j.rows * j.kvh * splits, 1}, {(long)j.rows * j.kvh, 1}}, [&](int ph, long lo, long hi){
      if(ph == 0) lk_attention_part_units(j, part.data(), splits, lo, hi); else lk_attention_merge_units(j, part.data(), splits, lo, hi);
    });
  };
  auto both = [&](auto j, int rows, int splits){
    std::vector<float> a(rows * SOP), b(rows * SOP);
    j.out = a.data(); whole(j); put(out, a.data(), a.size() * 4);
    j.out = b.data(); spread(j, splits); put(out, b.data(), b.size() * 4);
  };
  const lk_attn_job prompt{sq.data(), H * HD, skc.data(), svc.data(), sk.data(), nullptr, SOP, SR, H, KV, HD, HD, LK_ATTN_SPLIT - 6, SL, 0, KV * HD, KV * HD, 0.25f, 3.f};
  both(prompt, SR, lk_attention_splits(SL));
  for(int r = 0; r < SR; ++r){
    lk_attn_job step = prompt; step.q = sq.data() + r * H * HD; step.rows = 1; step.p0 = LK_ATTN_SPLIT - 6 + r;
    std::vector<float> o(SOP); step.out = o.data(); spread(step, lk_attention_splits(SL)); put(out, o.data(), o.size() * 4);
  }
  lk_attn_job windowed = prompt; windowed.rows = 3; windowed.p0 = SL - 4; windowed.win = LK_ATTN_SPLIT + 44;
  both(windowed, 3, lk_attention_splits(LK_ATTN_SPLIT + 44));
  lk_attn_job sinkless = prompt; sinkless.rows = 2; sinkless.p0 = SL - 3; sinkless.sinks = nullptr; sinkless.cap = 0.f;
  both(sinkless, 2, lk_attention_splits(SL));
  const lk_attn_job_h16 halves{sq.data(), H * HD, hkc.data(), hvc.data(), sk.data(), nullptr, SOP, SR, H, KV, HD, HD, LK_ATTN_SPLIT - 6, SL, 0, KV * HD, KV * HD, 0.25f, 3.f};
  both(halves, SR, lk_attention_splits(SL));
  lk_attn_job narrow = prompt; narrow.hd = narrow.vd = HD - 5;
  both(narrow, SR, lk_attention_splits(SL));
  // The same cache kept in int8 rows a head's row a unit, plain and turned (its first unit all zero), each written out;
  // the prompt over each — the turned one's query turned first and its output turned back.
  for(int i = 0; i < HD; ++i) skc[i] = svc[i] = 0.f;
  for(bool turned : {false, true}){
    std::vector<int8_t> kq(skc.size()), vq(svc.size()); std::vector<uint16_t> ks(skc.size() / HD), vs(svc.size() / HD);
    const lk_i8rows kr{kq.data(), ks.data(), HD, 0}, vr{vq.data(), vs.data(), HD, 0};
    lk_phases(pool, {{(long)ks.size(), 16}}, [&](int, long lo, long hi){ for(long u = lo; u < hi; ++u){ lk_keep_row(skc.data() + u * HD, HD, turned, kr, u * HD); lk_keep_row(svc.data() + u * HD, HD, turned, vr, u * HD); } });
    put(out, kq.data(), kq.size()); put(out, ks.data(), ks.size() * 2); put(out, vq.data(), vq.size()); put(out, vs.data(), vs.size() * 2);
    std::vector<float> tq(sq.size());
    if(turned) lk_phases(pool, {{SR * H, 4}}, [&](int, long lo, long hi){ lk_turn_units(sq.data(), H * HD, tq.data(), H * HD, HD, H, lo, hi); });
    lk_attn_job_i8 kept{turned ? tq.data() : sq.data(), H * HD, kr, vr, sk.data(), nullptr, SOP, SR, H, KV, HD, HD, LK_ATTN_SPLIT - 6, SL, 0, KV * HD, KV * HD, 0.25f, 3.f};
    std::vector<float> a(SR * SOP), b(SR * SOP);
    kept.out = a.data(); whole(kept);
    kept.out = b.data(); spread(kept, lk_attention_splits(SL));
    if(turned) for(float* o : {a.data(), b.data()}) lk_phases(pool, {{SR * H, 4}}, [&](int, long lo, long hi){ lk_turn_units(o, SOP, o, SOP, HD, H, lo, hi); });
    put(out, a.data(), a.size() * 4); put(out, b.data(), b.size() * 4);
  }
  std::vector<float> ex(20001), ey(20001);
  for(int i = 0; i <= 20000; ++i) ex[i] = -104.f + (88.7f + 104.f) * (float)i / 20000.f;
  for(int i = 0; i + 8 <= 20000; i += 8) _mm256_storeu_ps(&ey[i], lk_exp8(_mm256_loadu_ps(&ex[i])));
  ey[20000] = lk_exp(ex[20000]);
  put(out, ex.data(), ex.size() * 4); put(out, ey.data(), ey.size() * 4);
  if(argc > 4){
    lk_pool wide(48);
    std::vector<int> a(5000), b(5000);
    long missed = 0;
    for(int it = 0; it < 100000; ++it){
      const int n = 37 + it % 4000;
      int* v = it & 1 ? a.data() : b.data(); int* u = it & 1 ? b.data() : a.data();
      lk_phases(wide, {{n, 1}, {n, 1}}, [&](int ph, long lo, long hi){
        if(ph == 0) for(long i = lo; i < hi; ++i) v[i] = it;
        else for(long i = lo; i < hi; ++i) u[i] = v[(i * 7 + 3) % n];
      });
      for(int i = 0; i < n; ++i) missed += (v[i] != it) + (u[i] != it);
    }
    for(int it = 0; it < 2000; ++it){
      const int n = 37 + it % 2000;
      lk_phases(wide, {{n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1},
                       {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1},
                       {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}, {n, 1}},
                 [&](int ph, long lo, long hi){
        int* dst = ph & 1 ? b.data() : a.data(); const int* src = ph & 1 ? a.data() : b.data();
        for(long i = lo; i < hi; ++i) dst[i] = ph == 0 ? it : src[(i * 7 + 3) % n] + 1;
      });
      for(int i = 0; i < n; ++i) missed += b[i] != it + 39;
    }
    std::printf("MISSED %ld\n", missed);
  }
  std::fclose(out);
  return 0;
}
"#;
    let (cpp, bin) = (dir.join("library.cpp"), dir.join("library"));
    std::fs::write(&cpp, format!("{LIBRARY_X86_AVX2_VNNI}{harness}")).unwrap();
    let built = Command::new("g++").args(["-O2", "-std=c++17", "-mavx2", "-mfma", "-mavxvnni", "-mf16c", "-pthread", "-o"]).arg(&bin).arg(&cpp).output().unwrap();
    assert!(built.status.success(), "the library builds:\n{}", String::from_utf8_lossy(&built.stderr));
    let (k, n, rmax) = (512usize, 600usize, 13usize);
    let f32s = |len: usize, salt: u64, m: f64| -> Vec<f32> { (0..len).map(|i| (unit(i, salt) * m) as f32).collect() };
    let mut x = f32s(rmax * k, 1, 3.0);
    for v in &mut x[64..96] {
        *v = 0.0;
    }
    x[0] = -3.5;
    // The weights in each format, every byte a deterministic pattern and each block's scales set: a Q8_0 code never
    // −128, and a −127 beside the activation's −127 (row 0's first block, output 3).
    let mut weights: Vec<(GgmlType, Vec<u8>)> = Vec::new();
    for (ty, block, salt) in [(GgmlType::Q8_0, 34usize, 2u64), (GgmlType::Q4K, 144, 12), (GgmlType::Q6K, 210, 14)] {
        let per = if ty == GgmlType::Q8_0 { 32 } else { 256 };
        let mut w: Vec<u8> = (0..n * (k / per) * block).map(|i| (unit(i, salt) * 128.0 + 128.0) as u8).collect();
        for o in 0..n {
            for b in 0..k / per {
                let at = (o * (k / per) + b) * block;
                let dh = f32_to_f16(0.01 + 0.001 * ((o + b) % 17) as f32).to_le_bytes();
                match ty {
                    GgmlType::Q8_0 => {
                        w[at..at + 2].copy_from_slice(&dh);
                        for i in 0..32 {
                            if w[at + 2 + i] == 0x80 {
                                w[at + 2 + i] = 0x81;
                            }
                        }
                        if o == 3 && b == 0 {
                            w[at + 2] = (-127i8) as u8;
                        }
                    }
                    GgmlType::Q4K => {
                        w[at..at + 2].copy_from_slice(&dh);
                        w[at + 2..at + 4].copy_from_slice(&f32_to_f16(0.004 + 0.0005 * ((o * 3 + b) % 7) as f32).to_le_bytes());
                    }
                    _ => w[at + 208..at + 210].copy_from_slice(&dh),
                }
            }
        }
        weights.push((ty, w));
    }
    let (r, c) = (3usize, 64usize);
    let (nx, nr, nw) = (f32s(r * c, 3, 2.0), f32s(r * c, 4, 1.0), f32s(c, 5, 1.0));
    let (h, kv, hd, l) = (4usize, 2usize, 16usize, 9usize);
    let opitch = h * hd + 8;
    let (aq, ak, av, sk) = (f32s(r * h * hd, 7, 1.0), f32s(l * kv * hd, 8, 1.0), f32s(l * kv * hd, 9, 1.0), f32s(h, 10, 1.0));
    let (sl, sr, sop) = (3 * ATTN_SPLIT - 68, 10usize, h * hd + 4);
    let (sq, skc, svc) = (f32s(sr * h * hd, 21, 4.0), f32s(sl * kv * hd, 22, 1.0), f32s(sl * kv * hd, 23, 1.0));
    let mut input: Vec<u8> = x.iter().flat_map(|f| f.to_le_bytes()).collect();
    for (_, w) in &weights {
        input.extend_from_slice(w);
    }
    for v in [&nx, &nr, &nw, &aq, &ak, &av, &sk, &sq, &skc, &svc] {
        input.extend(v.iter().flat_map(|f| f.to_le_bytes()));
    }
    std::fs::write(dir.join("in.bin"), &input).unwrap();
    let mut runs = Vec::new();
    for threads in [1, 2, 7] {
        let out = dir.join(format!("out{threads}.bin"));
        let ran = Command::new(&bin).arg(threads.to_string()).arg(dir.join("in.bin")).arg(&out).output().unwrap();
        assert!(ran.status.success(), "the library runs on {threads} threads: {}", String::from_utf8_lossy(&ran.stderr));
        runs.push(std::fs::read(&out).unwrap());
    }
    assert!(runs.iter().all(|r| r == &runs[0]), "every thread count computes the same bits");
    let stressed = Command::new(&bin).arg("1").arg(dir.join("in.bin")).arg(dir.join("stress.bin")).arg("stress").output().unwrap();
    let said = String::from_utf8_lossy(&stressed.stdout).into_owned();
    assert!(stressed.status.success() && said.contains("MISSED 0"), "every chunk of every phase written before the next is taken: {said}");
    let bytes = &runs[0];
    let f32_at = |at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let i32_at = |at: usize| i32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    // The codes, scales and code sums, bit for bit.
    let mut at = 0;
    let q: Vec<i8> = bytes[..rmax * k].iter().map(|&b| b as i8).collect();
    at += rmax * k;
    let d: Vec<f32> = (0..rmax * k / 32).map(|bi| f32_at(at + 4 * bi)).collect();
    at += rmax * k / 32 * 4;
    let sums: Vec<i32> = (0..rmax * k / 32).map(|bi| i32_at(at + 4 * bi)).collect();
    at += rmax * k / 32 * 4;
    for row in 0..rmax {
        for b in 0..k / 32 {
            let blk = &x[row * k + 32 * b..][..32];
            let amax = blk.iter().fold(0f32, |m, v| m.max(v.abs()));
            let (s, inv) = (amax / 127.0, if amax > 0.0 { 127.0 / amax } else { 0.0 });
            assert_eq!(d[row * (k / 32) + b].to_bits(), s.to_bits(), "row {row} block {b}: the scale");
            let mut total = 0i32;
            for i in 0..32 {
                let want = (blk[i] * inv).round_ties_even() as i8;
                assert_eq!(q[row * k + 32 * b + i], want, "row {row} block {b} value {i}: the code");
                total += i32::from(want);
            }
            assert_eq!(sums[row * (k / 32) + b], total, "row {row} block {b}: the code sum");
        }
    }
    // The products, against the exact sum of the weights' values times the activation's codes and scales; then each
    // row of the 13 the bits of that row alone.
    for (ty, w) in &weights {
        let mut values = Vec::new();
        ty.dequantize(w, &mut values);
        let mut chunk13 = Vec::new();
        for rows in [1usize, 3, 4, 5, 13] {
            for row in 0..rows {
                for o in 0..n {
                    let (mut exact, mut mag) = (0f64, 0f64);
                    for i in 0..k {
                        let t = f64::from(values[o * k + i]) * f64::from(q[row * k + i]) * f64::from(d[row * (k / 32) + i / 32]);
                        exact += t;
                        mag += t.abs();
                    }
                    let got = f32_at(at + 4 * (row * n + o));
                    assert!((f64::from(got) - exact).abs() <= EXACT * mag.max(1e-30), "{ty:?}, {rows} rows: output {row},{o} {got} against {exact}");
                    if rows == 13 {
                        chunk13.push(got.to_bits());
                    }
                }
            }
            at += rows * n * 4;
        }
        for row in 0..rmax {
            for o in 0..n {
                assert_eq!(f32_at(at + 4 * o).to_bits(), chunk13[row * n + o], "{ty:?}: row {row} of a chunk of 13, output {o}, the bits it is alone");
            }
            at += n * 4;
        }
    }
    // The norm of each row's sum with its residual, by the weight.
    for row in 0..r {
        let e: Vec<f64> = (0..c).map(|i| f64::from(nx[row * c + i]) + f64::from(nr[row * c + i])).collect();
        let inv = 1.0 / (e.iter().map(|v| v * v).sum::<f64>() / c as f64 + 1e-5).sqrt();
        for i in 0..c {
            let want = e[i] * inv * f64::from(nw[i]);
            let got = f64::from(f32_at(at + 4 * (row * c + i)));
            assert!((got - want).abs() <= EXACT * (1.0 + want.abs()), "norm row {row} value {i}: {got} against {want}");
        }
    }
    at += r * c * 4;
    // The attention: row t attends positions [lo, 6 + t) under a window of 5, scores scaled by 1/4 and capped at 3, a
    // sink in each head's denominator; each row's heads `opitch` floats from the last's.
    for t in 0..r {
        let end = 6 + t;
        let lo = end.saturating_sub(5);
        for head in 0..h {
            let g = head / (h / kv);
            let scores: Vec<f64> = (lo..end)
                .map(|j| {
                    let dot: f64 = (0..hd).map(|e| f64::from(aq[(t * h + head) * hd + e]) * f64::from(ak[j * kv * hd + g * hd + e])).sum();
                    3.0 * (dot * 0.25 / 3.0).tanh()
                })
                .collect();
            let m = scores.iter().fold(f64::from(sk[head]), |m, s| m.max(*s));
            let den = (f64::from(sk[head]) - m).exp() + scores.iter().map(|s| (s - m).exp()).sum::<f64>();
            for e in 0..hd {
                let want: f64 = (lo..end).map(|j| (scores[j - lo] - m).exp() / den * f64::from(av[j * kv * hd + g * hd + e])).sum();
                let got = f64::from(f32_at(at + 4 * (t * opitch + head * hd + e)));
                assert!((got - want).abs() <= EXACT * (1.0 + want.abs()), "attention row {t} head {head} value {e}: {got} against {want}");
            }
        }
    }
    at += r * opitch * 4;
    // The split attention: query row t of the prompt attends positions [0, split − 5 + t) of a cache of 3·split − 68
    // positions, so rows 0–5 take one pass and rows 6–9 two splits merged; each of those rows alone as a step attends the
    // same and is the same bits; three rows within a window of split + 44 at the cache's end take two splits each; two
    // rows at the cache's end with no sink and no cap take three; the prompt again over binary16 caches, and with heads
    // 11 wide, packed 11 apart in the same rows (a vector's width and a tail of three); and over the cache kept in int8
    // rows, plain and turned. Each is within
    // `EXACT` of the f64 reference of the same inputs, its splits run one after another in a unit or spread over the
    // threads the same bits.
    assert!(LIBRARY_X86_AVX2_VNNI.contains(&format!("constexpr int LK_ATTN_SPLIT = {ATTN_SPLIT};")), "the library splits where the link counts its splits");
    let wide = |v: &[f32]| -> Vec<f64> { v.iter().map(|&x| f64::from(x)).collect() };
    let widened = |v: &[f32]| -> Vec<f64> { v.iter().map(|&x| f64::from(f16_to_f32(f32_to_f16(x)))).collect() };
    let (dkc, dvc, hkc, hvc) = (wide(&skc), wide(&svc), widened(&skc), widened(&svc));
    let floats = |at: usize, len: usize| -> Vec<u32> { (0..len).map(|i| f32_at(at + 4 * i).to_bits()).collect() };
    #[allow(clippy::too_many_arguments)]
    let check = |at: usize, what: &str, rows: usize, end0: usize, win: usize, kc: &[f64], vc: &[f64], sinks: bool, cap: f64, width: usize| {
        for t in 0..rows {
            let end = end0 + t;
            let lo = if win > 0 { end.saturating_sub(win) } else { 0 };
            for head in 0..h {
                let g = head / (h / kv);
                let scores: Vec<f64> = (lo..end)
                    .map(|j| {
                        let dot: f64 = (0..width).map(|e| f64::from(sq[t * h * hd + head * width + e]) * kc[j * kv * hd + g * width + e]).sum();
                        if cap > 0.0 { cap * (dot * 0.25 / cap).tanh() } else { dot * 0.25 }
                    })
                    .collect();
                let sink = sinks.then(|| f64::from(sk[head]));
                let m = scores.iter().fold(sink.unwrap_or(f64::NEG_INFINITY), |m, s| m.max(*s));
                let den = sink.map_or(0.0, |s| (s - m).exp()) + scores.iter().map(|s| (s - m).exp()).sum::<f64>();
                for e in 0..width {
                    let want: f64 = (lo..end).map(|j| (scores[j - lo] - m).exp() / den * vc[j * kv * hd + g * width + e]).sum();
                    let got = f64::from(f32_at(at + 4 * (t * sop + head * width + e)));
                    assert!((got - want).abs() <= EXACT * (1.0 + want.abs()), "{what}: row {t} (span {}) head {head} value {e}: {got} against {want}", end - lo);
                }
            }
        }
    };
    #[allow(clippy::too_many_arguments)]
    let pair = |at: &mut usize, what: &str, rows: usize, end0: usize, win: usize, kc: &[f64], vc: &[f64], sinks: bool, cap: f64, width: usize| -> Vec<u32> {
        check(*at, what, rows, end0, win, kc, vc, sinks, cap, width);
        let (a, b) = (floats(*at, rows * sop), floats(*at + rows * sop * 4, rows * sop));
        assert_eq!(a, b, "{what}: its splits spread over the threads are the bits of its splits one after another");
        *at += 2 * rows * sop * 4;
        a
    };
    let prompt = pair(&mut at, "the prompt", sr, ATTN_SPLIT - 5, 0, &dkc, &dvc, true, 3.0, hd);
    for t in 0..sr {
        assert_eq!(floats(at, h * hd), prompt[t * sop..t * sop + h * hd], "row {t} attended alone as a step is the bits of the prompt's row {t}");
        at += sop * 4;
    }
    pair(&mut at, "the windowed rows", 3, sl - 3, ATTN_SPLIT + 44, &dkc, &dvc, true, 3.0, hd);
    pair(&mut at, "the rows with no sink", 2, sl - 2, 0, &dkc, &dvc, false, 0.0, hd);
    pair(&mut at, "the prompt over binary16 caches", sr, ATTN_SPLIT - 5, 0, &hkc, &hvc, true, 3.0, hd);
    pair(&mut at, "the prompt's heads 11 wide", sr, ATTN_SPLIT - 5, 0, &dkc, &dvc, true, 3.0, hd - 5);
    // The cache kept in int8 rows, its first unit zero: every unit's codes and scale the reference's bit for bit (each
    // giving back what `Keep::keep_row` keeps), and the prompt over the rows kept so, plain and turned.
    let mut zeroed = (skc.clone(), svc.clone());
    zeroed.0[..hd].fill(0.0);
    zeroed.1[..hd].fill(0.0);
    for turned in [false, true] {
        let keep = if turned { Keep::Int8RowsRotated } else { Keep::Int8Rows };
        let mut kept = Vec::new();
        for (what, values) in [("keys", &zeroed.0), ("values", &zeroed.1)] {
            let units = values.len() / hd;
            let codes: Vec<i8> = bytes[at..at + values.len()].iter().map(|&b| b as i8).collect();
            at += values.len();
            let scales: Vec<u16> = (0..units).map(|u| u16::from_le_bytes([bytes[at + 2 * u], bytes[at + 2 * u + 1]])).collect();
            at += units * 2;
            let mut back = Vec::new();
            for u in 0..units {
                let row = &values[u * hd..(u + 1) * hd];
                let (want, scale) = int8_unit(row, turned);
                assert_eq!((&codes[u * hd..(u + 1) * hd], scales[u]), (&want[..], scale), "{what}, turned {turned}: unit {u}'s codes and scale");
                let meaning = keep.keep_row(&wide(row)).unwrap();
                let got = int8_values(&codes[u * hd..(u + 1) * hd], scales[u], turned);
                assert!(got == meaning, "{what}, turned {turned}: unit {u} gives back what Keep::keep_row keeps");
                back.extend(got);
            }
            assert!(scales[0] == 0 && codes[..hd].iter().all(|&c| c == 0), "{what}: a zero unit's scale and codes are zero");
            kept.push(back);
        }
        pair(&mut at, &format!("the prompt over int8 rows, turned {turned}"), sr, ATTN_SPLIT - 5, 0, &kept[0], &kept[1], true, 3.0, hd);
    }
    // e^x within one and a half ulp of the exact value, the spacing of the floats about it.
    for i in 0..=20000 {
        let v = f32_at(at + 4 * i);
        let got = f32_at(at + 4 * (20001 + i));
        let exact = f64::from(v).exp();
        let ulp = (f64::from(f32::from_bits((exact as f32).to_bits() + 1)) - f64::from(exact as f32)).abs().max(f64::from(f32::from_bits(1)));
        assert!((f64::from(got) - exact).abs() <= 1.5 * ulp, "e^{v}: {got} against {exact}, more than one and a half ulp off");
    }
}
