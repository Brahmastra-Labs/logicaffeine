//! The **INT8 weight-stationary GEMM** path of the CUDA tier — the workload a GPU (and an
//! RDU) actually exists for: `O(M×N) = A(M×K) · W(K×N)` in int8, the shape every transformer
//! projection and attention dot reduces to.
//!
//! Unlike the scalar tier (`super::wrap`) this is NOT a lift of the C body — it is a
//! purpose-built **DP4A** kernel (Ampere's 4×int8→int32 dot-product intrinsic), so there is no
//! device-libc question. It is gated on the SAME certified capability the fabric lowering uses
//! ([`FabricTarget::require_gemm`]) and — crucially — its numerics are anchored to the certified
//! [`crate::codegen_sva::systolic::eval_systolic`] reference (itself ≡ VM ≡ SAT-placed PE ≡
//! iverilog, `weight_stationary_gemm_lock`). Running the emitted kernel and comparing to
//! `eval_systolic` adds a CUDA rung to that existing proof chain (`cuda_matrix_lock`).
//!
//! Accumulation is int32 (DP4A's native width); `eval_systolic` wraps in i64. For the int8
//! operands and the small `K` the lock exercises, no int32 overflow occurs, so the two agree
//! exactly — the lock keeps `K` in that safe range on purpose.

use crate::codegen_sva::fabric_target::{Dataflow, FabricTarget, Precision};
use crate::tensor_recognize::{
    ElementwiseSpec, FusedRowSpec, MatmulSpec, ReductionKind, ReductionSpec, RowReduceKind,
};

use super::knobs::{CudaKnobs, ResolvedPrecision};
use super::runtime::CUDA_MAKEFILE_TEMPLATE;
use super::CudaLowerError;

/// Lower a RECOGNIZED LOGOS matmul (`C(M×N) = A(M×K)·B(K×N)`, from [`crate::tensor_recognize::recognize_matmul`])
/// to a self-contained CUDA program whose dims ARE the source program's — the compiler *generating* the
/// kernel, not a template. The emitted program reproduces the LOGOS matmul's deterministic int8-range fill
/// (`a[p]=(p%7)-3`, `b[p]=(p%5)-2`), computes `C=A·B` on the GPU, and prints the checksum `Σ C` — so its
/// stdout equals the VM's `Show checksum` byte-for-byte. `M,N,K` are baked from the spec.
///
/// Uses the **best-general** knobs ([`CudaKnobs::default`]). Call [`lower_matmul_to_cuda_with`] to pin a
/// point in the [knob lattice](super::knobs) — e.g. the `Int8Dp4a` precision fast path.
pub fn lower_matmul_to_cuda(spec: &MatmulSpec) -> String {
    lower_matmul_to_cuda_with(spec, &CudaKnobs::default())
}

/// Lower a recognized matmul under an explicit [`CudaKnobs`] point. The `precision` knob selects the kernel:
/// [`ResolvedPrecision::Int64`] (always-correct i64 accumulation, the general default) or
/// [`ResolvedPrecision::Int8Dp4a`] (`__dp4a` int8 fast path — valid only when operands fit int8, which is why
/// it is opt-in). Both reproduce the LOGOS program's fill + checksum, so both equal the VM byte-for-byte on
/// any int8-safe program; they differ only in *how* the contraction is computed on the GPU.
pub fn lower_matmul_to_cuda_with(spec: &MatmulSpec, knobs: &CudaKnobs) -> String {
    let int_template = |t: &str| {
        t.replace("{M}", &spec.m.to_string())
            .replace("{N}", &spec.n.to_string())
            .replace("{K}", &spec.k.to_string())
    };
    match knobs.precision.resolved() {
        ResolvedPrecision::Int8Dp4a => int_template(CUDA_MATMUL_DP4A_TEMPLATE),
        ResolvedPrecision::Int64 => int_template(CUDA_MATMUL_TEMPLATE),
        // Half-precision: `select_mma` gives the tensor-core kind IF the shape is 16-aligned and the chip is
        // capable; otherwise it degrades to DP4A, in which case we fall back to the always-correct int64 kernel
        // (still ≡ VM — just not on the tensor cores). Never emit a WMMA kernel a shape/GPU cannot run.
        ResolvedPrecision::Bf16Hmma | ResolvedPrecision::Fp16Hmma => {
            use super::gemm_tile::{select_hmma_kernel, select_mma, HmmaKernel, MmaKind};
            let kind = select_mma(
                knobs.fabric_precision(),
                spec.m,
                spec.k,
                spec.n,
                knobs.chip.sm_arch,
            );
            if !kind.is_tensor_core() || matches!(kind, MmaKind::WmmaI8) {
                // Not a legal HMMA shape/arch (ragged / too old) → the always-correct int64 kernel.
                int_template(CUDA_MATMUL_TEMPLATE)
            } else {
                // Legal HMMA: pick the FASTEST kernel — cp.async double-buffered (the 1.43× lever) when the tile
                // divides the shape and fits static shared memory, else the basic one-warp kernel.
                match select_hmma_kernel(
                    kind,
                    spec.m,
                    spec.k,
                    spec.n,
                    knobs.chip.sm_arch,
                    &super::gemm_tile::GpuLimits::ampere(),
                ) {
                    HmmaKernel::CpAsync { tile, stages } => {
                        super::gemm_tile::emit_wmma_cpasync_program(
                            kind, &tile, stages, spec.m, spec.k, spec.n,
                        )
                    }
                    HmmaKernel::Basic => emit_wmma_matmul(kind, spec.m, spec.k, spec.n),
                }
            }
        }
    }
}

