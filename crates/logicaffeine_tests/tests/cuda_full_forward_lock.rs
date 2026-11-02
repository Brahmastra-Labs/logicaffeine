//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! FULL TRANSFORMER FORWARD — a COMPLETE multi-layer transformer forward authored in LOGOS, composed by the
//! orchestrator into ~36 chained parallel CUDA kernels, run on the GPU, producing the SAME logits (and the
//! SAME argmax next-token) as the VM.
//!
//! This is the whole thing at small scale: per layer — RMSNorm → Q/K/V projections → attention
//! (transpose · Q·Kᵀ · softmax · P·V) → O-projection → residual → RMSNorm → SwiGLU FFN (gate/up matmuls,
//! SiLU, hadamard, down matmul) → residual — then a final RMSNorm and the LM-head matmul to logits. Every
//! stage is a first-class LOGOS tensor op; the orchestrator emits one parallel kernel per stage and a host
//! that chains them through device buffers. The GPU's logits equal the VM's within f32 tolerance, and the
//! greedy token (argmax) is identical. Scaling this to Qwen3-8B is the same graph over the GGUF weight
//! manifest with Q8_0-resident weights.
//!
//! Gated on `nvcc` + a live GPU; SKIPS cleanly otherwise (emission + the stage count are still asserted).
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::process::Command;

use logicaffeine_compile::compile::{compile_to_cuda_forward, vm_outcome};

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

const D: usize = 8;
const DFF: usize = 16;
const VOCAB: usize = 16;
const LAYERS: usize = 2;

/// A small deterministic weight value for `(tag, index)` — a stable pseudo-random in a small range so exp/silu
/// stay well-conditioned. Both the VM and the GPU read the SAME `.lg`, so any choice keeps them in lockstep.
fn wv(tag: u64, i: usize) -> f64 {
    let h = (tag.wrapping_mul(2654435761).wrapping_add(i as u64 * 40503)) % 1000;
    (h as f64 / 1000.0 - 0.5) * 0.3
}

/// Emit `Let <name> be a <rows> by <cols> tensor of Real.` + its `Set item` values (seeded by `tag`).
fn tensor(src: &mut String, name: &str, rows: usize, cols: usize, tag: u64) {
    src.push_str(&format!(
        "Let {name} be a {rows} by {cols} tensor of Real.\n"
    ));
    for i in 0..rows * cols {
        src.push_str(&format!(
            "Set item {} of {name} to {:?}.\n",
            i + 1,
            wv(tag, i)
        ));
    }
}

/// Build a complete `LAYERS`-layer transformer forward over a seq-length-1 input, ending in the last-position
/// logits (`1 × VOCAB`) which the program `Show`s.
fn build_forward() -> String {
    let mut s = String::from("## Main\n");
    tensor(&mut s, "x0", 1, D, 1);
    let mut xin = "x0".to_string();
    for l in 0..LAYERS {
        let t = |k: u64| 1000 + l as u64 * 100 + k;
        tensor(&mut s, &format!("anorm{l}"), 1, D, t(1));
        s.push_str(&format!(
            "Let xn{l} be rmsnorm of {xin} with weight anorm{l}.\n"
        ));
        tensor(&mut s, &format!("Wq{l}"), D, D, t(2));
        tensor(&mut s, &format!("Wk{l}"), D, D, t(3));
        tensor(&mut s, &format!("Wv{l}"), D, D, t(4));
        s.push_str(&format!("Let q{l} be xn{l} matmul Wq{l}.\n"));
        s.push_str(&format!("Let k{l} be xn{l} matmul Wk{l}.\n"));
        s.push_str(&format!("Let v{l} be xn{l} matmul Wv{l}.\n"));
        s.push_str(&format!("Let kt{l} be transpose of k{l}.\n"));
        s.push_str(&format!("Let sc{l} be q{l} matmul kt{l}.\n"));
        s.push_str(&format!("Let p{l} be softmax of sc{l}.\n"));
        s.push_str(&format!("Let ctx{l} be p{l} matmul v{l}.\n"));
        tensor(&mut s, &format!("Wo{l}"), D, D, t(5));
        s.push_str(&format!("Let ao{l} be ctx{l} matmul Wo{l}.\n"));
        s.push_str(&format!("Let xr{l} be {xin} plus ao{l}.\n"));
        tensor(&mut s, &format!("fnorm{l}"), 1, D, t(6));
        s.push_str(&format!(
            "Let xn2{l} be rmsnorm of xr{l} with weight fnorm{l}.\n"
        ));
        tensor(&mut s, &format!("Wg{l}"), D, DFF, t(7));
        tensor(&mut s, &format!("Wu{l}"), D, DFF, t(8));
        s.push_str(&format!("Let g{l} be xn2{l} matmul Wg{l}.\n"));
        s.push_str(&format!("Let u{l} be xn2{l} matmul Wu{l}.\n"));
        s.push_str(&format!("Let sg{l} be silu of g{l}.\n"));
        s.push_str(&format!("Let h{l} be sg{l} hadamard u{l}.\n"));
        tensor(&mut s, &format!("Wd{l}"), DFF, D, t(9));
        s.push_str(&format!("Let dn{l} be h{l} matmul Wd{l}.\n"));
        s.push_str(&format!("Let xout{l} be xr{l} plus dn{l}.\n"));
        xin = format!("xout{l}");
    }
    tensor(&mut s, "onorm", 1, D, 9000);
    s.push_str(&format!("Let xf be rmsnorm of {xin} with weight onorm.\n"));
    tensor(&mut s, "Wout", D, VOCAB, 9001);
    s.push_str("Let logits be xf matmul Wout.\n");
    for o in 0..VOCAB {
        s.push_str(&format!("Show item {} of logits.\n", o + 1));
    }
    s
}

