//! cuda_k32swz_peak_gpu_lock — push the XOR-swizzled k32 kernel to its peak vs cuBLAS-TN via a broad config sweep.
#![cfg(feature = "codegen")]

mod common;

use logicaffeine_compile::codegen_cuda::gemm_tile::emit_mma_tn_k32swz_bench;
const CUBLAS_TN: f64 = 72511.0;
fn have() -> bool {
    common::require_tool("nvcc") && common::require_gpu()
}
fn gops(cu: &str, l: &str) -> Option<f64> {
    let d = std::env::temp_dir().join(format!("logos_pk_{}_{}", std::process::id(), l));
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
fn find_the_swizzled_k32_peak() {
    if !have() {
        return;
    }
    let (m, k, n) = (2048usize, 2048, 2048);
    let cfgs = [
        (128usize, 128usize, 2usize, 4usize, 4usize),
        (128, 128, 2, 4, 5),
        (128, 128, 4, 2, 4),
        (256, 128, 4, 2, 3),
        (256, 128, 4, 2, 4),
        (256, 128, 2, 4, 3),
        (128, 256, 2, 4, 3),
        (256, 64, 2, 2, 4),
        (128, 128, 2, 2, 4),
        (64, 128, 2, 4, 4),
        (256, 256, 4, 4, 2),
        (128, 64, 2, 2, 6),
    ];
    let mut best = (0usize, 0usize, 0usize, 0usize, 0usize, 0.0f64);
    for (bm, bn, wm, wn, st) in cfgs {
        if let Some(g) = gops(
            &emit_mma_tn_k32swz_bench(bm, bn, wm, wn, st, m, k, n),
            &format!("{bm}x{bn}w{wm}{wn}s{st}"),
        ) {
            eprintln!(
                "  {bm}x{bn} w{wm}x{wn} s{st} -> {g:.1} ({:.1}% cuBLAS-TN)",
                100.0 * g / CUBLAS_TN
            );
            if g > best.5 {
                best = (bm, bn, wm, wn, st, g);
            }
        }
    }
    eprintln!(
        "PEAK swizzled-k32 {}x{} w{}x{} s{} = {:.1} GOPS = {:.1}% of cuBLAS-TN {CUBLAS_TN:.0}",
        best.0,
        best.1,
        best.2,
        best.3,
        best.4,
        best.5,
        100.0 * best.5 / CUBLAS_TN
    );
    assert!(
        best.5 > 50000.0,
        "peak swizzled-k32 ({:.1}) must exceed 50000",
        best.5
    );
}