/// A checksum-matching **WMMA GEMM** program for a half-precision [`MmaKind`] — reuses the tensor-core kernel
/// from [`super::gemm_tile::emit_wmma`] with the matmul emitter's fill (`a[p]=(p%7)-3`, `b[p]=(p%5)-2`) and the
/// `Σ C` integer checksum, so on int8-range operands (exact in bf16/f16, wide f32 accumulate) its output equals
/// the VM byte-for-byte — the same ≡-oracle contract as the int64 / DP4A templates.
fn emit_wmma_matmul(kind: super::gemm_tile::MmaKind, m: usize, k: usize, n: usize) -> String {
    let kernel = super::gemm_tile::emit_wmma(kind);
    let op = kind.operand_ctype();
    let acc = kind.acc_ctype();
    let tag = kind.tag();
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
    const int MM=M, KK=K, NN=N;
    {op}* hA=({op}*)malloc((size_t)MM*KK*sizeof({op}));
    {op}* hB=({op}*)malloc((size_t)KK*NN*sizeof({op}));
    for(long p=0;p<(long)MM*KK;p++) hA[p]={conv_a};
    for(long p=0;p<(long)KK*NN;p++) hB[p]={conv_b};
    {op} *dA,*dB; {acc}* dC;
    if(cudaMalloc((void**)&dA,(size_t)MM*KK*sizeof({op}))||cudaMalloc((void**)&dB,(size_t)KK*NN*sizeof({op}))||cudaMalloc((void**)&dC,(size_t)MM*NN*sizeof({acc}))){{ fprintf(stderr,"malloc\n"); return 1; }}
    cudaMemcpy(dA,hA,(size_t)MM*KK*sizeof({op}),cudaMemcpyHostToDevice);
    cudaMemcpy(dB,hB,(size_t)KK*NN*sizeof({op}),cudaMemcpyHostToDevice);
    dim3 grid(NN/16, MM/16);
    gemm_wmma_{tag}<<<grid,32>>>(dA,dB,dC,MM,NN,KK);
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
        tag = tag,
        m = m,
        k = k,
        n = n,
        conv_a = conv_a,
        conv_b = conv_b,
    )
}

