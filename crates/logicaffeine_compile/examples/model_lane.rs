//! A checkpoint's compiled generation lane (`codegen_cuda::lane`), built and run over a llama.cpp golden corpus: the
//! bring-up loop for a model on the GPU.
//!
//!   model_lane <gguf> <golden-f32.json> <golden-cpu.json> [numerics=int8|two|precise] [prefill=16] [ctx=128]
//!   model_lane <gguf> --bench <numerics> <n_gen> <tok,tok,…> [prefill=16] [ctx=1024]
//!   model_lane <gguf> --plan <numerics> [prefill=16] [ctx=1024]
//!
//! `--hold=<rule>,<rule>` anywhere compiles the lane with those rules of `plan::RULES` held back
//! (`lane::compile_holding`): the same program without them, to measure what each is worth. `--batch=<B>` compiles the
//! lane with its batch step of `B` sequences (`lane::compile_batch_holding`); `--bench` then decodes `B` prompts
//! together, sequence `b`'s the given tokens each moved `7919 b` on modulo the vocabulary, and its decode rate is every
//! sequence's tokens a second, the batch's aggregate. `--state=bf16` stores the gated-delta memories in bf16 between
//! steps (the `recurrent_state` knob; `f32` unless given); `--cache=f16` stores every attention's cache in binary16 (the
//! `cache_precision` knob; `f32` unless given). `--device-bytes=<n>` runs the
//! program on n bytes of the device, the matrices past them read from mapped host memory (`lane::residence`); without
//! it the program takes the memory it finds free.
//! For every prompt the lane generates the golden's continuation length and prints how many steps it agrees with
//! llama.cpp's `f32` run and with its shipped run, and how far its logits after the prompt sit from the `f32` run's top
//! five beside how far the shipped run's sit, and a `GOLDEN <p> TOKENS … FIRST <id>=<logit> …` line — its tokens, and its
//! logits after the prompt of the `f32` run's top five, which the scoreboard's quality column reads; then the totals and
//! the decode rate. `--bench` runs one prompt once to warm
//! the GPU out of its idle clocks, then `--repeats=<n>` times (5 unless given, as llama-bench's `-r 5`), and prints every
//! run's decode and prefill rates and their mean and sample standard deviation, the way llama-bench reports its own,
//! after the identity of each program the lane compiled and the program's provenance line. Needs `nvcc` and a GPU;
//! the program and its sidecar land in `$TMPDIR/logos_model_lane/`. `--plan` builds nothing: it prints what the program
//! allocates (`Lane::plan`), where it places its matrices on the chip compiled for and on the `--device-bytes` given,
//! the weight bytes a decode step reads a token (`Gguf::decode_read_bytes`) and whether it routes through experts, and
//! the decode step's launches: how many, how many of each kind (`plan::kind`), and each in order with what it writes;
//! with `--batch=<B>`, the batch step's census beside it.

use std::path::{Path, PathBuf};
use std::process::Command;

use logicaffeine_compile::codegen_cuda::knobs::{CachePrecision, CudaKnobs, StatePrecision};
use logicaffeine_compile::codegen_cuda::lane::plan::kind;
use logicaffeine_compile::codegen_cuda::lane::residence::place;
use logicaffeine_compile::codegen_cuda::lane::{compile_batch_holding, compile_holding, LaneOptions};
use logicaffeine_compile::codegen_cuda::qwen_emit::Q8MatKind;
use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::spec::ModelSpec;

fn ids(v: &serde_json::Value) -> Vec<u32> {
    v.as_array().expect("ids").iter().map(|t| t.as_u64().expect("an id") as u32).collect()
}

/// A golden's prompts: tokens, continuation, and the top logits after the prompt.
fn golden(path: &str) -> Vec<(Vec<u32>, Vec<u32>, Vec<(u32, f64)>)> {
    let g: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).expect("the golden")).expect("json");
    g["prompts"]
        .as_array()
        .expect("prompts")
        .iter()
        .map(|p| {
            let top = p["top"].as_array().expect("top").iter().map(|e| (e[0].as_u64().unwrap() as u32, e[1].as_f64().unwrap())).collect();
            (ids(&p["tokens"]), ids(&p["continuation"]), top)
        })
        .collect()
}

