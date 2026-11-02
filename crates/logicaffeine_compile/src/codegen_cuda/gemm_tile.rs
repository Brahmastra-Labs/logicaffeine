//! The CUDA GEMM **tiling lattice** — the search space our optimizing compiler sweeps to climb from the
//! naive one-thread-per-output DP4A kernel toward the cuBLAS ceiling.
//!
//! A [`CudaTile`] is one point: a thread block computes a `bm×bn` output tile by staging `bk`-deep slices of
//! A and B through shared memory (so each byte is reused across the tile instead of re-read from global
//! memory — the ~20× "tiling/reuse" lever), and each thread holds a `tm×tn` register accumulator. A point is
//! **legal** only if it tiles the block evenly and fits the GPU's shared-memory + thread budget. The
//! autotuner enumerates the legal lattice, measures each point on the device, and locks the fastest — the
//! same divisor-lattice discipline as the Tenstorrent `tune_tiling`.

use crate::codegen_sva::fabric_target::Precision;

/// The per-block hardware budget the lattice is bounded by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuLimits {
    /// Shared memory available per block, in bytes.
    pub smem_bytes: usize,
    /// Threads per block.
    pub max_threads: usize,
}

impl GpuLimits {
    /// Ampere (sm_86 — the RTX 3060): 48 KiB shared memory per block (the portable default; up to ~99 KiB is
    /// opt-in via `cudaFuncAttributeMaxDynamicSharedMemorySize`) and 1024 threads per block.
    pub fn ampere() -> Self {
        GpuLimits {
            smem_bytes: 48 * 1024,
            max_threads: 1024,
        }
    }
}

/// A point in the GEMM tiling lattice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CudaTile {
    /// Block (output) tile height — rows of C computed per block.
    pub bm: usize,
    /// Block (output) tile width — cols of C computed per block.
    pub bn: usize,
    /// K-stage depth — contraction tiles staged through shared memory per iteration (bigger ⇒ more reuse
    /// per global load, up to the shared-memory budget).
    pub bk: usize,
    /// Per-thread register-tile height.
    pub tm: usize,
    /// Per-thread register-tile width.
    pub tn: usize,
}

impl CudaTile {
    /// Threads per block: one thread per `tm×tn` register sub-tile, so `(bm/tm)·(bn/tn)`.
    pub fn threads(&self) -> usize {
        if self.tm == 0 || self.tn == 0 {
            return usize::MAX;
        }
        (self.bm / self.tm) * (self.bn / self.tn)
    }

    /// Shared-memory footprint in bytes: the A stage (`bm×bk`) + the B stage (`bk×bn`), int8 = 1 byte per
    /// element, double-buffered (`×2`) so the next K-stage streams in while the current one computes.
    pub fn smem_bytes(&self) -> usize {
        2 * (self.bm * self.bk + self.bk * self.bn)
    }

    /// A point is legal iff it tiles the block evenly, is at least one warp, and fits the GPU's thread and
    /// shared-memory budgets.
    pub fn is_legal(&self, gpu: &GpuLimits) -> bool {
        self.bm > 0
            && self.bn > 0
            && self.bk > 0
            && self.tm > 0
            && self.tn > 0
            && self.bm % self.tm == 0
            && self.bn % self.tn == 0
            && self.threads() >= 32
            && self.threads() <= gpu.max_threads
            && self.smem_bytes() <= gpu.smem_bytes
    }
}

/// Enumerate the legal tiling lattice for `gpu`: the divisor space of block tiles × K-stages × thread tiles.
/// Only points that pass [`CudaTile::is_legal`] are returned, so every element is directly emittable.
pub fn cuda_tile_lattice(gpu: &GpuLimits) -> Vec<CudaTile> {
    let mut out = Vec::new();
    for &bm in &[32usize, 64, 128] {
        for &bn in &[32usize, 64, 128] {
            for &bk in &[8usize, 16, 32] {
                for &tm in &[2usize, 4, 8] {
                    for &tn in &[2usize, 4, 8] {
                        let t = CudaTile { bm, bn, bk, tm, tn };
                        if t.is_legal(gpu) {
                            out.push(t);
                        }
                    }
                }
            }
        }
    }
    out
}

/// Emit a **shared-memory + register-blocked tiled DP4A GEMM** kernel for `tile`. Each block stages a
/// `bm×bk` slice of A and a `bk×bn` slice of B into shared memory (every byte reused across the block —
/// the reuse the naive one-thread-per-output kernel throws away), and each thread accumulates a `tm×tn`
/// register tile with `__dp4a` (4 signed-int8 MACs/instruction) over the K-stage. Computes
/// `C(M×N) = A(M×K)·B(K×N)` in int8 with int32 accumulation. The blocking constexprs are baked from `tile`,
/// so a different lattice point yields a different kernel — genuine compiler output, not a fixed template.
pub fn emit_tiled_dp4a(tile: &CudaTile) -> String {
    let defs = format!(
        "#define BM {}\n#define BN {}\n#define BK {}\n#define TM {}\n#define TN {}\n",
        tile.bm, tile.bn, tile.bk, tile.tm, tile.tn,
    );
    // Header + baked #defines via format!, then a literal body that uses the macros (no brace-escaping).
    let body = r#"#define NTHREAD ((BM / TM) * (BN / TN))
#define COL_THREADS (BN / TN)

// C(M x N) = A(M x K) . B(K x N), int8 operands, int32 accumulate, __dp4a. Block/K must divide the problem
// (host pads); ragged M/N are bounds-guarded on the write-back.
extern "C" __global__ void gemm_tiled_dp4a(const signed char* A, const signed char* B, int* C,
                                           int M, int N, int K) {
    __shared__ signed char As[BM * BK];
    __shared__ signed char Bs[BK * BN];
    const int block_row = blockIdx.y * BM;
    const int block_col = blockIdx.x * BN;
    const int tid = threadIdx.x;
    const int thread_row = (tid / COL_THREADS) * TM;
    const int thread_col = (tid % COL_THREADS) * TN;

    int acc[TM][TN];
    #pragma unroll
    for (int i = 0; i < TM; ++i)
        #pragma unroll
        for (int j = 0; j < TN; ++j) acc[i][j] = 0;

    for (int k0 = 0; k0 < K; k0 += BK) {
        for (int idx = tid; idx < BM * BK; idx += NTHREAD) {
            int r = idx / BK, c = idx % BK;
            As[idx] = A[(block_row + r) * K + (k0 + c)];
        }
        for (int idx = tid; idx < BK * BN; idx += NTHREAD) {
            int r = idx / BN, c = idx % BN;
            Bs[idx] = B[(k0 + r) * N + (block_col + c)];
        }
        __syncthreads();

        #pragma unroll
        for (int kk = 0; kk < BK; kk += 4) {
            #pragma unroll
            for (int i = 0; i < TM; ++i) {
                signed char a0 = As[(thread_row + i) * BK + kk + 0];
                signed char a1 = As[(thread_row + i) * BK + kk + 1];
                signed char a2 = As[(thread_row + i) * BK + kk + 2];
                signed char a3 = As[(thread_row + i) * BK + kk + 3];
                int aa = (a0 & 0xff) | ((a1 & 0xff) << 8) | ((a2 & 0xff) << 16) | ((a3 & 0xff) << 24);
                #pragma unroll
                for (int j = 0; j < TN; ++j) {
                    signed char b0 = Bs[(kk + 0) * BN + thread_col + j];
                    signed char b1 = Bs[(kk + 1) * BN + thread_col + j];
                    signed char b2 = Bs[(kk + 2) * BN + thread_col + j];
                    signed char b3 = Bs[(kk + 3) * BN + thread_col + j];
                    int bb = (b0 & 0xff) | ((b1 & 0xff) << 8) | ((b2 & 0xff) << 16) | ((b3 & 0xff) << 24);
                    acc[i][j] = __dp4a(aa, bb, acc[i][j]);
                }
            }
        }
        __syncthreads();
    }

    #pragma unroll
    for (int i = 0; i < TM; ++i)
        #pragma unroll
        for (int j = 0; j < TN; ++j) {
            int gr = block_row + thread_row + i;
            int gc = block_col + thread_col + j;
            if (gr < M && gc < N) C[gr * N + gc] = acc[i][j];
        }
}
"#;
    format!(
        "// Auto-generated by LOGOS codegen_cuda::gemm_tile — tiled DP4A GEMM (bm={} bn={} bk={} tm={} tn={}).\n\
         #include <cstdint>\n{defs}{body}",
        tile.bm, tile.bn, tile.bk, tile.tm, tile.tn,
    )
}

/// Emit a COMPLETE self-contained tiled-DP4A program for shape `M×K×N`: the tiled kernel plus a host `main`
/// that fills A/B deterministically, launches `gemm_tiled_dp4a<<<(N/BN, M/BM), NTHREAD>>>`, and prints the
/// int32 result row-major (one value per line) — so its output is directly comparable to a scalar oracle.
/// Requires the shape to be tile-aligned (`M%bm==0`, `N%bn==0`, `K%bk==0`).
pub fn emit_tiled_dp4a_program(tile: &CudaTile, m: usize, k: usize, n: usize) -> String {
    let kernel = emit_tiled_dp4a(tile);
    let dims = format!("#define MM {m}\n#define KK {k}\n#define NN {n}\n");
    let host = r#"#include <cstdio>
#include <cstdlib>
int main() {
    const int M = MM, K = KK, N = NN;
    signed char* hA = (signed char*)malloc((size_t)M * K);
    signed char* hB = (signed char*)malloc((size_t)K * N);
    for (long i = 0; i < (long)M * K; i++) hA[i] = (signed char)(((i * 7 + 3) % 251) - 125);
    for (long i = 0; i < (long)K * N; i++) hB[i] = (signed char)(((i * 5 + 1) % 251) - 125);
    signed char *dA, *dB; int* dC;
    cudaMalloc((void**)&dA, (size_t)M * K);
    cudaMalloc((void**)&dB, (size_t)K * N);
    cudaMalloc((void**)&dC, (size_t)M * N * sizeof(int));
    cudaMemcpy(dA, hA, (size_t)M * K, cudaMemcpyHostToDevice);
    cudaMemcpy(dB, hB, (size_t)K * N, cudaMemcpyHostToDevice);
    dim3 grid(N / BN, M / BM);
    dim3 block(NTHREAD);
    gemm_tiled_dp4a<<<grid, block>>>(dA, dB, dC, M, N, K);
    cudaDeviceSynchronize();
    int* hC = (int*)malloc((size_t)M * N * sizeof(int));
    cudaMemcpy(hC, dC, (size_t)M * N * sizeof(int), cudaMemcpyDeviceToHost);
    for (long i = 0; i < (long)M * N; i++) printf("%d\n", hC[i]);
    return 0;
}
"#;
    format!("{kernel}{dims}{host}")
}

/// Emit a TIMING bench for the tiled kernel at `M×K×N`: fills A/B on device, warms up, times 30 launches
/// with CUDA events, and prints `GOPS <value>` (2·M·N·K / seconds). Used by the autotuner to measure each
/// lattice point on the real device and lock the fastest.
pub fn emit_tiled_dp4a_bench(tile: &CudaTile, m: usize, k: usize, n: usize) -> String {
    let kernel = emit_tiled_dp4a(tile);
    let dims = format!("#define MM {m}\n#define KK {k}\n#define NN {n}\n");
    let host = r#"#include <cstdio>
#include <cstdlib>
int main() {
    const long M = MM, K = KK, N = NN;
    signed char *dA, *dB; int* dC;
    cudaMalloc((void**)&dA, (size_t)M * K);
    cudaMalloc((void**)&dB, (size_t)K * N);
    cudaMalloc((void**)&dC, (size_t)M * N * sizeof(int));
    cudaMemset(dA, 1, (size_t)M * K);
    cudaMemset(dB, 1, (size_t)K * N);
    dim3 grid(N / BN, M / BM);
    dim3 block(NTHREAD);
    for (int w = 0; w < 5; w++) gemm_tiled_dp4a<<<grid, block>>>(dA, dB, dC, M, N, K);
    cudaDeviceSynchronize();
    cudaEvent_t s, e; cudaEventCreate(&s); cudaEventCreate(&e);
    const int ITERS = 30; float total = 0;
    for (int it = 0; it < ITERS; it++) {
        cudaEventRecord(s); gemm_tiled_dp4a<<<grid, block>>>(dA, dB, dC, M, N, K); cudaEventRecord(e); cudaEventSynchronize(e);
        float ms = 0; cudaEventElapsedTime(&ms, s, e); total += ms;
    }
    double gops = 2.0 * (double)M * (double)N * (double)K / ((total/ITERS) / 1000.0) / 1e9;
    printf("ms %.4f\nGOPS %.1f\n", total/ITERS, gops);
    return 0;
}
"#;
    format!("{kernel}{dims}{host}")
}

// ───────────────────────────── WMMA int8 tensor cores (the ~18× lever to cuBLAS) ─────────────────────────
// cuBLAS int8 runs on Ampere's IMMA tensor cores; our DP4A runs on the general CUDA cores. The only way to
// close the remaining gap is to emit the SAME tensor-core instructions — `nvcuda::wmma` int8 fragments
// (16×16×16 M×N×K). The fragment shape fixes the tile granularity, so a WMMA point is only legal on a
// 16-aligned shape (`wmma_legal`). This is the compute-primitive axis of the unified lattice; the autotuner
// picks WMMA when measured strictly better (big GEMM) and DP4A otherwise (GEMV / tiny / non-16 / overflow).

/// A shape is WMMA-legal iff every dimension is a multiple of the int8 fragment size (16).
pub fn wmma_legal(m: usize, k: usize, n: usize) -> bool {
    m % 16 == 0 && k % 16 == 0 && n % 16 == 0
}

/// Emit a WMMA int8 tensor-core GEMM kernel: one warp per 16×16 output tile, accumulating over K in
/// 16-wide fragments with `wmma::mma_sync` (the IMMA path). Row-major A(M×K)·B(K×N) → int32 C(M×N).
pub fn emit_wmma_i8() -> String {
    r#"// Auto-generated by LOGOS codegen_cuda::gemm_tile — WMMA int8 tensor-core GEMM (IMMA).
#include <cstdint>
#include <mma.h>
using namespace nvcuda;

#define WM 16
#define WN 16
#define WK 16

// C(M x N) = A(M x K) . B(K x N), int8 operands, int32 accumulate, on the tensor cores. One warp computes a
// 16x16 output tile; M, N, K must be multiples of 16 (wmma_legal).
extern "C" __global__ void gemm_wmma_i8(const signed char* A, const signed char* B, int* C,
                                        int M, int N, int K) {
    const int tile_row = blockIdx.y; // which 16-row block of C this warp owns
    const int tile_col = blockIdx.x; // which 16-col block of C this warp owns

    wmma::fragment<wmma::matrix_a, WM, WN, WK, signed char, wmma::row_major> a_frag;
    wmma::fragment<wmma::matrix_b, WM, WN, WK, signed char, wmma::row_major> b_frag;
    wmma::fragment<wmma::accumulator, WM, WN, WK, int> acc_frag;
    wmma::fill_fragment(acc_frag, 0);

    for (int k = 0; k < K; k += WK) {
        wmma::load_matrix_sync(a_frag, A + tile_row * WM * K + k, K);       // A[tile_row*16 .. , k ..], ld=K
        wmma::load_matrix_sync(b_frag, B + k * N + tile_col * WN, N);       // B[k .. , tile_col*16 ..], ld=N
        wmma::mma_sync(acc_frag, a_frag, b_frag, acc_frag);
    }
    wmma::store_matrix_sync(C + tile_row * WM * N + tile_col * WN, acc_frag, N, wmma::mem_row_major);
}
"#
    .to_string()
}

/// Full self-contained WMMA int8 program for `M×K×N`: kernel + host `main` that fills A/B deterministically
/// (same fill as the tiled program, so the SAME scalar oracle applies), launches
/// `gemm_wmma_i8<<<(N/16, M/16), 32>>>` (one warp per 16×16 tile), and prints int32 C row-major.
pub fn emit_wmma_i8_program(m: usize, k: usize, n: usize) -> String {
    let kernel = emit_wmma_i8();
    let dims = format!("#define MM {m}\n#define KK {k}\n#define NN {n}\n");
    let host = r#"#include <cstdio>
#include <cstdlib>
int main() {
    const int M = MM, K = KK, N = NN;
    signed char* hA = (signed char*)malloc((size_t)M * K);
    signed char* hB = (signed char*)malloc((size_t)K * N);
    for (long i = 0; i < (long)M * K; i++) hA[i] = (signed char)(((i * 7 + 3) % 251) - 125);
    for (long i = 0; i < (long)K * N; i++) hB[i] = (signed char)(((i * 5 + 1) % 251) - 125);
    signed char *dA, *dB; int* dC;
    cudaMalloc((void**)&dA, (size_t)M * K);
    cudaMalloc((void**)&dB, (size_t)K * N);
    cudaMalloc((void**)&dC, (size_t)M * N * sizeof(int));
    cudaMemcpy(dA, hA, (size_t)M * K, cudaMemcpyHostToDevice);
    cudaMemcpy(dB, hB, (size_t)K * N, cudaMemcpyHostToDevice);
    dim3 grid(N / 16, M / 16);
    gemm_wmma_i8<<<grid, 32>>>(dA, dB, dC, M, N, K);
    cudaDeviceSynchronize();
    int* hC = (int*)malloc((size_t)M * N * sizeof(int));
    cudaMemcpy(hC, dC, (size_t)M * N * sizeof(int), cudaMemcpyDeviceToHost);
    for (long i = 0; i < (long)M * N; i++) printf("%d\n", hC[i]);
    return 0;
}
"#;
    format!("{kernel}{dims}{host}")
}

/// Timing bench for the WMMA int8 kernel: warms up, times 30 launches, prints `GOPS <value>`.
pub fn emit_wmma_i8_bench(m: usize, k: usize, n: usize) -> String {
    let kernel = emit_wmma_i8();
    let dims = format!("#define MM {m}\n#define KK {k}\n#define NN {n}\n");
    let host = r#"#include <cstdio>
#include <cstdlib>
int main() {
    const long M = MM, K = KK, N = NN;
    signed char *dA, *dB; int* dC;
    cudaMalloc((void**)&dA, (size_t)M * K);
    cudaMalloc((void**)&dB, (size_t)K * N);
    cudaMalloc((void**)&dC, (size_t)M * N * sizeof(int));
    cudaMemset(dA, 1, (size_t)M * K);
    cudaMemset(dB, 1, (size_t)K * N);
    dim3 grid(N / 16, M / 16);
    for (int w = 0; w < 5; w++) gemm_wmma_i8<<<grid, 32>>>(dA, dB, dC, M, N, K);
    cudaDeviceSynchronize();
    cudaEvent_t s, e; cudaEventCreate(&s); cudaEventCreate(&e);
    const int ITERS = 30; float total = 0;
    for (int it = 0; it < ITERS; it++) {
        cudaEventRecord(s); gemm_wmma_i8<<<grid, 32>>>(dA, dB, dC, M, N, K); cudaEventRecord(e); cudaEventSynchronize(e);
        float ms = 0; cudaEventElapsedTime(&ms, s, e); total += ms;
    }
    double gops = 2.0 * (double)M * (double)N * (double)K / ((total/ITERS) / 1000.0) / 1e9;
    printf("ms %.4f\nGOPS %.1f\n", total/ITERS, gops);
    return 0;
}
"#;
    format!("{kernel}{dims}{host}")
}

// ─────────────────────── HMMA bf16/f16 tensor cores — the half-precision GEMM path ───────────────────────
// Blackwell/Hopper/Ampere run bf16/f16 on the SAME 16×16×16 WMMA fragments as int8 IMMA, differing only in the
// operand type (`__nv_bfloat16`/`__half`) and an f32 accumulator. So the whole int8 tile machinery (wmma_legal,
// one-warp-per-16×16-tile, the K-loop) carries over; `MmaKind` selects the instruction from the requested
// precision + shape + arch, and `emit_wmma` emits the fragment loop over the right types.

/// The tensor-core (or CUDA-core) MMA family a GEMM lowers to, keyed on operand PRECISION. Int8 (`Dp4aI8`
/// CUDA-core, `WmmaI8` IMMA) already exist; `HmmaBf16`/`HmmaF16` add the half-precision tensor-core path.
/// (FP8/NVFP4 join later via the `mma.sync`/`tcgen05` PTX path on Hopper/Blackwell.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MmaKind {
    /// `__dp4a` on the CUDA cores — the universal int8 fallback (any shape, sm_61+).
    Dp4aI8,
    /// IMMA int8 tensor cores, 16×16×16 s8→s32 (sm_75+, 16-aligned).
    WmmaI8,
    /// HMMA bf16 tensor cores, 16×16×16 bf16→f32 (sm_80+, 16-aligned).
    HmmaBf16,
    /// HMMA fp16 tensor cores, 16×16×16 f16→f32 (sm_70+, 16-aligned).
    HmmaF16,
}

impl MmaKind {
    /// The CUDA operand element type.
    pub fn operand_ctype(self) -> &'static str {
        match self {
            MmaKind::HmmaBf16 => "__nv_bfloat16",
            MmaKind::HmmaF16 => "__half",
            _ => "signed char",
        }
    }
    /// The accumulator / output element type — `float` for the half paths, `int` for int8.
    pub fn acc_ctype(self) -> &'static str {
        match self {
            MmaKind::HmmaBf16 | MmaKind::HmmaF16 => "float",
            _ => "int",
        }
    }
    /// The extra header the operand type needs.
    pub fn header(self) -> &'static str {
        match self {
            MmaKind::HmmaBf16 => "#include <cuda_bf16.h>\n",
            MmaKind::HmmaF16 => "#include <cuda_fp16.h>\n",
            _ => "",
        }
    }
    /// Short tag for kernel naming / provenance.
    pub fn tag(self) -> &'static str {
        match self {
            MmaKind::Dp4aI8 => "dp4a_i8",
            MmaKind::WmmaI8 => "i8",
            MmaKind::HmmaBf16 => "bf16",
            MmaKind::HmmaF16 => "f16",
        }
    }
    /// The minimum SM architecture the instruction needs.
    pub fn min_arch(self) -> u32 {
        match self {
            MmaKind::Dp4aI8 => 61,
            MmaKind::HmmaF16 => 70,
            MmaKind::WmmaI8 => 75,
            MmaKind::HmmaBf16 => 80,
        }
    }
    /// Whether this runs on the tensor cores (WMMA) vs the CUDA cores (DP4A).
    pub fn is_tensor_core(self) -> bool {
        !matches!(self, MmaKind::Dp4aI8)
    }
    /// Bytes per operand element — 2 for bf16/f16, 1 for int8. Sizes shared-memory footprints and the
    /// cp.async elements-per-16-byte-chunk (`EPC = 16 / operand_bytes`).
    pub fn operand_bytes(self) -> usize {
        match self {
            MmaKind::HmmaBf16 | MmaKind::HmmaF16 => 2,
            MmaKind::Dp4aI8 | MmaKind::WmmaI8 => 1,
        }
    }
    /// Host-side conversion of a float `expr` into this kind's operand type.
    pub fn host_convert(self, expr: &str) -> String {
        match self {
            MmaKind::HmmaBf16 => format!("__float2bfloat16((float)({expr}))"),
            MmaKind::HmmaF16 => format!("__float2half((float)({expr}))"),
            _ => format!("(signed char)({expr})"),
        }
    }
}

/// Select the MMA kind for a `(precision, shape, sm_arch)`. Tensor-core WMMA when the shape is 16-aligned and
/// the arch provides the precision's HMMA/IMMA; otherwise the always-legal DP4A CUDA-core fallback — we never
/// emit a tensor-core kernel on a shape or GPU that cannot run it.
pub fn select_mma(prec: Precision, m: usize, k: usize, n: usize, sm_arch: u32) -> MmaKind {
    let aligned = wmma_legal(m, k, n);
    match prec {
        Precision::Bf16 if aligned && sm_arch >= 80 => MmaKind::HmmaBf16,
        Precision::Fp16 if aligned && sm_arch >= 70 => MmaKind::HmmaF16,
        Precision::Int8 if aligned && sm_arch >= 75 => MmaKind::WmmaI8,
        _ => MmaKind::Dp4aI8,
    }
}

/// Which half-precision WMMA GEMM kernel is the FASTEST legal choice for a shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HmmaKernel {
    /// cp.async double-buffered — the measured 1.43× lever; needs sm_80+, the tile to divide the shape evenly,
    /// and the (operand-byte-sized) double-buffered footprint to fit static shared memory.
    CpAsync { tile: WmmaTile, stages: usize },
    /// The basic one-warp-per-16×16-tile kernel — any 16-aligned shape (the fallback when cp.async is illegal).
    Basic,
}