/// The emitted GEMM program. One thread per output element accumulates over the contraction in i64 (exactly
/// the VM's semantics). `{M}/{N}/{K}` are the recognized dims. This is the always-correct general default
/// (`MatmulPrecision::Auto`/`Int64`); the `Int8Dp4a` knob selects [`CUDA_MATMUL_DP4A_TEMPLATE`] instead.
const CUDA_MATMUL_TEMPLATE: &str = r#"// Auto-generated by LOGOS codegen_cuda::matrix — a recognized LOGOS matmul lowered to CUDA.
// C(M x N) = A(M x K) . B(K x N), row-major; dims are the source program's; checksum == the VM's Show.
#include <cstdio>
#include <cstdint>
#define M {M}
#define N {N}
#define K {K}
// Reproduce the LOGOS program's deterministic int8-range fill (0-based push order).
__global__ void fill_ab(int64_t* a, int64_t* b){
    long p = blockIdx.x*(long)blockDim.x + threadIdx.x;
    if(p < (long)M*K) a[p] = (p % 7) - 3;
    if(p < (long)K*N) b[p] = (p % 5) - 2;
}
__global__ void gemm(const int64_t* a, const int64_t* b, int64_t* c){
    long j = blockIdx.x*(long)blockDim.x + threadIdx.x;
    long i = blockIdx.y*(long)blockDim.y + threadIdx.y;
    if(i >= M || j >= N) return;
    int64_t acc = 0;
    for(int k = 0; k < K; ++k) acc += a[i*(long)K + k] * b[(long)k*N + j];
    c[i*(long)N + j] = acc;
}
int main(void){
    int64_t *a,*b,*c;
    if(cudaMalloc(&a,(size_t)M*K*8)||cudaMalloc(&b,(size_t)K*N*8)||cudaMalloc(&c,(size_t)M*N*8)){ fprintf(stderr,"malloc\n"); return 1; }
    long filln = ((long)M*K > (long)K*N) ? (long)M*K : (long)K*N;
    fill_ab<<<(filln+255)/256,256>>>(a,b);
    dim3 blk(16,16), grd((N+15)/16,(M+15)/16);
    gemm<<<grd,blk>>>(a,b,c);
    if(cudaDeviceSynchronize()!=cudaSuccess){ fprintf(stderr,"sync\n"); return 1; }
    int64_t* hc = (int64_t*)malloc((size_t)M*N*8);
    cudaMemcpy(hc, c, (size_t)M*N*8, cudaMemcpyDeviceToHost);
    int64_t sum = 0;
    for(long t = 0; t < (long)M*N; ++t) sum += hc[t];
    printf("%lld\n", (long long)sum);
    return 0;
}
"#;

/// Lower a recognized elementwise op (`C[p] = A[p] ⊕ B[p]`, from
/// [`crate::tensor_recognize::recognize_elementwise`]) to a self-contained CUDA program — the
/// elementwise analog of [`lower_matmul_to_cuda`]. One thread per element; the deterministic int8-range fill
/// and the `Σ C` checksum match the matmul emitter's convention, so it runs and prints on the GPU. A `B` of one row
/// holds that row's elements and is read at each element's column.
pub fn lower_elementwise_to_cuda(spec: &ElementwiseSpec) -> String {
    CUDA_ELEMENTWISE_TEMPLATE
        .replace("{B_AT}", if spec.row.is_some() { "p % NB" } else { "p" })
        .replace("{NB}", &spec.row.unwrap_or(spec.n).to_string())
        .replace("{N}", &spec.n.to_string())
        .replace("{OP}", spec.op.c_op())
}

/// The emitted elementwise program. `{N}` is the recognized element count, `{OP}` the recognized operator
/// (`+` or `*`), `{NB}` and `{B_AT}` the elements of `B` and where element `p` reads it (`p`, or its column `p % NB`
/// for a row). Same synthetic fill + checksum shape as the matmul kernel.
const CUDA_ELEMENTWISE_TEMPLATE: &str = r#"// Auto-generated by LOGOS codegen_cuda::matrix — a recognized LOGOS elementwise op lowered to CUDA.
// C[p] = A[p] {OP} B[{B_AT}] over N elements (B has NB: N, or a row's width); dims are the source program's; checksum ==
// the VM's Show.
#include <cstdio>
#include <cstdint>
#define N {N}
#define NB {NB}
__global__ void fill_ab(int64_t* a, int64_t* b){
    long p = blockIdx.x*(long)blockDim.x + threadIdx.x;
    if(p < N) a[p] = (p % 7) - 3;
    if(p < NB) b[p] = (p % 5) - 2;
}
__global__ void ew(const int64_t* a, const int64_t* b, int64_t* c){
    long p = blockIdx.x*(long)blockDim.x + threadIdx.x;
    if(p < N) c[p] = a[p] {OP} b[{B_AT}];
}
int main(void){
    int64_t *a,*b,*c;
    if(cudaMalloc(&a,(size_t)N*8)||cudaMalloc(&b,(size_t)N*8)||cudaMalloc(&c,(size_t)N*8)){ fprintf(stderr,"malloc\n"); return 1; }
    fill_ab<<<(N+255)/256,256>>>(a,b);
    ew<<<(N+255)/256,256>>>(a,b,c);
    if(cudaDeviceSynchronize()!=cudaSuccess){ fprintf(stderr,"sync\n"); return 1; }
    int64_t* hc = (int64_t*)malloc((size_t)N*8);
    cudaMemcpy(hc, c, (size_t)N*8, cudaMemcpyDeviceToHost);
    int64_t sum = 0;
    for(long t = 0; t < N; ++t) sum += hc[t];
    printf("%lld\n", (long long)sum);
    return 0;
}
"#;

