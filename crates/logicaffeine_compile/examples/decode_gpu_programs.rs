//! Prints the int8 tiny transformer's decode step for the RTX 3060 three ways into OUT_DIR, builds each with nvcc and
//! holds each to its oracle: the hand-written reference (`Int8Model::reference_cuda`, one warp running the whole trace on
//! the device) to the trainer's tokens, and LOGOS's one-block and general programs of the step (`spatial::lower::cuda`,
//! lowered returning the host's token) to the spatial evaluator, each then timed by its bench. It prints a line a
//! program — `reference: MATCH`, `flat: <n> tensors MATCH — … US_PER_STEP <µs>`, `general: …` — and leaves
//! `reference`, `flat_bench.cu` and `general_bench.cu` in OUT_DIR for `scripts/accel/decode-gpu-measure.sh`, which times
//! each sustained with its power; `scripts/accel/decode_gpu_ledger.py` records that run as
//! `benchmarks/silicon/decode_gpu.json`. A second argument, `reference` or `logos`, prints only those, so the script
//! times LOGOS's best launches in a window of their own.
//!
//! Run: `cargo run --release -p logicaffeine-compile --example decode_gpu_programs -- OUT_DIR [reference|logos]`

use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Value};
use logicaffeine_compile::spatial::int8_model::Int8Model;
use logicaffeine_compile::spatial::ir::TensorRole;
use logicaffeine_compile::spatial::lower::cuda::CudaPersistentBackend;
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial_returning;
use logicaffeine_compile::spatial::target::SpatialTarget;
use std::path::Path;
use std::process::Command;

/// Builds `name.cu` in `dir` with nvcc for the RTX 3060 (sm_86) at -O3 and `flags`, and runs it there: its output, or
/// why it did not build.
fn build_run(dir: &Path, name: &str, cu: &str, flags: &[&str]) -> Result<String, String> {
    std::fs::write(dir.join(format!("{name}.cu")), cu).map_err(|e| e.to_string())?;
    let build = Command::new("nvcc")
        .args(["-O3", "-arch=sm_86"])
        .args(flags)
        .arg("-o")
        .arg(dir.join(name))
        .arg(dir.join(format!("{name}.cu")))
        .output()
        .map_err(|e| format!("nvcc: {e}"))?;
    if !build.status.success() {
        return Err(format!("nvcc {name}: {}", String::from_utf8_lossy(&build.stderr)));
    }
    let run = Command::new(dir.join(name)).current_dir(dir).output().map_err(|e| format!("{name}: {e}"))?;
    Ok(format!("{}{}", String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr)))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(out), which) = (args.first(), args.get(1).map(String::as_str)) else {
        eprintln!("usage: decode_gpu_programs OUT_DIR [reference|logos]");
        std::process::exit(2)
    };
    if !matches!(which, None | Some("reference") | Some("logos")) {
        eprintln!("usage: decode_gpu_programs OUT_DIR [reference|logos]");
        std::process::exit(2)
    }
    let dir = Path::new(&out);
    std::fs::create_dir_all(dir).expect("OUT_DIR");
    let model = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(model).expect("the model")).expect("JSON");
    let m = Int8Model::from_json(&j).expect("the model");
    let tokens = |k: &str| -> Vec<i64> { j["golden_trace"][k].as_array().unwrap().iter().map(|t| t.as_i64().unwrap()).collect() };
    let (prompt, golden) = (tokens("prompt_tokens"), tokens("generated_tokens"));

    if which != Some("logos") {
        reference(dir, &m, &prompt, &golden);
    }
    if which != Some("reference") {
        logos(dir, &m, &prompt);
    }
}

/// The hand-written reference, built, run and held to the trainer's tokens.
fn reference(dir: &Path, m: &Int8Model, prompt: &[i64], golden: &[i64]) {
    let reference = build_run(dir, "reference", &m.reference_cuda(prompt, golden.len()).expect("one warp holds the model"), &[])
        .unwrap_or_else(|e| panic!("{e}"));
    let got: Vec<i64> = reference
        .lines()
        .filter_map(|l| l.strip_prefix("GEN "))
        .filter_map(|l| l.split_whitespace().nth(1)?.parse().ok())
        .collect();
    println!("reference: {} — {}", if got == golden { "MATCH" } else { "MISMATCH" }, reference.lines().find(|l| l.starts_with("US_PER_TOKEN")).unwrap_or("no timing"));
}

/// LOGOS's one-block and general programs of the step, each built, run, held to the evaluator and timed by its bench.
fn logos(dir: &Path, m: &Int8Model, prompt: &[i64]) {
    let target = SpatialTarget::rtx3060();
    let step = m.decode_step_lg(prompt[0] as usize, 0, &[], &[]);
    let p = compile_program_to_spatial_returning(&step, &target, &Int8Model::step_reads())
        .expect("a tensor program")
        .program
        .expect("the step lowers whole");
    let want = eval_choreography(&p, &Inputs::from_fills(&p).expect("deterministic fills")).expect("the step evaluates");
    let art = match CudaPersistentBackend::new(target).lower(&p) {
        Ok(SpatialArtifact::Cuda(a)) => a,
        other => panic!("the GPU lowers the step: {other:?}"),
    };
    let mut programs = vec![("flat", art.program_cu.clone(), art.bench_cu.clone())];
    if let Some(g) = &art.general {
        programs.push(("general", g.program_cu.clone(), g.bench_cu.clone()));
    }
    for (label, cu, bench) in programs {
        let printed = build_run(dir, label, &cu, &["-std=c++17"]).unwrap_or_else(|e| panic!("{e}"));
        let (mut ok, mut held) = (true, 0);
        for t in p.tensors.iter().filter(|t| matches!(t.role, TensorRole::Output | TensorRole::State)) {
            let Some(Value::Int(w)) = want.tensors.get(&t.id) else { continue };
            let mut got: Vec<(usize, i64)> = printed
                .lines()
                .filter_map(|l| {
                    let f: Vec<&str> = l.split_whitespace().collect();
                    if f.len() == 4 && f[0] == "OUT" && f[1] == t.id.idx().to_string() {
                        Some((f[2].parse().ok()?, f[3].parse().ok()?))
                    } else {
                        None
                    }
                })
                .collect();
            got.sort();
            ok &= got.into_iter().map(|(_, v)| v).collect::<Vec<_>>() == *w;
            held += 1;
        }
        let timing = build_run(dir, &format!("{label}_bench"), &bench, &["-std=c++17"]).unwrap_or_else(|e| panic!("{e}"));
        println!("{label}: {held} tensors {} — {}", if ok { "MATCH" } else { "MISMATCH" }, timing.trim());
    }
}
