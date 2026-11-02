//! cuda_hmma_bf16_gpu_lock — the half-precision tensor-core GEMM runs ≡ the scalar oracle on the RTX 3060.
//!
//! `emit_wmma(HmmaBf16|HmmaF16)` emits a 16×16×16 WMMA GEMM with bf16/f16 operands and an f32 accumulator. On
//! integer operands (exact in bf16/f16, wide-accumulated), the result is an exact integer GEMM — so a scalar
//! integer reference is the ≡ oracle. This lock compiles the emitted program with `nvcc` for the chip's arch
//! and runs it on the GPU, asserting `Σ C` equals the reference for BOTH bf16 and f16. Gated on nvcc + a
//! device (skips cleanly otherwise), mirroring the other cuda_*_gpu_lock gates.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_hmma_bf16_gpu_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::{emit_wmma_program, MmaKind};
use logicaffeine_compile::codegen_cuda::CudaKnobs;

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

/// The scalar oracle: `Σ_{m,n} Σ_k A[m,k]·B[k,n]` over the emitter's fills `A[i]=(i%7)-3`, `B[i]=(i%5)-2`
/// (row-major flat indices). All values are small integers (exact in bf16/f16), so `Σ C` is an exact integer.
fn cpu_gemm_sum(m: usize, k: usize, n: usize) -> i64 {
    let a = |mm: usize, kk: usize| ((mm * k + kk) as i64 % 7) - 3;
    let b = |kk: usize, nn: usize| ((kk * n + nn) as i64 % 5) - 2;
    let mut sum = 0i64;
    for mm in 0..m {
        for nn in 0..n {
            let mut c = 0i64;
            for kk in 0..k {
                c += a(mm, kk) * b(kk, nn);
            }
            sum += c;
        }
    }
    sum
}

#[test]
fn hmma_bf16_and_f16_gemm_equal_the_scalar_oracle_on_the_gpu() {
    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP HMMA GPU: needs nvcc + /dev/nvidia0.");
        return;
    }
    let knobs = CudaKnobs::default(); // rtx3060, sm_86 — provides both bf16 (sm_80+) and f16 (sm_70+) HMMA.
    let (m, k, n) = (32usize, 48usize, 64usize); // 16-aligned
    let want = cpu_gemm_sum(m, k, n) as f64;
    for kind in [MmaKind::HmmaBf16, MmaKind::HmmaF16] {
        let cu = emit_wmma_program(kind, m, k, n);
        let dir =
            std::env::temp_dir().join(format!("logos_hmma_{}_{}", kind.tag(), std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let (s, b) = (dir.join("g.cu"), dir.join("g"));
        std::fs::write(&s, &cu).unwrap();
        let c = std::process::Command::new("nvcc")
            .args(["-O2", &knobs.chip.nvcc_arch_flag(), "-o"])
            .arg(&b)
            .arg(&s)
            .output()
            .unwrap();
        assert!(
            c.status.success(),
            "{} WMMA must compile:\n{}\n\n{cu}",
            kind.tag(),
            String::from_utf8_lossy(&c.stderr)
        );
        let out = String::from_utf8_lossy(&std::process::Command::new(&b).output().unwrap().stdout)
            .trim()
            .to_string();
        let got: f64 = out
            .parse()
            .unwrap_or_else(|_| panic!("{} produced no number: {out:?}", kind.tag()));
        assert!(
            (got - want).abs() < 0.5,
            "{} WMMA GEMM Σ C on GPU={got} must equal the scalar oracle={want}",
            kind.tag()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