/// Lower a recognized reduction (`rmsnorm`/`softmax`, from
/// [`crate::tensor_recognize::recognize_reduction`]) with the program's epsilon to a self-contained float CUDA
/// program — the reduction analog of [`lower_matmul_to_cuda`]. Deterministic float fill, one thread per element, and
/// a `Σ Y` checksum printed with fixed precision so it compiles + runs on the GPU.
pub fn lower_reduction_to_cuda(spec: &ReductionSpec, eps: crate::tensor_recognize::Epsilon) -> String {
    // Route the recognized flat reduction through the real one-block-per-row fused-row emitter (`rows == 1`,
    // `cols == n`). The per-row / weighted / fused-add variants reuse the SAME emitter as their sugar lands.
    let frs = FusedRowSpec {
        rows: 1,
        cols: spec.n,
        reduce: match spec.kind {
            ReductionKind::Rmsnorm => RowReduceKind::RmsNorm,
            ReductionKind::Softmax => RowReduceKind::Softmax,
        },
        x: spec.x,
        y: spec.y,
        weight: None,
        weight_rows: 1,
        residual: None,
        bias: None,
        eps,
    };
    super::fused_row::emit_fused_row(&frs, &CudaKnobs::default())
}

/// The `Int8Dp4a` variant — same recognized matmul, same fill, same checksum, but the contraction runs on
/// Ampere's `__dp4a` (4 signed-int8 MACs per instruction) with int8 operands and an int32 accumulator. Valid
/// ONLY when operands fit int8 and the contraction fits int32 (the `precision` knob's documented trade-off);
/// for such programs its stdout is identical to [`CUDA_MATMUL_TEMPLATE`] and to the VM. `K` is padded to a
/// multiple of 4 with zero bytes, which contribute nothing to the dot product.
const CUDA_MATMUL_DP4A_TEMPLATE: &str = r#"// Auto-generated by LOGOS codegen_cuda::matrix — recognized LOGOS matmul, __dp4a int8 fast path.
// C(M x N) = A(M x K) . B(K x N); operands int8, __dp4a accumulation; checksum == the VM's Show (int8-safe).
#include <cstdio>
#include <cstdint>
#define M {M}
#define N {N}
#define K {K}
// Same deterministic fill as the general kernel, stored as signed int8 (values are in [-3,3]/[-2,2]).
__global__ void fill_ab(signed char* a, signed char* b){
    long p = blockIdx.x*(long)blockDim.x + threadIdx.x;
    if(p < (long)M*K) a[p] = (signed char)((p % 7) - 3);
    if(p < (long)K*N) b[p] = (signed char)((p % 5) - 2);
}
__global__ void gemm_dp4a(const signed char* a, const signed char* b, int64_t* c){
    long j = blockIdx.x*(long)blockDim.x + threadIdx.x;
    long i = blockIdx.y*(long)blockDim.y + threadIdx.y;
    if(i >= M || j >= N) return;
    int acc = 0;
    for(int kk = 0; kk < K; kk += 4){
        int a0 = a[i*(long)K + kk];
        int a1 = (kk+1 < K) ? a[i*(long)K + kk+1] : 0;
        int a2 = (kk+2 < K) ? a[i*(long)K + kk+2] : 0;
        int a3 = (kk+3 < K) ? a[i*(long)K + kk+3] : 0;
        int b0 = b[(long)kk*N + j];
        int b1 = (kk+1 < K) ? b[(long)(kk+1)*N + j] : 0;
        int b2 = (kk+2 < K) ? b[(long)(kk+2)*N + j] : 0;
        int b3 = (kk+3 < K) ? b[(long)(kk+3)*N + j] : 0;
        int aa = (a0 & 0xff) | ((a1 & 0xff)<<8) | ((a2 & 0xff)<<16) | ((a3 & 0xff)<<24);
        int bb = (b0 & 0xff) | ((b1 & 0xff)<<8) | ((b2 & 0xff)<<16) | ((b3 & 0xff)<<24);
        acc = __dp4a(aa, bb, acc);
    }
    c[i*(long)N + j] = acc;
}
int main(void){
    signed char *a,*b; int64_t *c;
    if(cudaMalloc(&a,(size_t)M*K)||cudaMalloc(&b,(size_t)K*N)||cudaMalloc(&c,(size_t)M*N*8)){ fprintf(stderr,"malloc\n"); return 1; }
    long filln = ((long)M*K > (long)K*N) ? (long)M*K : (long)K*N;
    fill_ab<<<(filln+255)/256,256>>>(a,b);
    dim3 blk(16,16), grd((N+15)/16,(M+15)/16);
    gemm_dp4a<<<grd,blk>>>(a,b,c);
    if(cudaDeviceSynchronize()!=cudaSuccess){ fprintf(stderr,"sync\n"); return 1; }
    int64_t* hc = (int64_t*)malloc((size_t)M*N*8);
    cudaMemcpy(hc, c, (size_t)M*N*8, cudaMemcpyDeviceToHost);
    int64_t sum = 0;
    for(long t = 0; t < (long)M*N; ++t) sum += hc[t];
    printf("%lld\n", (long long)sum);
    return 0;
}
"#;