/// Print where a run's program placed its matrices.
fn residence(stdout: &str) {
    let field = |key: &str| stdout.lines().find_map(|l| l.strip_prefix(key)).map_or("?", str::trim);
    println!("RESIDENCE {} bytes on the device, {} in host memory, {} matrices moved", field("DEVICE_BYTES:"), field("HOST_BYTES:"), field("MOVED:"));
}

fn agreed(a: &[u32], b: &[u32]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

fn main() {
    let (held, args): (Vec<String>, Vec<String>) = std::env::args().partition(|a| a.starts_with("--hold="));
    let held: Vec<&str> = held.iter().flat_map(|h| h["--hold=".len()..].split(',')).collect();
    let (repeats, args): (Vec<String>, Vec<String>) = args.into_iter().partition(|a| a.starts_with("--repeats="));
    let repeats: usize = repeats.last().map_or(5, |r| r["--repeats=".len()..].parse().expect("--repeats is a count"));
    assert!(repeats >= 2, "--repeats takes at least two runs, for a standard deviation");
    let (batch, args): (Vec<String>, Vec<String>) = args.into_iter().partition(|a| a.starts_with("--batch="));
    let batch: Option<usize> = batch.last().map(|b| b["--batch=".len()..].parse().expect("--batch is a count of sequences"));
    let (state, args): (Vec<String>, Vec<String>) = args.into_iter().partition(|a| a.starts_with("--state="));
    let recurrent_state = match state.last().map(|s| &s["--state=".len()..]) {
        None | Some("f32") => StatePrecision::F32,
        Some("bf16") => StatePrecision::Bf16,
        Some(other) => panic!("--state is f32 or bf16, not {other:?}"),
    };
    let (cache, args): (Vec<String>, Vec<String>) = args.into_iter().partition(|a| a.starts_with("--cache="));
    let cache_precision = match cache.last().map(|s| &s["--cache=".len()..]) {
        None | Some("f32") => CachePrecision::F32,
        Some("f16") => CachePrecision::F16,
        Some(other) => panic!("--cache is f32 or f16, not {other:?}"),
    };
    let (device, args): (Vec<String>, Vec<String>) = args.into_iter().partition(|a| a.starts_with("--device-bytes="));
    let device: Vec<String> = device
        .last()
        .map(|d| vec!["--device-bytes".to_string(), d["--device-bytes=".len()..].parse::<usize>().expect("--device-bytes is a byte count").to_string()])
        .unwrap_or_default();
    if args.len() < 4 && !(args.len() >= 3 && args[2] == "--plan") {
        eprintln!("usage: model_lane <gguf> <golden-f32.json> <golden-cpu.json> [int8|two|precise] [prefill] [ctx]");
        std::process::exit(2);
    }
    let ckpt = Path::new(&args[1]);
    let bench = args[2] == "--bench";
    let plan_only = args[2] == "--plan";
    let numerics = match args.get(if bench || plan_only { 3 } else { 4 }).map(String::as_str).unwrap_or("int8") {
        "int8" => Q8MatKind::Int8Act,
        "two" => Q8MatKind::TwoPassInt8,
        "precise" => Q8MatKind::Dequant,
        other => panic!("numerics {other:?} is int8, two or precise"),
    };
    let (pi, ci, cdef) = if bench { (6, 7, 1024) } else if plan_only { (4, 5, 1024) } else { (5, 6, 128) };
    let prefill: usize = args.get(pi).map_or(16, |s| s.parse().expect("prefill is a number"));
    let ctx: usize = args.get(ci).map_or(cdef, |s| s.parse().expect("ctx is a number"));
    let gguf = Gguf::open(ckpt).expect("the checkpoint opens");
    let spec = ModelSpec::from_gguf(&gguf).expect("the spec reads");
    let knobs = CudaKnobs { activations: numerics, recurrent_state, cache_precision, ..CudaKnobs::default() };
    let opts = LaneOptions { ctx, knobs, prefill };
    let lane = match batch {
        Some(b) => compile_batch_holding(&spec, ckpt, &opts, b, &held),
        None => compile_holding(&spec, ckpt, &opts, &held),
    }
    .unwrap_or_else(|e| panic!("the lane compiles: {e}"));
    if plan_only {
        let p = &lane.plan;
        let mib = |b: usize| b as f64 / (1 << 20) as f64;
        println!(
            "PLAN whole {:.1} MiB: arena {:.1}, fixed {:.1}, {} matrices {:.1}; spare {:.1}",
            mib(p.whole()),
            mib(p.arena),
            mib(p.fixed),
            p.order.len(),
            mib(p.order.iter().map(|(_, b)| b).sum()),
            mib(p.spare)
        );
        let mut budgets = vec![("the chip compiled for", lane.residence.clone())];
        if let Some(b) = device.get(1) {
            budgets.push(("--device-bytes", place(p, b.parse().unwrap()).unwrap_or_else(|e| panic!("{e}"))));
        }
        for (on, r) in budgets {
            let moved: Vec<String> = p.order[..r.moved].iter().map(|(m, b)| format!("{m} {:.1}", mib(*b))).collect();
            println!("ON {on}: {:.1} MiB on the device, {:.1} in host memory, {} moved: {}", mib(r.device_bytes), mib(r.host_bytes), r.moved, moved.join(", "));
        }
        println!("DECODE_READ_BYTES: {}", gguf.decode_read_bytes().expect("the checkpoint's formats read"));
        println!("DECODE_ROUTED: {}", gguf.tensors().iter().any(|t| t.name.contains("_exps.")));
        // A view names part of a buffer in place; it launches nothing.
        let mut kinds = std::collections::BTreeMap::new();
        for l in lane.decode().schedule.launches.iter().filter(|l| kind(l) != "view") {
            *kinds.entry(kind(l)).or_insert(0usize) += 1;
            println!("DECODE_STEP: {} {}", kind(l), l.writes().join(","));
        }
        let census: Vec<String> = kinds.iter().map(|(k, n)| format!("{k} {n}")).collect();
        println!("DECODE_LAUNCHES: {} ({})", kinds.values().sum::<usize>(), census.join(", "));
        if let Some(f) = lane.batch() {
            let mut kinds = std::collections::BTreeMap::new();
            for l in f.schedule.launches.iter().filter(|l| kind(l) != "view") {
                *kinds.entry(kind(l)).or_insert(0usize) += 1;
            }
            let census: Vec<String> = kinds.iter().map(|(k, n)| format!("{k} {n}")).collect();
            println!("BATCH_LAUNCHES: {} sequences, {} ({})", f.rows, kinds.values().sum::<usize>(), census.join(", "));
        }
        return;
    }
    let dir: PathBuf = std::env::temp_dir().join("logos_model_lane");
    std::fs::create_dir_all(&dir).expect("the work directory");
    let (cu, bin, side) = (dir.join("lane.cu"), dir.join("lane"), dir.join("lane.side"));
    std::fs::write(&cu, &lane.program.cu).expect("write the program");
    std::fs::write(&side, &lane.program.sidecar).expect("write the sidecar");
    let built = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().expect("nvcc");
    assert!(built.status.success(), "the program builds:\n{}", String::from_utf8_lossy(&built.stderr));
    if bench {
        let n_gen = &args[4];
        let toks: Vec<&str> = args[5].split(',').collect();
        // The batch's prompts, a line a sequence: its count to generate, then its tokens.
        let jobs = dir.join("lane.batch");
        if let Some(b) = batch {
            let lines: Vec<String> = (0..b)
                .map(|s| {
                    let moved: Vec<String> = toks.iter().map(|t| ((t.parse::<usize>().expect("a token id") + 7919 * s) % spec.vocab).to_string()).collect();
                    format!("{n_gen} {}", moved.join(" "))
                })
                .collect();
            std::fs::write(&jobs, lines.join("\n") + "\n").expect("write the batch's prompts");
        }
        println!("PROVENANCE {}", lane.program.cu.lines().next().unwrap_or(""));
        for f in &lane.forms {
            println!("PROGRAM {}", f.identity.line());
        }
        let (mut decode, mut prefill) = (Vec::new(), Vec::new());
        for run in 0..=repeats {
            let mut run_it = Command::new(&bin);
            run_it.arg(ckpt).arg(&side);
            match batch {
                Some(_) => run_it.arg("--batch").arg(&jobs).args(&device),
                None => run_it.arg(n_gen).args(&device).args(&toks),
            };
            let out = run_it.output().expect("run the program");
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(out.status.success(), "the bench runs:\n{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
            let field = |key: &str| stdout.lines().find_map(|l| l.strip_prefix(key)).map(|v| v.trim().parse::<f64>().expect("a number"));
            if run == 0 {
                residence(&stdout);
                continue;
            }
            let d = field("DECODE_TOK_PER_S:").expect("the program prints its decode rate");
            let p = (batch.unwrap_or(1) * toks.len()) as f64 * 1000.0 / field("PREFILL_MS:").expect("the program prints its prefill time");
            println!("RUN {run} decode {d:.2} tok/s  prefill {p:.1} tok/s");
            decode.push(d);
            prefill.push(p);
        }
        let stats = |v: &[f64]| {
            let mean = v.iter().sum::<f64>() / v.len() as f64;
            let var = v.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (v.len() - 1) as f64;
            (mean, var.sqrt())
        };
        let ((dm, ds), (pm, ps)) = (stats(&decode), stats(&prefill));
        println!(
            "BENCH decode {dm:.2} ± {ds:.2} tok/s  prefill {pm:.1} ± {ps:.1} tok/s ({} tokens), {repeats} runs after a warm-up{}",
            toks.len(),
            batch.map_or(String::new(), |b| format!(", {b} sequences together"))
        );
        return;
    }
    let (f32g, cpu) = (golden(&args[2]), golden(&args[3]));
    let (mut ours_f32, mut ours_cpu, mut theirs, mut rates) = (0, 0, 0, Vec::new());
    let (mut our_dev, mut their_dev) = (0f64, 0f64);
    let dump = dir.join("lane.logits");
    for (p, ((tokens, want, top), (_, shipped, shipped_top))) in f32g.iter().zip(&cpu).enumerate() {
        let out = Command::new(&bin)
            .arg(ckpt)
            .arg(&side)
            .arg(want.len().to_string())
            .args(&device)
            .arg("--logits")
            .arg(&dump)
            .args(tokens.iter().map(u32::to_string))
            .output()
            .expect("run the program");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "prompt {p} runs:\n{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
        if p == 0 {
            residence(&stdout);
        }
        let field = |key: &str| stdout.lines().find_map(|l| l.strip_prefix(key)).map(str::trim);
        let got: Vec<u32> = field("TOKENS:").expect("tokens").split_whitespace().map(|t| t.parse().expect("an id")).collect();
        if let Some(r) = field("DECODE_TOK_PER_S:") {
            rates.push(r.parse::<f64>().expect("a rate"));
        }
        let (a, b, c) = (agreed(&got, want), agreed(&got, shipped), agreed(shipped, want));
        let bytes = std::fs::read(&dump).expect("the logits");
        let first: Vec<f32> = bytes[..spec.vocab * 4].chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
        let ours = top.iter().map(|&(id, v)| (first[id as usize] as f64 - v).abs()).fold(0f64, f64::max);
        let shipped_at: std::collections::HashMap<u32, f64> = shipped_top.iter().copied().collect();
        let theirs_dev = top.iter().filter_map(|(id, v)| shipped_at.get(id).map(|s| (s - v).abs())).fold(0f64, f64::max);
        our_dev = our_dev.max(ours);
        their_dev = their_dev.max(theirs_dev);
        println!(
            "prompt {p:2}: lane≡f32 {a:2}  lane≡shipped {b:2}  shipped≡f32 {c:2}  ({})   top logits: lane {ours:.4}  shipped {theirs_dev:.4}",
            want.len()
        );
        let tokens: Vec<String> = got.iter().map(u32::to_string).collect();
        let at: Vec<String> = top.iter().map(|&(id, _)| format!("{id}={}", first[id as usize])).collect();
        println!("GOLDEN {p} TOKENS {} FIRST {}", tokens.join(" "), at.join(" "));
        ours_f32 += a;
        ours_cpu += b;
        theirs += c;
    }
    rates.sort_by(|a, b| b.partial_cmp(a).expect("a rate"));
    println!(
        "TOTAL lane≡f32 {ours_f32}  lane≡shipped {ours_cpu}  shipped≡f32 {theirs}  top logits: lane {our_dev:.4} shipped {their_dev:.4}  best decode {:.2} tok/s",
        rates.first().copied().unwrap_or(0.0)
    );
}
