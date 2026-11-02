//! cuda_wmma_gpu_lock — the WMMA int8 tensor-core GEMM on the real RTX 3060: numerically identical to the
//! scalar oracle, AND measurably faster than the tiled DP4A CUDA-core kernel (the tensor-core lever firing).
//! This is the step that reaches toward the cuBLAS ceiling.
//!
//! ⚠️ If a lock goes RED, fix the kernel (`codegen_cuda::gemm_tile`), never the assertion.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_wmma_gpu_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::{emit_wmma_i8_bench, emit_wmma_i8_program};

const TILED_DP4A_GOPS: f64 = 4889.0; // best tiled DP4A (CUDA cores), measured in cuda_gemm_tiled_bench_lock
const CUBLAS_INT8_GOPS: f64 = 88490.0;

/// Standard row-major int8 GEMM oracle, same deterministic fill as the emitted program.
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

fn nvcc_run(cu: &str, label: &str) -> String {
    let dir = std::env::temp_dir().join(format!("logos_wmma_{}_{}", std::process::id(), label));
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("k.cu");
    let bin = dir.join("k");
    std::fs::write(&src, cu).expect("write");
    let c = std::process::Command::new("nvcc")
        .args(["-O3", "-arch=sm_86", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .expect("nvcc");
    assert!(
        c.status.success(),
        "`{label}`: compile\n{}",
        String::from_utf8_lossy(&c.stderr)
    );
    let r = std::process::Command::new(&bin).output().expect("run");
    assert!(
        r.status.success(),
        "`{label}`: run\n{}",
        String::from_utf8_lossy(&r.stderr)
    );
    let out = String::from_utf8_lossy(&r.stdout).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[test]
fn the_wmma_gemm_equals_the_scalar_oracle_on_the_gpu() {
    if !have() {
        eprintln!("SKIP: needs nvcc + a GPU driver (/dev/nvidia0)");
        return;
    }
    for &(m, k, n) in &[
        (16usize, 16usize, 16usize),
        (128, 64, 256),
        (256, 256, 128),
        (512, 128, 512),
    ] {
        let out = nvcc_run(&emit_wmma_i8_program(m, k, n), &format!("wmma_{m}x{k}x{n}"));
        let got: Vec<i64> = out
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.trim().parse().expect("int"))
            .collect();
        let want = oracle(m, k, n);
        assert_eq!(got.len(), want.len(), "({m},{k},{n}): size");
        assert_eq!(
            got, want,
            "({m},{k},{n}): WMMA int8 GPU output must equal the scalar oracle"
        );
    }
}

#[test]
fn the_wmma_tensor_core_lever_beats_the_tiled_cuda_core_kernel() {
    if !have() {
        eprintln!("SKIP: needs nvcc + a GPU driver");
        return;
    }
    let (m, k, n) = (2048usize, 2048usize, 2048usize);
    let out = nvcc_run(&emit_wmma_i8_bench(m, k, n), "wmma_bench");
    let gops = out
        .lines()
        .find_map(|l| {
            l.strip_prefix("GOPS ")
                .and_then(|v| v.trim().parse::<f64>().ok())
        })
        .expect("GOPS line");
    eprintln!(
        "WMMA int8 = {gops:.1} GOPS  |  {:.1}× over tiled DP4A {TILED_DP4A_GOPS:.0}  |  {:.1}% of cuBLAS {CUBLAS_INT8_GOPS:.0}",
        gops / TILED_DP4A_GOPS,
        100.0 * gops / CUBLAS_INT8_GOPS
    );
    // The tensor cores must be doing real work: WMMA must beat the best CUDA-core (tiled DP4A) kernel.
    assert!(
        gops > TILED_DP4A_GOPS,
        "WMMA ({gops:.1} GOPS) must beat the tiled DP4A CUDA-core kernel ({TILED_DP4A_GOPS} GOPS) — else the tensor cores aren't engaged"
    );
}