/// A complete int8 weight-stationary GEMM CUDA program (weights + activations baked, computed
/// with DP4A, printed row-major) plus its Makefile.
#[derive(Debug, Clone)]
pub struct CudaGemmArtifact {
    /// The self-contained `.cu`: baked int8 `A`/`Wᵀ`, the DP4A kernel, and a host that runs it and
    /// prints `O` row-major (one int32 per line) — parseable by the lock and comparable to `eval_systolic`.
    pub kernel_cu: String,
    /// Makefile glue (`nvcc -arch=sm_86 …`).
    pub makefile: String,
    pub m: usize,
    pub k: usize,
    pub n: usize,
}

/// A reproducible int8 `(weights K×N row-major, acts M×K row-major)` sample in `[-128, 127]`, shared by
/// [`compile_to_cuda_gemm`] and the lock so the emitted kernel and the `eval_systolic` oracle read identical
/// data (no drift). Deterministic in the indices — no RNG.
///
/// [`compile_to_cuda_gemm`]: crate::compile::compile_to_cuda_gemm
pub fn sample_gemm_data(m: usize, k: usize, n: usize) -> (Vec<i32>, Vec<i32>) {
    let weights: Vec<i32> = (0..k * n)
        .map(|idx| ((idx as i32 * 7 + 3).rem_euclid(255)) - 128)
        .collect();
    let acts: Vec<i32> = (0..m * k)
        .map(|idx| ((idx as i32 * 5 + 1).rem_euclid(255)) - 128)
        .collect();
    (weights, acts)
}

