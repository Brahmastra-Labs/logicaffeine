//! cuda_gemm_lattice_lock — the CUDA GEMM **tiling lattice**: the search space our optimizing compiler
//! sweeps to climb from the naive one-thread-per-output DP4A kernel toward the cuBLAS ceiling. A `CudaTile`
//! is a point in that lattice (block tile bm×bn, K-stage bk, thread tile tm×tn); a point is LEGAL only if it
//! fits the GPU's shared-memory + thread budget and tiles the block evenly. This lock proves the lattice is
//! non-empty and every enumerated point is legal, and that an oversized tile is rejected — the same
//! divisor-lattice discipline as the Tenstorrent `tune_tiling`.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_gemm_lattice_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::{cuda_tile_lattice, CudaTile, GpuLimits};

#[test]
fn the_tiling_lattice_is_nonempty_and_every_point_is_legal() {
    let gpu = GpuLimits::ampere(); // 48 KiB default shared mem, 1024 threads/block
    let lattice = cuda_tile_lattice(&gpu);
    assert!(
        lattice.len() >= 8,
        "the lattice must offer real choices, got {}",
        lattice.len()
    );
    for t in &lattice {
        assert!(
            t.is_legal(&gpu),
            "enumerated tile {t:?} must be legal on {gpu:?}"
        );
        // legality invariants a legal point must satisfy:
        assert!(
            t.threads() <= gpu.max_threads,
            "{t:?}: {} threads > {}",
            t.threads(),
            gpu.max_threads
        );
        assert!(t.threads() >= 32, "{t:?}: fewer than one warp");
        assert!(
            t.smem_bytes() <= gpu.smem_bytes,
            "{t:?}: {} smem bytes > {}",
            t.smem_bytes(),
            gpu.smem_bytes
        );
        assert_eq!(
            t.bm % t.tm,
            0,
            "{t:?}: block-M must be a multiple of thread-M"
        );
        assert_eq!(
            t.bn % t.tn,
            0,
            "{t:?}: block-N must be a multiple of thread-N"
        );
    }
    // the lattice is a genuine SET of distinct points (not one repeated config).
    let distinct: std::collections::BTreeSet<_> = lattice
        .iter()
        .map(|t| (t.bm, t.bn, t.bk, t.tm, t.tn))
        .collect();
    assert_eq!(
        distinct.len(),
        lattice.len(),
        "lattice points must be distinct"
    );
}

#[test]
fn an_oversized_tile_is_rejected() {
    let gpu = GpuLimits::ampere();
    // 256×256 block with a 1×1 thread tile = 65,536 threads/block >> 1024.
    let too_many_threads = CudaTile {
        bm: 256,
        bn: 256,
        bk: 32,
        tm: 1,
        tn: 1,
    };
    assert!(
        !too_many_threads.is_legal(&gpu),
        "65,536 threads/block must be rejected"
    );
    // a shared-memory buster: 256×256 block, deep K-stage.
    let smem_bust = CudaTile {
        bm: 256,
        bn: 256,
        bk: 128,
        tm: 8,
        tn: 8,
    };
    assert!(
        !smem_bust.is_legal(&gpu),
        "{smem_bust:?} needs {} smem bytes, must be rejected",
        smem_bust.smem_bytes()
    );
    // ragged tile: block-M not divisible by thread-M.
    let ragged = CudaTile {
        bm: 64,
        bn: 64,
        bk: 16,
        tm: 3,
        tn: 4,
    };
    assert!(
        !ragged.is_legal(&gpu),
        "a ragged (non-dividing) thread tile must be rejected"
    );
}