/// Select the fastest LEGAL half-precision kernel for `(kind, shape, arch)`: cp.async when the default tile
/// divides the shape and its bf16/f16 footprint fits static shared memory on an Ampere+ part, else the basic
/// kernel. Callers pass a shape already known WMMA-legal (16-aligned) via [`select_mma`].
pub fn select_hmma_kernel(
    kind: MmaKind,
    m: usize,
    k: usize,
    n: usize,
    sm_arch: u32,
    gpu: &GpuLimits,
) -> HmmaKernel {
    let tile = default_wmma_tile();
    let stages = 2;
    let epc = 16 / kind.operand_bytes(); // elements per 16-byte cp.async chunk
    let tile_divides = m % tile.bm == 0 && n % tile.bn == 0 && k % tile.bk == 0;
    let chunks_align = tile.bk % epc == 0 && tile.bn % epc == 0;
    // Double-buffered footprint in BYTES (cp.async's own predicate is element-sized / int8-only).
    let footprint = stages * (tile.bm * tile.bk + tile.bk * tile.bn) * kind.operand_bytes();
    if sm_arch >= 80
        && tile_divides
        && chunks_align
        && footprint <= gpu.smem_bytes
        && tile.threads() <= gpu.max_threads
    {
        HmmaKernel::CpAsync { tile, stages }
    } else {
        HmmaKernel::Basic
    }
}

/// The knob lattice's own best bf16 rung for a shape, as a runnable benchmark: the kernel
/// [`select_hmma_kernel`] picks (cp.async when its tile divides the shape and fits, else the basic WMMA
/// kernel). This is the backend-native baseline a lowering measures itself against — the fair opponent is
/// the best thing this lattice can already emit, not its worst.
pub fn lattice_best_bf16_bench(
    m: usize,
    k: usize,
    n: usize,
    sm_arch: u32,
    gpu: &GpuLimits,
) -> String {
    match select_hmma_kernel(MmaKind::HmmaBf16, m, k, n, sm_arch, gpu) {
        HmmaKernel::CpAsync { tile, stages } => {
            emit_wmma_cpasync_bench(MmaKind::HmmaBf16, &tile, stages, m, k, n)
        }
        HmmaKernel::Basic => emit_wmma_basic_bench(MmaKind::HmmaBf16, m, k, n),
    }
}

/// Emit a WMMA tensor-core GEMM kernel for a tensor-core [`MmaKind`] — the SAME 16×16×16 fragment loop as the
/// int8 IMMA path, over the operand/accumulator types the precision fixes (bf16/f16 → f32 accumulate). Row-major
/// `A(M×K)·B(K×N)`. (Not for `Dp4aI8`, which is a CUDA-core kernel; use [`emit_tiled_dp4a`].)
pub fn emit_wmma(kind: MmaKind) -> String {
    let op = kind.operand_ctype();
    let acc = kind.acc_ctype();
    let tag = kind.tag();
    let header = kind.header();
    let zero = if acc == "float" { "0.0f" } else { "0" };
    format!(
        r#"// Auto-generated by LOGOS codegen_cuda::gemm_tile — WMMA {tag} tensor-core GEMM ({op} -> {acc}).
#include <cstdint>
#include <mma.h>
{header}using namespace nvcuda;

#define WM 16
#define WN 16
#define WK 16

// C(M x N) = A(M x K) . B(K x N); {op} operands, {acc} accumulate, on the tensor cores. One warp = a 16x16
// output tile; M, N, K multiples of 16 (wmma_legal).
extern "C" __global__ void gemm_wmma_{tag}(const {op}* A, const {op}* B, {acc}* C, int M, int N, int K) {{
    const int tile_row = blockIdx.y;
    const int tile_col = blockIdx.x;
    wmma::fragment<wmma::matrix_a, WM, WN, WK, {op}, wmma::row_major> a_frag;
    wmma::fragment<wmma::matrix_b, WM, WN, WK, {op}, wmma::row_major> b_frag;
    wmma::fragment<wmma::accumulator, WM, WN, WK, {acc}> acc_frag;
    wmma::fill_fragment(acc_frag, {zero});
    for (int k = 0; k < K; k += WK) {{
        wmma::load_matrix_sync(a_frag, A + tile_row * WM * K + k, K);
        wmma::load_matrix_sync(b_frag, B + k * N + tile_col * WN, N);
        wmma::mma_sync(acc_frag, a_frag, b_frag, acc_frag);
    }}
    wmma::store_matrix_sync(C + tile_row * WM * N + tile_col * WN, acc_frag, N, wmma::mem_row_major);
}}
"#,
        tag = tag,
        op = op,
        acc = acc,
        header = header,
        zero = zero,
    )
}

/// Full self-contained WMMA program for `M×K×N` at [`MmaKind`] `kind` — fills A/B with small integers (exact in
/// bf16/f16/int8), runs one warp per 16×16 tile, and prints `Σ C` (a double). Because the operands are exact
/// small integers and the accumulate is wide (f32/int32), `Σ C` is an exact integer, so a scalar integer GEMM
/// is the ≡ oracle regardless of precision.
pub fn emit_wmma_program(kind: MmaKind, m: usize, k: usize, n: usize) -> String {
    let kernel = emit_wmma(kind);
    let op = kind.operand_ctype();
    let acc = kind.acc_ctype();
    let tag = kind.tag();
    let conv_a = kind.host_convert("(i % 7) - 3");
    let conv_b = kind.host_convert("(i % 5) - 2");
    format!(
        r#"{kernel}#include <cstdio>
#include <cstdlib>
#define MM {m}
#define KK {k}
#define NN {n}
int main() {{
    const int M = MM, K = KK, N = NN;
    {op}* hA = ({op}*)malloc((size_t)M * K * sizeof({op}));
    {op}* hB = ({op}*)malloc((size_t)K * N * sizeof({op}));
    for (long i = 0; i < (long)M * K; i++) hA[i] = {conv_a};
    for (long i = 0; i < (long)K * N; i++) hB[i] = {conv_b};
    {op} *dA, *dB; {acc}* dC;
    cudaMalloc((void**)&dA, (size_t)M * K * sizeof({op}));
    cudaMalloc((void**)&dB, (size_t)K * N * sizeof({op}));
    cudaMalloc((void**)&dC, (size_t)M * N * sizeof({acc}));
    cudaMemcpy(dA, hA, (size_t)M * K * sizeof({op}), cudaMemcpyHostToDevice);
    cudaMemcpy(dB, hB, (size_t)K * N * sizeof({op}), cudaMemcpyHostToDevice);
    dim3 grid(N / 16, M / 16);
    gemm_wmma_{tag}<<<grid, 32>>>(dA, dB, dC, M, N, K);
    cudaDeviceSynchronize();
    {acc}* hC = ({acc}*)malloc((size_t)M * N * sizeof({acc}));
    cudaMemcpy(hC, dC, (size_t)M * N * sizeof({acc}), cudaMemcpyDeviceToHost);
    double sum = 0.0; for (long i = 0; i < (long)M * N; i++) sum += (double)hC[i];
    printf("%.1f\n", sum);
    return 0;
}}
"#,
        kernel = kernel,
        op = op,
        acc = acc,
        tag = tag,
        m = m,
        k = k,
        n = n,
        conv_a = conv_a,
        conv_b = conv_b,
    )
}

// ─────────────────────── correct-by-construction tile selection (roofline cost model) ───────────────────────
// "Mathematically optimally tile" = CONSTRUCT the tile that minimizes an analytical cost model over the
// provably-complete FEASIBLE region, rather than sweep-and-hope. The primary term is closed-form and
// data-confirmed: global memory traffic ∝ (1/BM + 1/BN) (each A-element is read N/BN times, each B-element
// M/BM times), so a bigger block tile strictly reduces traffic — until it stops fitting registers/shared
// memory. The construction returns the min-traffic FEASIBLE tile; the GPU then verifies the last mile (the
// second-order occupancy/pipeline effects the roofline does not model). Correct by construction: legal (fits
// the hardware) + cost-optimal (min over the feasible region) + semantically the GEMM (the emitter + oracle).

/// SM-level register file on Ampere (sm_86): 65,536 32-bit registers per SM. A tile needs
/// `regs_per_thread · threads ≤ this` to place at least one block per SM.
pub const AMPERE_SM_REGS: usize = 65536;

impl CudaTile {
    /// Registers per thread ≈ the `tm·tn` int32 accumulator tile plus ~24 for indices/fragments/loop state.
    pub fn regs_per_thread(&self) -> usize {
        self.tm * self.tn + 24
    }

    /// Feasible-for-construction: legal on the block budget AND at least one block fits the SM register file.
    pub fn is_feasible(&self, gpu: &GpuLimits) -> bool {
        self.is_legal(gpu) && self.regs_per_thread() * self.threads() <= AMPERE_SM_REGS
    }

    /// Estimated resident blocks per SM (Ampere sm_86: 100 KiB smem, 65,536 regs, 1536 threads, ≤16 blocks) —
    /// the min over the four limiters. Bigger tiles (more regs/smem) → fewer blocks → less latency hiding.
    pub fn blocks_per_sm(&self) -> usize {
        let by_regs = AMPERE_SM_REGS / (self.regs_per_thread() * self.threads()).max(1);
        let by_smem = 102_400 / self.smem_bytes().max(1);
        let by_thread = 1536 / self.threads().max(1);
        by_regs.min(by_smem).min(by_thread).min(16)
    }

    /// Occupancy fraction: resident warps / 48 (Ampere's max warps per SM).
    pub fn occupancy(&self) -> f64 {
        (self.blocks_per_sm() * self.threads() / 32) as f64 / 48.0
    }

    /// DRAM traffic per unit of work (lower = better): global→shared reuse from the *block* tile. Each A
    /// element is read `N/BN` times, each B element `M/BM` times, so operand bytes ∝ `1/BM + 1/BN`. EXACT byte
    /// count — no fitted constant.
    pub fn dram_traffic(&self) -> f64 {
        1.0 / self.bm as f64 + 1.0 / self.bn as f64
    }

    /// Shared-memory traffic per unit of work (lower = better): shared→register reuse from the *thread* tile.
    /// Each thread does `TM·TN` MACs per `TM + TN` shared loads, so shared bytes ∝ `1/TM + 1/TN`. EXACT byte
    /// count — no fitted constant.
    pub fn smem_traffic(&self) -> f64 {
        1.0 / self.tm as f64 + 1.0 / self.tn as f64
    }

    /// Pareto dominance over the THREE physical axes — DRAM traffic ↓, SMEM traffic ↓, occupancy ↑ — with NO
    /// weights, NO fitted constants, no assumption about which matters more. `self` dominates `other` iff it is
    /// weakly better on all three and strictly better on at least one. A dominated tile cannot be the optimum
    /// (under the monotonicity assumption: less traffic and more occupancy never hurt), so it need not be
    /// measured. This is the assumption-free pruning that shrinks the space WITHOUT telling it the answer.
    pub fn dominates(&self, other: &CudaTile) -> bool {
        let weakly = self.dram_traffic() <= other.dram_traffic()
            && self.smem_traffic() <= other.smem_traffic()
            && self.occupancy() >= other.occupancy();
        let strict = self.dram_traffic() < other.dram_traffic()
            || self.smem_traffic() < other.smem_traffic()
            || self.occupancy() > other.occupancy();
        weakly && strict
    }
}

/// The full feasible tile grid — every legal tile that fits Ampere's smem + register file. PROVABLY COMPLETE
/// over this parameter space: nothing legal is silently dropped.
pub fn feasible_tiles(gpu: &GpuLimits) -> Vec<CudaTile> {
    let mut v = Vec::new();
    for &bm in &[32usize, 64, 128, 256] {
        for &bn in &[32usize, 64, 128, 256] {
            for &bk in &[8usize, 16, 32] {
                for &tm in &[4usize, 8, 16] {
                    for &tn in &[4usize, 8, 16] {
                        let t = CudaTile { bm, bn, bk, tm, tn };
                        if t.is_feasible(gpu) {
                            v.push(t);
                        }
                    }
                }
            }
        }
    }
    v
}

/// CONSTRUCT the measurement frontier: the Pareto-optimal feasible tiles — those NOT dominated by any other
/// feasible tile on the three physical axes (DRAM traffic ↓, SMEM traffic ↓, occupancy ↑). This is the honest
/// "let it construct the answer" step: we do NOT pick a winner and we assume NO weighting — we only discard
/// tiles that are provably worse on every axis (they cannot be optimal), then hand the survivors to the GPU,
/// which crowns the winner by measurement. The frontier is a small superset of the true optimum; the machine,
/// not us, resolves the un-modelable physics (bank conflicts, scheduling) that separates the survivors.
pub fn pareto_frontier(gpu: &GpuLimits) -> Vec<CudaTile> {
    let all = feasible_tiles(gpu);
    all.iter()
        .filter(|t| !all.iter().any(|o| o.dominates(t)))
        .copied()
        .collect()
}

// ─────────────────────── compute-primitive selection: use the tensor cores when we have them ───────────────
/// Which compute hardware a GEMM kernel targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputePrimitive {
    /// DP4A on the CUDA cores — works on ANY shape (including GEMV), the universal fallback.
    Dp4aCudaCore,
    /// WMMA (IMMA) on the tensor cores — needs a tensor-core GPU AND 16-aligned M,K,N. The ~18× lever.
    WmmaTensorCore,
}

/// SELECT the compute primitive FROM THE PROBLEM: use the tensor cores when the GPU has them AND the shape is
/// WMMA-legal (16-aligned); otherwise fall back to DP4A on the CUDA cores. Correct by construction — we never
/// emit a WMMA kernel on a shape or GPU that cannot run it. (Which of the two is actually *faster* when both
/// are legal is settled by measurement on the frontier, but WMMA is the strictly-more-powerful default.)
pub fn select_primitive(m: usize, k: usize, n: usize, has_tensor_cores: bool) -> ComputePrimitive {
    if has_tensor_cores && wmma_legal(m, k, n) {
        ComputePrimitive::WmmaTensorCore
    } else {
        ComputePrimitive::Dp4aCudaCore
    }
}

/// Emit a full runnable GEMM program, choosing the compute primitive from the problem + hardware. When the GPU
/// has tensor cores and the shape is 16-aligned, this emits the WMMA (tensor-core) kernel; otherwise the tiled
/// DP4A (CUDA-core) kernel driven by `tile`. Both paths are byte-exact vs the scalar oracle.
pub fn emit_gemm_program(
    m: usize,
    k: usize,
    n: usize,
    has_tensor_cores: bool,
    tile: &CudaTile,
) -> String {
    match select_primitive(m, k, n, has_tensor_cores) {
        ComputePrimitive::WmmaTensorCore => emit_wmma_i8_program(m, k, n),
        ComputePrimitive::Dp4aCudaCore => emit_tiled_dp4a_program(tile, m, k, n),
    }
}

// ─────────────────────── rung 3: pipelined WMMA — tensor cores FED FROM SHARED MEMORY, warp-tiled ──────────
// Basic WMMA (emit_wmma_i8) reads A/B from global every step → tensor cores starve. This kernel fuses the
// proven tiling reuse WITH the tensor cores: a BM×BN block stages A(BM×BK)+B(BK×BN) into shared memory once
// per K-step, then WARPS_M×WARPS_N warps each compute a (BM/WARPS_M)×(BN/WARPS_N) warp-tile as a grid of int8
// WMMA 16×16×16 fragments held in registers, reusing every shared byte across the warp-tile. This IS the
// cuBLAS/CUTLASS kernel structure (minus cp.async double-buffering, rung 4). The block/warp shape is a tuning
// knob — bigger tiles = more reuse per shared load, until registers/shared/threads run out.

/// A pipelined-WMMA configuration: block tile `BM×BN`, K-stage `BK`, warp grid `WARPS_M×WARPS_N`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WmmaTile {
    pub bm: usize,
    pub bn: usize,
    pub bk: usize,
    pub warps_m: usize,
    pub warps_n: usize,
}

impl WmmaTile {
    pub fn warps(&self) -> usize {
        self.warps_m * self.warps_n
    }
    pub fn threads(&self) -> usize {
        self.warps() * 32
    }
    /// Warp-tile height/width (rows/cols of C each warp owns).
    pub fn wm(&self) -> usize {
        self.bm / self.warps_m
    }
    pub fn wn(&self) -> usize {
        self.bn / self.warps_n
    }
    /// int8 shared-memory footprint (single-buffered): A stage + B stage.
    pub fn smem_bytes(&self) -> usize {
        self.bm * self.bk + self.bk * self.bn
    }
    /// Accumulator registers per thread ≈ (frags per warp) × 8 int32 regs per 16×16 acc fragment.
    pub fn acc_regs(&self) -> usize {
        (self.wm() / 16) * (self.wn() / 16) * 8
    }
    /// Legal iff everything divides on 16-boundaries, the warp grid tiles the block, and it fits Ampere.
    pub fn is_legal(&self, gpu: &GpuLimits) -> bool {
        self.bm % 16 == 0
            && self.bn % 16 == 0
            && self.bk % 16 == 0
            && self.warps_m > 0
            && self.warps_n > 0
            && self.bm % self.warps_m == 0
            && self.bn % self.warps_n == 0
            && self.wm() % 16 == 0
            && self.wn() % 16 == 0
            && self.threads() <= gpu.max_threads
            && self.smem_bytes() <= gpu.smem_bytes
            && self.acc_regs() + 32 <= 255 // per-thread register cap headroom
    }
}

/// A sensible default rung-3 config: 128×128 block, BK=32, 8 warps (2×4), so each warp owns a 64×32 warp-tile
/// = 4×2 = 8 fragments. This is the classic WMMA GEMM shape.
pub fn default_wmma_tile() -> WmmaTile {
    WmmaTile {
        bm: 128,
        bn: 128,
        bk: 32,
        warps_m: 2,
        warps_n: 4,
    }
}

fn wmma_tiled_kernel(t: &WmmaTile) -> String {
    wmma_tiled_kernel_for(t, MmaKind::WmmaI8)
}

/// The pipelined-WMMA kernel (shared-memory staged, warp-tiled) for any tensor-core [`MmaKind`]. bf16/f16 swap
/// the operand type (`__nv_bfloat16`/`__half`) and the accumulator (`float`); the tiling is precision-invariant.
/// `WmmaI8` reproduces the original int8 kernel byte-for-byte (the substitutions are no-ops there).
fn wmma_tiled_kernel_for(t: &WmmaTile, kind: MmaKind) -> String {
    let defs = format!(
        "#define BM {}\n#define BN {}\n#define BK {}\n#define WARPS_M {}\n#define WARPS_N {}\n",
        t.bm, t.bn, t.bk, t.warps_m, t.warps_n,
    );
    let op = kind.operand_ctype();
    let acc = kind.acc_ctype();
    let zero = if acc == "float" { "0.0f" } else { "0" };
    let body = WMMA_TILED_BODY_I8
        .replace(
            "#include <mma.h>\n",
            &format!("#include <mma.h>\n{}", kind.header()),
        )
        .replace("signed char", op)
        .replace("int* C,", &format!("{acc}* C,"))
        .replace(
            "wmma::accumulator, 16, 16, 16, int>",
            &format!("wmma::accumulator, 16, 16, 16, {acc}>"),
        )
        .replace(
            "wmma::fill_fragment(acc[i][j], 0)",
            &format!("wmma::fill_fragment(acc[i][j], {zero})"),
        );
    format!("{defs}{body}")
}

const WMMA_TILED_BODY_I8: &str = r#"#include <mma.h>
using namespace nvcuda;
#define WM (BM / WARPS_M)
#define WN (BN / WARPS_N)
#define NTHREAD (WARPS_M * WARPS_N * 32)
// C(M x N) = A(M x K) . B(K x N); A,B row-major int8; int32 accumulate; tensor cores fed from shared memory.
extern "C" __global__ void gemm_wmma_tiled(const signed char* A, const signed char* B, int* C,
                                           int M, int N, int K) {
    __shared__ signed char As[BM * BK];
    __shared__ signed char Bs[BK * BN];
    const int tid = threadIdx.x;
    const int warpId = tid / 32;
    const int warp_row = (warpId / WARPS_N) * WM;
    const int warp_col = (warpId % WARPS_N) * WN;
    const int block_row = blockIdx.y * BM;
    const int block_col = blockIdx.x * BN;

    wmma::fragment<wmma::accumulator, 16, 16, 16, int> acc[WM / 16][WN / 16];
    #pragma unroll
    for (int i = 0; i < WM / 16; ++i)
        #pragma unroll
        for (int j = 0; j < WN / 16; ++j) wmma::fill_fragment(acc[i][j], 0);

    for (int k0 = 0; k0 < K; k0 += BK) {
        for (int idx = tid; idx < BM * BK; idx += NTHREAD)
            As[idx] = A[(block_row + idx / BK) * K + (k0 + idx % BK)];
        for (int idx = tid; idx < BK * BN; idx += NTHREAD)
            Bs[idx] = B[(k0 + idx / BN) * N + (block_col + idx % BN)];
        __syncthreads();

        #pragma unroll
        for (int kk = 0; kk < BK; kk += 16) {
            wmma::fragment<wmma::matrix_a, 16, 16, 16, signed char, wmma::row_major> a_frag[WM / 16];
            wmma::fragment<wmma::matrix_b, 16, 16, 16, signed char, wmma::row_major> b_frag[WN / 16];
            #pragma unroll
            for (int i = 0; i < WM / 16; ++i)
                wmma::load_matrix_sync(a_frag[i], As + (warp_row + i * 16) * BK + kk, BK);
            #pragma unroll
            for (int j = 0; j < WN / 16; ++j)
                wmma::load_matrix_sync(b_frag[j], Bs + kk * BN + (warp_col + j * 16), BN);
            #pragma unroll
            for (int i = 0; i < WM / 16; ++i)
                #pragma unroll
                for (int j = 0; j < WN / 16; ++j)
                    wmma::mma_sync(acc[i][j], a_frag[i], b_frag[j], acc[i][j]);
        }
        __syncthreads();
    }

    #pragma unroll
    for (int i = 0; i < WM / 16; ++i)
        #pragma unroll
        for (int j = 0; j < WN / 16; ++j)
            wmma::store_matrix_sync(C + (block_row + warp_row + i * 16) * N + (block_col + warp_col + j * 16),
                                    acc[i][j], N, wmma::mem_row_major);
}
"#;

/// Emit just the pipelined-WMMA kernel for `tile` (structural / nvcc-compile checks).
pub fn emit_wmma_tiled_i8_for(tile: &WmmaTile) -> String {
    wmma_tiled_kernel(tile)
}

/// Emit the pipelined-WMMA kernel with the default config.
pub fn emit_wmma_tiled_i8() -> String {
    wmma_tiled_kernel(&default_wmma_tile())
}

/// Checksum-matching **pipelined-WMMA** GEMM program for a tensor-core `kind` (bf16/f16) — the shared-memory,
/// warp-tiled kernel with the matmul emitters' fill (`a[p]=(p%7)-3`, `b[p]=(p%5)-2`) and the `Σ C` integer
/// checksum, so on int8-range operands (exact in bf16/f16) it equals the scalar oracle byte-for-byte.
pub fn emit_wmma_tiled_program(
    kind: MmaKind,
    tile: &WmmaTile,
    m: usize,
    k: usize,
    n: usize,
) -> String {
    let kernel = wmma_tiled_kernel_for(tile, kind);
    let op = kind.operand_ctype();
    let acc = kind.acc_ctype();
    let conv_a = kind.host_convert("(p % 7) - 3");
    let conv_b = kind.host_convert("(p % 5) - 2");
    format!(
        r#"{kernel}#include <cstdio>
#include <cstdlib>
#include <cmath>
#define M {m}
#define N {n}
#define K {k}
int main(void){{
    const int MM=M,KK=K,NN=N;
    {op}* hA=({op}*)malloc((size_t)MM*KK*sizeof({op}));
    {op}* hB=({op}*)malloc((size_t)KK*NN*sizeof({op}));
    for(long p=0;p<(long)MM*KK;p++) hA[p]={conv_a};
    for(long p=0;p<(long)KK*NN;p++) hB[p]={conv_b};
    {op} *dA,*dB; {acc}* dC;
    if(cudaMalloc((void**)&dA,(size_t)MM*KK*sizeof({op}))||cudaMalloc((void**)&dB,(size_t)KK*NN*sizeof({op}))||cudaMalloc((void**)&dC,(size_t)MM*NN*sizeof({acc}))){{ fprintf(stderr,"malloc\n"); return 1; }}
    cudaMemcpy(dA,hA,(size_t)MM*KK*sizeof({op}),cudaMemcpyHostToDevice);
    cudaMemcpy(dB,hB,(size_t)KK*NN*sizeof({op}),cudaMemcpyHostToDevice);
    dim3 grid(NN/{bn}, MM/{bm}); gemm_wmma_tiled<<<grid, {threads}>>>(dA,dB,dC,MM,NN,KK);
    if(cudaDeviceSynchronize()!=cudaSuccess){{ fprintf(stderr,"sync\n"); return 1; }}
    {acc}* hC=({acc}*)malloc((size_t)MM*NN*sizeof({acc}));
    cudaMemcpy(hC,dC,(size_t)MM*NN*sizeof({acc}),cudaMemcpyDeviceToHost);
    double sum=0; for(long t=0;t<(long)MM*NN;t++) sum+=(double)hC[t];
    printf("%lld\n",(long long)llround(sum));
    return 0;
}}
"#,
        kernel = kernel,
        op = op,
        acc = acc,
        m = m,
        k = k,
        n = n,
        conv_a = conv_a,
        conv_b = conv_b,
        bm = tile.bm,
        bn = tile.bn,
        threads = tile.threads()
    )
}

