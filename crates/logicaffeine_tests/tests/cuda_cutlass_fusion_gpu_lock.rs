//! cuda_cutlass_fusion_gpu_lock — where our compiler BEATS cuBLAS: epilogue fusion.
//!
//! cuBLAS is a dense-GEMM black box — it cannot fuse a pointwise tail into the GEMM kernel, so a
//! `Gemm + activation` layer forces TWO kernels: the GEMM, then a separate elementwise kernel that reads
//! the whole M*N output back from global memory, applies the activation, and writes it again. Our compiler
//! folds the activation into the CUTLASS epilogue (`emit_cutlass_i8_gemm_relu_bench`) — one kernel, no extra
//! round-trip. Measured on an RTX 3060: fused GEMM+ReLU beats cuBLAS-GEMM + separate-ReLU by ~1.4x, and is
//! numerically identical (both == relu(A*B), oracle-checked). This is the KernelBench Level-2 win: not
//! beating cuBLAS's GEMM (we're at parity via the same CUTLASS kernel), but beating the cuBLAS BASELINE a
//! real layer needs, via fusion cuBLAS structurally cannot do.
//!
//! Gated on nvcc + a GPU + CUTLASS headers (`CUTLASS_INCLUDE=/path/to/cutlass/include`).
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::{
    emit_cutlass_i8_gemm_relu_bench, CutlassI8Gemm,
};
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
fn fused_gemm_relu_beats_cublas_plus_separate_activation() {
    if !have_gpu() {
        eprintln!("SKIP: needs nvcc + a GPU");
        return;
    }
    let Some(inc) = cutlass_include() else {
        eprintln!("SKIP: CUTLASS headers not found (set CUTLASS_INCLUDE=/path/to/cutlass/include)");
        return;
    };

    let cfg = CutlassI8Gemm::cublas_reference();
    let src = emit_cutlass_i8_gemm_relu_bench(&cfg, 2048, 2048, 2048);
    assert!(
        src.contains("LinearCombinationRelu"),
        "must emit the fused ReLU epilogue"
    );
    assert!(
        src.contains("relu_kernel"),
        "must emit the separate-kernel baseline for comparison"
    );

    let dir = std::env::temp_dir().join(format!("logos_fusion_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let cu = dir.join("fused.cu");
    let bin = dir.join("fused");
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
        "fused kernel must compile: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let run = Command::new(&bin).output().expect("run");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        !stdout.contains("INCORRECT"),
        "fused kernel must equal relu(A*B)"
    );
    assert!(
        !stdout.contains("MISMATCH"),
        "fused output must match the cuBLAS+relu baseline exactly"
    );
    let speedup: f64 = stdout
        .lines()
        .find_map(|l| {
            l.rsplit_once("SPEEDUP ").and_then(|(_, r)| {
                r.trim_end_matches("x\n")
                    .trim_end_matches('x')
                    .trim()
                    .parse()
                    .ok()
            })
        })
        .unwrap_or(0.0);
    eprintln!("FUSION WIN: {}", stdout.trim());

    // We must BEAT the cuBLAS baseline a real Gemm+activation layer needs (floor 1.10x — measured ~1.43x).
    assert!(
        speedup >= 1.10,
        "fused GEMM+ReLU must beat cuBLAS+separate-activation (>=1.10x); got {speedup:.3}x"
    );
}
