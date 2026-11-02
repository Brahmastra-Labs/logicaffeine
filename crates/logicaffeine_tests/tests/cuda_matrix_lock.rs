//! ★ THE CUDA INT8 GEMM LOCK ★ — the DP4A weight-stationary GEMM path of the CUDA tier, anchored to the
//! certified `eval_systolic` reference (itself ≡ VM ≡ SAT-placed PE ≡ iverilog in `weight_stationary_gemm_lock`).
//! Running the emitted DP4A kernel and comparing to `eval_systolic` adds a CUDA rung to that proof chain.
//!
//! Two halves over the same shapes:
//!  1. **Structural (always runs):** the int8 weight-stationary GEMM is a certified capability of the
//!     RDU-shaped fabric, and the emitted `.cu` is a well-formed DP4A program.
//!  2. **Numeric biconditional (gated on nvcc):** the compiled kernel's output equals `eval_systolic`
//!     byte-for-byte over the shapes. `K` is kept ≤ 32 so DP4A's int32 accumulation cannot overflow and
//!     thus equals the reference's i64 wrapping exactly.
//!
//! ⚠️ If a lock goes RED, fix the kernel (`codegen_cuda::matrix`), never the assertion.

use logicaffeine_compile::codegen_cuda::{emit_int8_gemm, sample_gemm_data};
use logicaffeine_compile::codegen_sva::systolic::{eval_systolic, Systolic};

/// GEMM shapes `(m, k, n)` — `k ≤ 32` keeps DP4A's int32 accumulator exact vs the i64 reference. Includes a
/// non-multiple-of-4 `k` (padding path) and a rectangular shape.
const SHAPES: &[(usize, usize, usize)] =
    &[(4, 8, 4), (2, 5, 2), (3, 16, 5), (8, 32, 8), (1, 12, 7)];

fn oracle(m: usize, k: usize, n: usize, weights: &[i32], acts: &[i32]) -> Vec<i64> {
    let sys = Systolic::new(k, n, weights.iter().map(|&x| x as i64).collect());
    eval_systolic(&sys, &acts.iter().map(|&x| x as i64).collect::<Vec<_>>(), m)
}

#[test]
fn int8_weight_stationary_gemm_emits_a_wellformed_dp4a_kernel() {
    for &(m, k, n) in SHAPES {
        let (w, a) = sample_gemm_data(m, k, n);
        let art = emit_int8_gemm(m, k, n, &w, &a).unwrap_or_else(|e| {
            panic!("({m},{k},{n}) int8 WS GEMM must be a certified capability: {e:?}")
        });
        let cu = &art.kernel_cu;
        assert!(
            cu.contains("__dp4a("),
            "({m},{k},{n}): must use the DP4A int8 intrinsic"
        );
        assert!(
            cu.contains("__global__ void gemm_i8"),
            "({m},{k},{n}): kernel missing"
        );
        assert!(cu.contains(&format!("#define M {m}")), "({m},{k},{n}): M");
        assert!(cu.contains(&format!("#define N {n}")), "({m},{k},{n}): N");
        assert_eq!((art.m, art.k, art.n), (m, k, n));
    }
}

fn have_nvcc() -> bool {
    std::process::Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A runnable GPU (driver loaded) — `/dev/nvidia0` exists iff the kernel module is up.
fn have_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
}

/// nvcc-compile a `.cu` WITHOUT running — verifies the DP4A kernel is valid `sm_86` code with no driver.
fn nvcc_compiles(cu: &str, label: &str) {
    let dir = std::env::temp_dir().join(format!("logos_cuda_gc_{}_{}", std::process::id(), label));
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("logos_cuda.cu");
    let bin = dir.join("logos_cuda");
    std::fs::write(&src, cu).expect("write .cu");
    let compile = std::process::Command::new("nvcc")
        .args(["-O2", "-arch=sm_86", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .expect("run nvcc");
    assert!(
        compile.status.success(),
        "`{label}`: DP4A GEMM must compile with nvcc.\nstderr:\n{}\n\n.cu:\n{cu}",
        String::from_utf8_lossy(&compile.stderr),
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dp4a_gemm_compiles_with_nvcc() {
    if !have_nvcc() {
        eprintln!(
            "SKIP cuda GEMM compile: nvcc not installed (install the CUDA toolkit — Phase 0)"
        );
        return;
    }
    for &(m, k, n) in SHAPES {
        let (w, a) = sample_gemm_data(m, k, n);
        let art = emit_int8_gemm(m, k, n, &w, &a).expect("emit");
        nvcc_compiles(&art.kernel_cu, &format!("gemm_{m}x{k}x{n}"));
    }
}

/// Compile a `.cu` with nvcc, run it, and parse the printed row-major int32 outputs.
fn nvcc_run_gemm(cu: &str, label: &str) -> Vec<i64> {
    let dir =
        std::env::temp_dir().join(format!("logos_cuda_gemm_{}_{}", std::process::id(), label));
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("logos_cuda.cu");
    let bin = dir.join("logos_cuda");
    std::fs::write(&src, cu).expect("write .cu");
    let compile = std::process::Command::new("nvcc")
        .args(["-O2", "-arch=sm_86", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .expect("run nvcc");
    assert!(
        compile.status.success(),
        "`{label}`: DP4A GEMM must compile with nvcc.\nstderr:\n{}\n\n.cu:\n{cu}",
        String::from_utf8_lossy(&compile.stderr),
    );
    let run = std::process::Command::new(&bin)
        .output()
        .expect("run gemm binary");
    assert!(
        run.status.success(),
        "`{label}`: GEMM binary should run.\nstderr:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let out = String::from_utf8_lossy(&run.stdout);
    let vals: Vec<i64> = out
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().parse().expect("int output"))
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    vals
}

#[test]
fn dp4a_gemm_equals_the_certified_systolic_reference() {
    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP cuda GEMM execution: needs nvcc + a GPU driver (/dev/nvidia0). Compile is covered \
                   by dp4a_gemm_compiles_with_nvcc; running awaits the driver.");
        return;
    }
    for &(m, k, n) in SHAPES {
        let (w, a) = sample_gemm_data(m, k, n);
        let art = emit_int8_gemm(m, k, n, &w, &a).expect("emit");
        let label = format!("gemm_{m}x{k}x{n}");
        let got = nvcc_run_gemm(&art.kernel_cu, &label);
        let want = oracle(m, k, n, &w, &a);
        assert_eq!(
            got, want,
            "`{label}`: DP4A GEMM output must equal eval_systolic (the certified reference)"
        );
    }
}