/// Timing bench for the BASIC (one-warp-per-16×16-tile) WMMA kernel at `kind` — prints `GOPS <value>`.
pub fn emit_wmma_basic_bench(kind: MmaKind, m: usize, k: usize, n: usize) -> String {
    let kernel = emit_wmma(kind);
    let op = kind.operand_ctype();
    let acc = kind.acc_ctype();
    let tag = kind.tag();
    gemm_timed_main(
        &kernel,
        op,
        acc,
        &format!("gemm_wmma_{tag}<<<grid, 32>>>"),
        "dim3 grid(N/16, M/16);",
        m,
        k,
        n,
    )
}

/// Timing bench for the PIPELINED (shared-memory, warp-tiled) WMMA kernel at `kind` + `tile` — prints `GOPS`.
pub fn emit_wmma_tiled_bench(
    kind: MmaKind,
    tile: &WmmaTile,
    m: usize,
    k: usize,
    n: usize,
) -> String {
    let kernel = wmma_tiled_kernel_for(tile, kind);
    let op = kind.operand_ctype();
    let acc = kind.acc_ctype();
    let launch = format!("gemm_wmma_tiled<<<grid, {}>>>", tile.threads());
    let grid = format!("dim3 grid(N/{}, M/{});", tile.bn, tile.bm);
    gemm_timed_main(&kernel, op, acc, &launch, &grid, m, k, n)
}

/// Shared timing harness: warm up, time 30 launches of `launch` under `grid`, print `GOPS`. Values are memset
/// (timing is value-independent) so both benches share one host.
fn gemm_timed_main(
    kernel: &str,
    op: &str,
    acc: &str,
    launch: &str,
    grid: &str,
    m: usize,
    k: usize,
    n: usize,
) -> String {
    format!(
        r#"{kernel}#include <cstdio>
int main(){{
    const long M={m}, K={k}, N={n};
    {op} *dA,*dB; {acc}* dC;
    cudaMalloc((void**)&dA,(size_t)M*K*sizeof({op})); cudaMalloc((void**)&dB,(size_t)K*N*sizeof({op})); cudaMalloc((void**)&dC,(size_t)M*N*sizeof({acc}));
    cudaMemset(dA,1,(size_t)M*K*sizeof({op})); cudaMemset(dB,1,(size_t)K*N*sizeof({op}));
    {grid}
    for(int w=0;w<5;w++) {launch}(dA,dB,dC,M,N,K);
    cudaDeviceSynchronize();
    cudaEvent_t s,e; cudaEventCreate(&s); cudaEventCreate(&e);
    const int ITERS=30; float total=0;
    for(int it=0;it<ITERS;it++){{ cudaEventRecord(s); {launch}(dA,dB,dC,M,N,K); cudaEventRecord(e); cudaEventSynchronize(e); float ms=0; cudaEventElapsedTime(&ms,s,e); total+=ms; }}
    double gops = 2.0*(double)M*(double)N*(double)K/((total/ITERS)/1000.0)/1e9;
    printf("GOPS %.1f\n", gops);
    return 0;
}}
"#,
        kernel = kernel,
        op = op,
        acc = acc,
        launch = launch,
        grid = grid,
        m = m,
        k = k,
        n = n
    )
}

fn wmma_tiled_program_for(tile: &WmmaTile, m: usize, k: usize, n: usize) -> String {
    format!(
        r#"{kernel}
#include <cstdio>
#include <cstdlib>
int main() {{
    const int M = {m}, K = {k}, N = {n};
    signed char *hA = (signed char*)malloc((size_t)M*K), *hB = (signed char*)malloc((size_t)K*N);
    for (long i = 0; i < (long)M*K; ++i) hA[i] = (signed char)(((i*7+3) % 251) - 125);
    for (long i = 0; i < (long)K*N; ++i) hB[i] = (signed char)(((i*5+1) % 251) - 125);
    signed char *dA, *dB; int *dC, *hC = (int*)malloc((size_t)M*N*sizeof(int));
    cudaMalloc(&dA, (size_t)M*K); cudaMalloc(&dB, (size_t)K*N); cudaMalloc(&dC, (size_t)M*N*sizeof(int));
    cudaMemcpy(dA, hA, (size_t)M*K, cudaMemcpyHostToDevice);
    cudaMemcpy(dB, hB, (size_t)K*N, cudaMemcpyHostToDevice);
    dim3 grid(N/{bn}, M/{bm}); gemm_wmma_tiled<<<grid, {threads}>>>(dA, dB, dC, M, N, K);
    cudaDeviceSynchronize();
    cudaMemcpy(hC, dC, (size_t)M*N*sizeof(int), cudaMemcpyDeviceToHost);
    for (long i = 0; i < (long)M*N; ++i) printf("%d\n", hC[i]);
    return 0;
}}
"#,
        kernel = wmma_tiled_kernel(tile),
        m = m,
        k = k,
        n = n,
        bm = tile.bm,
        bn = tile.bn,
        threads = tile.threads()
    )
}

fn wmma_tiled_bench_for(tile: &WmmaTile, m: usize, k: usize, n: usize) -> String {
    format!(
        r#"{kernel}
#include <cstdio>
#include <cstdlib>
int main() {{
    const int M = {m}, K = {k}, N = {n};
    signed char *dA, *dB; int *dC;
    cudaMalloc(&dA, (size_t)M*K); cudaMalloc(&dB, (size_t)K*N); cudaMalloc(&dC, (size_t)M*N*sizeof(int));
    cudaMemset(dA, 1, (size_t)M*K); cudaMemset(dB, 1, (size_t)K*N);
    dim3 grid(N/{bn}, M/{bm});
    gemm_wmma_tiled<<<grid, {threads}>>>(dA, dB, dC, M, N, K); cudaDeviceSynchronize();
    cudaEvent_t s, e; cudaEventCreate(&s); cudaEventCreate(&e);
    const int IT = 30; float total = 0;
    for (int it = 0; it < IT; ++it) {{
        cudaEventRecord(s); gemm_wmma_tiled<<<grid, {threads}>>>(dA, dB, dC, M, N, K); cudaEventRecord(e); cudaEventSynchronize(e);
        float ms = 0; cudaEventElapsedTime(&ms, s, e); total += ms;
    }}
    double gops = 2.0*M*N*K / ((total/IT)/1e3) / 1e9;
    printf("GOPS %.1f\n", gops);
    return 0;
}}
"#,
        kernel = wmma_tiled_kernel(tile),
        m = m,
        k = k,
        n = n,
        bm = tile.bm,
        bn = tile.bn,
        threads = tile.threads()
    )
}

/// Emit a full runnable program (default config). Same deterministic fill as the other GEMM programs.
pub fn emit_wmma_tiled_i8_program(m: usize, k: usize, n: usize) -> String {
    wmma_tiled_program_for(&default_wmma_tile(), m, k, n)
}
/// Emit a timing harness (default config): 30 launches, prints `GOPS <value>`.
pub fn emit_wmma_tiled_i8_bench(m: usize, k: usize, n: usize) -> String {
    wmma_tiled_bench_for(&default_wmma_tile(), m, k, n)
}
/// Emit a program for a specific config (for the config sweep).
pub fn emit_wmma_tiled_i8_program_for(tile: &WmmaTile, m: usize, k: usize, n: usize) -> String {
    wmma_tiled_program_for(tile, m, k, n)
}
/// Emit a bench for a specific config (for the config sweep).
pub fn emit_wmma_tiled_i8_bench_for(tile: &WmmaTile, m: usize, k: usize, n: usize) -> String {
    wmma_tiled_bench_for(tile, m, k, n)
}

// ─────────────────────── rung 4: cp.async double-buffering — pipeline global loads behind compute ──────────
// Rung 3 serializes load → __syncthreads → compute → __syncthreads. cp.async (Ampere) streams the NEXT K-tile
// from global into a second shared buffer WHILE the tensor cores chew the current one — the WMMA units stop
// starving on memory. 16-byte vectorized async copies (needs BK%16==0 for A rows, BN%16==0 for B rows), two
// shared buffers, __pipeline_commit / __pipeline_wait_prior. This is the last big PUBLIC lever before the
// hand-tuned-SASS moat.

impl WmmaTile {
    /// cp.async needs 16-byte contiguous chunks: A rows are `BK` wide, B rows `BN` wide.
    pub fn cpasync_legal(&self, gpu: &GpuLimits) -> bool {
        self.is_legal(gpu)
            && self.bk % 16 == 0
            && self.bn % 16 == 0
            && (self.bm * self.bk) % 16 == 0
            && 2 * self.smem_bytes() <= gpu.smem_bytes // double-buffered
    }
}

fn wmma_cpasync_kernel(t: &WmmaTile) -> String {
    wmma_cpasync_kernel_staged(t, 2)
}

fn wmma_cpasync_kernel_staged(t: &WmmaTile, stages: usize) -> String {
    wmma_cpasync_kernel_staged_for(t, stages, MmaKind::WmmaI8)
}

/// cp.async double-buffered WMMA for any tensor-core [`MmaKind`]. bf16/f16 swap the operand/accumulator AND the
/// async copy's elements-per-16-byte-chunk (`EPC` = 16 for 1-byte int8, 8 for 2-byte bf16/f16 — the `cp.async`
/// copy is always 16 bytes). `WmmaI8` reproduces the int8 kernel semantics exactly (`EPC = 16`).
fn wmma_cpasync_kernel_staged_for(t: &WmmaTile, stages: usize, kind: MmaKind) -> String {
    let op = kind.operand_ctype();
    let acc = kind.acc_ctype();
    let zero = if acc == "float" { "0.0f" } else { "0" };
    let epc = if op == "signed char" { 16 } else { 8 }; // 16 bytes / sizeof(operand)
    let defs = format!(
        "#define BM {}\n#define BN {}\n#define BK {}\n#define WARPS_M {}\n#define WARPS_N {}\n#define STAGES {}\n#define EPC {}\n",
        t.bm, t.bn, t.bk, t.warps_m, t.warps_n, stages, epc,
    );
    let body = CPASYNC_BODY_I8
        .replace(
            "#include <cuda_pipeline.h>\n",
            &format!("#include <cuda_pipeline.h>\n{}", kind.header()),
        )
        .replace("signed char", op)
        .replace("int* C,", &format!("{acc}* C,"))
        .replace(
            "wmma::accumulator, 16, 16, 16, int>",
            &format!("wmma::accumulator, 16, 16, 16, {acc}>"),
        )
        .replace(
            "wmma::fill_fragment(acc[i][j], 0)",
            &format!("wmma::fill_fragment(acc[i][j], {zero})"),
        )
        .replace("(BM * BK) / 16", "(BM * BK) / EPC")
        .replace("(BK * BN) / 16", "(BK * BN) / EPC")
        .replace(
            "(idx * 16) / BK, c = (idx * 16) % BK",
            "(idx * EPC) / BK, c = (idx * EPC) % BK",
        )
        .replace(
            "(idx * 16) / BN, c = (idx * 16) % BN",
            "(idx * EPC) / BN, c = (idx * EPC) % BN",
        );
    format!("{defs}{body}")
}

const CPASYNC_BODY_I8: &str = r#"#include <mma.h>
#include <cuda_pipeline.h>
using namespace nvcuda;
#define WM (BM / WARPS_M)
#define WN (BN / WARPS_N)
#define NTHREAD (WARPS_M * WARPS_N * 32)
extern "C" __global__ void gemm_wmma_cpasync(const signed char* A, const signed char* B, int* C,
                                             int M, int N, int K) {
    __shared__ signed char As[STAGES][BM * BK];
    __shared__ signed char Bs[STAGES][BK * BN];
    const int tid = threadIdx.x;
    const int warpId = tid / 32;
    const int warp_row = (warpId / WARPS_N) * WM;
    const int warp_col = (warpId % WARPS_N) * WN;
    const int block_row = blockIdx.y * BM;
    const int block_col = blockIdx.x * BN;
    const int nstages = K / BK;

    wmma::fragment<wmma::accumulator, 16, 16, 16, int> acc[WM / 16][WN / 16];
    #pragma unroll
    for (int i = 0; i < WM / 16; ++i)
        #pragma unroll
        for (int j = 0; j < WN / 16; ++j) wmma::fill_fragment(acc[i][j], 0);

    // stream stage `st` (K offset `st*BK`) into shared buffer `buf` with 16-byte async copies.
    #define LOAD_STAGE(buf, st) do { \
        int _k0 = (st) * BK; \
        for (int idx = tid; idx < (BM * BK) / 16; idx += NTHREAD) { \
            int r = (idx * 16) / BK, c = (idx * 16) % BK; \
            __pipeline_memcpy_async(&As[buf][r * BK + c], &A[(block_row + r) * K + _k0 + c], 16); \
        } \
        for (int idx = tid; idx < (BK * BN) / 16; idx += NTHREAD) { \
            int r = (idx * 16) / BN, c = (idx * 16) % BN; \
            __pipeline_memcpy_async(&Bs[buf][r * BN + c], &B[(_k0 + r) * N + block_col + c], 16); \
        } \
        __pipeline_commit(); \
    } while (0)

    // prologue: prefetch the first STAGES-1 K-tiles so STAGES-1 async groups are always in flight.
    #pragma unroll
    for (int i = 0; i < STAGES - 1; ++i) { if (i < nstages) LOAD_STAGE(i, i); else __pipeline_commit(); }

    for (int s = 0; s < nstages; ++s) {
        __pipeline_wait_prior(STAGES - 2); // block until stage s's buffer has landed
        __syncthreads();

        int buf = s % STAGES;
        #pragma unroll
        for (int kk = 0; kk < BK; kk += 16) {
            wmma::fragment<wmma::matrix_a, 16, 16, 16, signed char, wmma::row_major> a_frag[WM / 16];
            wmma::fragment<wmma::matrix_b, 16, 16, 16, signed char, wmma::row_major> b_frag[WN / 16];
            #pragma unroll
            for (int i = 0; i < WM / 16; ++i)
                wmma::load_matrix_sync(a_frag[i], As[buf] + (warp_row + i * 16) * BK + kk, BK);
            #pragma unroll
            for (int j = 0; j < WN / 16; ++j)
                wmma::load_matrix_sync(b_frag[j], Bs[buf] + kk * BN + (warp_col + j * 16), BN);
            #pragma unroll
            for (int i = 0; i < WM / 16; ++i)
                #pragma unroll
                for (int j = 0; j < WN / 16; ++j)
                    wmma::mma_sync(acc[i][j], a_frag[i], b_frag[j], acc[i][j]);
        }
        __syncthreads();
        // prefetch the tile STAGES-1 ahead into its rotating buffer, keeping the pipeline full.
        int nxt = s + STAGES - 1;
        if (nxt < nstages) LOAD_STAGE(nxt % STAGES, nxt); else __pipeline_commit();
    }

    #pragma unroll
    for (int i = 0; i < WM / 16; ++i)
        #pragma unroll
        for (int j = 0; j < WN / 16; ++j)
            wmma::store_matrix_sync(C + (block_row + warp_row + i * 16) * N + (block_col + warp_col + j * 16),
                                    acc[i][j], N, wmma::mem_row_major);
}
"#;

pub fn emit_wmma_cpasync_i8_for(tile: &WmmaTile) -> String {
    wmma_cpasync_kernel(tile)
}

fn wmma_cpasync_harness(
    tile: &WmmaTile,
    stages: usize,
    m: usize,
    k: usize,
    n: usize,
    bench: bool,
) -> String {
    let launch = format!(
        "dim3 grid(N/{}, M/{}); gemm_wmma_cpasync<<<grid, {}>>>",
        tile.bn,
        tile.bm,
        tile.threads()
    );
    if bench {
        format!(
            r#"{kernel}
#include <cstdio>
#include <cstdlib>
int main() {{
    const int M = {m}, K = {k}, N = {n};
    signed char *dA, *dB; int *dC;
    cudaMalloc(&dA, (size_t)M*K); cudaMalloc(&dB, (size_t)K*N); cudaMalloc(&dC, (size_t)M*N*sizeof(int));
    // A,B all ones → every C[i][j] must equal K. In-harness correctness GUARD: a kernel that races (computes
    // garbage fast) or fails to launch can never print a GOPS number.
    cudaMemset(dA, 1, (size_t)M*K); cudaMemset(dB, 1, (size_t)K*N);
    {launch}(dA, dB, dC, M, N, K);
    cudaError_t err = cudaDeviceSynchronize();
    if (err != cudaSuccess) {{ printf("LAUNCH_ERROR %s\n", cudaGetErrorString(err)); return 1; }}
    int *hC = (int*)malloc((size_t)M*N*sizeof(int));
    cudaMemcpy(hC, dC, (size_t)M*N*sizeof(int), cudaMemcpyDeviceToHost);
    long bad = 0; long step = ((long)M*N) / 4096 + 1;
    for (long i = 0; i < (long)M*N; i += step) if (hC[i] != K) {{ if (bad < 3) printf("WRONG at %ld: got %d want %d\n", i, hC[i], K); bad++; }}
    if (bad) {{ printf("INCORRECT %ld samples\n", bad); return 1; }}
    // Per-iteration SYNCED timing: each launch completes before the next is queued. Firing async launches in a
    // tight loop and bracketing with events under-times badly (rapid launches error/no-op while a stale
    // correct C sits in place) — this is the honest method.
    cudaEvent_t s, e; cudaEventCreate(&s); cudaEventCreate(&e);
    const int IT = 30; float total = 0;
    for (int it = 0; it < IT; ++it) {{
        cudaEventRecord(s); {launch}(dA, dB, dC, M, N, K); cudaEventRecord(e); cudaEventSynchronize(e);
        float ms = 0; cudaEventElapsedTime(&ms, s, e); total += ms;
    }}
    cudaError_t le = cudaGetLastError();
    if (le != cudaSuccess) {{ printf("LOOP_ERROR %s\n", cudaGetErrorString(le)); return 1; }}
    printf("GOPS %.1f\n", 2.0*M*N*K / ((total/IT)/1e3) / 1e9);
    return 0;
}}
"#,
            kernel = wmma_cpasync_kernel_staged(tile, stages),
            m = m,
            k = k,
            n = n,
            launch = launch
        )
    } else {
        format!(
            r#"{kernel}
#include <cstdio>
#include <cstdlib>
int main() {{
    const int M = {m}, K = {k}, N = {n};
    signed char *hA = (signed char*)malloc((size_t)M*K), *hB = (signed char*)malloc((size_t)K*N);
    for (long i = 0; i < (long)M*K; ++i) hA[i] = (signed char)(((i*7+3) % 251) - 125);
    for (long i = 0; i < (long)K*N; ++i) hB[i] = (signed char)(((i*5+1) % 251) - 125);
    signed char *dA, *dB; int *dC, *hC = (int*)malloc((size_t)M*N*sizeof(int));
    cudaMalloc(&dA, (size_t)M*K); cudaMalloc(&dB, (size_t)K*N); cudaMalloc(&dC, (size_t)M*N*sizeof(int));
    cudaMemcpy(dA, hA, (size_t)M*K, cudaMemcpyHostToDevice);
    cudaMemcpy(dB, hB, (size_t)K*N, cudaMemcpyHostToDevice);
    {launch}(dA, dB, dC, M, N, K); cudaDeviceSynchronize();
    cudaMemcpy(hC, dC, (size_t)M*N*sizeof(int), cudaMemcpyDeviceToHost);
    for (long i = 0; i < (long)M*N; ++i) printf("%d\n", hC[i]);
    return 0;
}}
"#,
            kernel = wmma_cpasync_kernel_staged(tile, stages),
            m = m,
            k = k,
            n = n,
            launch = launch
        )
    }
}

pub fn emit_wmma_cpasync_i8_program_for(tile: &WmmaTile, m: usize, k: usize, n: usize) -> String {
    wmma_cpasync_harness(tile, 2, m, k, n, false)
}
pub fn emit_wmma_cpasync_i8_bench_for(tile: &WmmaTile, m: usize, k: usize, n: usize) -> String {
    wmma_cpasync_harness(tile, 2, m, k, n, true)
}
/// Program/bench for a specific pipeline DEPTH (`stages` shared buffers). 2 = double-buffer; 3-4 = deeper
/// pipeline that keeps the tensor cores fed further ahead (needs `stages × smem` to fit).
pub fn emit_wmma_cpasync_i8_program_staged(
    tile: &WmmaTile,
    stages: usize,
    m: usize,
    k: usize,
    n: usize,
) -> String {
    wmma_cpasync_harness(tile, stages, m, k, n, false)
}
pub fn emit_wmma_cpasync_i8_bench_staged(
    tile: &WmmaTile,
    stages: usize,
    m: usize,
    k: usize,
    n: usize,
) -> String {
    wmma_cpasync_harness(tile, stages, m, k, n, true)
}

/// Checksum-matching cp.async GEMM program for a tensor-core `kind` (bf16/f16) — Σ C ≡ the scalar oracle on
/// int8-range operands (exact in bf16/f16, wide f32 accumulate). Launches the double-buffered `gemm_wmma_cpasync`.
pub fn emit_wmma_cpasync_program(
    kind: MmaKind,
    tile: &WmmaTile,
    stages: usize,
    m: usize,
    k: usize,
    n: usize,
) -> String {
    let kernel = wmma_cpasync_kernel_staged_for(tile, stages, kind);
    let op = kind.operand_ctype();
    let acc = kind.acc_ctype();
    let conv_a = kind.host_convert("(p % 7) - 3");
    let conv_b = kind.host_convert("(p % 5) - 2");
    format!(
        r#"{kernel}#include <cstdio>
#include <cstdlib>
#include <cmath>
#define M {m}
#define N {n}
#define K {k}
int main(void){{
    const int MM=M,KK=K,NN=N;
    {op}* hA=({op}*)malloc((size_t)MM*KK*sizeof({op}));
    {op}* hB=({op}*)malloc((size_t)KK*NN*sizeof({op}));
    for(long p=0;p<(long)MM*KK;p++) hA[p]={conv_a};
    for(long p=0;p<(long)KK*NN;p++) hB[p]={conv_b};
    {op} *dA,*dB; {acc}* dC;
    if(cudaMalloc((void**)&dA,(size_t)MM*KK*sizeof({op}))||cudaMalloc((void**)&dB,(size_t)KK*NN*sizeof({op}))||cudaMalloc((void**)&dC,(size_t)MM*NN*sizeof({acc}))){{ fprintf(stderr,"malloc\n"); return 1; }}
    cudaMemcpy(dA,hA,(size_t)MM*KK*sizeof({op}),cudaMemcpyHostToDevice);
    cudaMemcpy(dB,hB,(size_t)KK*NN*sizeof({op}),cudaMemcpyHostToDevice);
    dim3 grid(NN/{bn}, MM/{bm}); gemm_wmma_cpasync<<<grid, {threads}>>>(dA,dB,dC,MM,NN,KK);
    if(cudaDeviceSynchronize()!=cudaSuccess){{ fprintf(stderr,"sync\n"); return 1; }}
    {acc}* hC=({acc}*)malloc((size_t)MM*NN*sizeof({acc}));
    cudaMemcpy(hC,dC,(size_t)MM*NN*sizeof({acc}),cudaMemcpyDeviceToHost);
    double sum=0; for(long t=0;t<(long)MM*NN;t++) sum+=(double)hC[t];
    printf("%lld\n",(long long)llround(sum));
    return 0;
}}
"#,
        kernel = kernel,
        op = op,
        acc = acc,
        m = m,
        k = k,
        n = n,
        conv_a = conv_a,
        conv_b = conv_b,
        bm = tile.bm,
        bn = tile.bn,
        threads = tile.threads()
    )
}

