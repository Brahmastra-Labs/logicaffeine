//! cuda_ldm_sweep_gpu_lock — sweep the ldmatrix TN kernel's block/warp/stage shapes on-device to find the
//! best EARNED climb toward cuBLAS-TN (72,511). All-ones guarded.
#![cfg(feature = "codegen")]

mod common;

use logicaffeine_compile::codegen_cuda::gemm_tile::emit_mma_tn_ldm_cfg_bench;
const CUBLAS_TN: f64 = 72511.0;
const BASE_LDM: f64 = 45533.0;
fn have() -> bool {
    common::require_tool("nvcc") && common::require_gpu()
}
fn run(cu: &str, l: &str) -> Option<f64> {
    let d = std::env::temp_dir().join(format!("logos_lsw_{}_{}", std::process::id(), l));
    let _ = std::fs::create_dir_all(&d);
    let s = d.join("k.cu");
    let b = d.join("k");
    std::fs::write(&s, cu).ok()?;
    let c = std::process::Command::new("nvcc")
        .args(["-O3", "-arch=sm_86", "-o"])
        .arg(&b)
        .arg(&s)
        .output()
        .ok()?;
    if !c.status.success() {
        eprintln!(
            "nvcc[{l}]:{}",
            String::from_utf8_lossy(&c.stderr)
                .lines()
                .take(3)
                .collect::<Vec<_>>()
                .join(" ")
        );
        return None;
    }
    let r = std::process::Command::new(&b).output().ok()?;
    let o = String::from_utf8_lossy(&r.stdout);
    let g = o.lines().find_map(|x| {
        x.strip_prefix("GOPS ")
            .and_then(|v| v.trim().parse::<f64>().ok())
    });
    let _ = std::fs::remove_dir_all(&d);
    g
}
#[test]
fn sweep_ldmatrix_toward_cublas_tn() {
    if !have() {
        return;
    }
    // (bm,bn,bk,wm,wn,stages) — WN=bn/wn mult 8, WM=bm/wm mult 16, threads=wm*wn*32<=1024
    let cfgs = [
        (128usize, 64usize, 32usize, 2usize, 2usize, 2usize),
        (128, 64, 32, 2, 2, 3),
        (128, 128, 32, 2, 4, 3),
        (128, 128, 32, 4, 2, 3),
        (256, 64, 32, 4, 2, 3),
        (128, 128, 64, 2, 4, 3),
        (128, 256, 32, 2, 4, 2),
        (64, 128, 32, 2, 4, 3),
    ];
    let (m, k, n) = (2048usize, 2048, 2048);
    let mut best = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0.0f64);
    for (bm, bn, bk, wm, wn, st) in cfgs {
        if let Some(g) = run(
            &emit_mma_tn_ldm_cfg_bench(bm, bn, bk, wm, wn, st, m, k, n),
            &format!("{bm}x{bn}x{bk}w{wm}x{wn}s{st}"),
        ) {
            eprintln!(
                "  {bm}x{bn} bk{bk} warps {wm}x{wn} stages{st} -> {g:.1} GOPS ({:.1}% cuBLAS-TN)",
                100.0 * g / CUBLAS_TN
            );
            if g > best.6 {
                best = (bm, bn, bk, wm, wn, st, g);
            }
        }
    }
    eprintln!("BEST ldmatrix {}x{} bk{} warps {}x{} stages{} = {:.1} GOPS = {:.1}% of cuBLAS-TN {CUBLAS_TN:.0}",
        best.0,best.1,best.2,best.3,best.4,best.5,best.6, 100.0*best.6/CUBLAS_TN);
    assert!(
        best.6 >= BASE_LDM,
        "best swept ldmatrix ({:.1}) must be >= the base ldmatrix ({BASE_LDM})",
        best.6
    );
}