fn argmax(v: &[f64]) -> usize {
    let mut best = 0;
    for i in 1..v.len() {
        if v[i] > v[best] {
            best = i;
        }
    }
    best
}

/// The whole forward — a complete transformer authored in LOGOS — composes to chained parallel kernels, runs
/// on the GPU, and produces the VM's logits and the VM's greedy token.
#[test]
fn a_complete_transformer_forward_authored_in_logos_runs_on_the_gpu_and_matches_the_vm() {
    let src = build_forward();
    let cu = compile_to_cuda_forward(&src).expect("the full forward must lower to composed CUDA");
    // The composed program must contain the whole stage vocabulary, many kernels deep.
    for kern in ["k0_", "_row", "_matmul", "_transpose", "_silu", "_elem"] {
        assert!(
            cu.contains(kern),
            "the full forward must compose {kern} kernels:\n{}",
            &cu[..cu.len().min(600)]
        );
    }
    let n_kernels = cu.matches("__global__").count();
    assert!(
        n_kernels >= 30,
        "a {LAYERS}-layer forward should compose many parallel kernels, got {n_kernels}"
    );

    if !(have_nvcc() && have_gpu()) {
        eprintln!(
            "SKIP full-forward GPU run: needs nvcc + a live GPU. ({n_kernels} kernels emitted.)"
        );
        return;
    }

    let vm: Vec<f64> = {
        let out = vm_outcome(&src);
        assert!(out.error.is_none(), "VM error: {:?}", out.error);
        out.output
            .split_whitespace()
            .map(|s| s.parse().unwrap())
            .collect()
    };
    assert_eq!(
        vm.len(),
        VOCAB,
        "VM must produce {VOCAB} logits, got {}",
        vm.len()
    );

    let dir = std::env::temp_dir().join(format!("logos_fullfwd_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (cu_path, bin) = (dir.join("f.cu"), dir.join("f"));
    std::fs::write(&cu_path, &cu).unwrap();
    let comp = Command::new("nvcc")
        .args(["-O2", "-arch=sm_86"])
        .arg(&cu_path)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("nvcc");
    assert!(
        comp.status.success(),
        "the {n_kernels}-kernel forward must compile:\n{}",
        String::from_utf8_lossy(&comp.stderr)
    );
    let run = Command::new(&bin).output().expect("run");
    assert!(
        run.status.success(),
        "run failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let gpu: Vec<f32> = String::from_utf8_lossy(&run.stdout)
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(
        gpu.len(),
        VOCAB,
        "GPU must produce {VOCAB} logits, got {}",
        gpu.len()
    );
    // Logits agree within f32 tolerance (a 2-layer forward accumulates rounding across dozens of kernels)…
    for (o, (g, r)) in gpu.iter().zip(&vm).enumerate() {
        assert!((*g as f64 - r).abs() < 2e-2, "logit {o}: GPU {g} vs VM {r}");
    }
    // …and the greedy next token is IDENTICAL.
    let gpu_f64: Vec<f64> = gpu.iter().map(|&x| x as f64).collect();
    assert_eq!(
        argmax(&gpu_f64),
        argmax(&vm),
        "the greedy token must match: GPU logits {gpu:?} vs VM {vm:?}"
    );
    println!(
        "COMPLETE transformer forward authored in LOGOS → {n_kernels} chained parallel CUDA kernels on the GPU ≡ VM; greedy token = {}.",
        argmax(&vm)
    );
}