/// Timing bench for the cp.async double-buffered WMMA kernel at `kind` + `tile` + `stages` — prints `GOPS`.
pub fn emit_wmma_cpasync_bench(
    kind: MmaKind,
    tile: &WmmaTile,
    stages: usize,
    m: usize,
    k: usize,
    n: usize,
) -> String {
    let kernel = wmma_cpasync_kernel_staged_for(tile, stages, kind);
    gemm_timed_main(
        &kernel,
        kind.operand_ctype(),
        kind.acc_ctype(),
        &format!("gemm_wmma_cpasync<<<grid, {}>>>", tile.threads()),
        &format!("dim3 grid(N/{}, M/{});", tile.bn, tile.bm),
        m,
        k,
        n,
    )
}
/// A `stages`-deep pipeline fits iff `stages` shared buffers (each = the single-stage footprint) fit smem.
pub fn cpasync_stages_fit(tile: &WmmaTile, stages: usize, gpu: &GpuLimits) -> bool {
    tile.bk % 16 == 0
        && tile.bn % 16 == 0
        && (tile.bm * tile.bk) % 16 == 0
        && tile.bm % 16 == 0
        && tile.bn % 16 == 0
        && tile.wm() % 16 == 0
        && tile.wn() % 16 == 0
        && tile.warps_m > 0
        && tile.warps_n > 0
        && tile.bm % tile.warps_m == 0
        && tile.bn % tile.warps_n == 0
        && tile.threads() <= gpu.max_threads
        && stages * (tile.bm * tile.bk + tile.bk * tile.bn) <= gpu.smem_bytes
}

// ─────────────────────── the LAYOUT knob — a first-class frontier dimension with real tradeoffs ────────────
// The int8 tensor cores (mma.sync.s8) have a NATIVE operand layout: TN (op(A)=A^T, op(B)=B — the contraction
// dim K contiguous for both, so no on-chip transpose/shuffle). cuBLAS is ~3x faster in TN than in NN for
// exactly this reason (MEASURED on this box: TN 71,140 vs NN 24,515 GOPS). So LAYOUT is not cosmetic — it is
// a knob with a 3x tradeoff, and it belongs on the tuning frontier alongside block/warp/pipeline/primitive.
// Honest state: our WMMA kernel runs NN and already BEATS cuBLAS's NN fallback (32,299 vs 24,515); realizing
// cuBLAS's TN *speed* needs a TN-native mma.sync + ldmatrix kernel (the next build) — this enum makes layout a
// selectable dimension so that kernel slots in as another frontier point rather than a special case.

/// GEMM operand layout (BLAS transpose convention). The int8 tensor cores are fastest in `Tn`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GemmLayout {
    /// A row-major, B row-major — the layout most callers hand you. Universal; tensor cores must rearrange.
    Nn,
    /// A row-major, B col-major.
    Nt,
    /// A col-major (= A^T), B row-major — the mma.sync.s8 NATIVE layout. Fastest on the tensor cores.
    Tn,
    /// A col-major, B col-major.
    Tt,
}

impl GemmLayout {
    /// Whether this layout is the tensor cores' native contraction-contiguous layout (zero rearrange cost).
    pub fn is_tensor_core_native(&self) -> bool {
        matches!(self, GemmLayout::Tn)
    }
    /// All four layouts — the layout axis of the frontier.
    pub fn all() -> [GemmLayout; 4] {
        [
            GemmLayout::Nn,
            GemmLayout::Nt,
            GemmLayout::Tn,
            GemmLayout::Tt,
        ]
    }
}

/// Select the layout to emit FROM THE PROBLEM: if the caller's data is already TN (or we are free to choose
/// the staging), prefer the tensor-core-native `Tn` for max throughput; if the data is fixed as `Nn` and a
/// transpose would cost more than it saves, stay in `Nn` (where our kernel already beats cuBLAS). `caller_is_tn`
/// = the operands are already available contraction-contiguous. This is the tradeoff knob, not a fixed choice.
pub fn select_layout(caller_is_tn: bool) -> GemmLayout {
    if caller_is_tn {
        GemmLayout::Tn
    } else {
        GemmLayout::Nn
    }
}

// ─────────────────────── rung 6: TN-native mma.sync.s8 — feed the tensor cores their native layout ─────────
// cuBLAS's 72,511 GOPS comes from the int8 tensor cores' NATIVE instruction `mma.sync.aligned.m16n8k16.s8`
// (below the nvcuda::wmma wrapper), with operands placed in the exact register layout the hardware wants (A
// row-major m16k16, B col-major k16n8). We stage A/B in shared, then each warp loads the mma fragments BY HAND
// per the PTX fragment map and issues mma.sync. Correctness is checked against the VARIED-fill oracle (all-ones
// cannot catch a fragment permutation). Fixed 64×64 block, BK=16, 2×2 warps, each warp a 32×32 warp-tile
// (2 m-tiles × 4 n-tiles of m16n8).

fn mma_tn_kernel() -> &'static str {
    r#"#define BM 64
#define BN 64
#define BK 16
#define WARPS_M 2
#define WARPS_N 2
#define WM (BM / WARPS_M)
#define WN (BN / WARPS_N)
#define NTHREAD (WARPS_M * WARPS_N * 32)
#define PK4(p,st) (((p)[0]&0xff)|(((p)[st]&0xff)<<8)|(((p)[2*(st)]&0xff)<<16)|(((p)[3*(st)]&0xff)<<24))
// C(MxN)=A(MxK).B(KxN), int8, int32 accumulate, via the native mma.sync.aligned.m16n8k16.row.col.s32.s8.s8.s32.
extern "C" __global__ void gemm_mma_tn(const signed char* A, const signed char* B, int* C, int M, int N, int K) {
    __shared__ signed char As[BM * BK];
    __shared__ signed char Bs[BK * BN];
    const int tid = threadIdx.x, warpId = tid / 32, lane = tid % 32;
    const int gid = lane / 4, tg = lane % 4;          // groupID (0..7), thread-in-group (0..3)
    const int warp_row = (warpId / WARPS_N) * WM, warp_col = (warpId % WARPS_N) * WN;
    const int block_row = blockIdx.y * BM, block_col = blockIdx.x * BN;

    int c[WM / 16][WN / 8][4];
    #pragma unroll
    for (int mi = 0; mi < WM / 16; ++mi)
        #pragma unroll
        for (int ni = 0; ni < WN / 8; ++ni) { c[mi][ni][0]=c[mi][ni][1]=c[mi][ni][2]=c[mi][ni][3]=0; }

    for (int k0 = 0; k0 < K; k0 += BK) {
        for (int idx = tid; idx < BM * BK; idx += NTHREAD) { int r = idx / BK, cc = idx % BK; As[idx] = A[(block_row + r) * K + k0 + cc]; }
        for (int idx = tid; idx < BK * BN; idx += NTHREAD) { int r = idx / BN, cc = idx % BN; Bs[idx] = B[(k0 + r) * N + block_col + cc]; }
        __syncthreads();

        #pragma unroll
        for (int kk = 0; kk < BK; kk += 16) {
            #pragma unroll
            for (int mi = 0; mi < WM / 16; ++mi) {
                signed char* Ap0 = &As[(warp_row + mi * 16 + gid) * BK + kk + tg * 4];
                signed char* Ap1 = &As[(warp_row + mi * 16 + gid + 8) * BK + kk + tg * 4];
                int a0 = PK4(Ap0, 1), a1 = PK4(Ap1, 1);
                #pragma unroll
                for (int ni = 0; ni < WN / 8; ++ni) {
                    signed char* Bp = &Bs[(kk + tg * 4) * BN + (warp_col + ni * 8 + gid)];
                    int b0 = PK4(Bp, BN);
                    asm volatile(
                        "mma.sync.aligned.m16n8k16.row.col.s32.s8.s8.s32 {%0,%1,%2,%3},{%4,%5},{%6},{%0,%1,%2,%3};"
                        : "+r"(c[mi][ni][0]), "+r"(c[mi][ni][1]), "+r"(c[mi][ni][2]), "+r"(c[mi][ni][3])
                        : "r"(a0), "r"(a1), "r"(b0));
                }
            }
        }
        __syncthreads();
    }

    #pragma unroll
    for (int mi = 0; mi < WM / 16; ++mi)
        #pragma unroll
        for (int ni = 0; ni < WN / 8; ++ni) {
            int rb = block_row + warp_row + mi * 16, cb = block_col + warp_col + ni * 8;
            C[(rb + gid) * N + cb + tg * 2]     = c[mi][ni][0];
            C[(rb + gid) * N + cb + tg * 2 + 1] = c[mi][ni][1];
            C[(rb + gid + 8) * N + cb + tg * 2]     = c[mi][ni][2];
            C[(rb + gid + 8) * N + cb + tg * 2 + 1] = c[mi][ni][3];
        }
}
"#
}

pub fn emit_mma_tn_i8() -> String {
    mma_tn_kernel().to_string()
}

pub fn emit_mma_tn_i8_program(m: usize, k: usize, n: usize) -> String {
    format!(
        r#"{kernel}
#include <cstdio>
#include <cstdlib>
int main() {{
    const int M={m}, K={k}, N={n};
    signed char *hA=(signed char*)malloc((size_t)M*K), *hB=(signed char*)malloc((size_t)K*N);
    for (long i=0;i<(long)M*K;++i) hA[i]=(signed char)(((i*7+3)%251)-125);
    for (long i=0;i<(long)K*N;++i) hB[i]=(signed char)(((i*5+1)%251)-125);
    signed char *dA,*dB; int *dC,*hC=(int*)malloc((size_t)M*N*4);
    cudaMalloc(&dA,(size_t)M*K); cudaMalloc(&dB,(size_t)K*N); cudaMalloc(&dC,(size_t)M*N*4);
    cudaMemcpy(dA,hA,(size_t)M*K,cudaMemcpyHostToDevice); cudaMemcpy(dB,hB,(size_t)K*N,cudaMemcpyHostToDevice);
    dim3 grid(N/64, M/64); gemm_mma_tn<<<grid,128>>>(dA,dB,dC,M,N,K); cudaDeviceSynchronize();
    cudaMemcpy(hC,dC,(size_t)M*N*4,cudaMemcpyDeviceToHost);
    for (long i=0;i<(long)M*N;++i) printf("%d\n", hC[i]);
    return 0;
}}
"#,
        kernel = mma_tn_kernel(),
        m = m,
        k = k,
        n = n
    )
}

pub fn emit_mma_tn_i8_bench(m: usize, k: usize, n: usize) -> String {
    format!(
        r#"{kernel}
#include <cstdio>
#include <cstdlib>
int main() {{
    const int M={m}, K={k}, N={n};
    signed char *dA,*dB; int *dC;
    cudaMalloc(&dA,(size_t)M*K); cudaMalloc(&dB,(size_t)K*N); cudaMalloc(&dC,(size_t)M*N*4);
    cudaMemset(dA,1,(size_t)M*K); cudaMemset(dB,1,(size_t)K*N);
    dim3 grid(N/64,M/64);
    gemm_mma_tn<<<grid,128>>>(dA,dB,dC,M,N,K);
    cudaError_t err=cudaDeviceSynchronize(); if(err){{printf("LAUNCH_ERROR %s\n",cudaGetErrorString(err));return 1;}}
    int *hC=(int*)malloc((size_t)M*N*4); cudaMemcpy(hC,dC,(size_t)M*N*4,cudaMemcpyDeviceToHost);
    long step=((long)M*N)/4096+1; for(long i=0;i<(long)M*N;i+=step) if(hC[i]!=K){{printf("INCORRECT at %ld: %d!=%d\n",i,hC[i],K);return 1;}}
    cudaEvent_t s,e; cudaEventCreate(&s); cudaEventCreate(&e); int IT=30; float total=0;
    for(int it=0;it<IT;it++){{ cudaEventRecord(s); gemm_mma_tn<<<grid,128>>>(dA,dB,dC,M,N,K); cudaEventRecord(e); cudaEventSynchronize(e); float ms=0; cudaEventElapsedTime(&ms,s,e); total+=ms; }}
    printf("GOPS %.1f\n", 2.0*M*N*K/((total/IT)/1e3)/1e9);
    return 0;
}}
"#,
        kernel = mma_tn_kernel(),
        m = m,
        k = k,
        n = n
    )
}

// ─────────────────────── rung 6b: PIPELINED mma.sync.s8 TN — proven fragment map + proven cp.async ─────────
// Fuses the two proven pieces: the correct mma.sync.aligned.m16n8k16.s8 fragment map (rung 6, oracle-verified)
// + cp.async double-buffered staging (rung 4). The naive rung-6 starved because the global loads were on the
// critical path; here the next K-tile streams while the tensor cores compute. Block 128×64, BK=32, 2×2 warps
// (WM=64, WN=32 → 4×4 = 16 m16n8 tiles/warp). All-ones guard in the bench; varied-fill oracle in the program.
fn mma_tn_cpasync_kernel() -> &'static str {
    r#"#include <cuda_pipeline.h>
#define BM 128
#define BN 64
#define BK 32
#define WARPS_M 2
#define WARPS_N 2
#define WM (BM / WARPS_M)
#define WN (BN / WARPS_N)
#define NTHREAD (WARPS_M * WARPS_N * 32)
#define PK4(p,st) (((p)[0]&0xff)|(((p)[st]&0xff)<<8)|(((p)[2*(st)]&0xff)<<16)|(((p)[3*(st)]&0xff)<<24))
extern "C" __global__ void gemm_mma_tn_cpasync(const signed char* A, const signed char* B, int* C, int M, int N, int K) {
    __shared__ signed char As[2][BM * BK];
    __shared__ signed char Bs[2][BK * BN];
    const int tid = threadIdx.x, warpId = tid / 32, lane = tid % 32;
    const int gid = lane / 4, tg = lane % 4;
    const int warp_row = (warpId / WARPS_N) * WM, warp_col = (warpId % WARPS_N) * WN;
    const int block_row = blockIdx.y * BM, block_col = blockIdx.x * BN;
    const int nstages = K / BK;

    int c[WM / 16][WN / 8][4];
    #pragma unroll
    for (int mi = 0; mi < WM / 16; ++mi)
        #pragma unroll
        for (int ni = 0; ni < WN / 8; ++ni) { c[mi][ni][0]=c[mi][ni][1]=c[mi][ni][2]=c[mi][ni][3]=0; }

    #define LOAD_STAGE(buf, st) do { int _k0 = (st) * BK; \
        for (int idx = tid; idx < (BM * BK) / 16; idx += NTHREAD) { int r=(idx*16)/BK, cc=(idx*16)%BK; \
            __pipeline_memcpy_async(&As[buf][r*BK+cc], &A[(block_row+r)*K + _k0+cc], 16); } \
        for (int idx = tid; idx < (BK * BN) / 16; idx += NTHREAD) { int r=(idx*16)/BN, cc=(idx*16)%BN; \
            __pipeline_memcpy_async(&Bs[buf][r*BN+cc], &B[(_k0+r)*N + block_col+cc], 16); } \
        __pipeline_commit(); } while (0)

    LOAD_STAGE(0, 0);
    for (int s = 0; s < nstages; ++s) {
        if (s + 1 < nstages) LOAD_STAGE((s + 1) & 1, s + 1);
        __pipeline_wait_prior(s + 1 < nstages ? 1 : 0);
        __syncthreads();
        int buf = s & 1;
        #pragma unroll
        for (int kk = 0; kk < BK; kk += 16) {
            #pragma unroll
            for (int mi = 0; mi < WM / 16; ++mi) {
                signed char* Ap0 = &As[buf][(warp_row + mi*16 + gid) * BK + kk + tg*4];
                signed char* Ap1 = &As[buf][(warp_row + mi*16 + gid + 8) * BK + kk + tg*4];
                int a0 = PK4(Ap0, 1), a1 = PK4(Ap1, 1);
                #pragma unroll
                for (int ni = 0; ni < WN / 8; ++ni) {
                    signed char* Bp = &Bs[buf][(kk + tg*4) * BN + (warp_col + ni*8 + gid)];
                    int b0 = PK4(Bp, BN);
                    asm volatile("mma.sync.aligned.m16n8k16.row.col.s32.s8.s8.s32 {%0,%1,%2,%3},{%4,%5},{%6},{%0,%1,%2,%3};"
                        : "+r"(c[mi][ni][0]),"+r"(c[mi][ni][1]),"+r"(c[mi][ni][2]),"+r"(c[mi][ni][3])
                        : "r"(a0),"r"(a1),"r"(b0));
                }
            }
        }
        __syncthreads();
    }
    #pragma unroll
    for (int mi = 0; mi < WM / 16; ++mi)
        #pragma unroll
        for (int ni = 0; ni < WN / 8; ++ni) {
            int rb = block_row + warp_row + mi*16, cb = block_col + warp_col + ni*8;
            C[(rb+gid)*N + cb+tg*2]       = c[mi][ni][0];
            C[(rb+gid)*N + cb+tg*2+1]     = c[mi][ni][1];
            C[(rb+gid+8)*N + cb+tg*2]     = c[mi][ni][2];
            C[(rb+gid+8)*N + cb+tg*2+1]   = c[mi][ni][3];
        }
}
"#
}

pub fn emit_mma_tn_cpasync_program(m: usize, k: usize, n: usize) -> String {
    format!(
        r#"{kernel}
#include <cstdio>
#include <cstdlib>
int main() {{ const int M={m},K={k},N={n};
    signed char *hA=(signed char*)malloc((size_t)M*K),*hB=(signed char*)malloc((size_t)K*N);
    for(long i=0;i<(long)M*K;++i) hA[i]=(signed char)(((i*7+3)%251)-125);
    for(long i=0;i<(long)K*N;++i) hB[i]=(signed char)(((i*5+1)%251)-125);
    signed char *dA,*dB; int *dC,*hC=(int*)malloc((size_t)M*N*4);
    cudaMalloc(&dA,(size_t)M*K);cudaMalloc(&dB,(size_t)K*N);cudaMalloc(&dC,(size_t)M*N*4);
    cudaMemcpy(dA,hA,(size_t)M*K,cudaMemcpyHostToDevice);cudaMemcpy(dB,hB,(size_t)K*N,cudaMemcpyHostToDevice);
    dim3 grid(N/64,M/128); gemm_mma_tn_cpasync<<<grid,128>>>(dA,dB,dC,M,N,K); cudaDeviceSynchronize();
    cudaMemcpy(hC,dC,(size_t)M*N*4,cudaMemcpyDeviceToHost);
    for(long i=0;i<(long)M*N;++i) printf("%d\n",hC[i]); return 0; }}
"#,
        kernel = mma_tn_cpasync_kernel(),
        m = m,
        k = k,
        n = n
    )
}

pub fn emit_mma_tn_cpasync_bench(m: usize, k: usize, n: usize) -> String {
    format!(
        r#"{kernel}
#include <cstdio>
#include <cstdlib>
int main() {{ const int M={m},K={k},N={n};
    signed char *dA,*dB; int *dC;
    cudaMalloc(&dA,(size_t)M*K);cudaMalloc(&dB,(size_t)K*N);cudaMalloc(&dC,(size_t)M*N*4);
    cudaMemset(dA,1,(size_t)M*K);cudaMemset(dB,1,(size_t)K*N);
    dim3 grid(N/64,M/128);
    gemm_mma_tn_cpasync<<<grid,128>>>(dA,dB,dC,M,N,K);
    cudaError_t err=cudaDeviceSynchronize(); if(err){{printf("LAUNCH_ERROR %s\n",cudaGetErrorString(err));return 1;}}
    int *hC=(int*)malloc((size_t)M*N*4); cudaMemcpy(hC,dC,(size_t)M*N*4,cudaMemcpyDeviceToHost);
    long step=((long)M*N)/4096+1; for(long i=0;i<(long)M*N;i+=step) if(hC[i]!=K){{printf("INCORRECT at %ld: %d!=%d\n",i,hC[i],K);return 1;}}
    cudaEvent_t s,e; cudaEventCreate(&s); cudaEventCreate(&e); int IT=30; float total=0;
    for(int it=0;it<IT;it++){{ cudaEventRecord(s); gemm_mma_tn_cpasync<<<grid,128>>>(dA,dB,dC,M,N,K); cudaEventRecord(e); cudaEventSynchronize(e); float ms=0; cudaEventElapsedTime(&ms,s,e); total+=ms; }}
    printf("GOPS %.1f\n", 2.0*M*N*K/((total/IT)/1e3)/1e9); return 0; }}
"#,
        kernel = mma_tn_cpasync_kernel(),
        m = m,
        k = k,
        n = n
    )
}

// ─────────────────────── rung 7: col-major-B TN mma.sync.s8 — cuBLAS's OWN data layout ────────────────────
// The last starvation was the B fragment load: byte-strided (stride BN) because B was row-major. cuBLAS is
// fast in TN precisely because B arrives COL-MAJOR (K contiguous), so the tensor-core fragment load is one
// contiguous int32. Our COMPILER, choosing the Tn layout, emits exactly this: A row-major (contiguous k), B
// col-major (contiguous k) — both operands K-contiguous, the mma.sync.s8 native feed. cp.async double-buffered.
// This is the compiler emitting cuBLAS's kernel shape. Block 128x64, BK=32, 2x2 warps.
fn mma_tn_colb_kernel() -> &'static str {
    r#"#include <cuda_pipeline.h>
#define BM 128
#define BN 64
#define BK 32
#define WARPS_M 2
#define WARPS_N 2
#define WM (BM / WARPS_M)
#define WN (BN / WARPS_N)
#define NTHREAD (WARPS_M * WARPS_N * 32)
#define PK4(p,st) (((p)[0]&0xff)|(((p)[st]&0xff)<<8)|(((p)[2*(st)]&0xff)<<16)|(((p)[3*(st)]&0xff)<<24))
// C(MxN) = A(MxK, row-major) . B(KxN), with B stored COL-MAJOR: Bcol[n*K + k]. Both operands K-contiguous.
extern "C" __global__ void gemm_mma_tn_colb(const signed char* A, const signed char* Bcol, int* C, int M, int N, int K) {
    __shared__ signed char As[2][BM * BK];   // [m][k], k contiguous
    __shared__ signed char Bs[2][BN * BK];   // [n][k], k contiguous  (col-major tile)
    const int tid = threadIdx.x, warpId = tid / 32, lane = tid % 32;
    const int gid = lane / 4, tg = lane % 4;
    const int warp_row = (warpId / WARPS_N) * WM, warp_col = (warpId % WARPS_N) * WN;
    const int block_row = blockIdx.y * BM, block_col = blockIdx.x * BN;
    const int nstages = K / BK;

    int c[WM / 16][WN / 8][4];
    #pragma unroll
    for (int mi = 0; mi < WM / 16; ++mi)
        #pragma unroll
        for (int ni = 0; ni < WN / 8; ++ni) { c[mi][ni][0]=c[mi][ni][1]=c[mi][ni][2]=c[mi][ni][3]=0; }

    #define LOAD_STAGE(buf, st) do { int _k0 = (st) * BK; \
        for (int idx = tid; idx < (BM * BK) / 16; idx += NTHREAD) { int r=(idx*16)/BK, cc=(idx*16)%BK; \
            __pipeline_memcpy_async(&As[buf][r*BK+cc], &A[(block_row+r)*K + _k0+cc], 16); } \
        for (int idx = tid; idx < (BN * BK) / 16; idx += NTHREAD) { int r=(idx*16)/BK, cc=(idx*16)%BK; \
            __pipeline_memcpy_async(&Bs[buf][r*BK+cc], &Bcol[(block_col+r)*K + _k0+cc], 16); } \
        __pipeline_commit(); } while (0)

    LOAD_STAGE(0, 0);
    for (int s = 0; s < nstages; ++s) {
        if (s + 1 < nstages) LOAD_STAGE((s + 1) & 1, s + 1);
        __pipeline_wait_prior(s + 1 < nstages ? 1 : 0);
        __syncthreads();
        int buf = s & 1;
        #pragma unroll
        for (int kk = 0; kk < BK; kk += 16) {
            #pragma unroll
            for (int mi = 0; mi < WM / 16; ++mi) {
                signed char* Ap0 = &As[buf][(warp_row + mi*16 + gid) * BK + kk + tg*4];
                signed char* Ap1 = &As[buf][(warp_row + mi*16 + gid + 8) * BK + kk + tg*4];
                int a0 = PK4(Ap0, 1), a1 = PK4(Ap1, 1);
                #pragma unroll
                for (int ni = 0; ni < WN / 8; ++ni) {
                    // B fragment: col=warp_col+ni*8+gid, k=kk+tg*4..+3 — now CONTIGUOUS (stride 1).
                    signed char* Bp = &Bs[buf][(warp_col + ni*8 + gid) * BK + kk + tg*4];
                    int b0 = PK4(Bp, 1);
                    asm volatile("mma.sync.aligned.m16n8k16.row.col.s32.s8.s8.s32 {%0,%1,%2,%3},{%4,%5},{%6},{%0,%1,%2,%3};"
                        : "+r"(c[mi][ni][0]),"+r"(c[mi][ni][1]),"+r"(c[mi][ni][2]),"+r"(c[mi][ni][3])
                        : "r"(a0),"r"(a1),"r"(b0));
                }
            }
        }
        __syncthreads();
    }
    #pragma unroll
    for (int mi = 0; mi < WM / 16; ++mi)
        #pragma unroll
        for (int ni = 0; ni < WN / 8; ++ni) {
            int rb = block_row + warp_row + mi*16, cb = block_col + warp_col + ni*8;
            C[(rb+gid)*N + cb+tg*2]     = c[mi][ni][0];
            C[(rb+gid)*N + cb+tg*2+1]   = c[mi][ni][1];
            C[(rb+gid+8)*N + cb+tg*2]   = c[mi][ni][2];
            C[(rb+gid+8)*N + cb+tg*2+1] = c[mi][ni][3];
        }
}
"#
}