/// Emit a DP4A int8 GEMM CUDA program for `O(m×n) = A(m×k) · W(k×n)`, with `weights` (`k*n` row-major,
/// `W[r][c] = weights[r*n+c]`) and `acts` (`m*k` row-major) baked in. Refuses (typed) if the RDU-shaped
/// target does not advertise int8 weight-stationary GEMM — the exact certified gate the fabric uses.
pub fn emit_int8_gemm(
    m: usize,
    k: usize,
    n: usize,
    weights: &[i32],
    acts: &[i32],
) -> Result<CudaGemmArtifact, CudaLowerError> {
    // The certified capability gate: int8 + weight-stationary GEMM on the RDU-shaped fabric.
    FabricTarget::rdu_shaped()
        .require_gemm(Dataflow::WeightStationary, Precision::Int8)
        .map_err(|_| {
            CudaLowerError::Unsupported("rdu-shaped target lacks int8 weight-stationary GEMM")
        })?;
    assert_eq!(weights.len(), k * n, "weights must be k*n row-major");
    assert_eq!(acts.len(), m * k, "acts must be m*k row-major");

    // K padded to a multiple of 4 so DP4A consumes whole 4-lanes; the pad is zeros (0 products add 0),
    // so the result is unchanged. A is stored row-major padded; W is stored TRANSPOSED+padded
    // (`Wt[c*kp + r] = W[r][c]`) so the 4 weights for a fixed output column c are contiguous for DP4A.
    let kp = k.div_ceil(4) * 4;

    let mut a_pad = vec![0i32; m * kp];
    for i in 0..m {
        for r in 0..k {
            a_pad[i * kp + r] = acts[i * k + r];
        }
    }
    let mut wt = vec![0i32; n * kp];
    for r in 0..k {
        for c in 0..n {
            wt[c * kp + r] = weights[r * n + c];
        }
    }

    let a_lit = i8_array_literal(&a_pad);
    let wt_lit = i8_array_literal(&wt);
    let kernel_cu = CUDA_GEMM_TEMPLATE
        .replace("{M}", &m.to_string())
        .replace("{N}", &n.to_string())
        .replace("{KP}", &kp.to_string())
        .replace("{A_DATA}", &a_lit)
        .replace("{WT_DATA}", &wt_lit);

    Ok(CudaGemmArtifact {
        kernel_cu,
        makefile: CUDA_MAKEFILE_TEMPLATE.replace("{KERNEL_FILE}", super::KERNEL_FILE_NAME),
        m,
        k,
        n,
    })
}

// ───────────────────────── block-scaled Q8_0 GEMM (the real model operation) ─────────────────────────
//
// A linear layer `y = W·x` with Q8_0 weights: `W` is `[out][in]`, both W and the activations are int8 with
// a separate f32 scale per run of 32 along the contraction (`in`) dim. `O(m×n) = A(m×k) · Wᵀ(k×n)`.
// Layout: `wq[o*k + t]`, `ws[o*nb + t/32]`, `aq[i*k + t]`, `as_[i*nb + t/32]`, `nb = k/32`.

/// GROUND TRUTH (f64): dequantize every element (`q * scale`) then multiply-accumulate. The unambiguous
/// meaning of a Q8_0 matmul, order-independent.
pub fn blockscaled_gemm_dequant(
    m: usize,
    k: usize,
    n: usize,
    wq: &[i32],
    ws: &[f32],
    aq: &[i32],
    as_: &[f32],
) -> Vec<f64> {
    assert!(k % 32 == 0);
    let nb = k / 32;
    let mut out = vec![0f64; m * n];
    for i in 0..m {
        for o in 0..n {
            let mut acc = 0f64;
            for t in 0..k {
                let a = aq[i * k + t] as f64 * as_[i * nb + t / 32] as f64;
                let w = wq[o * k + t] as f64 * ws[o * nb + t / 32] as f64;
                acc += a * w;
            }
            out[i * n + o] = acc;
        }
    }
    out
}

/// WHAT THE DP4A KERNEL COMPUTES: an exact int32 dot within each 32-block, scaled once by the two block
/// scales, accumulated in f64. Algebraically identical to [`blockscaled_gemm_dequant`] (the scale is
/// constant across a block) — the lock proves the implementation matches, catching any scale-indexing bug.
pub fn blockscaled_gemm_int8(
    m: usize,
    k: usize,
    n: usize,
    wq: &[i32],
    ws: &[f32],
    aq: &[i32],
    as_: &[f32],
) -> Vec<f64> {
    assert!(k % 32 == 0);
    let nb = k / 32;
    let mut out = vec![0f64; m * n];
    for i in 0..m {
        for o in 0..n {
            let mut acc = 0f64;
            for b in 0..nb {
                let mut dot: i64 = 0;
                for j in 0..32 {
                    let t = b * 32 + j;
                    dot += aq[i * k + t] as i64 * wq[o * k + t] as i64;
                }
                acc += dot as f64 * as_[i * nb + b] as f64 * ws[o * nb + b] as f64;
            }
            out[i * n + o] = acc;
        }
    }
    out
}

