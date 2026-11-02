//! cuda_mma_k32swz_gpu_lock — XOR-swizzled (bank-conflict-free, ZERO extra memory) k32 ldmatrix mma.sync.s8,
//! col-major TN, cp.async. The real CUTLASS-style swizzle. Correct vs the col-major oracle; swept vs cuBLAS-TN.
#![cfg(feature = "codegen")]

mod common;

use logicaffeine_compile::codegen_cuda::gemm_tile::{
    emit_mma_tn_k32swz_bench, emit_mma_tn_k32swz_program,
};
const CUBLAS_TN: f64 = 72511.0;
const UNSWIZZLED_K32: f64 = 49792.0;
fn oracle(m: usize, k: usize, n: usize) -> Vec<i64> {
    let a: Vec<i64> = (0..m * k)
        .map(|i| ((i as i64 * 7 + 3) % 251) - 125)
        .collect();
    let mut c = vec![0i64; m * n];
    for i in 0..m {
        for j in 0..n {
            let mut s = 0i64;
            for kk in 0..k {
                let bkn = (kk as i64 * n as i64 + j as i64);
                let bv = ((bkn * 5 + 1) % 251) - 125;
                s += a[i * k + kk] * bv;
            }
            c[i * n + j] = s;
        }
    }
    c
}
fn have() -> bool {
    common::require_tool("nvcc") && common::require_gpu()
}
fn runp(cu: &str, l: &str) -> Option<String> {
    let d = std::env::temp_dir().join(format!("logos_swk_{}_{}", std::process::id(), l));
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
                .take(4)
                .collect::<Vec<_>>()
                .join(" ")
        );
        return None;
    }
    let r = std::process::Command::new(&b).output().ok()?;
    let o = String::from_utf8_lossy(&r.stdout).into_owned();
    let _ = std::fs::remove_dir_all(&d);
    Some(o)
}
fn gops(cu: &str, l: &str) -> Option<f64> {
    runp(cu, l)?.lines().find_map(|x| {
        x.strip_prefix("GOPS ")
            .and_then(|v| v.trim().parse::<f64>().ok())
    })
}
#[test]
fn xor_swizzled_k32_is_correct_and_climbs() {
    if !have() {
        return;
    }
    for &(m, k, n) in &[
        (128usize, 32usize, 64usize),
        (128, 64, 128),
        (256, 256, 256),
        (384, 128, 512),
    ] {
        let out = runp(
            &emit_mma_tn_k32swz_program(128, 64, 2, 2, 2, m, k, n),
            &format!("c{m}x{k}x{n}"),
        )
        .expect("runs");
        let got: Vec<i64> = out
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.trim().parse().expect("int"))
            .collect();
        assert_eq!(
            got,
            oracle(m, k, n),
            "({m},{k},{n}) XOR-swizzled k32 must equal oracle"
        );
    }
    let (m, k, n) = (2048usize, 2048, 2048);
    let cfgs = [
        (128usize, 64usize, 2usize, 2usize, 2usize),
        (128, 64, 2, 2, 3),
        (128, 64, 2, 2, 4),
        (128, 128, 2, 4, 3),
        (256, 64, 4, 2, 3),
        (128, 128, 2, 4, 4),
    ];
    let mut best = (0usize, 0usize, 0.0f64);
    for (bm, bn, wm, wn, st) in cfgs {
        if let Some(g) = gops(
            &emit_mma_tn_k32swz_bench(bm, bn, wm, wn, st, m, k, n),
            &format!("{bm}x{bn}w{wm}{wn}s{st}"),
        ) {
            eprintln!(
                "  swz-k32 {bm}x{bn} w{wm}x{wn} s{st} -> {g:.1} ({:.1}% cuBLAS-TN)",
                100.0 * g / CUBLAS_TN
            );
            if g > best.2 {
                best = (bm, bn, g);
            }
        }
    }
    eprintln!("BEST XOR-swizzled k32 = {:.1} GOPS = {:.1}% of cuBLAS-TN {CUBLAS_TN:.0}  (unswizzled k32 was {:.1}%)", best.2, 100.0*best.2/CUBLAS_TN, 100.0*UNSWIZZLED_K32/CUBLAS_TN);
    assert!(
        best.2 > UNSWIZZLED_K32,
        "XOR-swizzled k32 ({:.1}) must beat the unswizzled k32 ({UNSWIZZLED_K32})",
        best.2
    );
}
