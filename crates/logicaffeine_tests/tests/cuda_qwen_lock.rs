//! ★ REAL QWEN3-8B WEIGHTS ON THE CERTIFIED KERNEL ★ — Phase 3 grounding.
//!
//! Reads the actual downloaded Qwen3-8B Q8_0 checkpoint, confirms its architecture, pulls a REAL int8
//! weight tile out of block 0's Q projection, and proves our DP4A int8 GEMM computes that tile's product
//! byte-for-byte identical to the certified `eval_systolic` reference (≡ VM ≡ SAT-placed PE ≡ iverilog).
//! So it is not a synthetic GEMM: real frontier-model weights ride the same proven kernel.
//!
//! Gated on the checkpoint being present (it is gitignored, multi-GB) — SKIPS cleanly when absent, exactly
//! like the nvcc/GPU gates. `nvcc` compile always runs when present; the on-GPU numeric run needs a driver.

use std::path::PathBuf;

use logicaffeine_compile::codegen_cuda::matrix::{
    blockscaled_gemm_dequant, blockscaled_gemm_int8, emit_scaled_int8_gemm,
};
use logicaffeine_compile::codegen_cuda::emit_int8_gemm;
use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::codegen_sva::systolic::{eval_systolic, Systolic};

fn model_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models/qwen3-8b/Qwen_Qwen3-8B-Q8_0.gguf")
}

fn have_nvcc() -> bool {
    std::process::Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
fn have_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
}

#[test]
fn the_checkpoint_is_qwen3_8b_with_the_expected_architecture() {
    let path = model_path();
    if !path.exists() {
        eprintln!(
            "SKIP qwen: checkpoint not present ({}). Pull it: see the Phase-3 runbook.",
            path.display()
        );
        return;
    }
    let g = Gguf::open(&path).expect("open GGUF");
    let a = &g.arch;
    assert_eq!(a.arch, "qwen3", "architecture");
    assert_eq!(a.block_count, 36, "layers");
    assert_eq!(a.embedding_length, 4096, "d_model");
    assert_eq!(a.feed_forward_length, 12288, "ffn");
    assert_eq!(a.head_count, 32, "query heads");
    assert_eq!(a.head_count_kv, 8, "kv heads (GQA)");
    assert_eq!(a.head_dim, 128, "head dim");
    // GQA group size is a clean 4 (32 query heads share 8 KV heads).
    assert_eq!(a.head_count / a.head_count_kv, 4, "GQA group size");
    // Block-0 Q projection is Q8_0 and square d_model×d_model.
    assert_eq!(
        g.dims("blk.0.attn_q.weight"),
        Some([4096u64, 4096].as_slice()),
        "attn_q shape"
    );
}

/// The GEMM shape lifted from a real weight tile. k=64,n=64 → int32 accumulation is exact vs the i64
/// reference (64·127² ≈ 1.0e6 ≪ 2³¹). cols(=64) is a multiple of the Q8_0 block (32).
const M: usize = 8;
const K: usize = 64;
const N: usize = 64;

/// Deterministic int8 activations (M×K row-major), shared by the kernel and the oracle.
fn activations() -> Vec<i32> {
    (0..M * K)
        .map(|idx| ((idx as i32 * 5 + 1).rem_euclid(255)) - 128)
        .collect()
}

/// Pull the real Q8_0 int8 weight tile from block-0's Q projection (K×N), as the GEMM weights.
fn real_weight_tile() -> Option<Vec<i32>> {
    let path = model_path();
    if !path.exists() {
        return None;
    }
    let mut g = Gguf::open(&path).expect("open GGUF");
    let tile = g
        .q8_0_tile("blk.0.attn_q.weight", K, N)
        .expect("read Q8_0 tile");
    assert_eq!(tile.len(), K * N, "tile is K*N");
    assert!(
        tile.iter().all(|&w| (-128..=127).contains(&w)),
        "int8 range"
    );
    assert!(
        tile.iter().any(|&w| w != 0),
        "real weights are not all zero"
    );
    Some(tile)
}

