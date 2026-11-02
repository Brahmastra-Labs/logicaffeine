//! cuda_mma_tn_colb_gpu_lock — the compiler emitting cuBLAS's OWN kernel shape: TN mma.sync.s8 with B
//! col-major (both operands K-contiguous, contiguous fragment loads) + cp.async. Correct vs the col-major
//! oracle; measured against cuBLAS-TN (72,511) — the match target.
#![cfg(feature = "codegen")]

mod common;

use logicaffeine_compile::codegen_cuda::gemm_tile::{
    emit_mma_tn_colb_bench, emit_mma_tn_colb_program,
};
const CUBLAS_TN: f64 = 72511.0;
fn oracle(m: usize, k: usize, n: usize) -> Vec<i64> {
    let a: Vec<i64> = (0..m * k)
        .map(|i| ((i as i64 * 7 + 3) % 251) - 125)
        .collect();
    // B col-major: b_kn value at Bcol[n*K+k] = ((k*N+n)*5+1)%251-125
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
fn run(cu: &str, l: &str) -> Option<String> {
    let d = std::env::temp_dir().join(format!("logos_colb_{}_{}", std::process::id(), l));
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
        eprintln!("nvcc [{l}]:\n{}", String::from_utf8_lossy(&c.stderr));
        return None;
    }
    let r = std::process::Command::new(&b).output().ok()?;
    let o = String::from_utf8_lossy(&r.stdout).into_owned();
    let _ = std::fs::remove_dir_all(&d);
    Some(o)
}
#[test]
fn compiler_emitted_tn_colb_matches_oracle_and_chases_cublas() {
    if !have() {
        return;
    }
    for &(m, k, n) in &[
        (128usize, 32usize, 64usize),
        (128, 64, 128),
        (256, 256, 256),
        (384, 128, 512),
    ] {
        let out = run(&emit_mma_tn_colb_program(m, k, n), &format!("{m}x{k}x{n}")).expect("runs");
        let got: Vec<i64> = out
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.trim().parse().expect("int"))
            .collect();
        assert_eq!(
            got,
            oracle(m, k, n),
            "({m},{k},{n}) col-major-B TN must equal oracle"
        );
    }
    let out = run(&emit_mma_tn_colb_bench(2048, 2048, 2048), "bench").expect("bench");
    let g = out
        .lines()
        .find_map(|l| {
            l.strip_prefix("GOPS ")
                .and_then(|v| v.trim().parse::<f64>().ok())
        })
        .expect("GOPS");
    eprintln!(
        "COMPILER TN (col-major B) = {g:.1} GOPS  |  {:.1}% of cuBLAS-TN {CUBLAS_TN:.0}",
        100.0 * g / CUBLAS_TN
    );
    assert!(
        g > 20000.0,
        "col-major-B TN ({g:.1}) must be a serious tensor-core kernel (>20000)"
    );
}
