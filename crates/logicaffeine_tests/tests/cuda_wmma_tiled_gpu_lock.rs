//! cuda_wmma_tiled_gpu_lock — the ratchet's rung 3: tensor cores FED FROM SHARED MEMORY (reuse fused with
//! WMMA, warp-tiled). Byte-exact vs the scalar oracle on the RTX 3060, and — sweeping the block/warp shape —
//! its GOPS floor must climb ABOVE the basic (starved) WMMA kernel. This is the supercompiler emitting the
//! cuBLAS kernel STRUCTURE (minus cp.async, rung 4).
//!
//! ⚠️ If a lock goes RED, fix/extend the kernel + config sweep (`codegen_cuda::gemm_tile`), never the assertion.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_wmma_tiled_gpu_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::{
    emit_wmma_tiled_i8_bench_for, emit_wmma_tiled_i8_program, GpuLimits, WmmaTile,
};

const BASIC_WMMA_GOPS: f64 = 8680.0; // starved one-warp-per-tile WMMA, measured in cuda_wmma_gpu_lock
const CUBLAS_INT8_GOPS: f64 = 88490.0;

fn oracle(m: usize, k: usize, n: usize) -> Vec<i64> {
    let a: Vec<i64> = (0..m * k)
        .map(|i| ((i as i64 * 7 + 3) % 251) - 125)
        .collect();
    let b: Vec<i64> = (0..k * n)
        .map(|i| ((i as i64 * 5 + 1) % 251) - 125)
        .collect();
    let mut c = vec![0i64; m * n];
    for i in 0..m {
        for j in 0..n {
            let mut acc = 0i64;
            for kk in 0..k {
                acc += a[i * k + kk] * b[kk * n + j];
            }
            c[i * n + j] = acc;
        }
    }
    c
}

fn have() -> bool {
    std::process::Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        && std::path::Path::new("/dev/nvidia0").exists()
}

fn nvcc_out(cu: &str, label: &str) -> Option<String> {
    let dir = std::env::temp_dir().join(format!("logos_wmt_{}_{}", std::process::id(), label));
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
        eprintln!(
            "nvcc failed [{label}]:\n{}",
            String::from_utf8_lossy(&c.stderr)
        );
        return None;
    }
    let r = std::process::Command::new(&bin).output().ok()?;
    let out = String::from_utf8_lossy(&r.stdout).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    Some(out)
}

#[test]
fn the_pipelined_wmma_equals_the_scalar_oracle_on_the_gpu() {
    if !have() {
        eprintln!("SKIP: needs nvcc + a GPU driver (/dev/nvidia0)");
        return;
    }
    // Default config is 128×128 block, BK=32 → shapes must be M%128==0, N%128==0, K%32==0.
    for &(m, k, n) in &[
        (128usize, 32usize, 128usize),
        (128, 64, 256),
        (256, 256, 256),
        (384, 128, 512),
    ] {
        let out = nvcc_out(
            &emit_wmma_tiled_i8_program(m, k, n),
            &format!("t_{m}x{k}x{n}"),
        )
        .expect("program runs");
        let got: Vec<i64> = out
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.trim().parse().expect("int"))
            .collect();
        let want = oracle(m, k, n);
        assert_eq!(got.len(), want.len(), "({m},{k},{n}): size");
        assert_eq!(
            got, want,
            "({m},{k},{n}): pipelined WMMA GPU output must equal the scalar oracle"
        );
    }
}

#[test]
fn the_pipelined_wmma_climbs_above_the_starved_basic_wmma() {
    if !have() {
        eprintln!("SKIP: needs nvcc + a GPU driver");
        return;
    }
    let (m, k, n) = (2048usize, 2048usize, 2048usize);
    let gpu = GpuLimits::ampere();
    // Sweep the block/warp shape — the supercompiler tuning the cuBLAS-shaped kernel.
    let configs = [
        WmmaTile {
            bm: 128,
            bn: 128,
            bk: 32,
            warps_m: 2,
            warps_n: 4,
        },
        WmmaTile {
            bm: 128,
            bn: 128,
            bk: 32,
            warps_m: 4,
            warps_n: 2,
        },
        WmmaTile {
            bm: 128,
            bn: 128,
            bk: 16,
            warps_m: 2,
            warps_n: 4,
        },
        WmmaTile {
            bm: 128,
            bn: 256,
            bk: 32,
            warps_m: 2,
            warps_n: 4,
        },
        WmmaTile {
            bm: 256,
            bn: 128,
            bk: 32,
            warps_m: 4,
            warps_n: 2,
        },
        WmmaTile {
            bm: 128,
            bn: 128,
            bk: 64,
            warps_m: 2,
            warps_n: 4,
        },
        WmmaTile {
            bm: 256,
            bn: 128,
            bk: 32,
            warps_m: 2,
            warps_n: 4,
        },
    ];
    let mut best: Option<(WmmaTile, f64)> = None;
    for t in configs {
        if !t.is_legal(&gpu) {
            eprintln!("  (skip illegal {t:?})");
            continue;
        }
        if let Some(out) = nvcc_out(
            &emit_wmma_tiled_i8_bench_for(&t, m, k, n),
            &format!("{}x{}w{}x{}bk{}", t.bm, t.bn, t.warps_m, t.warps_n, t.bk),
        ) {
            if let Some(g) = out.lines().find_map(|l| {
                l.strip_prefix("GOPS ")
                    .and_then(|v| v.trim().parse::<f64>().ok())
            }) {
                eprintln!(
                    "  {}x{} bk{} warps {}x{} ({} frags/warp) -> {:.1} GOPS",
                    t.bm,
                    t.bn,
                    t.bk,
                    t.warps_m,
                    t.warps_n,
                    (t.wm() / 16) * (t.wn() / 16),
                    g
                );
                if best.map_or(true, |(_, bg)| g > bg) {
                    best = Some((t, g));
                }
            }
        }
    }
    let (bt, bg) = best.expect("at least one config must run");
    eprintln!(
        "BEST pipelined WMMA {}x{} bk{} warps {}x{} = {:.1} GOPS  |  {:.2}× over basic WMMA {BASIC_WMMA_GOPS:.0}  |  {:.1}% of cuBLAS {CUBLAS_INT8_GOPS:.0}",
        bt.bm, bt.bn, bt.bk, bt.warps_m, bt.warps_n, bg, bg / BASIC_WMMA_GOPS, 100.0 * bg / CUBLAS_INT8_GOPS
    );
    // The ratchet: rung 3 must climb above rung 2 (basic WMMA) — fused reuse must feed the tensor cores.
    assert!(
        bg > BASIC_WMMA_GOPS,
        "best pipelined WMMA ({bg:.1} GOPS) must beat the starved basic WMMA ({BASIC_WMMA_GOPS} GOPS)"
    );
}