pub fn emit_mma_tn_colb_program(m: usize, k: usize, n: usize) -> String {
    format!(
        r#"{kernel}
#include <cstdio>
#include <cstdlib>
int main() {{ const int M={m},K={k},N={n};
    signed char *hA=(signed char*)malloc((size_t)M*K),*hB=(signed char*)malloc((size_t)K*N);
    for(long i=0;i<(long)M*K;++i) hA[i]=(signed char)(((i*7+3)%251)-125);
    // B stored COL-MAJOR: hB[n*K + k] holds B[k][n]; fill so oracle C[i][j]=sum_k A[i][k]*B[k][j].
    for(long n=0;n<N;++n) for(long kk=0;kk<K;++kk){{ long bkn=(long)kk*N+n; hB[n*K+kk]=(signed char)(((bkn*5+1)%251)-125); }}
    signed char *dA,*dB; int *dC,*hC=(int*)malloc((size_t)M*N*4);
    cudaMalloc(&dA,(size_t)M*K);cudaMalloc(&dB,(size_t)K*N);cudaMalloc(&dC,(size_t)M*N*4);
    cudaMemcpy(dA,hA,(size_t)M*K,cudaMemcpyHostToDevice);cudaMemcpy(dB,hB,(size_t)K*N,cudaMemcpyHostToDevice);
    dim3 grid(N/64,M/128); gemm_mma_tn_colb<<<grid,128>>>(dA,dB,dC,M,N,K); cudaDeviceSynchronize();
    cudaMemcpy(hC,dC,(size_t)M*N*4,cudaMemcpyDeviceToHost);
    for(long i=0;i<(long)M*N;++i) printf("%d\n",hC[i]); return 0; }}
"#,
        kernel = mma_tn_colb_kernel(),
        m = m,
        k = k,
        n = n
    )
}

pub fn emit_mma_tn_colb_bench(m: usize, k: usize, n: usize) -> String {
    format!(
        r#"{kernel}
#include <cstdio>
#include <cstdlib>
int main() {{ const int M={m},K={k},N={n};
    signed char *dA,*dB; int *dC;
    cudaMalloc(&dA,(size_t)M*K);cudaMalloc(&dB,(size_t)K*N);cudaMalloc(&dC,(size_t)M*N*4);
    cudaMemset(dA,1,(size_t)M*K);cudaMemset(dB,1,(size_t)K*N);
    dim3 grid(N/64,M/128);
    gemm_mma_tn_colb<<<grid,128>>>(dA,dB,dC,M,N,K);
    cudaError_t err=cudaDeviceSynchronize(); if(err){{printf("LAUNCH_ERROR %s\n",cudaGetErrorString(err));return 1;}}
    int *hC=(int*)malloc((size_t)M*N*4); cudaMemcpy(hC,dC,(size_t)M*N*4,cudaMemcpyDeviceToHost);
    long step=((long)M*N)/4096+1; for(long i=0;i<(long)M*N;i+=step) if(hC[i]!=K){{printf("INCORRECT at %ld: %d!=%d\n",i,hC[i],K);return 1;}}
    cudaEvent_t s,e; cudaEventCreate(&s); cudaEventCreate(&e); int IT=30; float total=0;
    for(int it=0;it<IT;it++){{ cudaEventRecord(s); gemm_mma_tn_colb<<<grid,128>>>(dA,dB,dC,M,N,K); cudaEventRecord(e); cudaEventSynchronize(e); float ms=0; cudaEventElapsedTime(&ms,s,e); total+=ms; }}
    printf("GOPS %.1f\n", 2.0*M*N*K/((total/IT)/1e3)/1e9); return 0; }}
"#,
        kernel = mma_tn_colb_kernel(),
        m = m,
        k = k,
        n = n
    )
}

// ─────────────────────── rung 8: ldmatrix + mma.sync.s8 TN — the EARNED cuBLAS-class fragment feed ─────────
// The manual per-thread fragment loads were the wall. `ldmatrix.sync` loads a whole warp's mma fragment
// cooperatively in ONE instruction, in the exact register layout mma.sync wants — this is how cuBLAS feeds
// the tensor cores. A (row-major) via ldmatrix.x2 → {a0,a1}; B (col-major, K-contiguous) via ldmatrix.x1 →
// {b0}. The ldmatrix output distribution provably equals the fragment map we already verified by hand. Both
// operands K-contiguous (cuBLAS's TN layout), cp.async double-buffered. This is the earned cuBLAS-class kernel.
fn mma_tn_ldmatrix_kernel() -> &'static str {
    r#"#include <cuda_pipeline.h>
#define BM 128
#define BN 64
#define BK 32
#define WARPS_M 2
#define WARPS_N 2
#define WM (BM / WARPS_M)
#define WN (BN / WARPS_N)
#define NTHREAD (WARPS_M * WARPS_N * 32)
// C(MxN)=A(MxK,row-major).B(KxN, stored COL-MAJOR: Bcol[n*K+k]). ldmatrix feeds mma.sync.aligned.m16n8k16.s8.
extern "C" __global__ void gemm_mma_tn_ldm(const signed char* A, const signed char* Bcol, int* C, int M, int N, int K) {
    __shared__ signed char As[2][BM * BK];   // [m][k]
    __shared__ signed char Bs[2][BN * BK];   // [n][k]  (col-major tile)
    const int tid = threadIdx.x, warpId = tid / 32, lane = tid % 32;
    const int gid = lane / 4, tg = lane % 4;
    const int warp_row = (warpId / WARPS_N) * WM, warp_col = (warpId % WARPS_N) * WN;
    const int block_row = blockIdx.y * BM, block_col = blockIdx.x * BN;
    const int nstages = K / BK;

    int c[WM / 16][WN / 8][4];
    #pragma unroll
    for (int mi = 0; mi < WM / 16; ++mi)
        #pragma unroll
        for (int ni = 0; ni < WN / 8; ++ni) { c[mi][ni][0]=c[mi][ni][1]=c[mi][ni][2]=c[mi][ni][3]=0; }

    #define LOAD_STAGE(buf, st) do { int _k0 = (st) * BK; \
        for (int idx = tid; idx < (BM * BK) / 16; idx += NTHREAD) { int r=(idx*16)/BK, cc=(idx*16)%BK; \
            __pipeline_memcpy_async(&As[buf][r*BK+cc], &A[(block_row+r)*K + _k0+cc], 16); } \
        for (int idx = tid; idx < (BN * BK) / 16; idx += NTHREAD) { int r=(idx*16)/BK, cc=(idx*16)%BK; \
            __pipeline_memcpy_async(&Bs[buf][r*BK+cc], &Bcol[(block_col+r)*K + _k0+cc], 16); } \
        __pipeline_commit(); } while (0)

    LOAD_STAGE(0, 0);
    for (int s = 0; s < nstages; ++s) {
        if (s + 1 < nstages) LOAD_STAGE((s + 1) & 1, s + 1);
        __pipeline_wait_prior(s + 1 < nstages ? 1 : 0);
        __syncthreads();
        int buf = s & 1;
        #pragma unroll
        for (int kk = 0; kk < BK; kk += 16) {
            int a[WM / 16][2], b[WN / 8];
            #pragma unroll
            for (int mi = 0; mi < WM / 16; ++mi) {
                unsigned aAddr = (unsigned)__cvta_generic_to_shared(&As[buf][(warp_row + mi*16 + (lane & 15)) * BK + kk]);
                asm volatile("ldmatrix.sync.aligned.m8n8.x2.shared.b16 {%0,%1},[%2];" : "=r"(a[mi][0]), "=r"(a[mi][1]) : "r"(aAddr));
            }
            #pragma unroll
            for (int ni = 0; ni < WN / 8; ++ni) {
                unsigned bAddr = (unsigned)__cvta_generic_to_shared(&Bs[buf][(warp_col + ni*8 + (lane & 7)) * BK + kk]);
                asm volatile("ldmatrix.sync.aligned.m8n8.x1.shared.b16 {%0},[%1];" : "=r"(b[ni]) : "r"(bAddr));
            }
            #pragma unroll
            for (int mi = 0; mi < WM / 16; ++mi)
                #pragma unroll
                for (int ni = 0; ni < WN / 8; ++ni)
                    asm volatile("mma.sync.aligned.m16n8k16.row.col.s32.s8.s8.s32 {%0,%1,%2,%3},{%4,%5},{%6},{%0,%1,%2,%3};"
                        : "+r"(c[mi][ni][0]),"+r"(c[mi][ni][1]),"+r"(c[mi][ni][2]),"+r"(c[mi][ni][3])
                        : "r"(a[mi][0]),"r"(a[mi][1]),"r"(b[ni]));
        }
        __syncthreads();
    }
    #pragma unroll
    for (int mi = 0; mi < WM / 16; ++mi)
        #pragma unroll
        for (int ni = 0; ni < WN / 8; ++ni) {
            int rb = block_row + warp_row + mi*16, cb = block_col + warp_col + ni*8;
            C[(rb+gid)*N + cb+tg*2]     = c[mi][ni][0];
            C[(rb+gid)*N + cb+tg*2+1]   = c[mi][ni][1];
            C[(rb+gid+8)*N + cb+tg*2]   = c[mi][ni][2];
            C[(rb+gid+8)*N + cb+tg*2+1] = c[mi][ni][3];
        }
}
"#
}

pub fn emit_mma_tn_ldmatrix_program(m: usize, k: usize, n: usize) -> String {
    format!(
        r#"{kernel}
#include <cstdio>
#include <cstdlib>
int main() {{ const int M={m},K={k},N={n};
    signed char *hA=(signed char*)malloc((size_t)M*K),*hB=(signed char*)malloc((size_t)K*N);
    for(long i=0;i<(long)M*K;++i) hA[i]=(signed char)(((i*7+3)%251)-125);
    for(long n=0;n<N;++n) for(long kk=0;kk<K;++kk){{ long bkn=(long)kk*N+n; hB[n*K+kk]=(signed char)(((bkn*5+1)%251)-125); }}
    signed char *dA,*dB; int *dC,*hC=(int*)malloc((size_t)M*N*4);
    cudaMalloc(&dA,(size_t)M*K);cudaMalloc(&dB,(size_t)K*N);cudaMalloc(&dC,(size_t)M*N*4);
    cudaMemcpy(dA,hA,(size_t)M*K,cudaMemcpyHostToDevice);cudaMemcpy(dB,hB,(size_t)K*N,cudaMemcpyHostToDevice);
    dim3 grid(N/64,M/128); gemm_mma_tn_ldm<<<grid,128>>>(dA,dB,dC,M,N,K);
    cudaError_t e=cudaDeviceSynchronize(); if(e){{printf("ERR %s\n",cudaGetErrorString(e));return 1;}}
    cudaMemcpy(hC,dC,(size_t)M*N*4,cudaMemcpyDeviceToHost);
    for(long i=0;i<(long)M*N;++i) printf("%d\n",hC[i]); return 0; }}
"#,
        kernel = mma_tn_ldmatrix_kernel(),
        m = m,
        k = k,
        n = n
    )
}

pub fn emit_mma_tn_ldmatrix_bench(m: usize, k: usize, n: usize) -> String {
    format!(
        r#"{kernel}
#include <cstdio>
#include <cstdlib>
int main() {{ const int M={m},K={k},N={n};
    signed char *dA,*dB; int *dC;
    cudaMalloc(&dA,(size_t)M*K);cudaMalloc(&dB,(size_t)K*N);cudaMalloc(&dC,(size_t)M*N*4);
    cudaMemset(dA,1,(size_t)M*K);cudaMemset(dB,1,(size_t)K*N);
    dim3 grid(N/64,M/128);
    gemm_mma_tn_ldm<<<grid,128>>>(dA,dB,dC,M,N,K);
    cudaError_t err=cudaDeviceSynchronize(); if(err){{printf("LAUNCH_ERROR %s\n",cudaGetErrorString(err));return 1;}}
    int *hC=(int*)malloc((size_t)M*N*4); cudaMemcpy(hC,dC,(size_t)M*N*4,cudaMemcpyDeviceToHost);
    long step=((long)M*N)/4096+1; for(long i=0;i<(long)M*N;i+=step) if(hC[i]!=K){{printf("INCORRECT at %ld: %d!=%d\n",i,hC[i],K);return 1;}}
    cudaEvent_t s,e; cudaEventCreate(&s); cudaEventCreate(&e); int IT=30; float total=0;
    for(int it=0;it<IT;it++){{ cudaEventRecord(s); gemm_mma_tn_ldm<<<grid,128>>>(dA,dB,dC,M,N,K); cudaEventRecord(e); cudaEventSynchronize(e); float ms=0; cudaEventElapsedTime(&ms,s,e); total+=ms; }}
    printf("GOPS %.1f\n", 2.0*M*N*K/((total/IT)/1e3)/1e9); return 0; }}
"#,
        kernel = mma_tn_ldmatrix_kernel(),
        m = m,
        k = k,
        n = n
    )
}

// ─────────────────────── rung 8b: parameterized ldmatrix kernel (config sweep toward cuBLAS-TN) ────────────
/// Emit the ldmatrix+mma.sync.s8 TN kernel for a specific config so the frontier can sweep block/warp/stage
/// shapes on-device. `stages` shared buffers (cp.async N-stage). WN must be a multiple of 8, WM of 16.
pub fn emit_mma_tn_ldm_cfg_bench(
    bm: usize,
    bn: usize,
    bk: usize,
    warps_m: usize,
    warps_n: usize,
    stages: usize,
    m: usize,
    k: usize,
    n: usize,
) -> String {
    let defs = format!(
        "#define BM {bm}\n#define BN {bn}\n#define BK {bk}\n#define WARPS_M {warps_m}\n#define WARPS_N {warps_n}\n#define STAGES {stages}\n"
    );
    let kernel = format!("{defs}{}", MMA_TN_LDM_CFG_BODY);
    format!(
        r#"{kernel}
#include <cstdio>
#include <cstdlib>
int main() {{ const int M={m},K={k},N={n};
    signed char *dA,*dB; int *dC;
    cudaMalloc(&dA,(size_t)M*K);cudaMalloc(&dB,(size_t)K*N);cudaMalloc(&dC,(size_t)M*N*4);
    cudaMemset(dA,1,(size_t)M*K);cudaMemset(dB,1,(size_t)K*N);
    dim3 grid(N/{bn}, M/{bm});
    gemm_mma_tn_ldm<<<grid,{threads}>>>(dA,dB,dC,M,N,K);
    cudaError_t err=cudaDeviceSynchronize(); if(err){{printf("LAUNCH_ERROR %s\n",cudaGetErrorString(err));return 1;}}
    int *hC=(int*)malloc((size_t)M*N*4); cudaMemcpy(hC,dC,(size_t)M*N*4,cudaMemcpyDeviceToHost);
    long step=((long)M*N)/4096+1; for(long i=0;i<(long)M*N;i+=step) if(hC[i]!=K){{printf("INCORRECT %ld: %d!=%d\n",i,hC[i],K);return 1;}}
    cudaEvent_t s,e; cudaEventCreate(&s); cudaEventCreate(&e); int IT=30; float total=0;
    for(int it=0;it<IT;it++){{ cudaEventRecord(s); gemm_mma_tn_ldm<<<grid,{threads}>>>(dA,dB,dC,M,N,K); cudaEventRecord(e); cudaEventSynchronize(e); float ms=0; cudaEventElapsedTime(&ms,s,e); total+=ms; }}
    printf("GOPS %.1f\n", 2.0*M*N*K/((total/IT)/1e3)/1e9); return 0; }}
"#,
        kernel = kernel,
        m = m,
        k = k,
        n = n,
        bm = bm,
        bn = bn,
        threads = warps_m * warps_n * 32
    )
}

const MMA_TN_LDM_CFG_BODY: &str = r#"#include <cuda_pipeline.h>
#define WM (BM / WARPS_M)
#define WN (BN / WARPS_N)
#define NTHREAD (WARPS_M * WARPS_N * 32)
extern "C" __global__ void gemm_mma_tn_ldm(const signed char* A, const signed char* Bcol, int* C, int M, int N, int K) {
    __shared__ signed char As[STAGES][BM * BK];
    __shared__ signed char Bs[STAGES][BN * BK];
    const int tid = threadIdx.x, warpId = tid / 32, lane = tid % 32;
    const int gid = lane / 4, tg = lane % 4;
    const int warp_row = (warpId / WARPS_N) * WM, warp_col = (warpId % WARPS_N) * WN;
    const int block_row = blockIdx.y * BM, block_col = blockIdx.x * BN;
    const int nstages = K / BK;
    int c[WM / 16][WN / 8][4];
    #pragma unroll
    for (int mi=0; mi<WM/16; ++mi)
      #pragma unroll
      for (int ni=0; ni<WN/8; ++ni) { c[mi][ni][0]=c[mi][ni][1]=c[mi][ni][2]=c[mi][ni][3]=0; }
    #define LOAD_STAGE(buf, st) do { int _k0=(st)*BK; \
        for (int idx=tid; idx<(BM*BK)/16; idx+=NTHREAD){ int r=(idx*16)/BK, cc=(idx*16)%BK; \
            __pipeline_memcpy_async(&As[buf][r*BK+cc], &A[(block_row+r)*K + _k0+cc], 16); } \
        for (int idx=tid; idx<(BN*BK)/16; idx+=NTHREAD){ int r=(idx*16)/BK, cc=(idx*16)%BK; \
            __pipeline_memcpy_async(&Bs[buf][r*BK+cc], &Bcol[(block_col+r)*K + _k0+cc], 16); } \
        __pipeline_commit(); } while(0)
    #pragma unroll
    for (int i=0;i<STAGES-1;++i){ if(i<nstages) LOAD_STAGE(i,i); else __pipeline_commit(); }
    for (int s=0; s<nstages; ++s) {
        __pipeline_wait_prior(STAGES-2);
        __syncthreads();
        int buf = s % STAGES;
        #pragma unroll
        for (int kk=0; kk<BK; kk+=16) {
            int a[WM/16][2], b[WN/8];
            #pragma unroll
            for (int mi=0; mi<WM/16; ++mi){
                unsigned aA=(unsigned)__cvta_generic_to_shared(&As[buf][(warp_row+mi*16+(lane&15))*BK+kk]);
                asm volatile("ldmatrix.sync.aligned.m8n8.x2.shared.b16 {%0,%1},[%2];":"=r"(a[mi][0]),"=r"(a[mi][1]):"r"(aA)); }
            #pragma unroll
            for (int ni=0; ni<WN/8; ++ni){
                unsigned bA=(unsigned)__cvta_generic_to_shared(&Bs[buf][(warp_col+ni*8+(lane&7))*BK+kk]);
                asm volatile("ldmatrix.sync.aligned.m8n8.x1.shared.b16 {%0},[%1];":"=r"(b[ni]):"r"(bA)); }
            #pragma unroll
            for (int mi=0; mi<WM/16; ++mi)
              #pragma unroll
              for (int ni=0; ni<WN/8; ++ni)
                asm volatile("mma.sync.aligned.m16n8k16.row.col.s32.s8.s8.s32 {%0,%1,%2,%3},{%4,%5},{%6},{%0,%1,%2,%3};"
                  :"+r"(c[mi][ni][0]),"+r"(c[mi][ni][1]),"+r"(c[mi][ni][2]),"+r"(c[mi][ni][3])
                  :"r"(a[mi][0]),"r"(a[mi][1]),"r"(b[ni]));
        }
        __syncthreads();
        int nxt=s+STAGES-1; if(nxt<nstages) LOAD_STAGE(nxt%STAGES, nxt); else __pipeline_commit();
    }
    #pragma unroll
    for (int mi=0; mi<WM/16; ++mi)
      #pragma unroll
      for (int ni=0; ni<WN/8; ++ni) {
        int rb=block_row+warp_row+mi*16, cb=block_col+warp_col+ni*8;
        C[(rb+gid)*N+cb+tg*2]=c[mi][ni][0]; C[(rb+gid)*N+cb+tg*2+1]=c[mi][ni][1];
        C[(rb+gid+8)*N+cb+tg*2]=c[mi][ni][2]; C[(rb+gid+8)*N+cb+tg*2+1]=c[mi][ni][3];
      }
}
"#;

// ─────────────────────── rung 9: SWIZZLED ldmatrix TN — the bank-conflict-free tensor-core feed ────────────
// ldmatrix has each of 8 threads read a 16-byte row; with row stride BK=32 bytes, rows 0&4, 1&5, 2&6, 3&7 hit
// the same 32-bank set (2-way conflict) — the last thing serializing the tensor-core feed. The swizzle is pure
// math: give each shared row a stride SBK = BK + PAD so 8 consecutive rows land in distinct bank-sets. With
// PAD=16 (SBK=48), consecutive rows differ by 48B = 12 banks: rows 0..7 -> banks 0,12,24,4,16,28,8,20, all
// distinct -> conflict-free ldmatrix. cp.async writes the padded layout; ldmatrix reads it. Same result, no
// conflicts. This is the earned cuBLAS-class feed.
pub fn emit_mma_tn_ldm_swizzle_bench(
    bm: usize,
    bn: usize,
    bk: usize,
    warps_m: usize,
    warps_n: usize,
    stages: usize,
    pad: usize,
    m: usize,
    k: usize,
    n: usize,
) -> String {
    let defs = format!(
        "#define BM {bm}\n#define BN {bn}\n#define BK {bk}\n#define WARPS_M {warps_m}\n#define WARPS_N {warps_n}\n#define STAGES {stages}\n#define PAD {pad}\n"
    );
    let kernel = format!("{defs}{}", MMA_TN_LDM_SWZ_BODY);
    format!(
        r#"{kernel}
#include <cstdio>
#include <cstdlib>
int main() {{ const int M={m},K={k},N={n};
    signed char *dA,*dB; int *dC;
    cudaMalloc(&dA,(size_t)M*K);cudaMalloc(&dB,(size_t)K*N);cudaMalloc(&dC,(size_t)M*N*4);
    cudaMemset(dA,1,(size_t)M*K);cudaMemset(dB,1,(size_t)K*N);
    dim3 grid(N/{bn}, M/{bm});
    gemm_mma_tn_ldm<<<grid,{threads}>>>(dA,dB,dC,M,N,K);
    cudaError_t err=cudaDeviceSynchronize(); if(err){{printf("LAUNCH_ERROR %s\n",cudaGetErrorString(err));return 1;}}
    int *hC=(int*)malloc((size_t)M*N*4); cudaMemcpy(hC,dC,(size_t)M*N*4,cudaMemcpyDeviceToHost);
    long step=((long)M*N)/4096+1; for(long i=0;i<(long)M*N;i+=step) if(hC[i]!=K){{printf("INCORRECT %ld: %d!=%d\n",i,hC[i],K);return 1;}}
    cudaEvent_t s,e; cudaEventCreate(&s); cudaEventCreate(&e); int IT=30; float total=0;
    for(int it=0;it<IT;it++){{ cudaEventRecord(s); gemm_mma_tn_ldm<<<grid,{threads}>>>(dA,dB,dC,M,N,K); cudaEventRecord(e); cudaEventSynchronize(e); float ms=0; cudaEventElapsedTime(&ms,s,e); total+=ms; }}
    printf("GOPS %.1f\n", 2.0*M*N*K/((total/IT)/1e3)/1e9); return 0; }}
"#,
        kernel = kernel,
        m = m,
        k = k,
        n = n,
        bm = bm,
        bn = bn,
        threads = warps_m * warps_n * 32
    )
}

