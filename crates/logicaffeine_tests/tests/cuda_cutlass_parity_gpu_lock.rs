//! cuda_cutlass_parity_gpu_lock — our compiler EMITS the exact kernel cuBLAS runs, and it hits cuBLAS parity.
//!
//! The whole cuBLAS-parity campaign resolved here. Disassembling `cublasGemmEx` (CUDA_R_8I, TN, 2048^3, sm_86)
//! showed cuBLAS dispatch a CUTLASS kernel — `cutlass_80_tensorop_i16832gemm_s8_256x128_64x3_tn_align16`.
//! CUTLASS is open source, so instead of approximating it with a hand-written kernel that tops out at ~90%
//! (ptxas will not schedule from-scratch source into CUTLASS's perfect pointer-increment addressing), our
//! compiler emits the SAME kernel via `emit_cutlass_i8_gemm` and reaches cuBLAS parity — measured 99.6-100.0%
//! head-to-head, best-of-100, same process, varied-fill oracle-checked.
//!
//! Gated on nvcc + a GPU + CUTLASS headers (set `CUTLASS_INCLUDE=/path/to/cutlass/include`, or the test
//! probes a couple of common locations). Skips loudly if any is missing.
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::{emit_cutlass_i8_gemm_bench, CutlassI8Gemm};
use std::path::{Path, PathBuf};
use std::process::Command;

fn cutlass_include() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("CUTLASS_INCLUDE") {
        let pb = PathBuf::from(p);
        if pb.join("cutlass/gemm/device/gemm.h").exists() {
            return Some(pb);
        }
    }
    let in_home = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("cutlass_hdr/include"));
    for pb in in_home
        .into_iter()
        .chain(["/opt/cutlass/include", "/usr/local/cutlass/include"].map(PathBuf::from))
    {
        if pb.join("cutlass/gemm/device/gemm.h").exists() {
            return Some(pb);
        }
    }
    None
}

fn have_gpu() -> bool {
    Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        && Path::new("/dev/nvidia0").exists()
}

#[test]
fn our_compiler_emits_the_cutlass_kernel_cublas_runs_and_hits_parity() {
    if !have_gpu() {
        eprintln!("SKIP: needs nvcc + a GPU");
        return;
    }
    let Some(inc) = cutlass_include() else {
        eprintln!("SKIP: CUTLASS headers not found (set CUTLASS_INCLUDE=/path/to/cutlass/include)");
        return;
    };

    // The config our compiler selects is the one cuBLAS dispatches (verified by disassembly).
    let cfg = CutlassI8Gemm::cublas_reference();
    assert!(
        cfg.is_legal(),
        "the reference config must be legal on Ampere"
    );

    let src = emit_cutlass_i8_gemm_bench(&cfg, 2048, 2048, 2048);
    // Our compiler emits the exact CUTLASS kernel name cuBLAS uses.
    assert!(
        src.contains("cutlass::gemm::device::Gemm"),
        "emitter must produce a CUTLASS device GEMM"
    );
    assert!(
        src.contains("GemmShape<256,128,64>") && src.contains("GemmShape<16,8,32>"),
        "emitter must produce cuBLAS's 256x128x64 / m16n8k32 config"
    );

    let dir = std::env::temp_dir().join(format!("logos_cutlass_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let cu = dir.join("cutlass_gemm.cu");
    let bin = dir.join("cutlass_gemm");
    std::fs::write(&cu, &src).unwrap();

    let out = Command::new("nvcc")
        .args(["-O3", "-arch=sm_86", "-I"])
        .arg(&inc)
        .arg("-o")
        .arg(&bin)
        .arg(&cu)
        .arg("-lcublas")
        .output()
        .expect("nvcc");
    assert!(
        out.status.success(),
        "our emitted CUTLASS kernel must compile: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let run = Command::new(&bin).output().expect("run");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        !stdout.contains("INCORRECT"),
        "emitted kernel must be numerically correct vs the oracle"
    );
    assert!(
        !stdout.contains("CUTLASS_ERR") && !stdout.contains("ERR"),
        "kernel ran clean: {stdout}"
    );

    let parity: f64 = stdout
        .lines()
        .find_map(|l| {
            l.strip_prefix("PARITY ")
                .and_then(|r| r.split_whitespace().next())
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(0.0);
    eprintln!(
        "OUR-COMPILER CUTLASS kernel vs cuBLAS: {parity:.1}% parity  ({})",
        stdout.trim()
    );

    // cuBLAS IS this kernel, so parity is ~100% (run-to-run noise ~1%). Ratchet floor at 97%.
    assert!(
        parity >= 97.0,
        "our compiler-emitted CUTLASS kernel must hit cuBLAS parity (>=97%); got {parity:.1}%"
    );
}
