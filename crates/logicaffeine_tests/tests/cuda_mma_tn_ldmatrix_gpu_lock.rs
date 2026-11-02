//! cuda_mma_tn_ldmatrix_gpu_lock — the EARNED cuBLAS-class kernel: ldmatrix cooperative fragment loads feeding
//! mma.sync.s8, col-major TN layout, cp.async. Correct vs the col-major oracle; measured against cuBLAS-TN
//! (72,511). This is our OWN code chasing the cuBLAS ceiling — no vendor dispatch.
#![cfg(feature = "codegen")]

mod common;

use logicaffeine_compile::codegen_cuda::gemm_tile::{
    emit_mma_tn_ldmatrix_bench, emit_mma_tn_ldmatrix_program,
};
const CUBLAS_TN: f64 = 72511.0;
const PREV_BEST_TN: f64 = 25268.0; // col-major mma.sync WITHOUT ldmatrix
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
fn run(cu: &str, l: &str) -> Option<String> {
    let d = std::env::temp_dir().join(format!("logos_ldm_{}_{}", std::process::id(), l));
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
fn ldmatrix_mma_tn_is_correct_and_climbs() {
    if !have() {
        return;
    }
    for &(m, k, n) in &[
        (128usize, 32usize, 64usize),
        (128, 64, 128),
        (256, 256, 256),
        (384, 128, 512),
    ] {
        let out = run(
            &emit_mma_tn_ldmatrix_program(m, k, n),
            &format!("{m}x{k}x{n}"),
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
            "({m},{k},{n}) ldmatrix mma.sync must equal the col-major oracle"
        );
    }
    let out = run(&emit_mma_tn_ldmatrix_bench(2048, 2048, 2048), "bench").expect("bench");
    let g = out
        .lines()
        .find_map(|l| {
            l.strip_prefix("GOPS ")
                .and_then(|v| v.trim().parse::<f64>().ok())
        })
        .expect("GOPS");
    eprintln!("LDMATRIX mma.sync.s8 TN = {g:.1} GOPS  |  {:.2}x over manual-load {PREV_BEST_TN:.0}  |  {:.1}% of cuBLAS-TN {CUBLAS_TN:.0}", g/PREV_BEST_TN, 100.0*g/CUBLAS_TN);
    assert!(
        g > PREV_BEST_TN,
        "ldmatrix ({g:.1}) must beat the manual-load col-major kernel ({PREV_BEST_TN})"
    );
}