/// The swizzled program (varied-fill oracle correctness).
pub fn emit_mma_tn_ldm_swizzle_program(
    bm: usize,
    bn: usize,
    bk: usize,
    warps_m: usize,
    warps_n: usize,
    stages: usize,
    pad: usize,
    m: usize,
    k: usize,
    n: usize,
) -> String {
    let defs = format!(
        "#define BM {bm}\n#define BN {bn}\n#define BK {bk}\n#define WARPS_M {warps_m}\n#define WARPS_N {warps_n}\n#define STAGES {stages}\n#define PAD {pad}\n"
    );
    let kernel = format!("{defs}{}", MMA_TN_LDM_SWZ_BODY);
    format!(
        r#"{kernel}
#include <cstdio>
#include <cstdlib>
int main() {{ const int M={m},K={k},N={n};
    signed char *hA=(signed char*)malloc((size_t)M*K),*hB=(signed char*)malloc((size_t)K*N);
    for(long i=0;i<(long)M*K;++i) hA[i]=(signed char)(((i*7+3)%251)-125);
    for(long nn=0;nn<N;++nn) for(long kk=0;kk<K;++kk){{ long bkn=(long)kk*N+nn; hB[nn*K+kk]=(signed char)(((bkn*5+1)%251)-125); }}
    signed char *dA,*dB; int *dC,*hC=(int*)malloc((size_t)M*N*4);
    cudaMalloc(&dA,(size_t)M*K);cudaMalloc(&dB,(size_t)K*N);cudaMalloc(&dC,(size_t)M*N*4);
    cudaMemcpy(dA,hA,(size_t)M*K,cudaMemcpyHostToDevice);cudaMemcpy(dB,hB,(size_t)K*N,cudaMemcpyHostToDevice);
    dim3 grid(N/{bn},M/{bm}); gemm_mma_tn_ldm<<<grid,{threads}>>>(dA,dB,dC,M,N,K);
    cudaError_t e=cudaDeviceSynchronize(); if(e){{printf("ERR %s\n",cudaGetErrorString(e));return 1;}}
    cudaMemcpy(hC,dC,(size_t)M*N*4,cudaMemcpyDeviceToHost);
    for(long i=0;i<(long)M*N;++i) printf("%d\n",hC[i]); return 0; }}
"#,
        kernel = kernel,
        m = m,
        k = k,
        n = n,
        bm = bm,
        bn = bn,
        threads = warps_m * warps_n * 32
    )
}

const MMA_TN_LDM_SWZ_BODY: &str = r#"#include <cuda_pipeline.h>
#define WM (BM / WARPS_M)
#define WN (BN / WARPS_N)
#define NTHREAD (WARPS_M * WARPS_N * 32)
#define SBK (BK + PAD)
extern "C" __global__ void gemm_mma_tn_ldm(const signed char* A, const signed char* Bcol, int* C, int M, int N, int K) {
    __shared__ signed char As[STAGES][BM * SBK];
    __shared__ signed char Bs[STAGES][BN * SBK];
    const int tid = threadIdx.x, warpId = tid / 32, lane = tid % 32;
    const int gid = lane / 4, tg = lane % 4;
    const int warp_row = (warpId / WARPS_N) * WM, warp_col = (warpId % WARPS_N) * WN;
    const int block_row = blockIdx.y * BM, block_col = blockIdx.x * BN;
    const int nstages = K / BK;
    int c[WM / 16][WN / 8][4];
    #pragma unroll
    for (int mi=0; mi<WM/16; ++mi)
      #pragma unroll
      for (int ni=0; ni<WN/8; ++ni) { c[mi][ni][0]=c[mi][ni][1]=c[mi][ni][2]=c[mi][ni][3]=0; }
    #define LOAD_STAGE(buf, st) do { int _k0=(st)*BK; \
        for (int idx=tid; idx<(BM*BK)/16; idx+=NTHREAD){ int r=(idx*16)/BK, cc=(idx*16)%BK; \
            __pipeline_memcpy_async(&As[buf][r*SBK+cc], &A[(block_row+r)*K + _k0+cc], 16); } \
        for (int idx=tid; idx<(BN*BK)/16; idx+=NTHREAD){ int r=(idx*16)/BK, cc=(idx*16)%BK; \
            __pipeline_memcpy_async(&Bs[buf][r*SBK+cc], &Bcol[(block_col+r)*K + _k0+cc], 16); } \
        __pipeline_commit(); } while(0)
    #pragma unroll
    for (int i=0;i<STAGES-1;++i){ if(i<nstages) LOAD_STAGE(i,i); else __pipeline_commit(); }
    for (int s=0; s<nstages; ++s) {
        __pipeline_wait_prior(STAGES-2);
        __syncthreads();
        int buf = s % STAGES;
        #pragma unroll
        for (int kk=0; kk<BK; kk+=16) {
            int a[WM/16][2], b[WN/8];
            #pragma unroll
            for (int mi=0; mi<WM/16; ++mi){
                unsigned aA=(unsigned)__cvta_generic_to_shared(&As[buf][(warp_row+mi*16+(lane&15))*SBK+kk]);
                asm volatile("ldmatrix.sync.aligned.m8n8.x2.shared.b16 {%0,%1},[%2];":"=r"(a[mi][0]),"=r"(a[mi][1]):"r"(aA)); }
            #pragma unroll
            for (int ni=0; ni<WN/8; ++ni){
                unsigned bA=(unsigned)__cvta_generic_to_shared(&Bs[buf][(warp_col+ni*8+(lane&7))*SBK+kk]);
                asm volatile("ldmatrix.sync.aligned.m8n8.x1.shared.b16 {%0},[%1];":"=r"(b[ni]):"r"(bA)); }
            #pragma unroll
            for (int mi=0; mi<WM/16; ++mi)
              #pragma unroll
              for (int ni=0; ni<WN/8; ++ni)
                asm volatile("mma.sync.aligned.m16n8k16.row.col.s32.s8.s8.s32 {%0,%1,%2,%3},{%4,%5},{%6},{%0,%1,%2,%3};"
                  :"+r"(c[mi][ni][0]),"+r"(c[mi][ni][1]),"+r"(c[mi][ni][2]),"+r"(c[mi][ni][3])
                  :"r"(a[mi][0]),"r"(a[mi][1]),"r"(b[ni]));
        }
        __syncthreads();
        int nxt=s+STAGES-1; if(nxt<nstages) LOAD_STAGE(nxt%STAGES, nxt); else __pipeline_commit();
    }
    #pragma unroll
    for (int mi=0; mi<WM/16; ++mi)
      #pragma unroll
      for (int ni=0; ni<WN/8; ++ni) {
        int rb=block_row+warp_row+mi*16, cb=block_col+warp_col+ni*8;
        C[(rb+gid)*N+cb+tg*2]=c[mi][ni][0]; C[(rb+gid)*N+cb+tg*2+1]=c[mi][ni][1];
        C[(rb+gid+8)*N+cb+tg*2]=c[mi][ni][2]; C[(rb+gid+8)*N+cb+tg*2+1]=c[mi][ni][3];
      }
}
"#;

// ─────────────────────── rung 10: m16n8k32 mma.sync.s8 — 2x MACs/instruction (cuBLAS's mma shape) ──────────
// m16n8k16 issues one mma per 16 of K; m16n8k32 does 32 of K per instruction = HALF the mma + ldmatrix issue
// overhead for the same work. This is the int8 tensor-core op cuBLAS actually uses. A (16x32) via ldmatrix.x4
// -> {a0,a1,a2,a3}; col-major B (32x8) via ldmatrix.x2 -> {b0,b1}. Fragment map: a{0,2}=rows gid (k 0-3, 16-19),
// a{1,3}=rows gid+8; b0/b1 = k first/second 16-half. cp.async pipelined. Our OWN kernel, earned.
pub fn emit_mma_tn_k32_bench(
    bm: usize,
    bn: usize,
    warps_m: usize,
    warps_n: usize,
    stages: usize,
    m: usize,
    k: usize,
    n: usize,
) -> String {
    let defs = format!("#define BM {bm}\n#define BN {bn}\n#define BK 32\n#define WARPS_M {warps_m}\n#define WARPS_N {warps_n}\n#define STAGES {stages}\n");
    let kernel = format!("{defs}{}", MMA_TN_K32_BODY);
    format!(
        r#"{kernel}
#include <cstdio>
#include <cstdlib>
int main() {{ const int M={m},K={k},N={n};
    signed char *dA,*dB; int *dC;
    cudaMalloc(&dA,(size_t)M*K);cudaMalloc(&dB,(size_t)K*N);cudaMalloc(&dC,(size_t)M*N*4);
    cudaMemset(dA,1,(size_t)M*K);cudaMemset(dB,1,(size_t)K*N);
    dim3 grid(N/{bn}, M/{bm});
    gemm_mma_tn_k32<<<grid,{threads}>>>(dA,dB,dC,M,N,K);
    cudaError_t err=cudaDeviceSynchronize(); if(err){{printf("LAUNCH_ERROR %s\n",cudaGetErrorString(err));return 1;}}
    int *hC=(int*)malloc((size_t)M*N*4); cudaMemcpy(hC,dC,(size_t)M*N*4,cudaMemcpyDeviceToHost);
    long step=((long)M*N)/4096+1; for(long i=0;i<(long)M*N;i+=step) if(hC[i]!=K){{printf("INCORRECT %ld: %d!=%d\n",i,hC[i],K);return 1;}}
    cudaEvent_t s,e; cudaEventCreate(&s); cudaEventCreate(&e); int IT=30; float total=0;
    for(int it=0;it<IT;it++){{ cudaEventRecord(s); gemm_mma_tn_k32<<<grid,{threads}>>>(dA,dB,dC,M,N,K); cudaEventRecord(e); cudaEventSynchronize(e); float ms=0; cudaEventElapsedTime(&ms,s,e); total+=ms; }}
    printf("GOPS %.1f\n", 2.0*M*N*K/((total/IT)/1e3)/1e9); return 0; }}
"#,
        kernel = kernel,
        m = m,
        k = k,
        n = n,
        bm = bm,
        bn = bn,
        threads = warps_m * warps_n * 32
    )
}

pub fn emit_mma_tn_k32_program(
    bm: usize,
    bn: usize,
    warps_m: usize,
    warps_n: usize,
    stages: usize,
    m: usize,
    k: usize,
    n: usize,
) -> String {
    let defs = format!("#define BM {bm}\n#define BN {bn}\n#define BK 32\n#define WARPS_M {warps_m}\n#define WARPS_N {warps_n}\n#define STAGES {stages}\n");
    let kernel = format!("{defs}{}", MMA_TN_K32_BODY);
    format!(
        r#"{kernel}
#include <cstdio>
#include <cstdlib>
int main() {{ const int M={m},K={k},N={n};
    signed char *hA=(signed char*)malloc((size_t)M*K),*hB=(signed char*)malloc((size_t)K*N);
    for(long i=0;i<(long)M*K;++i) hA[i]=(signed char)(((i*7+3)%251)-125);
    for(long nn=0;nn<N;++nn) for(long kk=0;kk<K;++kk){{ long bkn=(long)kk*N+nn; hB[nn*K+kk]=(signed char)(((bkn*5+1)%251)-125); }}
    signed char *dA,*dB; int *dC,*hC=(int*)malloc((size_t)M*N*4);
    cudaMalloc(&dA,(size_t)M*K);cudaMalloc(&dB,(size_t)K*N);cudaMalloc(&dC,(size_t)M*N*4);
    cudaMemcpy(dA,hA,(size_t)M*K,cudaMemcpyHostToDevice);cudaMemcpy(dB,hB,(size_t)K*N,cudaMemcpyHostToDevice);
    dim3 grid(N/{bn},M/{bm}); gemm_mma_tn_k32<<<grid,{threads}>>>(dA,dB,dC,M,N,K);
    cudaError_t e=cudaDeviceSynchronize(); if(e){{printf("ERR %s\n",cudaGetErrorString(e));return 1;}}
    cudaMemcpy(hC,dC,(size_t)M*N*4,cudaMemcpyDeviceToHost);
    for(long i=0;i<(long)M*N;++i) printf("%d\n",hC[i]); return 0; }}
"#,
        kernel = kernel,
        m = m,
        k = k,
        n = n,
        bm = bm,
        bn = bn,
        threads = warps_m * warps_n * 32
    )
}

const MMA_TN_K32_BODY: &str = r#"#include <cuda_pipeline.h>
#define WM (BM / WARPS_M)
#define WN (BN / WARPS_N)
#define NTHREAD (WARPS_M * WARPS_N * 32)
extern "C" __global__ void gemm_mma_tn_k32(const signed char* A, const signed char* Bcol, int* C, int M, int N, int K) {
    __shared__ signed char As[STAGES][BM * BK];
    __shared__ signed char Bs[STAGES][BN * BK];
    const int tid = threadIdx.x, warpId = tid / 32, lane = tid % 32;
    const int gid = lane / 4, tg = lane % 4;
    const int warp_row = (warpId / WARPS_N) * WM, warp_col = (warpId % WARPS_N) * WN;
    const int block_row = blockIdx.y * BM, block_col = blockIdx.x * BN;
    const int nstages = K / BK;
    int c[WM / 16][WN / 8][4];
    #pragma unroll
    for (int mi=0; mi<WM/16; ++mi)
      #pragma unroll
      for (int ni=0; ni<WN/8; ++ni) { c[mi][ni][0]=c[mi][ni][1]=c[mi][ni][2]=c[mi][ni][3]=0; }
    #define LOAD_STAGE(buf, st) do { int _k0=(st)*BK; \
        for (int idx=tid; idx<(BM*BK)/16; idx+=NTHREAD){ int r=(idx*16)/BK, cc=(idx*16)%BK; \
            __pipeline_memcpy_async(&As[buf][r*BK+cc], &A[(block_row+r)*K + _k0+cc], 16); } \
        for (int idx=tid; idx<(BN*BK)/16; idx+=NTHREAD){ int r=(idx*16)/BK, cc=(idx*16)%BK; \
            __pipeline_memcpy_async(&Bs[buf][r*BK+cc], &Bcol[(block_col+r)*K + _k0+cc], 16); } \
        __pipeline_commit(); } while(0)
    #pragma unroll
    for (int i=0;i<STAGES-1;++i){ if(i<nstages) LOAD_STAGE(i,i); else __pipeline_commit(); }
    for (int s=0; s<nstages; ++s) {
        __pipeline_wait_prior(STAGES-2);
        __syncthreads();
        int buf = s % STAGES;
        int a[WM/16][4], b[WN/8][2];
        #pragma unroll
        for (int mi=0; mi<WM/16; ++mi){
            int mtx=lane/8, row=lane%8;
            unsigned aA=(unsigned)__cvta_generic_to_shared(&As[buf][(warp_row+mi*16 + (mtx&1)*8 + row)*BK + (mtx>>1)*16]);
            asm volatile("ldmatrix.sync.aligned.m8n8.x4.shared.b16 {%0,%1,%2,%3},[%4];":"=r"(a[mi][0]),"=r"(a[mi][1]),"=r"(a[mi][2]),"=r"(a[mi][3]):"r"(aA)); }
        #pragma unroll
        for (int ni=0; ni<WN/8; ++ni){
            int mtx=(lane>>3)&1, row=lane&7;
            unsigned bA=(unsigned)__cvta_generic_to_shared(&Bs[buf][(warp_col+ni*8 + row)*BK + mtx*16]);
            asm volatile("ldmatrix.sync.aligned.m8n8.x2.shared.b16 {%0,%1},[%2];":"=r"(b[ni][0]),"=r"(b[ni][1]):"r"(bA)); }
        #pragma unroll
        for (int mi=0; mi<WM/16; ++mi)
          #pragma unroll
          for (int ni=0; ni<WN/8; ++ni)
            asm volatile("mma.sync.aligned.m16n8k32.row.col.s32.s8.s8.s32 {%0,%1,%2,%3},{%4,%5,%6,%7},{%8,%9},{%0,%1,%2,%3};"
              :"+r"(c[mi][ni][0]),"+r"(c[mi][ni][1]),"+r"(c[mi][ni][2]),"+r"(c[mi][ni][3])
              :"r"(a[mi][0]),"r"(a[mi][1]),"r"(a[mi][2]),"r"(a[mi][3]),"r"(b[ni][0]),"r"(b[ni][1]));
        __syncthreads();
        int nxt=s+STAGES-1; if(nxt<nstages) LOAD_STAGE(nxt%STAGES, nxt); else __pipeline_commit();
    }
    #pragma unroll
    for (int mi=0; mi<WM/16; ++mi)
      #pragma unroll
      for (int ni=0; ni<WN/8; ++ni) {
        int rb=block_row+warp_row+mi*16, cb=block_col+warp_col+ni*8;
        C[(rb+gid)*N+cb+tg*2]=c[mi][ni][0]; C[(rb+gid)*N+cb+tg*2+1]=c[mi][ni][1];
        C[(rb+gid+8)*N+cb+tg*2]=c[mi][ni][2]; C[(rb+gid+8)*N+cb+tg*2+1]=c[mi][ni][3];
      }
}
"#;

// ─────────────────────── rung 11: XOR-swizzled k32 ldmatrix — bank-conflict-free, ZERO extra memory ────────
// The 8 rows ldmatrix reads collide 2-WAY (row r vs r+4 hit the same 4-bank group). A 1-bit XOR — flip the
// 16-byte half for rows 4-7 of each 8-group — resolves a 2-way collision exactly: bank starts become
// 0,8,16,24,4,12,20,28 (all distinct), with NO padding/extra shared memory. Applied identically at cp.async
// store and ldmatrix load, so correctness is preserved (proven vs the col-major oracle). This is the real
// CUTLASS-style swizzle. SW(row) = ((row>>2)&1)*16.
pub fn emit_mma_tn_k32swz_bench(
    bm: usize,
    bn: usize,
    warps_m: usize,
    warps_n: usize,
    stages: usize,
    m: usize,
    k: usize,
    n: usize,
) -> String {
    let defs = format!("#define BM {bm}\n#define BN {bn}\n#define BK 32\n#define WARPS_M {warps_m}\n#define WARPS_N {warps_n}\n#define STAGES {stages}\n");
    let kernel = format!("{defs}{}", MMA_TN_K32SWZ_BODY);
    format!(
        r#"{kernel}
#include <cstdio>
#include <cstdlib>
int main() {{ const int M={m},K={k},N={n};
    signed char *dA,*dB; int *dC;
    cudaMalloc(&dA,(size_t)M*K);cudaMalloc(&dB,(size_t)K*N);cudaMalloc(&dC,(size_t)M*N*4);
    cudaMemset(dA,1,(size_t)M*K);cudaMemset(dB,1,(size_t)K*N);
    dim3 grid(N/{bn}, M/{bm});
    gemm_mma_tn_k32swz<<<grid,{threads}>>>(dA,dB,dC,M,N,K);
    cudaError_t err=cudaDeviceSynchronize(); if(err){{printf("LAUNCH_ERROR %s\n",cudaGetErrorString(err));return 1;}}
    int *hC=(int*)malloc((size_t)M*N*4); cudaMemcpy(hC,dC,(size_t)M*N*4,cudaMemcpyDeviceToHost);
    long step=((long)M*N)/4096+1; for(long i=0;i<(long)M*N;i+=step) if(hC[i]!=K){{printf("INCORRECT %ld: %d!=%d\n",i,hC[i],K);return 1;}}
    cudaEvent_t s,e; cudaEventCreate(&s); cudaEventCreate(&e); int IT=30; float total=0;
    for(int it=0;it<IT;it++){{ cudaEventRecord(s); gemm_mma_tn_k32swz<<<grid,{threads}>>>(dA,dB,dC,M,N,K); cudaEventRecord(e); cudaEventSynchronize(e); float ms=0; cudaEventElapsedTime(&ms,s,e); total+=ms; }}
    printf("GOPS %.1f\n", 2.0*M*N*K/((total/IT)/1e3)/1e9); return 0; }}
"#,
        kernel = kernel,
        m = m,
        k = k,
        n = n,
        bm = bm,
        bn = bn,
        threads = warps_m * warps_n * 32
    )
}

pub fn emit_mma_tn_k32swz_program(
    bm: usize,
    bn: usize,
    warps_m: usize,
    warps_n: usize,
    stages: usize,
    m: usize,
    k: usize,
    n: usize,
) -> String {
    let defs = format!("#define BM {bm}\n#define BN {bn}\n#define BK 32\n#define WARPS_M {warps_m}\n#define WARPS_N {warps_n}\n#define STAGES {stages}\n");
    let kernel = format!("{defs}{}", MMA_TN_K32SWZ_BODY);
    format!(
        r#"{kernel}
#include <cstdio>
#include <cstdlib>
int main() {{ const int M={m},K={k},N={n};
    signed char *hA=(signed char*)malloc((size_t)M*K),*hB=(signed char*)malloc((size_t)K*N);
    for(long i=0;i<(long)M*K;++i) hA[i]=(signed char)(((i*7+3)%251)-125);
    for(long nn=0;nn<N;++nn) for(long kk=0;kk<K;++kk){{ long bkn=(long)kk*N+nn; hB[nn*K+kk]=(signed char)(((bkn*5+1)%251)-125); }}
    signed char *dA,*dB; int *dC,*hC=(int*)malloc((size_t)M*N*4);
    cudaMalloc(&dA,(size_t)M*K);cudaMalloc(&dB,(size_t)K*N);cudaMalloc(&dC,(size_t)M*N*4);
    cudaMemcpy(dA,hA,(size_t)M*K,cudaMemcpyHostToDevice);cudaMemcpy(dB,hB,(size_t)K*N,cudaMemcpyHostToDevice);
    dim3 grid(N/{bn},M/{bm}); gemm_mma_tn_k32swz<<<grid,{threads}>>>(dA,dB,dC,M,N,K);
    cudaError_t e=cudaDeviceSynchronize(); if(e){{printf("ERR %s\n",cudaGetErrorString(e));return 1;}}
    cudaMemcpy(hC,dC,(size_t)M*N*4,cudaMemcpyDeviceToHost);
    for(long i=0;i<(long)M*N;++i) printf("%d\n",hC[i]); return 0; }}
"#,
        kernel = kernel,
        m = m,
        k = k,
        n = n,
        bm = bm,
        bn = bn,
        threads = warps_m * warps_n * 32
    )
}

const MMA_TN_K32SWZ_BODY: &str = r#"#include <cuda_pipeline.h>
#define WM (BM / WARPS_M)
#define WN (BN / WARPS_N)
#define NTHREAD (WARPS_M * WARPS_N * 32)
#define SW(row) ((((row)>>2)&1)*16)
extern "C" __global__ void gemm_mma_tn_k32swz(const signed char* A, const signed char* Bcol, int* C, int M, int N, int K) {
    __shared__ signed char As[STAGES][BM * BK];
    __shared__ signed char Bs[STAGES][BN * BK];
    const int tid = threadIdx.x, warpId = tid / 32, lane = tid % 32;
    const int gid = lane / 4, tg = lane % 4;
    const int warp_row = (warpId / WARPS_N) * WM, warp_col = (warpId % WARPS_N) * WN;
    const int block_row = blockIdx.y * BM, block_col = blockIdx.x * BN;
    const int nstages = K / BK;
    int c[WM / 16][WN / 8][4];
    #pragma unroll
    for (int mi=0; mi<WM/16; ++mi)
      #pragma unroll
      for (int ni=0; ni<WN/8; ++ni) { c[mi][ni][0]=c[mi][ni][1]=c[mi][ni][2]=c[mi][ni][3]=0; }
    // store with the SW swizzle: cc is the 16-byte chunk offset (0 or 16), XORed by SW(row).
    #define LOAD_STAGE(buf, st) do { int _k0=(st)*BK; \
        for (int idx=tid; idx<(BM*BK)/16; idx+=NTHREAD){ int r=(idx*16)/BK, cc=(idx*16)%BK; \
            __pipeline_memcpy_async(&As[buf][r*BK + (cc ^ SW(r))], &A[(block_row+r)*K + _k0+cc], 16); } \
        for (int idx=tid; idx<(BN*BK)/16; idx+=NTHREAD){ int r=(idx*16)/BK, cc=(idx*16)%BK; \
            __pipeline_memcpy_async(&Bs[buf][r*BK + (cc ^ SW(r))], &Bcol[(block_col+r)*K + _k0+cc], 16); } \
        __pipeline_commit(); } while(0)
    #pragma unroll
    for (int i=0;i<STAGES-1;++i){ if(i<nstages) LOAD_STAGE(i,i); else __pipeline_commit(); }
    for (int s=0; s<nstages; ++s) {
        __pipeline_wait_prior(STAGES-2);
        __syncthreads();
        int buf = s % STAGES;
        int a[WM/16][4], b[WN/8][2];
        #pragma unroll
        for (int mi=0; mi<WM/16; ++mi){
            int mtx=lane/8, row=lane%8; int shrow=warp_row+mi*16 + (mtx&1)*8 + row; int koff=(mtx>>1)*16;
            unsigned aA=(unsigned)__cvta_generic_to_shared(&As[buf][shrow*BK + (koff ^ SW(shrow))]);
            asm volatile("ldmatrix.sync.aligned.m8n8.x4.shared.b16 {%0,%1,%2,%3},[%4];":"=r"(a[mi][0]),"=r"(a[mi][1]),"=r"(a[mi][2]),"=r"(a[mi][3]):"r"(aA)); }
        #pragma unroll
        for (int ni=0; ni<WN/8; ++ni){
            int mtx=(lane>>3)&1, row=lane&7; int shrow=warp_col+ni*8 + row; int koff=mtx*16;
            unsigned bA=(unsigned)__cvta_generic_to_shared(&Bs[buf][shrow*BK + (koff ^ SW(shrow))]);
            asm volatile("ldmatrix.sync.aligned.m8n8.x2.shared.b16 {%0,%1},[%2];":"=r"(b[ni][0]),"=r"(b[ni][1]):"r"(bA)); }
        #pragma unroll
        for (int mi=0; mi<WM/16; ++mi)
          #pragma unroll
          for (int ni=0; ni<WN/8; ++ni)
            asm volatile("mma.sync.aligned.m16n8k32.row.col.s32.s8.s8.s32 {%0,%1,%2,%3},{%4,%5,%6,%7},{%8,%9},{%0,%1,%2,%3};"
              :"+r"(c[mi][ni][0]),"+r"(c[mi][ni][1]),"+r"(c[mi][ni][2]),"+r"(c[mi][ni][3])
              :"r"(a[mi][0]),"r"(a[mi][1]),"r"(a[mi][2]),"r"(a[mi][3]),"r"(b[ni][0]),"r"(b[ni][1]));
        __syncthreads();
        int nxt=s+STAGES-1; if(nxt<nstages) LOAD_STAGE(nxt%STAGES, nxt); else __pipeline_commit();
    }
    #pragma unroll
    for (int mi=0; mi<WM/16; ++mi)
      #pragma unroll
      for (int ni=0; ni<WN/8; ++ni) {
        int rb=block_row+warp_row+mi*16, cb=block_col+warp_col+ni*8;
        C[(rb+gid)*N+cb+tg*2]=c[mi][ni][0]; C[(rb+gid)*N+cb+tg*2+1]=c[mi][ni][1];
        C[(rb+gid+8)*N+cb+tg*2]=c[mi][ni][2]; C[(rb+gid+8)*N+cb+tg*2+1]=c[mi][ni][3];
      }
}
"#;

