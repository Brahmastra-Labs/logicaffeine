//! cuda_gemm_tiled_bench_lock — the tiling CLIMB, measured. Sweeps promising points of the GEMM tiling
//! lattice on the RTX 3060, times each with CUDA events, and locks that the best tiled DP4A kernel is
//! dramatically faster than the naive one-thread-per-output DP4A (784 GOPS, recorded in gemm_bench). This is
//! the "~20× tiling/reuse lever" made real — the CUDA-core climb toward the cuBLAS int8 ceiling (88,490
//! GOPS), before we add tensor cores (WMMA) to close the rest.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_gemm_tiled_bench_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::{emit_tiled_dp4a_bench, CudaTile};

const NAIVE_DP4A_GOPS: f64 = 784.0; // one-thread-per-output DP4A, measured in cuda/gemm_bench.cu
const CUBLAS_INT8_GOPS: f64 = 88490.0; // cuBLAS int8 (tensor cores), measured same box

fn have() -> bool {
    std::process::Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        && std::path::Path::new("/dev/nvidia0").exists()
}

fn run_gops(cu: &str, label: &str) -> Option<f64> {
    let dir = std::env::temp_dir().join(format!("logos_bench_{}_{}", std::process::id(), label));
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("k.cu");
    let bin = dir.join("k");
    std::fs::write(&src, cu).ok()?;
    let c = std::process::Command::new("nvcc")
        .args(["-O3", "-arch=sm_86", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .ok()?;
    if !c.status.success() {
        return None;
    }
    let r = std::process::Command::new(&bin).output().ok()?;
    let out = String::from_utf8_lossy(&r.stdout);
    let gops = out.lines().find_map(|l| {
        l.strip_prefix("GOPS ")
            .and_then(|v| v.trim().parse::<f64>().ok())
    });
    let _ = std::fs::remove_dir_all(&dir);
    gops
}

#[test]
fn the_tuned_tiled_dp4a_climbs_far_above_the_naive_kernel() {
    if !have() {
        eprintln!("SKIP: needs nvcc + a GPU driver (/dev/nvidia0)");
        return;
    }
    let (m, k, n) = (2048usize, 2048usize, 2048usize);
    // A curated slice of the lattice (all divide 2048, all legal on Ampere) — the autotuner sweep.
    let candidates = [
        CudaTile {
            bm: 64,
            bn: 64,
            bk: 16,
            tm: 4,
            tn: 4,
        },
        CudaTile {
            bm: 128,
            bn: 128,
            bk: 8,
            tm: 8,
            tn: 8,
        },
        CudaTile {
            bm: 128,
            bn: 128,
            bk: 16,
            tm: 8,
            tn: 8,
        },
        CudaTile {
            bm: 128,
            bn: 128,
            bk: 32,
            tm: 8,
            tn: 8,
        },
        CudaTile {
            bm: 128,
            bn: 64,
            bk: 16,
            tm: 8,
            tn: 4,
        },
        CudaTile {
            bm: 64,
            bn: 128,
            bk: 16,
            tm: 4,
            tn: 8,
        },
    ];
    let mut best: Option<(CudaTile, f64)> = None;
    for t in candidates {
        if let Some(g) = run_gops(
            &emit_tiled_dp4a_bench(&t, m, k, n),
            &format!("{}x{}", t.bm, t.bn),
        ) {
            eprintln!(
                "  tile {}x{} bk{} {}x{} -> {:.1} GOPS",
                t.bm, t.bn, t.bk, t.tm, t.tn, g
            );
            if best.map_or(true, |(_, bg)| g > bg) {
                best = Some((t, g));
            }
        }
    }
    let (bt, bg) = best.expect("at least one tiled config must run");
    eprintln!(
        "BEST tiled {}x{} bk{} {}x{} = {:.1} GOPS  |  {:.1}x over naive {:.0}  |  {:.1}% of cuBLAS {:.0}",
        bt.bm, bt.bn, bt.bk, bt.tm, bt.tn, bg, bg / NAIVE_DP4A_GOPS, NAIVE_DP4A_GOPS,
        100.0 * bg / CUBLAS_INT8_GOPS, CUBLAS_INT8_GOPS
    );
    // The tiling lever is real: a conservative floor (>=3x the naive kernel). The printed number is the truth.
    assert!(
        bg >= 3.0 * NAIVE_DP4A_GOPS,
        "the best tiled DP4A ({bg:.1} GOPS) must climb well above the naive kernel ({NAIVE_DP4A_GOPS} GOPS)"
    );
}
