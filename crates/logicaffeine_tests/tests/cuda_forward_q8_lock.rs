//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! INT8 (Q8_0) WEIGHT GEMV — a LOGOS activation matmul'd against a REAL Qwen3-8B weight kept INT8-RESIDENT,
//! dequantized in-kernel on the GPU. This is what makes real 8B fit a 12 GB card (8 GB int8, not 32 GB f32)
//! and is the DP4A path that beats llama.cpp.
//!
//! The orchestrator's `matmul` against a tensor named in the Q8 manifest loads it as int8 quants + f32 block
//! scales (never a 4×-larger f32 buffer) and emits a dequant-in-kernel GEMV: `y[n] = Σ_k a[k]·quant[n][k]·
//! scale[n][k/32]` — exactly `codegen_cuda::qwen`'s certified matvec. Here the weight is a real tile of
//! `blk.0.attn_q.weight` from the Q8_0 checkpoint; the GPU result matches a clean-room reference over the
//! same int8 quants + scales.
//!
//! Gated on the checkpoint + `nvcc` + a live GPU; SKIPS cleanly otherwise.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::codegen_cuda::qwen_emit::Q8Ref;
use logicaffeine_compile::compile::compile_to_cuda_forward_q8;

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

/// `Y = A · Wq` where `Wq` is a real Qwen3-8B weight kept INT8-resident (quants + scales), computed as a
/// dequant-in-kernel GEMV on the GPU, matches the clean-room reference over the same int8 data.
#[test]
fn logos_matmul_against_a_real_int8_qwen_weight_dequants_in_kernel_on_the_gpu() {
    if !(model_path().exists() && have_nvcc() && have_gpu()) {
        eprintln!("SKIP int8 Q8_0 GEMV: needs the 8B checkpoint + nvcc + a live GPU.");
        return;
    }
    let mut g = Gguf::open(&model_path()).expect("open gguf");
    let name = "blk.0.attn_q.weight";
    let (in_f, _out_f) = g.dims2(name).expect("dims");
    let (quants, scales) = g.q8_0_matrix(name).expect("Q8_0"); // quants [out*in] i8, scales [out*nb] f32
    let nb_full = in_f / 32;

    // A K×N tile of the real weight, INT8-resident: K=64 (2 blocks) input, N=8 outputs.
    let (k, n) = (64usize, 8usize);
    let nb = k / 32;
    let mut q_tile = vec![0i8; n * k];
    let mut s_tile = vec![0f32; n * nb];
    for out in 0..n {
        for t in 0..k {
            q_tile[out * k + t] = quants[out * in_f + t];
        }
        for b in 0..nb {
            s_tile[out * nb + b] = scales[out * nb_full + b];
        }
    }
    assert!(
        q_tile.iter().any(|&q| q != 0),
        "the real int8 weight tile must be nonzero"
    );

    let dir = std::env::temp_dir().join(format!("logos_q8_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (qf, sf) = (dir.join("wq.i8"), dir.join("wq.s"));
    std::fs::write(&qf, q_tile.iter().map(|&q| q as u8).collect::<Vec<u8>>()).unwrap();
    let mut sbytes = Vec::new();
    for &s in &s_tile {
        sbytes.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(&sf, &sbytes).unwrap();

    // The LOGOS program: an activation, and a Q8_0 weight tensor (shape only; data comes from the manifest).
    let a: Vec<f32> = (0..k).map(|t| ((t % 5) as f32 - 2.0) * 0.1).collect();
    let mut src = String::from("## Main\nLet A be a 1 by 64 tensor of Real.\n");
    for (t, &av) in a.iter().enumerate() {
        src.push_str(&format!("Set item {} of A to {:?}.\n", t + 1, av));
    }
    src.push_str("Let Wq be a 64 by 8 tensor of Real.\n");
    src.push_str("Let Y be A matmul Wq.\n");
    for o in 0..n {
        src.push_str(&format!("Show item {} of Y.\n", o + 1));
    }

    let mut q8 = HashMap::new();
    q8.insert(
        "Wq".to_string(),
        Q8Ref {
            quants: qf.to_string_lossy().to_string(),
            scales: sf.to_string_lossy().to_string(),
            in_dim: k,
            out_dim: n,
        },
    );
    let cu = compile_to_cuda_forward_q8(&src, &HashMap::new(), &q8).expect("Q8 forward must lower");
    assert!(
        cu.contains("k0_q8gemv"),
        "the matmul against the int8 weight must be a dequant-in-kernel GEMV:\n{cu}"
    );
    assert!(
        cu.contains("signed char* buf_Wq_q") && cu.contains("float* buf_Wq_s"),
        "int8 quants + f32 scales must be resident"
    );

    let (cu_path, bin) = (dir.join("q.cu"), dir.join("q"));
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
        "emitted Q8 GEMV must compile:\n{}",
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

    // Clean-room reference: y[out] = Σ_k a[k] · quant[out][k] · scale[out][k/32].
    let mut refy = vec![0f32; n];
    for out in 0..n {
        let mut acc = 0f32;
        for t in 0..k {
            acc += a[t] * q_tile[out * k + t] as f32 * s_tile[out * nb + t / 32];
        }
        refy[out] = acc;
    }

    assert_eq!(got.len(), n, "expected {n} outputs, got {got:?}");
    for (o, (gv, rv)) in got.iter().zip(&refy).enumerate() {
        assert!(
            (gv - rv).abs() < 1e-3,
            "int8 GEMV out {o}: GPU {gv} vs reference {rv}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
    println!("INT8 Q8_0 GEMV: a LOGOS activation matmul'd against a REAL int8-resident Qwen3-8B weight ran dequant-in-kernel on the GPU ≡ reference {refy:?}.");
}
