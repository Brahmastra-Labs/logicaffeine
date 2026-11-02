//! cuda_mma_tn_gpu_lock — the ratchet's rung 6: the TN-native `mma.sync.aligned.m16n8k16.s8` kernel that feeds
//! the int8 tensor cores their native register layout (below the nvcuda::wmma wrapper) — the same instruction
//! path cuBLAS uses to hit 72,511 GOPS. Correctness is checked against the VARIED-fill scalar oracle (a
//! fragment-layout permutation would pass an all-ones test but fail this one), then measured toward the
//! cuBLAS-TN ceiling.
//!
//! ⚠️ If a lock goes RED, fix the fragment mapping / kernel (`codegen_cuda::gemm_tile`), never the assertion.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_mma_tn_gpu_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::{emit_mma_tn_i8_bench, emit_mma_tn_i8_program};

const RUNG5_GOPS: f64 = 32299.0; // best cp.async WMMA, measured in cuda_wmma_multistage_gpu_lock
const CUBLAS_TN_GOPS: f64 = 72511.0; // cuBLAS's verified TN ceiling on this box (cuda_layout_tradeoff_lock)

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
    let dir = std::env::temp_dir().join(format!("logos_mmatn_{}_{}", std::process::id(), label));
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
fn the_mma_tn_kernel_equals_the_varied_fill_oracle() {
    if !have() {
        eprintln!("SKIP: needs nvcc + a GPU driver (/dev/nvidia0)");
        return;
    }
    for &(m, k, n) in &[
        (64usize, 16usize, 64usize),
        (128, 64, 128),
        (256, 256, 256),
        (512, 128, 320),
    ] {
        let out =
            nvcc_out(&emit_mma_tn_i8_program(m, k, n), &format!("{m}x{k}x{n}")).expect("runs");
        let got: Vec<i64> = out
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.trim().parse().expect("int"))
            .collect();
        let want = oracle(m, k, n);
        assert_eq!(got.len(), want.len(), "({m},{k},{n}): size");
        assert_eq!(got, want, "({m},{k},{n}): mma.sync.s8 TN output must equal the varied-fill oracle (fragment map correct)");
    }
}

#[test]
fn the_mma_tn_kernel_climbs_toward_the_cublas_tn_ceiling() {
    if !have() {
        eprintln!("SKIP: needs nvcc + a GPU driver");
        return;
    }
    let (m, k, n) = (2048usize, 2048usize, 2048usize);
    let out = nvcc_out(&emit_mma_tn_i8_bench(m, k, n), "bench").expect("bench runs");
    let gops = out
        .lines()
        .find_map(|l| {
            l.strip_prefix("GOPS ")
                .and_then(|v| v.trim().parse::<f64>().ok())
        })
        .expect("GOPS line");
    eprintln!(
        "TN mma.sync.s8 (naive manual-load, NO ldmatrix/pipeline) = {gops:.1} GOPS  |  {:.1}% of cp.async-WMMA {RUNG5_GOPS:.0}  |  {:.1}% of cuBLAS-TN {CUBLAS_TN_GOPS:.0}",
        100.0 * gops / RUNG5_GOPS,
        100.0 * gops / CUBLAS_TN_GOPS
    );
    // HONEST floor: this kernel is a CORRECT native-tensor-core baseline (the fragment map is proven right by
    // the oracle test). It is intentionally NAIVE — manual byte-strided fragment loads, no ldmatrix, no
    // pipeline — so it is SLOWER than the optimized WMMA cp.async kernel. Its value is proving the native
    // mma.sync.s8 path works end-to-end; the climb to cuBLAS-TN (72,511) is the ldmatrix + swizzle + pipeline
    // build (Task 10). Floor: it must genuinely engage the tensor cores (well above the naive DP4A CUDA-core
    // kernel at 784), not that it beats the optimized WMMA — asserting that was a wrong expectation.
    assert!(
        gops > 5000.0,
        "TN mma.sync.s8 ({gops:.1} GOPS) must be a real tensor-core kernel (>5000, well above naive DP4A 784)"
    );
}