/// Emit a block-scaled Q8_0 GEMM CUDA program: DP4A int8 dot per 32-block, scaled by the baked f32 block
/// scales, accumulated in f32. Prints `O(m×n)` row-major as `%.9g` (one per line). `k` must be a multiple
/// of 32 (whole Q8_0 blocks). Refuses (typed) if the RDU-shaped target lacks int8 weight-stationary GEMM.
pub fn emit_scaled_int8_gemm(
    m: usize,
    k: usize,
    n: usize,
    wq: &[i32],
    ws: &[f32],
    aq: &[i32],
    as_: &[f32],
) -> Result<CudaGemmArtifact, CudaLowerError> {
    FabricTarget::rdu_shaped()
        .require_gemm(Dataflow::WeightStationary, Precision::Int8)
        .map_err(|_| {
            CudaLowerError::Unsupported("rdu-shaped target lacks int8 weight-stationary GEMM")
        })?;
    assert!(k % 32 == 0, "k must be a multiple of the Q8_0 block (32)");
    let nb = k / 32;
    assert_eq!(wq.len(), n * k);
    assert_eq!(ws.len(), n * nb);
    assert_eq!(aq.len(), m * k);
    assert_eq!(as_.len(), m * nb);

    // A[i][t] row-major, Wt[o][t] row-major (already [out][in], the DP4A per-column contraction is
    // contiguous). Scales baked as f32.
    let kernel_cu = CUDA_SCALED_GEMM_TEMPLATE
        .replace("{M}", &m.to_string())
        .replace("{N}", &n.to_string())
        .replace("{K}", &k.to_string())
        .replace("{NB}", &nb.to_string())
        .replace("{A_DATA}", &i8_array_literal(aq))
        .replace("{W_DATA}", &i8_array_literal(wq))
        .replace("{AS_DATA}", &f32_array_literal(as_))
        .replace("{WS_DATA}", &f32_array_literal(ws));

    Ok(CudaGemmArtifact {
        kernel_cu,
        makefile: CUDA_MAKEFILE_TEMPLATE.replace("{KERNEL_FILE}", super::KERNEL_FILE_NAME),
        m,
        k,
        n,
    })
}

/// Format an f32 slice as a C initializer body with full round-trip precision.
fn f32_array_literal(xs: &[f32]) -> String {
    let mut s = String::with_capacity(xs.len() * 14);
    for (i, x) in xs.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        s.push_str(&format!("{:.9e}f", x));
    }
    s
}

/// Block-scaled DP4A GEMM. `A`/`W` are `[m|n][k]` row-major int8; `As`/`Ws` are `[m|n][nb]` f32 scales.
const CUDA_SCALED_GEMM_TEMPLATE: &str = r#"// Auto-generated by LOGOS codegen_cuda::matrix — do not edit.
// Block-scaled Q8_0 GEMM, O(M x N) = A(M x K) . W(N x K)^T, via Ampere DP4A with per-32-block f32 scales.
#include <cstdio>
#include <cstdint>

#define M {M}
#define N {N}
#define K {K}
#define NB {NB}   // K / 32 blocks

__device__ static const signed char A[M * K] = { {A_DATA} };
__device__ static const signed char W[N * K] = { {W_DATA} };  // W[o][t], [out][in]
__device__ static const float As[M * NB]     = { {AS_DATA} };
__device__ static const float Ws[N * NB]     = { {WS_DATA} };

__device__ static inline int pack4(const signed char *p) {
    return (p[0] & 0xff) | ((p[1] & 0xff) << 8) | ((p[2] & 0xff) << 16) | ((p[3] & 0xff) << 24);
}

__global__ void gemm_i8s(float *O) {
    int o = blockIdx.x * blockDim.x + threadIdx.x;  // output feature
    int i = blockIdx.y * blockDim.y + threadIdx.y;  // row
    if (i >= M || o >= N) return;
    float acc = 0.0f;
    for (int b = 0; b < NB; ++b) {
        int d = 0;
        for (int j = 0; j < 32; j += 4)
            d = __dp4a(pack4(&A[i * K + b * 32 + j]), pack4(&W[o * K + b * 32 + j]), d);
        acc += (float)d * As[i * NB + b] * Ws[o * NB + b];
    }
    O[i * N + o] = acc;
}