#[test]
fn a_real_qwen_weight_tile_gemm_matches_the_certified_reference() {
    let Some(weights) = real_weight_tile() else {
        eprintln!("SKIP qwen GEMM: checkpoint not present");
        return;
    };
    let acts = activations();

    // The emitted DP4A kernel is well-formed and (when nvcc is present) compiles for sm_86.
    let art =
        emit_int8_gemm(M, K, N, &weights, &acts).expect("int8 WS GEMM is a certified capability");
    assert!(
        art.kernel_cu.contains("__dp4a("),
        "uses the DP4A intrinsic on real weights"
    );

    if have_nvcc() {
        let dir = std::env::temp_dir().join(format!("logos_qwen_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let src = dir.join("logos_cuda.cu");
        let bin = dir.join("logos_cuda");
        std::fs::write(&src, &art.kernel_cu).unwrap();
        let compile = std::process::Command::new("nvcc")
            .args(["-O2", "-arch=sm_86", "-o"])
            .arg(&bin)
            .arg(&src)
            .output()
            .expect("nvcc");
        assert!(
            compile.status.success(),
            "the real-weight DP4A GEMM must compile:\n{}",
            String::from_utf8_lossy(&compile.stderr)
        );

        if have_gpu() {
            let run = std::process::Command::new(&bin).output().expect("run");
            assert!(
                run.status.success(),
                "run: {}",
                String::from_utf8_lossy(&run.stderr)
            );
            let got: Vec<i64> = String::from_utf8_lossy(&run.stdout)
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.trim().parse().unwrap())
                .collect();
            let sys = Systolic::new(K, N, weights.iter().map(|&x| x as i64).collect());
            let want = eval_systolic(&sys, &acts.iter().map(|&x| x as i64).collect::<Vec<_>>(), M);
            assert_eq!(
                got, want,
                "real-Qwen-weight DP4A GEMM must equal the certified reference"
            );
        } else {
            eprintln!(
                "SKIP qwen GEMM run: nvcc-compiled OK, but no GPU driver (/dev/nvidia0) to run on"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    } else {
        eprintln!("SKIP qwen GEMM compile: nvcc not installed");
    }
}

/// The REAL model operation: block-0's Q projection as a **block-scaled Q8_0 GEMM** (int8 values + a f32
/// scale per 32-block). Proves — on the checkpoint's own scales — that our block-scaled kernel semantics
/// equal the unambiguous full-dequant GEMM, and that the emitted CUDA kernel compiles (numeric run on-GPU
/// gated on the driver).
#[test]
fn a_real_qwen_q8_0_block_scaled_gemm_matches_full_dequant() {
    let path = model_path();
    if !path.exists() {
        eprintln!("SKIP qwen block-scaled GEMM: checkpoint not present");
        return;
    }
    let mut g = Gguf::open(&path).expect("open GGUF");
    // Q projection tile: N output features × K input features (K a multiple of 32).
    let (wq, ws) = g
        .q8_0_tile_blocks("blk.0.attn_q.weight", N, K)
        .expect("read Q8_0 blocks");
    assert_eq!(wq.len(), N * K, "weight quants");
    assert_eq!(ws.len(), N * (K / 32), "one f32 scale per 32-block");
    assert!(
        ws.iter().all(|s| s.is_finite() && *s != 0.0),
        "real f16→f32 scales are finite, nonzero"
    );
    assert!(
        ws.iter().any(|&s| (s - ws[0]).abs() > 1e-12),
        "real per-block scales genuinely vary"
    );

    // Synthetic int8 activations with their own per-block scales (real inference quantizes activations too).
    let aq = activations();
    let nb = K / 32;
    let as_: Vec<f32> = (0..M * nb)
        .map(|i| 0.008 + (i % 5) as f32 * 0.0017)
        .collect();

    // CPU PROOF (always): block-scaled int8 == full dequant, on the REAL Qwen weight scales.
    let scaled = blockscaled_gemm_int8(M, K, N, &wq, &ws, &aq, &as_);
    let dequant = blockscaled_gemm_dequant(M, K, N, &wq, &ws, &aq, &as_);
    for (x, y) in scaled.iter().zip(&dequant) {
        assert!(
            (x - y).abs() <= 1e-6 * x.abs().max(1.0),
            "block-scaled {x} != full dequant {y}"
        );
    }

    // CUDA: the block-scaled DP4A kernel compiles (and, on a live GPU, matches the dequant reference).
    if have_nvcc() {
        let art = emit_scaled_int8_gemm(M, K, N, &wq, &ws, &aq, &as_).expect("emit scaled gemm");
        assert!(
            art.kernel_cu.contains("__dp4a("),
            "block-scaled kernel uses DP4A on real weights"
        );
        let dir = std::env::temp_dir().join(format!("logos_qwen_bs_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let src = dir.join("logos_cuda.cu");
        let bin = dir.join("logos_cuda");
        std::fs::write(&src, &art.kernel_cu).unwrap();
        let compile = std::process::Command::new("nvcc")
            .args(["-O2", "-arch=sm_86", "-o"])
            .arg(&bin)
            .arg(&src)
            .output()
            .expect("nvcc");
        assert!(
            compile.status.success(),
            "block-scaled real-weight GEMM must compile:\n{}",
            String::from_utf8_lossy(&compile.stderr)
        );
        if have_gpu() {
            let run = std::process::Command::new(&bin).output().expect("run");
            assert!(
                run.status.success(),
                "run: {}",
                String::from_utf8_lossy(&run.stderr)
            );
            let got: Vec<f64> = String::from_utf8_lossy(&run.stdout)
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.trim().parse().unwrap())
                .collect();
            assert_eq!(got.len(), M * N);
            for (x, y) in got.iter().zip(&dequant) {
                assert!(
                    (x - y).abs() <= 1e-3 * y.abs().max(1.0),
                    "GPU {x} vs reference {y}"
                );
            }
        } else {
            eprintln!("SKIP qwen block-scaled run: compiled OK, no GPU driver to run on");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
