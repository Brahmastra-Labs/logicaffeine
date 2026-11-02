//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! QWEN-TINY (FLOAT) AUTHORED IN LOGOS → EMITTED CUDA → RUNS ON THE GPU ≡ VM — Phase 1.
//!
//! The float Qwen-shaped forward written in LOGOS (`scripts/accel/gen_qwen_tiny_lg.py`, proven ≡ a clean-room
//! f64 oracle on the VM by `qwen_tiny_lg_vm_lock`) is LOWERED TO CUDA by the compiler's own `codegen_cuda`
//! tier (`compile_to_cuda`) and RUN ON THE DEVICE. This is the first rung of "an LLM authored in LOGOS,
//! emitted to CUDA, running on the GPU": the emitted `.cu` reproduces the VM's greedy token at every
//! generation step, produced on the real GPU.
//!
//! This rung asserts CORRECTNESS of the emitted device program (the single-thread whole-program lowering),
//! not yet its speed — the parallel DP4A / int8-activation / CUDA-graph kernels that beat llama.cpp are the
//! subsequent phases. GPU≡VM here composes with VM≡oracle (Phase 0) to give GPU≡oracle transitively.
//!
//! Gated on the tiny model JSON + `python3` + `nvcc` + a live GPU (`/dev/nvidia0`); SKIPS cleanly otherwise.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;
use std::process::Command;

use logicaffeine_compile::compile::{compile_to_cuda, vm_outcome};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}
fn have_nvcc() -> bool {
    Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
fn have_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
}
fn python_available() -> bool {
    Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// The prompt + generation length the tiny model ships with (kept in sync with `qwen_tiny.json`).
fn model_meta() -> (Vec<usize>, usize) {
    let raw = std::fs::read_to_string(root().join("benchmarks/silicon/qwen_tiny.json"))
        .expect("read qwen_tiny.json");
    let j: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let prompt = j["prompt"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_u64().unwrap() as usize)
        .collect();
    let gen_len = j["gen_len"].as_u64().unwrap() as usize;
    (prompt, gen_len)
}

fn gen_lg_argmax(toks: &[usize]) -> String {
    let toks_arg = toks
        .iter()
        .map(|t| t.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let out = Command::new("python3")
        .current_dir(root())
        .args(["scripts/accel/gen_qwen_tiny_lg.py", "--toks", &toks_arg])
        .output()
        .expect("run gen_qwen_tiny_lg.py");
    assert!(
        out.status.success(),
        "generator failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn vm_token(src: &str) -> i64 {
    let out = vm_outcome(src);
    assert!(out.error.is_none(), "VM error: {:?}", out.error);
    out.output
        .split_whitespace()
        .next()
        .expect("a Show")
        .parse()
        .expect("int token")
}

/// The emitted CUDA program's greedy token equals the VM's, at every generation step, on the real GPU.
#[test]
fn qwen_tiny_authored_in_logos_emits_cuda_and_matches_the_vm_on_the_gpu() {
    if !(python_available() && have_nvcc() && have_gpu()) {
        eprintln!("SKIP qwen-tiny CUDA: needs the tiny model + python3 + nvcc + a live GPU (/dev/nvidia0).");
        return;
    }
    let (prompt, gen_len) = model_meta();
    let dir = std::env::temp_dir().join(format!("logos_qwen_tiny_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);

    let mut toks = prompt;
    for step in 0..gen_len {
        let lg = gen_lg_argmax(&toks);
        let vm_tok = vm_token(&lg);

        let artifact = compile_to_cuda(&lg).unwrap_or_else(|e| {
            panic!("step {step}: the LOGOS forward must lower to CUDA — {e:?}")
        });
        let cu = dir.join(format!("qwen_tiny_{step}.cu"));
        let bin = dir.join(format!("qwen_tiny_{step}"));
        std::fs::write(&cu, &artifact.kernel_cu).expect("write .cu");

        let compile = Command::new("nvcc")
            .args(["-O2", "-arch=sm_86"])
            .arg(&cu)
            .arg("-o")
            .arg(&bin)
            .output()
            .expect("run nvcc");
        assert!(
            compile.status.success(),
            "step {step}: emitted CUDA must compile:\n{}",
            String::from_utf8_lossy(&compile.stderr)
        );

        let run = Command::new(&bin).output().expect("run emitted qwen-tiny");
        assert!(
            run.status.success(),
            "step {step}: emitted program run failed:\n{}",
            String::from_utf8_lossy(&run.stderr)
        );
        let stdout = String::from_utf8_lossy(&run.stdout);
        let gpu_tok: i64 = stdout
            .split_whitespace()
            .next()
            .expect("a token")
            .parse()
            .expect("int token");

        assert_eq!(
            gpu_tok, vm_tok,
            "step {step}: emitted-CUDA token {gpu_tok} != VM token {vm_tok} (GPU stdout: {stdout:?})"
        );
        toks.push(vm_tok as usize);
    }
    let _ = std::fs::remove_dir_all(&dir);
    println!("Qwen-tiny authored in LOGOS → emitted CUDA → GPU ≡ VM: greedy tokens match over {gen_len} steps.");
}
