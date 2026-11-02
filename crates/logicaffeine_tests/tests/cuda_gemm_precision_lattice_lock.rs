//! cuda_gemm_precision_lattice_lock — the GEMM MMA selector + precision-parametric WMMA emitter (Phase 2).
//!
//! The half-precision tensor-core GEMM path reuses the int8 IMMA machinery: `select_mma` picks the MMA
//! instruction from `(precision, shape, sm_arch)`, and `emit_wmma` emits the SAME 16×16×16 fragment loop over
//! the operand/accumulator types the precision fixes (bf16/f16 → f32 accumulate). This lock pins that
//! selection + the emitted types so a wrong precision/shape/arch mapping — or a bf16 kernel that quietly emits
//! int8 fragments — fails RED.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_gemm_precision_lattice_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::{
    emit_wmma, emit_wmma_program, select_mma, MmaKind,
};
use logicaffeine_compile::codegen_sva::fabric_target::Precision;

#[test]
fn select_mma_picks_the_tensor_core_kind_for_precision_shape_and_arch() {
    // 16-aligned + capable arch → the precision's tensor-core kind.
    assert_eq!(
        select_mma(Precision::Bf16, 64, 64, 64, 86),
        MmaKind::HmmaBf16,
        "bf16 on Ampere -> HMMA bf16"
    );
    assert_eq!(
        select_mma(Precision::Fp16, 64, 64, 64, 86),
        MmaKind::HmmaF16,
        "fp16 -> HMMA f16"
    );
    assert_eq!(
        select_mma(Precision::Int8, 64, 64, 64, 86),
        MmaKind::WmmaI8,
        "int8 -> IMMA"
    );
    // Non-16-aligned int8 → the always-legal DP4A CUDA-core fallback.
    assert_eq!(
        select_mma(Precision::Int8, 30, 64, 64, 86),
        MmaKind::Dp4aI8,
        "ragged shape -> DP4A"
    );
    // bf16 needs sm_80; an older arch can't run HMMA bf16.
    assert_ne!(
        select_mma(Precision::Bf16, 64, 64, 64, 75),
        MmaKind::HmmaBf16,
        "bf16 needs sm_80+"
    );
}

#[test]
fn the_mma_kinds_carry_their_real_types_and_arch() {
    assert_eq!(
        (
            MmaKind::HmmaBf16.operand_ctype(),
            MmaKind::HmmaBf16.acc_ctype()
        ),
        ("__nv_bfloat16", "float")
    );
    assert_eq!(
        (
            MmaKind::HmmaF16.operand_ctype(),
            MmaKind::HmmaF16.acc_ctype()
        ),
        ("__half", "float")
    );
    assert_eq!(
        (MmaKind::WmmaI8.operand_ctype(), MmaKind::WmmaI8.acc_ctype()),
        ("signed char", "int")
    );
    assert_eq!(MmaKind::HmmaBf16.min_arch(), 80);
    assert_eq!(MmaKind::HmmaF16.min_arch(), 70);
    assert!(MmaKind::HmmaBf16.is_tensor_core() && !MmaKind::Dp4aI8.is_tensor_core());
}

#[test]
fn emit_wmma_specializes_the_fragment_loop_per_precision() {
    let bf16 = emit_wmma(MmaKind::HmmaBf16);
    assert!(bf16.contains("#include <cuda_bf16.h>"), "bf16 header");
    assert!(
        bf16.contains("wmma::fragment<wmma::matrix_a, WM, WN, WK, __nv_bfloat16, wmma::row_major>"),
        "bf16 operand fragment"
    );
    assert!(
        bf16.contains("wmma::fragment<wmma::accumulator, WM, WN, WK, float>"),
        "f32 accumulator"
    );
    assert!(
        bf16.contains("wmma::mma_sync(acc_frag, a_frag, b_frag, acc_frag)"),
        "the tensor-core MMA"
    );
    assert!(
        bf16.contains("gemm_wmma_bf16"),
        "kernel named for the precision"
    );

    let f16 = emit_wmma(MmaKind::HmmaF16);
    assert!(
        f16.contains("#include <cuda_fp16.h>") && f16.contains("__half, wmma::row_major"),
        "f16 operand"
    );

    // The int8 path keeps its int accumulator — the precisions don't cross-contaminate.
    let i8 = emit_wmma(MmaKind::WmmaI8);
    assert!(
        i8.contains("signed char, wmma::row_major")
            && i8.contains("wmma::accumulator, WM, WN, WK, int>")
    );
}

#[test]
fn emit_wmma_program_converts_the_host_fill_to_the_operand_type() {
    let p = emit_wmma_program(MmaKind::HmmaBf16, 32, 48, 64);
    assert!(
        p.contains("__float2bfloat16((float)((i % 7) - 3))"),
        "bf16 host conversion of A"
    );
    assert!(
        p.contains("#define MM 32") && p.contains("#define KK 48") && p.contains("#define NN 64"),
        "shape baked"
    );
    assert!(
        p.contains("gemm_wmma_bf16<<<grid, 32>>>"),
        "launches the bf16 kernel, one warp per tile"
    );
    // f16 program uses the half conversion.
    assert!(emit_wmma_program(MmaKind::HmmaF16, 16, 16, 16)
        .contains("__float2half((float)((i % 7) - 3))"));
}
