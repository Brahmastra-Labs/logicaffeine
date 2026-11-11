//! A checkpoint's decode step on the CPU tier (`codegen_cpu::model`), built and timed: the bring-up loop for a model on
//! the host's cores.
//!
//!   model_cpu <gguf> <n_gen> <tok,tok,…> [ctx=1024] [threads=16] [prefill=512] [--repeats=5] [--numerics=int8|precise]
//!             [--cache=f32|f16|int8|int8-rotated] [--apart]
//!
//! Links the model's decode step and prefill chunks of up to `prefill` rows under the x86 AVX2 and AVX-VNNI shim — a
//! product by checkpoint rows reading its activation in one word a block (`int8`, unless given) or as its values
//! (`precise`), its caches `f32`, binary16, int8 rows or turned int8 rows (`--cache`), its fused classes linked whole
//! unless `--apart` links them as their parts — writes the program and its sidecar to `$TMPDIR/logos_model_cpu/`,
//! builds it with the host's C++ compiler, runs the prompt once to warm the caches and then `--repeats` times — each a
//! prompt fed a token at a time and `n_gen` tokens decoded greedily — and prints every run's prefill time and decode
//! rate and their mean and sample standard deviation, the way llama-bench reports its own.

use std::path::Path;
use std::process::Command;

use logicaffeine_compile::codegen_cpu::model::compile_model;
use logicaffeine_compile::codegen_cpu::shim::{CpuActivations, CpuCache, CpuIsa, CpuShim};
use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::spec::ModelSpec;

fn main() {
    let (numerics, args): (Vec<String>, Vec<String>) = std::env::args().partition(|a| a.starts_with("--numerics="));
    let activations = match numerics.last().map(|n| &n["--numerics=".len()..]) {
        None | Some("int8") => CpuActivations::OneWord,
        Some("precise") => CpuActivations::Values,
        Some(other) => panic!("--numerics is int8 or precise, not {other:?}"),
    };
    let (caches, args): (Vec<String>, Vec<String>) = args.into_iter().partition(|a| a.starts_with("--cache="));
    let cache = match caches.last().map(|c| &c["--cache=".len()..]) {
        None | Some("f32") => CpuCache::F32,
        Some("f16") => CpuCache::F16,
        Some("int8") => CpuCache::Int8Rows,
        Some("int8-rotated") => CpuCache::Int8RowsRotated,
        Some(other) => panic!("--cache is f32, f16, int8 or int8-rotated, not {other:?}"),
    };
    let (apart, args): (Vec<String>, Vec<String>) = args.into_iter().partition(|a| a == "--apart");
    let fuse = apart.is_empty();
    let (repeats, args): (Vec<String>, Vec<String>) = args.into_iter().partition(|a| a.starts_with("--repeats="));
    let repeats: usize = repeats.last().map_or(5, |r| r["--repeats=".len()..].parse().expect("--repeats is a count"));
    assert!(repeats >= 2, "--repeats takes at least two runs, for a standard deviation");
    if args.len() < 4 {
        eprintln!("usage: model_cpu <gguf> <n_gen> <tok,tok,…> [ctx=1024] [threads=16] [prefill=512] [--repeats=5] [--numerics=int8|precise] [--cache=f32|f16|int8|int8-rotated] [--apart]");
        std::process::exit(2);
    }
    let ckpt = Path::new(&args[1]);
    let n_gen = &args[2];
    let toks: Vec<&str> = args[3].split(',').collect();
    let ctx: usize = args.get(4).map_or(1024, |c| c.parse().expect("ctx is a count"));
    let threads = args.get(5).cloned().unwrap_or_else(|| "16".to_string());
    let prefill: usize = args.get(6).map_or(512, |p| p.parse().expect("prefill is a count of rows"));
    let gguf = Gguf::open(ckpt).expect("the checkpoint opens");
    let spec = ModelSpec::from_gguf(&gguf).expect("the spec reads");
    let model = compile_model(&spec, ckpt, ctx, prefill, &CpuShim { isa: CpuIsa::X86Avx2Vnni, activations, cache, fuse }).unwrap_or_else(|e| panic!("the CPU tier: {e}"));
    let dir = std::env::temp_dir().join("logos_model_cpu");
    std::fs::create_dir_all(&dir).expect("the program's directory");
    let (src, bin, side) = (dir.join("model.cpp"), dir.join("model"), dir.join("model.side"));
    std::fs::write(&src, model.source()).expect("write the program");
    std::fs::write(&side, model.sidecar()).expect("write the sidecar");
    let built = Command::new("g++")
        .args(["-O3", "-std=c++17", "-mavx2", "-mfma", "-mavxvnni", "-mf16c", "-pthread", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .expect("g++ runs");
    assert!(built.status.success(), "the program builds:\n{}", String::from_utf8_lossy(&built.stderr));
    let (mut decode, mut prefill) = (Vec::new(), Vec::new());
    for run in 0..=repeats {
        let out = Command::new(&bin).arg(ckpt).arg(&side).arg(&threads).arg(n_gen).args(&toks).output().expect("run the program");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "the program runs:\n{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
        let field = |key: &str| stdout.lines().find_map(|l| l.strip_prefix(key)).map(|v| v.trim().to_string());
        if run == 0 {
            println!("TOKENS {}", field("TOKENS:").unwrap_or_default());
            continue;
        }
        let d: f64 = field("DECODE_TOK_PER_S:").expect("the decode rate").parse().unwrap();
        let p: f64 = field("PREFILL_MS:").expect("the prefill time").parse().unwrap();
        println!("RUN {run} decode {d:.2} tok/s  prefill {p:.2} ms");
        decode.push(d);
        prefill.push(p);
    }
    let stats = |v: &[f64]| {
        let mean = v.iter().sum::<f64>() / v.len() as f64;
        let var = v.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (v.len() - 1) as f64;
        (mean, var.sqrt())
    };
    let ((dm, ds), (pm, ps)) = (stats(&decode), stats(&prefill));
    println!("BENCH decode {dm:.2} ± {ds:.2} tok/s  prefill {pm:.2} ± {ps:.2} ms ({} tokens), {repeats} runs after a warm-up, {threads} threads", toks.len());
}
