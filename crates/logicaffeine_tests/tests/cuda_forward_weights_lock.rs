//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! WEIGHT-HANDLE — the orchestrator loads a REAL Qwen3-8B weight from the checkpoint and matmuls a LOGOS
//! activation against it on the GPU. The gating mechanism the real-8B forward needs (Phase 2/3).
//!
//! A transformer's weights are gigabytes — they cannot be inlined into the `.lg`. This lock proves the
//! weight-handle path: a `.lg` names a weight tensor by shape, a manifest maps it to a file of its float
//! values (here: a real tile of `blk.0.attn_q.weight`, dequantized from the Q8_0 GGUF), and the emitted host
//! LOADS it at runtime (`fread`) before launching the matmul kernel. The GPU's `A · W` over the real weight
//! matches a clean-room f32 reference. (The VM can't check this leg — the weights live outside the `.lg`,
//! which is the entire point of a weight handle.)
//!
//! Gated on the checkpoint + `nvcc` + a live GPU; SKIPS cleanly otherwise.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::compile::compile_to_cuda_forward_weighted;

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
fn model_path() -> PathBuf {
    manifest().join("../../models/qwen3-8b/Qwen_Qwen3-8B-Q8_0.gguf")
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

/// A LOGOS activation `A · Wq` where `Wq` is a real Qwen3-8B weight tile loaded from the checkpoint, computed
/// as a composed CUDA matmul on the GPU, matches the clean-room reference over that same real weight.
#[test]
fn orchestrator_matmuls_a_logos_activation_against_a_real_qwen_weight_on_the_gpu() {
    if !(model_path().exists() && have_nvcc() && have_gpu()) {
        eprintln!("SKIP weight-handle: needs the 8B checkpoint + nvcc + a live GPU.");
        return;
    }
    // ── extract a real weight tile from the Q8_0 checkpoint ─────────────────────────────────────────
    let mut g = Gguf::open(&model_path()).expect("open gguf");
    let name = "blk.0.attn_q.weight";
    let (in_f, _out_f) = g.dims2(name).expect("weight dims");
    let (quants, scales) = g.q8_0_matrix(name).expect("Q8_0 matrix");
    let nb = in_f / 32;
    let (k, n) = (32usize, 8usize); // a K×N tile (one Q8_0 block wide, 8 output cols)

    // Wq[t][o] = dequant(W_gguf[o][t]) — the transpose into the matmul's [K, N] row-major layout.
    let mut wq = vec![0f32; k * n];
    for t in 0..k {
        for o in 0..n {
            wq[t * n + o] = quants[o * in_f + t] as f32 * scales[o * nb]; // t < 32 → block 0
        }
    }
    // The real weight must not be trivially zero (a dead check would pass on garbage).
    assert!(
        wq.iter().any(|&w| w != 0.0),
        "the real weight tile must carry nonzero values"
    );

    let dir = std::env::temp_dir().join(format!("logos_wh_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let wf = dir.join("wq.f32");
    let mut bytes = Vec::with_capacity(wq.len() * 4);
    for &w in &wq {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    std::fs::write(&wf, &bytes).unwrap();

    // ── the LOGOS program: an activation, and a weight tensor loaded from the checkpoint ────────────
    let a: Vec<f32> = (0..k).map(|t| ((t % 5) as f32 - 2.0) * 0.25).collect();
    let mut src = String::from("## Main\nLet A be a 1 by 32 tensor of Real.\n");
    for (t, &av) in a.iter().enumerate() {
        src.push_str(&format!("Set item {} of A to {:?}.\n", t + 1, av));
    }
    src.push_str("Let Wq be a 32 by 8 tensor of Real.\n");
    src.push_str("Let Y be A matmul Wq.\n");
    for o in 0..n {
        src.push_str(&format!("Show item {} of Y.\n", o + 1));
    }

    let mut weights = HashMap::new();
    weights.insert("Wq".to_string(), (wf.to_string_lossy().to_string(), k * n));
    let cu = compile_to_cuda_forward_weighted(&src, &weights).expect("weighted forward must lower");
    assert!(
        cu.contains("fread(buf_Wq"),
        "the weight must be loaded from its file by the emitted host:\n{cu}"
    );
    assert!(
        cu.contains("k0_matmul"),
        "the activation·weight matmul must lower to a parallel kernel"
    );

    // ── run on the GPU ─────────────────────────────────────────────────────────────────────────────
    let cu_path = dir.join("wh.cu");
    let bin = dir.join("wh");
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
        "emitted weighted forward must compile:\n{}",
        String::from_utf8_lossy(&comp.stderr)
    );
    let run = Command::new(&bin).output().expect("run");
    assert!(
        run.status.success(),
        "run failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let got: Vec<f32> = String::from_utf8_lossy(&run.stdout)
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect();

    // ── clean-room reference over the SAME real weight ─────────────────────────────────────────────
    let mut refy = vec![0f32; n];
    for o in 0..n {
        let mut s = 0f32;
        for t in 0..k {
            s += a[t] * wq[t * n + o];
        }
        refy[o] = s;
    }

    assert_eq!(got.len(), n, "expected {n} outputs, got {got:?}");
    for (gv, rv) in got.iter().zip(&refy) {
        assert!(
            (gv - rv).abs() < 1e-3,
            "GPU A·W over the real Qwen weight {gv} vs reference {rv}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
    println!("Weight-handle: a LOGOS activation matmul'd against a REAL Qwen3-8B weight tile ran on the GPU ≡ reference {refy:?}.");
}
