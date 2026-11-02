//! cuda_gemm_tiled_gpu_lock — the tiled DP4A GEMM is NUMERICALLY CORRECT on the RTX 3060. The compiler's
//! shared-memory + register-blocked tiled kernel, run on real silicon, produces output byte-for-byte equal
//! to a scalar row-major int8 GEMM oracle across several tile-aligned shapes. Correctness is the gate before
//! we tune the lattice for speed (a fast wrong kernel is worthless).
//!
//! ⚠️ If this goes RED, fix the kernel (`codegen_cuda::gemm_tile`), never the assertion.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_gemm_tiled_gpu_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::{emit_tiled_dp4a_program, CudaTile};

/// Standard row-major int8 GEMM: C(M×N) = A(M×K)·B(K×N), same deterministic fill as the emitted program.
/// int32-safe for these shapes (|a·b| ≤ 125·125·K well under 2^31).
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

fn nvcc_run(cu: &str, label: &str) -> Vec<i64> {
    let dir = std::env::temp_dir().join(format!("logos_tiled_{}_{}", std::process::id(), label));
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("k.cu");
    let bin = dir.join("k");
    std::fs::write(&src, cu).expect("write .cu");
    let c = std::process::Command::new("nvcc")
        .args(["-O2", "-arch=sm_86", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .expect("nvcc");
    assert!(
        c.status.success(),
        "`{label}`: must compile.\n{}",
        String::from_utf8_lossy(&c.stderr)
    );
    let r = std::process::Command::new(&bin).output().expect("run");
    assert!(
        r.status.success(),
        "`{label}`: must run.\n{}",
        String::from_utf8_lossy(&r.stderr)
    );
    let vals = String::from_utf8_lossy(&r.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().parse().expect("int"))
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    vals
}

#[test]
fn the_tiled_dp4a_gemm_equals_the_scalar_oracle_on_the_gpu() {
    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP: needs nvcc + a GPU driver (/dev/nvidia0)");
        return;
    }
    // tile-aligned shapes for tile (bm=64, bn=64, bk=16, tm=4, tn=4): M%64=0, N%64=0, K%16=0.
    let tile = CudaTile {
        bm: 64,
        bn: 64,
        bk: 16,
        tm: 4,
        tn: 4,
    };
    for &(m, k, n) in &[
        (64usize, 64usize, 64usize),
        (128, 32, 128),
        (128, 64, 64),
        (64, 128, 128),
    ] {
        let cu = emit_tiled_dp4a_program(&tile, m, k, n);
        let got = nvcc_run(&cu, &format!("tiled_{m}x{k}x{n}"));
        let want = oracle(m, k, n);
        assert_eq!(got.len(), want.len(), "({m},{k},{n}): output size");
        assert_eq!(
            got, want,
            "({m},{k},{n}): tiled DP4A GPU output must equal the scalar int8 GEMM oracle"
        );
    }
}