int main(void) {
    float *dO = nullptr;
    if (cudaMalloc(&dO, (size_t)M * N * sizeof(float)) != cudaSuccess) { fprintf(stderr, "cudaMalloc\n"); return 1; }
    dim3 block(16, 16);
    dim3 grid((N + block.x - 1) / block.x, (M + block.y - 1) / block.y);
    gemm_i8s<<<grid, block>>>(dO);
    cudaError_t err = cudaGetLastError();
    if (err != cudaSuccess) { fprintf(stderr, "launch: %s\n", cudaGetErrorString(err)); return 1; }
    if (cudaDeviceSynchronize() != cudaSuccess) { fprintf(stderr, "sync\n"); return 1; }
    float hO[M * N];
    if (cudaMemcpy(hO, dO, (size_t)M * N * sizeof(float), cudaMemcpyDeviceToHost) != cudaSuccess) { fprintf(stderr, "memcpy\n"); return 1; }
    for (int idx = 0; idx < M * N; ++idx) printf("%.9g\n", hO[idx]);
    cudaFree(dO);
    return 0;
}
"#;

/// Format an int8-range slice as a C `signed char` initializer body (`-128, 4, …`).
fn i8_array_literal(xs: &[i32]) -> String {
    let mut s = String::with_capacity(xs.len() * 5);
    for (i, x) in xs.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        s.push_str(&x.to_string());
    }
    s
}

/// The single-file DP4A int8 GEMM program. `{M}`/`{N}`/`{KP}` are the padded dims; `{A_DATA}`/`{WT_DATA}`
/// are the baked int8 arrays. One thread per output element; `Wᵀ` layout makes each column's contraction
/// contiguous so `__dp4a` reads 4 int8 lanes at a stride of 1.
const CUDA_GEMM_TEMPLATE: &str = r#"// Auto-generated by LOGOS codegen_cuda::matrix — do not edit.
// Weight-stationary int8 GEMM, O(M x N) = A(M x K) . W(K x N), via Ampere DP4A.
// Prints O row-major, one int32 per line — byte-comparable to codegen_sva::eval_systolic.
#include <cstdio>
#include <cstdint>

#define M {M}
#define N {N}
#define KP {KP}   // K padded up to a multiple of 4 (pad lanes are 0)

__device__ static const signed char A[M * KP]  = { {A_DATA} };
__device__ static const signed char Wt[N * KP] = { {WT_DATA} };  // Wt[c*KP + r] = W[r][c]

// Pack 4 consecutive signed int8 into an int32 (little-endian lanes), the DP4A operand form.
__device__ static inline int pack4(const signed char *p) {
    return (p[0] & 0xff) | ((p[1] & 0xff) << 8) | ((p[2] & 0xff) << 16) | ((p[3] & 0xff) << 24);
}

__global__ void gemm_i8(int *O) {
    int c = blockIdx.x * blockDim.x + threadIdx.x;  // output column
    int i = blockIdx.y * blockDim.y + threadIdx.y;  // output row
    if (i >= M || c >= N) return;
    int acc = 0;
    for (int r = 0; r < KP; r += 4) {
        acc = __dp4a(pack4(&A[i * KP + r]), pack4(&Wt[c * KP + r]), acc);
    }
    O[i * N + c] = acc;
}

int main(void) {
    int *dO = nullptr;
    if (cudaMalloc(&dO, (size_t)M * N * sizeof(int)) != cudaSuccess) { fprintf(stderr, "cudaMalloc\n"); return 1; }
    dim3 block(16, 16);
    dim3 grid((N + block.x - 1) / block.x, (M + block.y - 1) / block.y);
    gemm_i8<<<grid, block>>>(dO);
    cudaError_t err = cudaGetLastError();
    if (err != cudaSuccess) { fprintf(stderr, "launch: %s\n", cudaGetErrorString(err)); return 1; }
    if (cudaDeviceSynchronize() != cudaSuccess) { fprintf(stderr, "sync\n"); return 1; }
    int hO[M * N];
    if (cudaMemcpy(hO, dO, (size_t)M * N * sizeof(int), cudaMemcpyDeviceToHost) != cudaSuccess) { fprintf(stderr, "memcpy\n"); return 1; }
    for (int idx = 0; idx < M * N; ++idx) printf("%d\n", hO[idx]);
    cudaFree(dO);
    return 0;
}
"#;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_cuda/matrix/tests.rs"]
mod tests;
