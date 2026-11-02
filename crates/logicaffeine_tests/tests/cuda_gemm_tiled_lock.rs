//! cuda_gemm_tiled_lock — the tiled DP4A GEMM emitter. Proves the compiler emits a shared-memory +
//! register-blocked DP4A kernel as a FUNCTION of the tiling-lattice point (change the tile ⇒ different
//! kernel), with the reuse structure the naive one-thread-per-output kernel lacks, and that it compiles
//! for the RTX 3060 (sm_86) under nvcc. Correctness on-device is the next lock (cuda_gemm_tiled_gpu).
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_gemm_tiled_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::{emit_tiled_dp4a, CudaTile};
use std::process::Command;

#[test]
fn the_tiled_kernel_is_a_function_of_the_tile_with_the_reuse_structure() {
    let big = emit_tiled_dp4a(&CudaTile {
        bm: 128,
        bn: 128,
        bk: 16,
        tm: 8,
        tn: 8,
    });
    let small = emit_tiled_dp4a(&CudaTile {
        bm: 64,
        bn: 64,
        bk: 8,
        tm: 4,
        tn: 4,
    });
    assert_ne!(
        big, small,
        "the tiled kernel must be a function of the tile, not a fixed string"
    );
    for needle in [
        "__shared__ signed char As", // A staged in shared memory (the reuse the naive kernel lacks)
        "__shared__ signed char Bs",
        "int acc[TM][TN]", // per-thread register accumulator tile
        "__dp4a(aa, bb",   // DP4A inner product
        "for (int k0 = 0; k0 < K; k0 += BK)", // the K-stage loop
        "__syncthreads",
        "#define BM 128",
        "#define TM 8",
    ] {
        assert!(big.contains(needle), "tiled DP4A kernel missing `{needle}`");
    }
    // the baked blocking reflects the tile, not a template constant.
    assert!(small.contains("#define BM 64") && small.contains("#define TM 4"));
}

#[test]
fn the_tiled_kernel_compiles_under_nvcc_for_sm86() {
    // Gated on a CUDA toolkit (the RTX 3060 box); skips cleanly elsewhere.
    let have_nvcc = Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !have_nvcc {
        eprintln!("skip: nvcc not available");
        return;
    }
    let cu = emit_tiled_dp4a(&CudaTile {
        bm: 128,
        bn: 128,
        bk: 16,
        tm: 8,
        tn: 8,
    });
    let src = std::env::temp_dir().join("logos_tiled_dp4a_lock.cu");
    std::fs::write(&src, &cu).expect("write .cu");
    let out = Command::new("nvcc")
        .args(["-c", "-arch=sm_86", "-o", "/dev/null"])
        .arg(&src)
        .output()
        .expect("run nvcc");
    assert!(
        out.status.success(),
        "the emitted tiled DP4A kernel must compile under nvcc:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
