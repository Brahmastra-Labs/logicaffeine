//! cuda_matmul_bf16_gpu_lock — a recognized LOGOS matmul lowered under the bf16/f16 tensor-core knob runs ≡ VM.
//!
//! The `MatmulPrecision::Bf16Hmma` / `Fp16Hmma` knobs route a recognized matmul through `select_mma` → the
//! HMMA tensor-core kernel on a 16-aligned shape. On the int8-range fill the LOGOS program uses (exact in
//! bf16/f16, wide f32 accumulate), the tensor-core result equals the VM's integer matmul byte-for-byte — so
//! the SAME program under the bf16 knob prints the SAME checksum as the VM. Gated on nvcc + a device.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_matmul_bf16_gpu_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::{lower_matmul_to_cuda_with, CudaKnobs, MatmulPrecision};
use logicaffeine_compile::compile::{compile_to_cuda_matmul, vm_outcome};

/// A LOGOS matmul over `M×K · K×N` with the deterministic int8-range fill + `Σ C` checksum the CUDA emitters
/// reproduce (identical to cuda_knobs_lock's program). 16-aligned dims → WMMA-legal.
fn gemm_program(m: usize, n: usize, k: usize) -> String {
    format!(
        r#"## Main
Let rows be {m}.
Let cols be {n}.
Let mid be {k}.
Let mutable a be a new Seq of Int.
Let mutable b be a new Seq of Int.
Let mutable c be a new Seq of Int.
Let mutable p be 0.
While p is less than rows * mid:
    Push (p % 7) - 3 to a.
    Set p to p + 1.
Set p to 0.
While p is less than mid * cols:
    Push (p % 5) - 2 to b.
    Set p to p + 1.
Set p to 0.
While p is less than rows * cols:
    Push 0 to c.
    Set p to p + 1.
Let mutable i be 0.
While i is less than rows:
    Let mutable k be 0.
    While k is less than mid:
        Let mutable j be 0.
        While j is less than cols:
            Let idx be i * cols + j + 1.
            Set item idx of c to item idx of c + item (i * mid + k + 1) of a * item (k * cols + j + 1) of b.
            Set j to j + 1.
        Set k to k + 1.
    Set i to i + 1.
Let mutable checksum be 0.
Let mutable q be 1.
While q is at most rows * cols:
    Set checksum to checksum + item q of c.
    Set q to q + 1.
Show checksum.
"#
    )
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

#[test]
fn the_bf16_knob_selects_the_fastest_legal_kernel() {
    let lower = |src: &str, prec| {
        let (spec, _) = compile_to_cuda_matmul(src).unwrap();
        lower_matmul_to_cuda_with(
            &spec,
            &CudaKnobs {
                precision: prec,
                ..Default::default()
            },
        )
    };
    // 16-aligned but NOT tile-dividing (16 < the 128 block tile) → the basic one-warp WMMA kernel.
    let basic = lower(&gemm_program(16, 16, 16), MatmulPrecision::Bf16Hmma);
    assert!(
        basic.contains("gemm_wmma_bf16") && !basic.contains("cpasync"),
        "small 16-aligned bf16 -> basic WMMA"
    );
    assert!(
        lower(&gemm_program(16, 16, 16), MatmulPrecision::Fp16Hmma).contains("gemm_wmma_f16"),
        "fp16 knob -> HMMA f16"
    );
    // Tile-dividing (mult of 128×128×32) → the FASTEST kernel: cp.async double-buffered.
    let fast = lower(&gemm_program(256, 256, 128), MatmulPrecision::Bf16Hmma);
    assert!(
        fast.contains("gemm_wmma_cpasync") && fast.contains("__pipeline_memcpy_async"),
        "tile-dividing bf16 -> cp.async"
    );
    assert!(
        fast.contains("#define EPC 8"),
        "bf16 cp.async uses 8-element (16-byte) chunks"
    );
    // Ragged (not 16-aligned) → the always-correct int64 kernel (never a WMMA kernel it cannot run).
    let ragged = lower(&gemm_program(24, 16, 16), MatmulPrecision::Bf16Hmma);
    assert!(
        !ragged.contains("wmma") && ragged.contains("int64_t"),
        "ragged shape -> int64 fallback"
    );
}

#[test]
fn bf16_and_f16_lowered_matmul_equal_the_vm_on_the_gpu() {
    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP matmul bf16 GPU: needs nvcc + /dev/nvidia0.");
        return;
    }
    let knobs = CudaKnobs::default();
    // (16³) and (32×64×48) 16-aligned → the basic WMMA path; (128×128×32) tile-divides → the wired cp.async path.
    // Proving the last one ≡ VM proves the cp.async auto-lowering selection is correct end-to-end, not just structurally.
    for (m, n, k) in [(16usize, 16usize, 16usize), (32, 64, 48), (128, 128, 32)] {
        let src = gemm_program(m, n, k);
        let want = vm_outcome(&src).output.trim().to_string();
        let (spec, _cu) = compile_to_cuda_matmul(&src).unwrap();
        let tile_dividing = m % 128 == 0 && n % 128 == 0 && k % 32 == 0;
        for prec in [MatmulPrecision::Bf16Hmma, MatmulPrecision::Fp16Hmma] {
            let cu = lower_matmul_to_cuda_with(
                &spec,
                &CudaKnobs {
                    precision: prec,
                    ..knobs
                },
            );
            assert_eq!(
                cu.contains("gemm_wmma_cpasync"),
                tile_dividing,
                "({m}×{n}×{k}) {prec:?}: cp.async iff tile-dividing"
            );
            let dir = std::env::temp_dir().join(format!(
                "logos_mm_hmma_{}_{m}x{n}x{k}_{prec:?}",
                std::process::id()
            ));
            let _ = std::fs::create_dir_all(&dir);
            let (s, b) = (dir.join("m.cu"), dir.join("m"));
            std::fs::write(&s, &cu).unwrap();
            let c = std::process::Command::new("nvcc")
                .args(["-O2", &knobs.chip.nvcc_arch_flag(), "-o"])
                .arg(&b)
                .arg(&s)
                .output()
                .unwrap();
            assert!(
                c.status.success(),
                "{prec:?} matmul must compile:\n{}\n\n{cu}",
                String::from_utf8_lossy(&c.stderr)
            );
            let got =
                String::from_utf8_lossy(&std::process::Command::new(&b).output().unwrap().stdout)
                    .trim()
                    .to_string();
            assert_eq!(
                got, want,
                "({m}×{n}×{k}) {prec:?} lowered matmul must equal the VM"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