// ─────────────────────── rung 12: register double-buffered k32 swizzle — ldmatrix HIDDEN behind mma ─────────
// The last latency on the critical path is the shared->register ldmatrix feeding each mma. Double-buffer it in
// REGISTERS: prefetch the next K-stage's fragments (af[nxt],bf[nxt]) via ldmatrix, THEN issue the current
// stage's mma on af[cur]. The mma does not depend on the prefetch, so the hardware overlaps the ldmatrix memory
// latency behind the tensor-core math. This is the CUTLASS mainloop. Same XOR swizzle, cp.async underneath.
pub fn emit_mma_tn_k32db_bench(
    bm: usize,
    bn: usize,
    warps_m: usize,
    warps_n: usize,
    stages: usize,
    m: usize,
    k: usize,
    n: usize,
) -> String {
    let defs = format!("#define BM {bm}\n#define BN {bn}\n#define BK 32\n#define WARPS_M {warps_m}\n#define WARPS_N {warps_n}\n#define STAGES {stages}\n");
    let kernel = format!("{defs}{}", MMA_TN_K32DB_BODY);
    format!(
        r#"{kernel}
#include <cstdio>
#include <cstdlib>
int main() {{ const int M={m},K={k},N={n};
    signed char *dA,*dB; int *dC;
    cudaMalloc(&dA,(size_t)M*K);cudaMalloc(&dB,(size_t)K*N);cudaMalloc(&dC,(size_t)M*N*4);
    cudaMemset(dA,1,(size_t)M*K);cudaMemset(dB,1,(size_t)K*N);
    dim3 grid(N/{bn}, M/{bm});
    gemm_mma_tn_k32db<<<grid,{threads}>>>(dA,dB,dC,M,N,K);
    cudaError_t err=cudaDeviceSynchronize(); if(err){{printf("LAUNCH_ERROR %s\n",cudaGetErrorString(err));return 1;}}
    int *hC=(int*)malloc((size_t)M*N*4); cudaMemcpy(hC,dC,(size_t)M*N*4,cudaMemcpyDeviceToHost);
    long step=((long)M*N)/4096+1; for(long i=0;i<(long)M*N;i+=step) if(hC[i]!=K){{printf("INCORRECT %ld: %d!=%d\n",i,hC[i],K);return 1;}}
    cudaEvent_t s,e; cudaEventCreate(&s); cudaEventCreate(&e); int IT=30; float total=0;
    for(int it=0;it<IT;it++){{ cudaEventRecord(s); gemm_mma_tn_k32db<<<grid,{threads}>>>(dA,dB,dC,M,N,K); cudaEventRecord(e); cudaEventSynchronize(e); float ms=0; cudaEventElapsedTime(&ms,s,e); total+=ms; }}
    printf("GOPS %.1f\n", 2.0*M*N*K/((total/IT)/1e3)/1e9); return 0; }}
"#,
        kernel = kernel,
        m = m,
        k = k,
        n = n,
        bm = bm,
        bn = bn,
        threads = warps_m * warps_n * 32
    )
}

pub fn emit_mma_tn_k32db_program(
    bm: usize,
    bn: usize,
    warps_m: usize,
    warps_n: usize,
    stages: usize,
    m: usize,
    k: usize,
    n: usize,
) -> String {
    let defs = format!("#define BM {bm}\n#define BN {bn}\n#define BK 32\n#define WARPS_M {warps_m}\n#define WARPS_N {warps_n}\n#define STAGES {stages}\n");
    let kernel = format!("{defs}{}", MMA_TN_K32DB_BODY);
    format!(
        r#"{kernel}
#include <cstdio>
#include <cstdlib>
int main() {{ const int M={m},K={k},N={n};
    signed char *hA=(signed char*)malloc((size_t)M*K),*hB=(signed char*)malloc((size_t)K*N);
    for(long i=0;i<(long)M*K;++i) hA[i]=(signed char)(((i*7+3)%251)-125);
    for(long nn=0;nn<N;++nn) for(long kk=0;kk<K;++kk){{ long bkn=(long)kk*N+nn; hB[nn*K+kk]=(signed char)(((bkn*5+1)%251)-125); }}
    signed char *dA,*dB; int *dC,*hC=(int*)malloc((size_t)M*N*4);
    cudaMalloc(&dA,(size_t)M*K);cudaMalloc(&dB,(size_t)K*N);cudaMalloc(&dC,(size_t)M*N*4);
    cudaMemcpy(dA,hA,(size_t)M*K,cudaMemcpyHostToDevice);cudaMemcpy(dB,hB,(size_t)K*N,cudaMemcpyHostToDevice);
    dim3 grid(N/{bn},M/{bm}); gemm_mma_tn_k32db<<<grid,{threads}>>>(dA,dB,dC,M,N,K);
    cudaError_t e=cudaDeviceSynchronize(); if(e){{printf("ERR %s\n",cudaGetErrorString(e));return 1;}}
    cudaMemcpy(hC,dC,(size_t)M*N*4,cudaMemcpyDeviceToHost);
    for(long i=0;i<(long)M*N;++i) printf("%d\n",hC[i]); return 0; }}
"#,
        kernel = kernel,
        m = m,
        k = k,
        n = n,
        bm = bm,
        bn = bn,
        threads = warps_m * warps_n * 32
    )
}

const MMA_TN_K32DB_BODY: &str = r#"#include <cuda_pipeline.h>
#define WM (BM / WARPS_M)
#define WN (BN / WARPS_N)
#define NTHREAD (WARPS_M * WARPS_N * 32)
#define SW(row) ((((row)>>2)&1)*16)
#define LDA(buf, dst) do { \
    for (int mi=0; mi<WM/16; ++mi){ int mtx=lane/8,row=lane%8; int shrow=warp_row+mi*16+(mtx&1)*8+row; int koff=(mtx>>1)*16; \
        unsigned aA=(unsigned)__cvta_generic_to_shared(&As[buf][shrow*BK + (koff ^ SW(shrow))]); \
        asm volatile("ldmatrix.sync.aligned.m8n8.x4.shared.b16 {%0,%1,%2,%3},[%4];":"=r"(af[dst][mi][0]),"=r"(af[dst][mi][1]),"=r"(af[dst][mi][2]),"=r"(af[dst][mi][3]):"r"(aA)); } \
    for (int ni=0; ni<WN/8; ++ni){ int mtx=(lane>>3)&1,row=lane&7; int shrow=warp_col+ni*8+row; int koff=mtx*16; \
        unsigned bA=(unsigned)__cvta_generic_to_shared(&Bs[buf][shrow*BK + (koff ^ SW(shrow))]); \
        asm volatile("ldmatrix.sync.aligned.m8n8.x2.shared.b16 {%0,%1},[%2];":"=r"(bf[dst][ni][0]),"=r"(bf[dst][ni][1]):"r"(bA)); } \
    } while(0)
extern "C" __global__ void gemm_mma_tn_k32db(const signed char* A, const signed char* Bcol, int* C, int M, int N, int K) {
    __shared__ signed char As[STAGES][BM * BK];
    __shared__ signed char Bs[STAGES][BN * BK];
    const int tid = threadIdx.x, warpId = tid / 32, lane = tid % 32;
    const int gid = lane / 4, tg = lane % 4;
    const int warp_row = (warpId / WARPS_N) * WM, warp_col = (warpId % WARPS_N) * WN;
    const int block_row = blockIdx.y * BM, block_col = blockIdx.x * BN;
    const int nstages = K / BK;
    int c[WM/16][WN/8][4];
    int af[2][WM/16][4], bf[2][WN/8][2];
    #pragma unroll
    for (int mi=0; mi<WM/16; ++mi)
      #pragma unroll
      for (int ni=0; ni<WN/8; ++ni) { c[mi][ni][0]=c[mi][ni][1]=c[mi][ni][2]=c[mi][ni][3]=0; }
    #define LOAD_STAGE(buf, st) do { int _k0=(st)*BK; \
        for (int idx=tid; idx<(BM*BK)/16; idx+=NTHREAD){ int r=(idx*16)/BK, cc=(idx*16)%BK; \
            __pipeline_memcpy_async(&As[buf][r*BK + (cc ^ SW(r))], &A[(block_row+r)*K + _k0+cc], 16); } \
        for (int idx=tid; idx<(BN*BK)/16; idx+=NTHREAD){ int r=(idx*16)/BK, cc=(idx*16)%BK; \
            __pipeline_memcpy_async(&Bs[buf][r*BK + (cc ^ SW(r))], &Bcol[(block_col+r)*K + _k0+cc], 16); } \
        __pipeline_commit(); } while(0)
    #pragma unroll
    for (int i=0;i<STAGES-1;++i){ if(i<nstages) LOAD_STAGE(i,i); else __pipeline_commit(); }
    __pipeline_wait_prior(STAGES-2);
    __syncthreads();
    LDA(0, 0);   // ldmatrix stage 0 -> af[0]/bf[0]
    for (int s=0; s<nstages; ++s) {
        int cur = s & 1, nxt = (s+1) & 1;
        if (s+1 < nstages) {
            __pipeline_wait_prior(STAGES-2);
            __syncthreads();
            LDA((s+1)%STAGES, nxt);      // prefetch next fragments (overlaps the mma below)
        }
        int fetch = s+STAGES-1; if (fetch<nstages) LOAD_STAGE(fetch%STAGES, fetch); else __pipeline_commit();
        #pragma unroll
        for (int mi=0; mi<WM/16; ++mi)
          #pragma unroll
          for (int ni=0; ni<WN/8; ++ni)
            asm volatile("mma.sync.aligned.m16n8k32.row.col.s32.s8.s8.s32 {%0,%1,%2,%3},{%4,%5,%6,%7},{%8,%9},{%0,%1,%2,%3};"
              :"+r"(c[mi][ni][0]),"+r"(c[mi][ni][1]),"+r"(c[mi][ni][2]),"+r"(c[mi][ni][3])
              :"r"(af[cur][mi][0]),"r"(af[cur][mi][1]),"r"(af[cur][mi][2]),"r"(af[cur][mi][3]),"r"(bf[cur][ni][0]),"r"(bf[cur][ni][1]));
    }
    #pragma unroll
    for (int mi=0; mi<WM/16; ++mi)
      #pragma unroll
      for (int ni=0; ni<WN/8; ++ni) {
        int rb=block_row+warp_row+mi*16, cb=block_col+warp_col+ni*8;
        C[(rb+gid)*N+cb+tg*2]=c[mi][ni][0]; C[(rb+gid)*N+cb+tg*2+1]=c[mi][ni][1];
        C[(rb+gid+8)*N+cb+tg*2]=c[mi][ni][2]; C[(rb+gid+8)*N+cb+tg*2+1]=c[mi][ni][3];
      }
}
"#;

/// Emit JUST the swizzled-k32 kernel (no host main) — so the compiler can compile it to a cubin and analyze its
/// SASS (register pressure, occupancy, tensor-core utilisation) via `codegen_sass::SassReport`.
pub fn emit_mma_tn_k32swz_kernel(
    bm: usize,
    bn: usize,
    warps_m: usize,
    warps_n: usize,
    stages: usize,
) -> String {
    let defs = format!("#define BM {bm}\n#define BN {bn}\n#define BK 32\n#define WARPS_M {warps_m}\n#define WARPS_N {warps_n}\n#define STAGES {stages}\n");
    format!("{defs}{}", MMA_TN_K32SWZ_BODY)
}

// ─────────────────────── rung 13: BK=64 double-k-step — the CUTLASS int8 config (83% of cuBLAS-TN) ─────────
// The real jump past 79.5%: BK=64 = TWO m16n8k32 steps per stage (more compute per shared load + ILP to hide
// ldmatrix behind IMMA), with a 64×64 warp tile (2×2 warps, high arithmetic intensity). This is the structure
// CUTLASS uses for Ampere int8. The XOR swizzle generalizes: BK=64's 4×16B-chunk row uses SW=((row>>1)&3)*16
// so the 8 ldmatrix rows land in distinct bank-sets. Fits 48KB shared at 3 stages. MEASURED 83.1% of cuBLAS-TN.
pub fn emit_mma_tn_k64_bench(
    bm: usize,
    bn: usize,
    warps_m: usize,
    warps_n: usize,
    stages: usize,
    m: usize,
    k: usize,
    n: usize,
) -> String {
    mma_tn_k64_harness(bm, bn, warps_m, warps_n, stages, m, k, n, true)
}
pub fn emit_mma_tn_k64_program(
    bm: usize,
    bn: usize,
    warps_m: usize,
    warps_n: usize,
    stages: usize,
    m: usize,
    k: usize,
    n: usize,
) -> String {
    mma_tn_k64_harness(bm, bn, warps_m, warps_n, stages, m, k, n, false)
}

fn mma_tn_k64_harness(
    bm: usize,
    bn: usize,
    warps_m: usize,
    warps_n: usize,
    stages: usize,
    m: usize,
    k: usize,
    n: usize,
    bench: bool,
) -> String {
    let defs = format!("#define BM {bm}\n#define BN {bn}\n#define BK 64\n#define WARPS_M {warps_m}\n#define WARPS_N {warps_n}\n#define STAGES {stages}\n");
    let kernel = format!("{defs}{}", MMA_TN_K64_BODY);
    let threads = warps_m * warps_n * 32;
    let body = if bench {
        format!(
            r#"
#include <cstdio>
#include <cstdlib>
int main(){{ const int M={m},K={k},N={n};
    signed char *dA,*dB; int *dC;
    cudaMalloc(&dA,(size_t)M*K);cudaMalloc(&dB,(size_t)K*N);cudaMalloc(&dC,(size_t)M*N*4);
    cudaMemset(dA,1,(size_t)M*K);cudaMemset(dB,1,(size_t)K*N);
    dim3 grid(N/{bn},M/{bm});
    gemm_mma_tn_k64<<<grid,{threads}>>>(dA,dB,dC,M,N,K);
    cudaError_t err=cudaDeviceSynchronize(); if(err){{printf("LAUNCH_ERROR %s\n",cudaGetErrorString(err));return 1;}}
    int *hC=(int*)malloc((size_t)M*N*4); cudaMemcpy(hC,dC,(size_t)M*N*4,cudaMemcpyDeviceToHost);
    long step=((long)M*N)/4096+1; for(long i=0;i<(long)M*N;i+=step) if(hC[i]!=K){{printf("INCORRECT %ld: %d!=%d\n",i,hC[i],K);return 1;}}
    cudaEvent_t s,e; cudaEventCreate(&s); cudaEventCreate(&e); int IT=30; float total=0;
    for(int it=0;it<IT;it++){{ cudaEventRecord(s); gemm_mma_tn_k64<<<grid,{threads}>>>(dA,dB,dC,M,N,K); cudaEventRecord(e); cudaEventSynchronize(e); float ms=0; cudaEventElapsedTime(&ms,s,e); total+=ms; }}
    printf("GOPS %.1f\n", 2.0*M*N*K/((total/IT)/1e3)/1e9); return 0; }}
"#,
            bn = bn,
            bm = bm,
            threads = threads,
            m = m,
            k = k,
            n = n
        )
    } else {
        format!(
            r#"
#include <cstdio>
#include <cstdlib>
int main(){{ const int M={m},K={k},N={n};
    signed char *hA=(signed char*)malloc((size_t)M*K),*hB=(signed char*)malloc((size_t)K*N);
    for(long i=0;i<(long)M*K;++i) hA[i]=(signed char)(((i*7+3)%251)-125);
    for(long nn=0;nn<N;++nn) for(long kk=0;kk<K;++kk){{ long bkn=(long)kk*N+nn; hB[nn*K+kk]=(signed char)(((bkn*5+1)%251)-125); }}
    signed char *dA,*dB; int *dC,*hC=(int*)malloc((size_t)M*N*4);
    cudaMalloc(&dA,(size_t)M*K);cudaMalloc(&dB,(size_t)K*N);cudaMalloc(&dC,(size_t)M*N*4);
    cudaMemcpy(dA,hA,(size_t)M*K,cudaMemcpyHostToDevice);cudaMemcpy(dB,hB,(size_t)K*N,cudaMemcpyHostToDevice);
    dim3 grid(N/{bn},M/{bm}); gemm_mma_tn_k64<<<grid,{threads}>>>(dA,dB,dC,M,N,K);
    cudaError_t e=cudaDeviceSynchronize(); if(e){{printf("ERR %s\n",cudaGetErrorString(e));return 1;}}
    cudaMemcpy(hC,dC,(size_t)M*N*4,cudaMemcpyDeviceToHost);
    for(long i=0;i<(long)M*N;++i) printf("%d\n",hC[i]); return 0; }}
"#,
            bn = bn,
            bm = bm,
            threads = threads,
            m = m,
            k = k,
            n = n
        )
    };
    format!("{kernel}{body}")
}

const MMA_TN_K64_BODY: &str = r#"#include <cuda_pipeline.h>
#define WM (BM/WARPS_M)
#define WN (BN/WARPS_N)
#define NTHREAD (WARPS_M*WARPS_N*32)
#define SW(row) ((((row)>>1)&3)*16)
extern "C" __global__ void gemm_mma_tn_k64(const signed char* A, const signed char* Bcol, int* C, int M, int N, int K){
  __shared__ signed char As[STAGES][BM*BK]; __shared__ signed char Bs[STAGES][BN*BK];
  const int tid=threadIdx.x,warpId=tid/32,lane=tid%32; const int gid=lane/4,tg=lane%4;
  const int warp_row=(warpId/WARPS_N)*WM,warp_col=(warpId%WARPS_N)*WN;
  const int block_row=blockIdx.y*BM,block_col=blockIdx.x*BN; const int nstages=K/BK;
  int c[WM/16][WN/8][4];
  for(int mi=0;mi<WM/16;++mi)for(int ni=0;ni<WN/8;++ni){c[mi][ni][0]=c[mi][ni][1]=c[mi][ni][2]=c[mi][ni][3]=0;}
  #define LS(buf,st) do{int _k0=(st)*BK; for(int idx=tid;idx<(BM*BK)/16;idx+=NTHREAD){int r=(idx*16)/BK,cc=(idx*16)%BK;__pipeline_memcpy_async(&As[buf][r*BK+(cc^SW(r))],&A[(block_row+r)*K+_k0+cc],16);} for(int idx=tid;idx<(BN*BK)/16;idx+=NTHREAD){int r=(idx*16)/BK,cc=(idx*16)%BK;__pipeline_memcpy_async(&Bs[buf][r*BK+(cc^SW(r))],&Bcol[(block_col+r)*K+_k0+cc],16);}__pipeline_commit();}while(0)
  for(int i=0;i<STAGES-1;++i){if(i<nstages)LS(i,i);else __pipeline_commit();}
  for(int s=0;s<nstages;++s){__pipeline_wait_prior(STAGES-2);__syncthreads();int buf=s%STAGES;
    // ILP hoist: load BOTH k-steps' fragments, THEN issue all mma — so the 2nd k-step's ldmatrix overlaps the
    // 1st k-step's tensor-core work (ptxas keeps 222 regs, no spill). +1 pt of cuBLAS parity (90.2->91.2%).
    { int a[2][WM/16][4],b[2][WN/8][2];
      #pragma unroll
      for(int ks=0;ks<2;ks++){int kk=ks*32;
        for(int mi=0;mi<WM/16;++mi){int mtx=lane/8,row=lane%8;int shrow=warp_row+mi*16+(mtx&1)*8+row;int koff=kk+(mtx>>1)*16;unsigned aA=(unsigned)__cvta_generic_to_shared(&As[buf][shrow*BK+(koff^SW(shrow))]);asm volatile("ldmatrix.sync.aligned.m8n8.x4.shared.b16 {%0,%1,%2,%3},[%4];":"=r"(a[ks][mi][0]),"=r"(a[ks][mi][1]),"=r"(a[ks][mi][2]),"=r"(a[ks][mi][3]):"r"(aA));}
        for(int ni=0;ni<WN/8;++ni){int mtx=(lane>>3)&1,row=lane&7;int shrow=warp_col+ni*8+row;int koff=kk+mtx*16;unsigned bA=(unsigned)__cvta_generic_to_shared(&Bs[buf][shrow*BK+(koff^SW(shrow))]);asm volatile("ldmatrix.sync.aligned.m8n8.x2.shared.b16 {%0,%1},[%2];":"=r"(b[ks][ni][0]),"=r"(b[ks][ni][1]):"r"(bA));}
      }
      #pragma unroll
      for(int ks=0;ks<2;ks++)for(int mi=0;mi<WM/16;++mi)for(int ni=0;ni<WN/8;++ni)asm volatile("mma.sync.aligned.m16n8k32.row.col.s32.s8.s8.s32 {%0,%1,%2,%3},{%4,%5,%6,%7},{%8,%9},{%0,%1,%2,%3};":"+r"(c[mi][ni][0]),"+r"(c[mi][ni][1]),"+r"(c[mi][ni][2]),"+r"(c[mi][ni][3]):"r"(a[ks][mi][0]),"r"(a[ks][mi][1]),"r"(a[ks][mi][2]),"r"(a[ks][mi][3]),"r"(b[ks][ni][0]),"r"(b[ks][ni][1]));
    }
    // No post-mma __syncthreads: the next iteration's pre-ldmatrix barrier already orders every thread's read
    // of this buffer before the cp.async that overwrites it (verified byte-exact vs the oracle). One fewer
    // barrier per stage => fewer overhead cycles between IMMA bursts => +2.6 pts of cuBLAS parity (87.6->90.2%).
    int nxt=s+STAGES-1;if(nxt<nstages)LS(nxt%STAGES,nxt);else __pipeline_commit();}
  for(int mi=0;mi<WM/16;++mi)for(int ni=0;ni<WN/8;++ni){int rb=block_row+warp_row+mi*16,cb=block_col+warp_col+ni*8;C[(rb+gid)*N+cb+tg*2]=c[mi][ni][0];C[(rb+gid)*N+cb+tg*2+1]=c[mi][ni][1];C[(rb+gid+8)*N+cb+tg*2]=c[mi][ni][2];C[(rb+gid+8)*N+cb+tg*2+1]=c[mi][ni][3];}
}
"#;

/// Emit JUST the BK=64 champion kernel (no host main) — for the head-to-head cuBLAS parity harness and SASS
/// analysis. Renames the entry to the given symbol so it can be linked beside cublas in one program.
pub fn emit_mma_tn_k64_kernel(
    bm: usize,
    bn: usize,
    warps_m: usize,
    warps_n: usize,
    stages: usize,
) -> String {
    let defs = format!("#define BM {bm}\n#define BN {bn}\n#define BK 64\n#define WARPS_M {warps_m}\n#define WARPS_N {warps_n}\n#define STAGES {stages}\n");
    format!("{defs}{}", MMA_TN_K64_BODY)
}

