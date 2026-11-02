//! cuda_primitive_select_lock — codegen uses the tensor cores when the hardware has them. The compute
//! primitive is SELECTED FROM THE PROBLEM: a tensor-core GPU + a 16-aligned shape → WMMA (the ~18× IMMA
//! lever); anything else (no tensor cores, or a GEMV / non-16-aligned shape) → the universal DP4A CUDA-core
//! fallback. Correct by construction: we never emit a WMMA kernel a shape or GPU cannot run.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_primitive_select_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::{
    emit_gemm_program, select_primitive, ComputePrimitive, CudaTile,
};

#[test]
fn tensor_cores_are_used_when_present_and_the_shape_is_aligned() {
    // Big 16-aligned GEMM on a tensor-core GPU → WMMA.
    assert_eq!(
        select_primitive(2048, 1024, 512, true),
        ComputePrimitive::WmmaTensorCore
    );
    assert_eq!(
        select_primitive(128, 64, 256, true),
        ComputePrimitive::WmmaTensorCore
    );
}

#[test]
fn falls_back_to_dp4a_without_tensor_cores() {
    // Same aligned shape, but the GPU has no tensor cores → DP4A on the CUDA cores.
    assert_eq!(
        select_primitive(2048, 1024, 512, false),
        ComputePrimitive::Dp4aCudaCore
    );
}

#[test]
fn falls_back_to_dp4a_on_shapes_wmma_cannot_run() {
    // Even WITH tensor cores, WMMA is illegal on non-16-aligned shapes and GEMV — must fall back, not crash.
    assert_eq!(
        select_primitive(1, 512, 512, true),
        ComputePrimitive::Dp4aCudaCore,
        "GEMV (M=1)"
    );
    assert_eq!(
        select_primitive(17, 16, 16, true),
        ComputePrimitive::Dp4aCudaCore,
        "M not 16-aligned"
    );
    assert_eq!(
        select_primitive(16, 24, 16, true),
        ComputePrimitive::Dp4aCudaCore,
        "K not 16-aligned"
    );
}

#[test]
fn the_emitted_program_matches_the_selected_primitive() {
    let tile = CudaTile {
        bm: 64,
        bn: 64,
        bk: 16,
        tm: 4,
        tn: 4,
    };
    // Tensor-core path carries the WMMA/IMMA surface.
    let wmma = emit_gemm_program(256, 256, 256, true, &tile);
    assert!(
        wmma.contains("wmma::mma_sync"),
        "tensor-core selection must emit the WMMA kernel"
    );
    assert!(wmma.contains("#include <mma.h>"));
    // Fallback path carries the DP4A surface.
    let dp4a = emit_gemm_program(256, 256, 256, false, &tile);
    assert!(
        dp4a.contains("__dp4a"),
        "no-tensor-core selection must emit the DP4A kernel"
    );
    assert!(
        !dp4a.contains("wmma::mma_sync"),
        "the DP4A fallback must not contain tensor-core ops"
    );
}
