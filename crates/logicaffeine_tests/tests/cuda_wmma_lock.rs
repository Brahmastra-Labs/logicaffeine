//! cuda_wmma_lock — the WMMA int8 tensor-core emitter: the compute-primitive that reaches the SAME Ampere
//! IMMA matrix hardware cuBLAS uses (the ~18× lever the DP4A CUDA-core path cannot close). Proves the kernel
//! carries the tensor-core API surface (`nvcuda::wmma` fragments + `mma_sync`), that WMMA is legal only on
//! 16-aligned shapes, and that it compiles for sm_86. On-GPU correctness + cp.async are the next lock.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_wmma_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::{emit_wmma_i8, wmma_legal};
use std::process::Command;

#[test]
fn the_wmma_kernel_carries_the_tensor_core_api_surface() {
    let cu = emit_wmma_i8();
    for needle in [
        "#include <mma.h>",
        "using namespace nvcuda;",
        "wmma::fragment<wmma::matrix_a",
        "wmma::fragment<wmma::matrix_b",
        "wmma::fragment<wmma::accumulator",
        "wmma::load_matrix_sync",
        "wmma::mma_sync(acc_frag, a_frag, b_frag, acc_frag)", // the tensor-core MAC
        "wmma::store_matrix_sync",
        "signed char", // int8 operands
    ] {
        assert!(cu.contains(needle), "WMMA kernel missing `{needle}`");
    }
}

#[test]
fn wmma_is_legal_only_on_16_aligned_shapes() {
    assert!(wmma_legal(16, 16, 16));
    assert!(wmma_legal(2048, 1024, 512)); // all multiples of 16
    assert!(wmma_legal(128, 64, 256));
    assert!(!wmma_legal(17, 16, 16), "M not 16-aligned");
    assert!(!wmma_legal(16, 24, 16), "K not 16-aligned (24 % 16 != 0)");
    assert!(!wmma_legal(16, 16, 100), "N not 16-aligned");
    assert!(
        !wmma_legal(1, 512, 512),
        "GEMV (M=1) is not WMMA-legal — the DP4A fallback handles it"
    );
}

#[test]
fn the_wmma_kernel_compiles_under_nvcc_for_sm86() {
    let have_nvcc = Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !have_nvcc {
        eprintln!("skip: nvcc not available");
        return;
    }
    let cu = emit_wmma_i8();
    let src = std::env::temp_dir().join("logos_wmma_i8_lock.cu");
    std::fs::write(&src, &cu).expect("write .cu");
    let out = Command::new("nvcc")
        .args(["-c", "-arch=sm_86", "-o", "/dev/null"])
        .arg(&src)
        .output()
        .expect("run nvcc");
    assert!(
        out.status.success(),
        "the emitted WMMA int8 kernel must compile under nvcc:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