/// A CUTLASS int8 tensorop-GEMM config. cuBLAS's own int8 GEMM kernel IS a CUTLASS kernel —
/// disassembly of `cublasGemmEx` (CUDA_R_8I, TN, 2048^3, sm_86) shows it dispatch
/// `cutlass_80_tensorop_i16832gemm_s8_256x128_64x3_tn_align16`. Because CUTLASS is open source, our
/// compiler can emit the *same* kernel and reach cuBLAS parity (measured 99.6-100.0% head-to-head),
/// instead of approximating it with a hand-written kernel that tops out at ~90% (ptxas will not schedule
/// our from-scratch source into the perfect pointer-increment addressing CUTLASS's templates coax out of it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CutlassI8Gemm {
    pub tb_m: usize,
    pub tb_n: usize,
    pub tb_k: usize,
    pub warp_m: usize,
    pub warp_n: usize,
    pub warp_k: usize,
    pub inst_m: usize,
    pub inst_n: usize,
    pub inst_k: usize,
    pub stages: usize,
}

impl CutlassI8Gemm {
    /// The exact config cuBLAS dispatches (i16832gemm_s8_256x128_64x3_tn): 256x128x64 threadblock,
    /// 64x64x64 warp, m16n8k32 IMMA, 3-stage. Verified 99.6-100.0% parity — cuBLAS IS this kernel.
    pub fn cublas_reference() -> Self {
        Self {
            tb_m: 256,
            tb_n: 128,
            tb_k: 64,
            warp_m: 64,
            warp_n: 64,
            warp_k: 64,
            inst_m: 16,
            inst_n: 8,
            inst_k: 32,
            stages: 3,
        }
    }
    /// Legality on Ampere: warp tile tiles the threadblock; instruction tiles the warp; shared fits.
    pub fn is_legal(&self) -> bool {
        self.tb_m % self.warp_m == 0
            && self.tb_n % self.warp_n == 0
            && self.tb_k % self.warp_k == 0
            && self.warp_m % self.inst_m == 0
            && self.warp_n % self.inst_n == 0
            && self.warp_k % self.inst_k == 0
            && self.stages >= 2
    }
}

/// A fused pointwise epilogue our compiler folds INTO the GEMM — the thing cuBLAS cannot do in-kernel.
/// cuBLAS must write the GEMM result to global memory and run a SECOND kernel for the activation (an extra
/// full-output read+write); fusing it here removes that round-trip and BEATS a cuBLAS+elementwise baseline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CutlassEpilogue {
    /// alpha*acc + beta*C — a plain GEMM (parity with cuBLAS).
    LinearCombination,
    /// relu(alpha*acc + beta*C) — GEMM + activation fused in one kernel (the KernelBench Level-2 win).
    Relu,
}

impl CutlassEpilogue {
    /// (extra include, epilogue type) for the CUTLASS instantiation.
    fn cutlass(&self) -> (&'static str, &'static str) {
        match self {
            CutlassEpilogue::LinearCombination => (
                "#include <cutlass/epilogue/thread/linear_combination.h>\n",
                "cutlass::epilogue::thread::LinearCombination<int32_t,4,int32_t,int32_t>",
            ),
            CutlassEpilogue::Relu => (
                "#include <cutlass/epilogue/thread/linear_combination_relu.h>\n",
                "cutlass::epilogue::thread::LinearCombinationRelu<int32_t,4,int32_t,int32_t>",
            ),
        }
    }
}

/// Emit the CUTLASS int8 TN GEMM kernel our compiler chooses — the same one cuBLAS runs (with an optional
/// fused epilogue). Produces the `using Gemm = ...` instantiation + an `extern "C" run_gemm_k` launcher.
/// Requires the CUTLASS headers on the include path (`-I <cutlass>/include`).
pub fn emit_cutlass_i8_gemm_ep(cfg: &CutlassI8Gemm, ep: CutlassEpilogue) -> String {
    let (ep_inc, ep_ty) = ep.cutlass();
    format!(
        r#"#include <cutlass/gemm/device/gemm.h>
{ep_inc}using Gemm = cutlass::gemm::device::Gemm<
  int8_t, cutlass::layout::RowMajor,
  int8_t, cutlass::layout::ColumnMajor,
  int32_t, cutlass::layout::RowMajor,
  int32_t, cutlass::arch::OpClassTensorOp, cutlass::arch::Sm80,
  cutlass::gemm::GemmShape<{tbm},{tbn},{tbk}>,
  cutlass::gemm::GemmShape<{wm},{wn},{wk}>,
  cutlass::gemm::GemmShape<{im},{inn},{ik}>,
  {ep_ty},
  cutlass::gemm::threadblock::GemmIdentityThreadblockSwizzle<>,
  {st}>;
extern "C" int run_gemm_k(const signed char* A, const signed char* B, int* C, int M, int N, int K){{
  Gemm gemm; Gemm::Arguments args({{M,N,K}},{{(int8_t*)A,K}},{{(int8_t*)B,K}},{{C,N}},{{C,N}},{{1,0}});
  return (int)gemm(args);
}}
"#,
        ep_inc = ep_inc,
        ep_ty = ep_ty,
        tbm = cfg.tb_m,
        tbn = cfg.tb_n,
        tbk = cfg.tb_k,
        wm = cfg.warp_m,
        wn = cfg.warp_n,
        wk = cfg.warp_k,
        im = cfg.inst_m,
        inn = cfg.inst_n,
        ik = cfg.inst_k,
        st = cfg.stages,
    )
}

/// Plain GEMM (parity with cuBLAS) — the un-fused kernel.
pub fn emit_cutlass_i8_gemm(cfg: &CutlassI8Gemm) -> String {
    emit_cutlass_i8_gemm_ep(cfg, CutlassEpilogue::LinearCombination)
}

/// Full head-to-head program: our FUSED GEMM+ReLU vs the baseline cuBLAS forces (GEMM then a SEPARATE relu
/// kernel). Both are oracle-checked against `relu(A*B)` and against each other; prints
/// `FUSED SPEEDUP <x>x` (our_time vs cublas+relu_time). This is where we BEAT cuBLAS — via fusion.
pub fn emit_cutlass_i8_gemm_relu_bench(
    cfg: &CutlassI8Gemm,
    m: usize,
    k: usize,
    n: usize,
) -> String {
    let kernel = emit_cutlass_i8_gemm_ep(cfg, CutlassEpilogue::Relu);
    format!(
        r#"{kernel}#include <cublas_v2.h>
#include <cstdio>
#include <cstdlib>
__global__ void relu_kernel(int* C, long nel){{long i=(long)blockIdx.x*blockDim.x+threadIdx.x; if(i<nel) C[i]=C[i]>0?C[i]:0;}}
static double bst(float*x,int c){{double b=1e30;for(int i=0;i<c;i++)if(x[i]<b)b=x[i];return b;}}
int main(){{const int M={m},K={k},N={n};signed char*dA,*dB;int*dC,*dC2;
 cudaMalloc(&dA,(size_t)M*K);cudaMalloc(&dB,(size_t)K*N);cudaMalloc(&dC,(size_t)M*N*4);cudaMalloc(&dC2,(size_t)M*N*4);
 signed char*hA=(signed char*)malloc((size_t)M*K),*hB=(signed char*)malloc((size_t)K*N);
 for(long i=0;i<(long)M*K;++i)hA[i]=(signed char)(((i*7+3)%251)-125);
 for(long nn=0;nn<N;++nn)for(long kk=0;kk<K;++kk){{long bkn=(long)kk*N+nn;hB[nn*K+kk]=(signed char)(((bkn*5+1)%251)-125);}}
 cudaMemcpy(dA,hA,(size_t)M*K,cudaMemcpyHostToDevice);cudaMemcpy(dB,hB,(size_t)K*N,cudaMemcpyHostToDevice);
 run_gemm_k(dA,dB,dC,M,N,K); if(cudaDeviceSynchronize()){{printf("ERR\n");return 1;}}
 int*hC=(int*)malloc((size_t)M*N*4);cudaMemcpy(hC,dC,(size_t)M*N*4,cudaMemcpyDeviceToHost);
 for(long q=0;q<12;q++){{long i=(q*7919)%M,j=(q*104729)%N;long acc=0;for(long kk=0;kk<K;kk++){{long av=((i*K+kk)*7+3)%251-125;long bv=((kk*N+j)*5+1)%251-125;acc+=av*bv;}}long want=acc>0?acc:0;if(hC[i*N+j]!=want){{printf("INCORRECT\n");return 1;}}}}
 cublasHandle_t h;cublasCreate(&h);cublasSetMathMode(h,CUBLAS_TENSOR_OP_MATH);int al=1,be=0;
 long nel=(long)M*N; int TPB=256; long blk=(nel+TPB-1)/TPB;
 cudaEvent_t s,ev;cudaEventCreate(&s);cudaEventCreate(&ev);int IT=100;float mf[100],mb[100];
 for(int i=0;i<10;i++){{run_gemm_k(dA,dB,dC,M,N,K);cublasGemmEx(h,CUBLAS_OP_T,CUBLAS_OP_N,N,M,K,&al,dB,CUDA_R_8I,K,dA,CUDA_R_8I,K,&be,dC2,CUDA_R_32I,N,CUBLAS_COMPUTE_32I,CUBLAS_GEMM_DEFAULT_TENSOR_OP);relu_kernel<<<blk,TPB>>>(dC2,nel);}}
 cudaDeviceSynchronize();
 for(int i=0;i<IT;i++){{
   cudaEventRecord(s);run_gemm_k(dA,dB,dC,M,N,K);cudaEventRecord(ev);cudaEventSynchronize(ev);cudaEventElapsedTime(&mf[i],s,ev);
   cudaEventRecord(s);cublasGemmEx(h,CUBLAS_OP_T,CUBLAS_OP_N,N,M,K,&al,dB,CUDA_R_8I,K,dA,CUDA_R_8I,K,&be,dC2,CUDA_R_32I,N,CUBLAS_COMPUTE_32I,CUBLAS_GEMM_DEFAULT_TENSOR_OP);relu_kernel<<<blk,TPB>>>(dC2,nel);cudaEventRecord(ev);cudaEventSynchronize(ev);cudaEventElapsedTime(&mb[i],s,ev);}}
 int*hC2=(int*)malloc((size_t)M*N*4);cudaMemcpy(hC2,dC2,(size_t)M*N*4,cudaMemcpyDeviceToHost);
 for(long q=0;q<8;q++){{long idx=(q*99991)%nel;if(hC[idx]!=hC2[idx]){{printf("MISMATCH\n");return 1;}}}}
 double tf=bst(mf,IT),tb=bst(mb,IT);
 printf("FUSED ours %.3f ms  cublas+relu %.3f ms  SPEEDUP %.3fx\n",tf,tb,tb/tf);return 0;}}
"#,
        kernel = kernel,
        m = m,
        k = k,
        n = n
    )
}

/// A CUTLASS Blackwell (sm_100) NVFP4 block-scaled GEMM config — the GPU MODE `nvfp4_gemm` leaderboard target.
/// Operands are FP4 (e2m1) with per-16-element FP8 (ue4m3) block scale factors; accumulate FP32; output FP16.
/// cuBLAS/`torch._scaled_mm`'s own NVFP4 path is a CUTLASS kernel (as with int8), so our compiler emits the
/// same CollectiveBuilder GEMM and then pulls ahead by consuming the *pre-permuted* GPU scale factors directly
/// (the reference re-derives them from a CPU tensor inside the timed region — free speedup) + epilogue fusion.
/// NOTE: sm_100 / NVFP4 requires CUTLASS >= 3.7 and a Blackwell GPU; validate on the leaderboard runner
/// (`popcorn submit --mode test --gpu B200`), not on sm_86 hardware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CutlassNvfp4Gemm {
    pub tile_m: usize,
    pub tile_n: usize,
    pub tile_k: usize,
    pub cluster_m: usize,
    pub cluster_n: usize,
}

impl CutlassNvfp4Gemm {
    /// A Blackwell NVFP4 config sized to the leaderboard's speed-of-light shapes (K a multiple of 256).
    pub fn b200_default() -> Self {
        Self {
            tile_m: 256,
            tile_n: 256,
            tile_k: 256,
            cluster_m: 2,
            cluster_n: 1,
        }
    }
}

/// Emit our compiler's CUTLASS Blackwell NVFP4 block-scaled GEMM as a torch-extension launcher for
/// `load_inline` on the Blackwell leaderboard runner. Consumes the pre-permuted FP8 block scales directly.
pub fn emit_cutlass_nvfp4_gemm(cfg: &CutlassNvfp4Gemm) -> String {
    format!(
        r#"#include <torch/extension.h>
#include <cutlass/cutlass.h>
#include <cutlass/float_subbyte.h>
#include <cutlass/gemm/collective/collective_builder.hpp>
#include <cutlass/epilogue/collective/collective_builder.hpp>
#include <cutlass/gemm/device/gemm_universal_adapter.h>
#include <cutlass/gemm/kernel/gemm_universal.hpp>
using namespace cute;
using ElementA   = cutlass::float_e2m1_t;
using ElementB   = cutlass::float_e2m1_t;
using ElementSF  = cutlass::float_ue4m3_t;
using ElementC   = cutlass::half_t;
using ElementAcc = float;
using LayoutA = cutlass::layout::RowMajor;
using LayoutB = cutlass::layout::ColumnMajor;
using LayoutC = cutlass::layout::RowMajor;
using ArchTag = cutlass::arch::Sm100;
using OpClass = cutlass::arch::OpClassBlockScaledTensorOp;
using TileShape    = Shape<_{tm}, _{tn}, _{tk}>;
using ClusterShape = Shape<_{cm}, _{cn}, _1>;
using CollectiveEpilogue = typename cutlass::epilogue::collective::CollectiveBuilder<
  ArchTag, OpClass, TileShape, ClusterShape,
  cutlass::epilogue::collective::EpilogueTileAuto, ElementAcc, ElementAcc,
  ElementC, LayoutC, 8, ElementC, LayoutC, 8,
  cutlass::epilogue::collective::EpilogueScheduleAuto>::CollectiveOp;
using CollectiveMainloop = typename cutlass::gemm::collective::CollectiveBuilder<
  ArchTag, OpClass,
  cute::tuple<ElementA, ElementSF>, LayoutA, 32,
  cute::tuple<ElementB, ElementSF>, LayoutB, 32,
  ElementAcc, TileShape, ClusterShape,
  cutlass::gemm::collective::StageCountAutoCarveout<(int)sizeof(typename CollectiveEpilogue::SharedStorage)>,
  cutlass::gemm::collective::KernelScheduleAuto>::CollectiveOp;
using GemmKernel = cutlass::gemm::kernel::GemmUniversal<Shape<int,int,int,int>, CollectiveMainloop, CollectiveEpilogue>;
using Gemm = cutlass::gemm::device::GemmUniversalAdapter<GemmKernel>;
torch::Tensor run_nvfp4_gemm(torch::Tensor a, torch::Tensor b, torch::Tensor sfa, torch::Tensor sfb, torch::Tensor c, int64_t M, int64_t N, int64_t K, int64_t L){{
  for(int64_t l=0; l<L; ++l){{
    Gemm gemm;
    typename Gemm::Arguments args{{
      cutlass::gemm::GemmUniversalMode::kGemm, {{(int)M,(int)N,(int)K,1}},
      {{ (ElementA*)a[l].data_ptr(), (ElementSF*)sfa[l].data_ptr(),
         (ElementB*)b[l].data_ptr(), (ElementSF*)sfb[l].data_ptr() }},
      {{ {{1.0f, 0.0f}}, nullptr, {{}}, (ElementC*)c[l].data_ptr(), {{}} }} }};
    auto st = gemm.run(args);
    TORCH_CHECK(st==cutlass::Status::kSuccess, "nvfp4 gemm slice failed");
  }}
  return c;
}}
"#,
        tm = cfg.tile_m,
        tn = cfg.tile_n,
        tk = cfg.tile_k,
        cm = cfg.cluster_m,
        cn = cfg.cluster_n
    )
}

/// Emit our compiler's CUTLASS Blackwell NVFP4 **grouped** GEMM (MoE) — all G experts in ONE kernel launch
/// via CUTLASS `GroupProblemShape` + per-group pointer arrays. This is the fusion edge on the GPU MODE
/// `nvfp4_group_gemm` leaderboard: the reference launches G separate GEMMs; we launch one grouped kernel over
/// all experts (no per-expert launch overhead, one wave of tensor-core work). Validate on the B200 runner.
pub fn emit_cutlass_nvfp4_group_gemm(cfg: &CutlassNvfp4Gemm) -> String {
    format!(
        r#"#include <torch/extension.h>
#include <vector>
#include <cutlass/cutlass.h>
#include <cutlass/float_subbyte.h>
#include <cutlass/gemm/group_array_problem_shape.hpp>
#include <cutlass/gemm/collective/collective_builder.hpp>
#include <cutlass/epilogue/collective/collective_builder.hpp>
#include <cutlass/gemm/device/gemm_universal_adapter.h>
#include <cutlass/gemm/kernel/gemm_universal.hpp>
using namespace cute;
using ElementA=cutlass::float_e2m1_t; using ElementB=cutlass::float_e2m1_t;
using ElementSF=cutlass::float_ue4m3_t; using ElementC=cutlass::half_t; using ElementAcc=float;
using LayoutA=cutlass::layout::RowMajor; using LayoutB=cutlass::layout::ColumnMajor; using LayoutC=cutlass::layout::RowMajor;
using ArchTag=cutlass::arch::Sm100; using OpClass=cutlass::arch::OpClassBlockScaledTensorOp;
using ProblemShape=cutlass::gemm::GroupProblemShape<Shape<int,int,int>>;
using TileShape=Shape<_{tm},_{tn},_{tk}>; using ClusterShape=Shape<_{cm},_{cn},_1>;
using CollectiveEpilogue = typename cutlass::epilogue::collective::CollectiveBuilder<
  ArchTag,OpClass,TileShape,ClusterShape,cutlass::epilogue::collective::EpilogueTileAuto,ElementAcc,ElementAcc,
  ElementC,LayoutC,8,ElementC,LayoutC,8,cutlass::epilogue::collective::EpilogueScheduleAuto>::CollectiveOp;
using CollectiveMainloop = typename cutlass::gemm::collective::CollectiveBuilder<
  ArchTag,OpClass,cute::tuple<ElementA,ElementSF>,LayoutA,32,cute::tuple<ElementB,ElementSF>,LayoutB,32,
  ElementAcc,TileShape,ClusterShape,
  cutlass::gemm::collective::StageCountAutoCarveout<(int)sizeof(typename CollectiveEpilogue::SharedStorage)>,
  cutlass::gemm::collective::KernelScheduleAuto>::CollectiveOp;
using GemmKernel=cutlass::gemm::kernel::GemmUniversal<ProblemShape,CollectiveMainloop,CollectiveEpilogue>;
using Gemm=cutlass::gemm::device::GemmUniversalAdapter<GemmKernel>;
// One grouped launch over all experts. Pointer/scale/problem arrays are device buffers the host fills per group.
void run_nvfp4_group_gemm(std::vector<torch::Tensor> a, std::vector<torch::Tensor> b,
                          std::vector<torch::Tensor> sfa, std::vector<torch::Tensor> sfb,
                          std::vector<torch::Tensor> c, std::vector<std::vector<int64_t>> sizes){{
  int G=(int)a.size();
  std::vector<ProblemShape::UnderlyingProblemShape> ps(G);
  std::vector<const ElementA*> pa(G); std::vector<const ElementB*> pb(G);
  std::vector<const ElementSF*> psa(G), psb(G); std::vector<ElementC*> pc(G);
  for(int i=0;i<G;++i){{ ps[i]={{(int)sizes[i][0],(int)sizes[i][1],(int)sizes[i][2]}};
    pa[i]=(const ElementA*)a[i].data_ptr(); pb[i]=(const ElementB*)b[i].data_ptr();
    psa[i]=(const ElementSF*)sfa[i].data_ptr(); psb[i]=(const ElementSF*)sfb[i].data_ptr(); pc[i]=(ElementC*)c[i].data_ptr(); }}
  Gemm gemm;
  typename Gemm::Arguments args{{ cutlass::gemm::GemmUniversalMode::kGrouped, {{G, ps.data(), nullptr}},
    {{ pa.data(), psa.data(), pb.data(), psb.data() }},
    {{ {{1.0f,0.0f}}, nullptr, {{}}, pc.data(), {{}} }} }};
  TORCH_CHECK(gemm.run(args)==cutlass::Status::kSuccess,"nvfp4 grouped gemm");
}}
"#,
        tm = cfg.tile_m,
        tn = cfg.tile_n,
        tk = cfg.tile_k,
        cm = cfg.cluster_m,
        cn = cfg.cluster_n
    )
}

/// Full head-to-head parity program: our compiler-emitted CUTLASS kernel vs `cublasGemmEx`, same process,
/// interleaved best-of-100, varied-fill oracle-checked. Prints `PARITY <pct> OURS <gops> CUBLAS <gops>`.
pub fn emit_cutlass_i8_gemm_bench(cfg: &CutlassI8Gemm, m: usize, k: usize, n: usize) -> String {
    let kernel = emit_cutlass_i8_gemm(cfg);
    format!(
        r#"{kernel}#include <cublas_v2.h>
#include <cstdio>
#include <cstdlib>
static double bst(float*x,int c){{double b=1e30;for(int i=0;i<c;i++)if(x[i]<b)b=x[i];return b;}}
int main(){{const int M={m},K={k},N={n};signed char*dA,*dB;int*dC;
 cudaMalloc(&dA,(size_t)M*K);cudaMalloc(&dB,(size_t)K*N);cudaMalloc(&dC,(size_t)M*N*4);
 signed char*hA=(signed char*)malloc((size_t)M*K),*hB=(signed char*)malloc((size_t)K*N);
 for(long i=0;i<(long)M*K;++i)hA[i]=(signed char)(((i*7+3)%251)-125);
 for(long nn=0;nn<N;++nn)for(long kk=0;kk<K;++kk){{long bkn=(long)kk*N+nn;hB[nn*K+kk]=(signed char)(((bkn*5+1)%251)-125);}}
 cudaMemcpy(dA,hA,(size_t)M*K,cudaMemcpyHostToDevice);cudaMemcpy(dB,hB,(size_t)K*N,cudaMemcpyHostToDevice);
 if(run_gemm_k(dA,dB,dC,M,N,K)!=0){{printf("CUTLASS_ERR\n");return 1;}}
 if(cudaDeviceSynchronize()){{printf("ERR\n");return 1;}}
 int*hC=(int*)malloc((size_t)M*N*4);cudaMemcpy(hC,dC,(size_t)M*N*4,cudaMemcpyDeviceToHost);
 for(long q=0;q<12;q++){{long i=(q*7919)%M,j=(q*104729)%N;long acc=0;for(long kk=0;kk<K;kk++){{long av=((i*K+kk)*7+3)%251-125;long bv=((kk*N+j)*5+1)%251-125;acc+=av*bv;}}if(hC[i*N+j]!=acc){{printf("INCORRECT\n");return 1;}}}}
 cudaMemset(dA,1,(size_t)M*K);cudaMemset(dB,1,(size_t)K*N);
 cublasHandle_t h;cublasCreate(&h);cublasSetMathMode(h,CUBLAS_TENSOR_OP_MATH);int al=1,be=0;
 cudaEvent_t s,ev;cudaEventCreate(&s);cudaEventCreate(&ev);int IT=100;float mo[100],mc[100];
 for(int i=0;i<10;i++){{run_gemm_k(dA,dB,dC,M,N,K);cublasGemmEx(h,CUBLAS_OP_T,CUBLAS_OP_N,N,M,K,&al,dB,CUDA_R_8I,K,dA,CUDA_R_8I,K,&be,dC,CUDA_R_32I,N,CUBLAS_COMPUTE_32I,CUBLAS_GEMM_DEFAULT_TENSOR_OP);}}
 cudaDeviceSynchronize();
 for(int i=0;i<IT;i++){{cudaEventRecord(s);run_gemm_k(dA,dB,dC,M,N,K);cudaEventRecord(ev);cudaEventSynchronize(ev);cudaEventElapsedTime(&mo[i],s,ev);
   cudaEventRecord(s);cublasGemmEx(h,CUBLAS_OP_T,CUBLAS_OP_N,N,M,K,&al,dB,CUDA_R_8I,K,dA,CUDA_R_8I,K,&be,dC,CUDA_R_32I,N,CUBLAS_COMPUTE_32I,CUBLAS_GEMM_DEFAULT_TENSOR_OP);cudaEventRecord(ev);cudaEventSynchronize(ev);cudaEventElapsedTime(&mc[i],s,ev);}}
 double go=2.0*M*N*K/(bst(mo,IT)/1e3)/1e9,gc=2.0*M*N*K/(bst(mc,IT)/1e3)/1e9;
 printf("PARITY %.1f OURS %.0f CUBLAS %.0f\n",100*go/gc,go,gc);return 0;}}
"#,
        kernel = kernel,
        m = m,
        k = k,
        n = n
    )
}
