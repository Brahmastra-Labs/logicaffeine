//! The lane's kernel library: one CUDA kernel per kind of op a step runs, each templated over the forms the fusion
//! rules ask of it (a residual added before a norm, an activation quantized in a producer's epilogue, a result written
//! into the row of a cache a position names). The planner instantiates them; no kernel here knows a model.
//!
//! Every kernel computes what the language's sugar means, the formula the VM runs: an RMSNorm divides by
//! `sqrt(mean + 1e-6)` then scales, an attention scores `(q·k)/√hd` and normalizes by a max-subtracted softmax, an
//! arg-max keeps the first of equal maxima. Accumulation is `f32` in a fixed order (the same inputs give the same
//! bits on every run); the projections are the one place the numerics are a choice — [`Q8MatKind`] picks the
//! activation precision of every Q8_0 GEMV:
//!
//!   • `Dequant` — the activation stays `f32`, the int8 weight is scaled in the kernel, the dot accumulates in `f64`:
//!     the precise lane, logits a hair from the `f64` reference;
//!   • `Int8Act` — the activation quantized to int8 per 32-block and multiplied by `__dp4a` against the weight
//!     (llama.cpp's MMVQ shape): the fast lane;
//!   • `TwoPassInt8` — the activation split into a high and a low int8 word, two `__dp4a` per block: ~14-bit
//!     activations at the fast lane's memory cost.
//!
//! A quantized activation is a group of 32 values sharing one scale `s = max|v| / 127`; the high word is
//! `round(v / s)`, the low word `round((v − hi·s) / (s/128))` with `v − hi·s` one fused multiply-add, as the `f64`
//! reference computes it.
//!
//! A Q8_0 matrix is uploaded split, its int8 values in one array and its block scales in another, so the GEMV reads
//! 16-byte words; an MXFP4 matrix's blocks are regrouped, each row's sub-blocks 16 to a group, the group's exponents
//! before its nibbles ([`row_bytes`]), so each sub-block's nibbles are one aligned 16-byte word; a Q6_K matrix's blocks
//! eight to a group, the group's f16 scales before its blocks' other 208 bytes, so every block's levels are aligned
//! decodes a 32-weight sub-block of a row into int8 levels with a scale and an offset per 16 — `weight = scale·level +
//! offset`, exactly the CPU dequantizer's arithmetic — so one row dot serves every format under every numerics: the
//! offset term multiplies the activation's sum, which the int8 lanes take with a `__dp4a` against ones.
//!
//! [`Q8MatKind`]: crate::codegen_cuda::qwen_emit::Q8MatKind

use crate::codegen_cuda::qwen_emit::Q8MatKind;

/// The template argument that names a numerics mode in the library: 0 `f32` activations, 1 int8, 2 two-pass int8.
pub fn mode(kind: Q8MatKind) -> u32 {
    match kind {
        Q8MatKind::Dequant => 0,
        Q8MatKind::Int8Act => 1,
        Q8MatKind::TwoPassInt8 => 2,
    }
}

/// The numerics a per-head matmul — a latent's expansion half — runs under: its activation `f32` under every numerics,
/// with `f64` sums under the precise one (0) and `f32` sums under the quantized ones (3), so the absorbed latent
/// attention computes what the expanded one does.
pub fn head_mode(kind: Q8MatKind) -> u32 {
    match kind {
        Q8MatKind::Dequant => 0,
        Q8MatKind::Int8Act | Q8MatKind::TwoPassInt8 => 3,
    }
}

/// The helpers every kernel shares: the storage types, row addressing, warp and block reductions, the activation
/// quantizer.
pub const PRELUDE: &str = r#"
#include <cstddef>
#include <cuda_fp16.h>
#include <cuda_bf16.h>
#include <cuda_fp8.h>
// An element as the storage type `T` holds it — `f32` as computed, binary16 and bfloat16 rounded to the nearest, ties
// to even, fp8 e4m3 rounded so and saturating at ±448 — and as the `f32` every kernel computes in, widened exactly.
template<typename T> __device__ __forceinline__ T lk_to(float v){ return v; }
template<> __device__ __forceinline__ __half lk_to<__half>(float v){ return __float2half_rn(v); }
template<> __device__ __forceinline__ __nv_bfloat16 lk_to<__nv_bfloat16>(float v){ return __float2bfloat16_rn(v); }
template<> __device__ __forceinline__ __nv_fp8_e4m3 lk_to<__nv_fp8_e4m3>(float v){ return __nv_fp8_e4m3(v); }
__device__ __forceinline__ float lk_from(float v){ return v; }
__device__ __forceinline__ float lk_from(__half v){ return __half2float(v); }
__device__ __forceinline__ float lk_from(__nv_bfloat16 v){ return __bfloat162float(v); }
__device__ __forceinline__ float lk_from(__nv_fp8_e4m3 v){ return float(v); }
template<typename T> __device__ __forceinline__ float lk_kept(float v){ return lk_from(lk_to<T>(v)); }
// Two and four consecutive elements from `p` as `f32`: one load of their bytes, eight-byte aligned for four binary16
// values as for two `f32`.
__device__ __forceinline__ float2 lk_from2(const float* p){ return *(const float2*)p; }
__device__ __forceinline__ float2 lk_from2(const __half* p){ return __half22float2(*(const __half2*)p); }
__device__ __forceinline__ float2 lk_from2(const __nv_fp8_e4m3* p){ return float2(*(const __nv_fp8x2_e4m3*)p); }
__device__ __forceinline__ float4 lk_from4(const float* p){ return *(const float4*)p; }
__device__ __forceinline__ float4 lk_from4(const __half* p){
  uint2 u = *(const uint2*)p;
  float2 a = __half22float2(*(const __half2*)&u.x), b = __half22float2(*(const __half2*)&u.y);
  return make_float4(a.x, a.y, b.x, b.y);
}
__device__ __forceinline__ float4 lk_from4(const __nv_fp8_e4m3* p){ return float4(*(const __nv_fp8x4_e4m3*)p); }
// The storage a result's array takes: `f32`, binary16, fp8 e4m3.
enum lk_store : unsigned char { LK_F32, LK_F16, LK_E4M3 };
// One element of a result's array as it is written: `f32`, or rounded to the nearest of its narrower float.
struct lk_ref {
  void* p; lk_store t;
  __device__ __forceinline__ void operator=(float v) const {
    if(t == LK_F16) *(__half*)p = __float2half_rn(v);
    else if(t == LK_E4M3) *(__nv_fp8_e4m3*)p = __nv_fp8_e4m3(v);
    else *(float*)p = v;
  }
};
// A result's array — `f32`, or a narrower float that every value written is rounded to — taken from a typed pointer,
// so a kernel writes any of them through one pointer type. It can be offset and written, never read.
struct lk_out {
  void* p = nullptr; lk_store t = LK_F32;
  lk_out() = default;
  __host__ __device__ lk_out(std::nullptr_t) {}
  __host__ __device__ lk_out(float* q) : p(q) {}
  __host__ __device__ lk_out(__half* q) : p(q), t(LK_F16) {}
  __host__ __device__ lk_out(__nv_fp8_e4m3* q) : p(q), t(LK_E4M3) {}
  __host__ __device__ explicit operator bool() const { return p != nullptr; }
  __device__ __forceinline__ void* at(long i) const {
    return t == LK_F16 ? (void*)((__half*)p + i) : t == LK_E4M3 ? (void*)((__nv_fp8_e4m3*)p + i) : (void*)((float*)p + i);
  }
  __device__ __forceinline__ lk_out operator+(long i) const { lk_out o; o.p = at(i); o.t = t; return o; }
  __device__ __forceinline__ lk_ref operator[](long i) const { return lk_ref{at(i), t}; }
};
// A result written into the row a 1-based position names, or the buffer itself when no position is given.
#define LK_ROW(p, row, pitch) ((p) + ((row) ? (long)(*(row) - 1) * (long)(pitch) : 0L))
// A product's epilogue on its output `v` at row `t`, column `c` of `n`: plus the bias `b[c]`, times the scale — `s`, or
// `cs[c]` a column's own, or `cs[t·n + c]` each output's own (`each`) — plus `b2` (a row, or each output's own:
// `each2`) (`gemm_scale`), then squared past its ReLU (`sq`, `relu_sqr`, as `lk_relu` squares it), each step rounded as
// its own launch rounds it. A bias pointer alone converts to one.
struct lk_epi {
  const float* b = nullptr; float s = 1.f; const float* cs = nullptr; bool each = false; const float* b2 = nullptr;
  bool each2 = false; bool sq = false;
  lk_epi() = default;
  __host__ __device__ lk_epi(std::nullptr_t) {}
  __host__ __device__ lk_epi(const float* bias) : b(bias) {}
  __host__ __device__ lk_epi(const float* bias, float scale, const float* scales, bool own, const float* after, bool own_after,
                             bool square)
      : b(bias), s(scale), cs(scales), each(own), b2(after), each2(own_after), sq(square) {}
  __device__ __forceinline__ float operator()(float v, long t, long c, long n) const {
    if(b) v = __fadd_rn(v, b[c]);
    if(cs) v = __fmul_rn(v, cs[each ? t * n + c : c]);
    else if(s != 1.f) v = __fmul_rn(v, s);
    if(b2) v = __fadd_rn(v, b2[each2 ? t * n + c : c]);
    if(sq){ float r = v > 0.f ? v : 0.f; v = __fmul_rn(r, r); }
    return v;
  }
};

__device__ __forceinline__ float lk_warp_sum(float v){
  for(int o = 16; o > 0; o >>= 1) v += __shfl_down_sync(0xffffffffu, v, o);
  return __shfl_sync(0xffffffffu, v, 0);
}
__device__ __forceinline__ double lk_warp_sum_d(double v){
  for(int o = 16; o > 0; o >>= 1) v += __shfl_down_sync(0xffffffffu, v, o);
  return __shfl_sync(0xffffffffu, v, 0);
}
__device__ __forceinline__ float lk_warp_max(float v){
  for(int o = 16; o > 0; o >>= 1) v = fmaxf(v, __shfl_down_sync(0xffffffffu, v, o));
  return __shfl_sync(0xffffffffu, v, 0);
}
// A product, a sum and a fused multiply-add each rounded once as written, in `float` or `double`: a value two kernels
// must compute alike is written in these, so no compiler fuses one kernel's product into its sum and not the other's.
__device__ __forceinline__ float lk_fma(float a, float b, float c){ return __fmaf_rn(a, b, c); }
__device__ __forceinline__ double lk_fma(double a, double b, double c){ return __fma_rn(a, b, c); }
__device__ __forceinline__ float lk_add(float a, float b){ return __fadd_rn(a, b); }
__device__ __forceinline__ double lk_add(double a, double b){ return __dadd_rn(a, b); }
__device__ __forceinline__ float lk_mul(float a, float b){ return __fmul_rn(a, b); }
__device__ __forceinline__ double lk_mul(double a, double b){ return __dmul_rn(a, b); }
// Output `i` of a warp's `N`, which every lane holds (each dot's sums reach every lane), by selection. An epilogue takes
// them a lane each (`for(int i = lane; i < N; i += 32)`), so the loads and stores of a warp's rows — a residual read
// where a row was just written, a convolution's ring — go out together, not each row's after the last's.
template<int N, typename T> __device__ __forceinline__ T lk_pick(const T (&d)[N], int i){
  T v = d[0];
  #pragma unroll
  for(int j = 1; j < N; ++j) v = i == j ? d[j] : v;
  return v;
}
__device__ __forceinline__ double lk_warp_sum(double v){ return lk_warp_sum_d(v); }
// The `V` values each lane holds summed over the warp's lanes, `V` a power of two to 32: lane `l` returns the total of
// value `l % V`. At a distance `W` below `V` each lane keeps one half of its values and sends the other, the halves
// summed apart; at and past `V` the whole set is summed. Starting at a distance `W` below 16 sums over each run of
// `2·W` lanes apart.
template<int V, int W = 16>
__device__ __forceinline__ float lk_lane_sums(float (&v)[V]){
  if constexpr(W >= V){
    #pragma unroll
    for(int i = 0; i < V; ++i) v[i] += __shfl_xor_sync(0xffffffffu, v[i], W);
  } else {
    bool up = threadIdx.x & W;
    #pragma unroll
    for(int i = 0; i < W; ++i){
      float send = up ? v[i] : v[i + W], keep = up ? v[i + W] : v[i];
      v[i] = keep + __shfl_xor_sync(0xffffffffu, send, W);
    }
  }
  if constexpr(W > 1) return lk_lane_sums<V, W / 2>(v);
  else return v[0];
}
// The precise numerics (Q 0) sum in f64, the quantized in f32.
template<int Q> struct lk_acc { typedef float t; };
template<> struct lk_acc<0> { typedef double t; };
// The products that read their activation `f32`: every one under the precise numerics (Q 0), and under Q 3 — the
// latent's expansion halves under the quantized numerics, which sum in f32.
template<int Q> __host__ __device__ constexpr bool lk_f32_acts(){ return Q == 0 || Q == 3; }
// Every thread of the block gets the total over `vw` warps, in the accumulator `A` (`float` or `double`); `red` is 32 of
// them in shared memory. Warp `w` of the block's `nw` plays the warps `w`, `w + nw`, … below `vw`, value `j` of each
// lane its lane's of the `j`-th: each warp summed over its lanes, then the warps' sums in order — the tree a block of
// `vw` warps sums its values by.
template<int VW, typename A>
__device__ __forceinline__ A lk_warps_sum(const A (&v)[VW], int vw, A* red){
  int lane = threadIdx.x & 31, warp = threadIdx.x >> 5, nw = blockDim.x >> 5;
  A s[VW];
  #pragma unroll
  for(int j = 0; j < VW; ++j) s[j] = lk_warp_sum(v[j]);
  __syncthreads();
  #pragma unroll
  for(int j = 0; j < VW; ++j) if(lane == 0 && warp + j * nw < vw) red[warp + j * nw] = s[j];
  __syncthreads();
  return lk_warp_sum(lane < vw ? red[lane] : (A)0);
}
// Every thread of the block gets the block's total.
template<typename A>
__device__ __forceinline__ A lk_block_sum(A v, A* red){
  const A one[1] = {v};
  return lk_warps_sum<1>(one, (blockDim.x + 31) >> 5, red);
}
__device__ __forceinline__ float lk_block_max(float v, float* red){
  int lane = threadIdx.x & 31, warp = threadIdx.x >> 5, nw = (blockDim.x + 31) >> 5;
  v = lk_warp_max(v);
  __syncthreads();
  if(lane == 0) red[warp] = v;
  __syncthreads();
  return lk_warp_max(lane < nw ? red[lane] : -INFINITY);
}
// An activation: ACT 0 silu `x / (1 + e^-x)`, 1 sigmoid `1 / (1 + e^-x)`, 2 gelu in ggml's tanh form
// `x / (1 + e^-2z)`, `z = √(2/π)·(x + 0.044715·x³)`.
template<int ACT>
__device__ __forceinline__ float lk_act(float v){
  if(ACT == 0) return v / (1.0f + expf(-v));
  if(ACT == 1) return 1.0f / (1.0f + expf(-v));
  float z = 0.7978845608028654f * (v + 0.044715f * v * v * v);
  return v / (1.0f + expf(-2.0f * z));
}
// The first cached row a query attending `end` rows keeps under its window `win`: the last `win` (a sliding window, `win`
// above 0), those of its own chunk of `-win` rows (a chunked-local window, `win` below 0), or every one (0).
__device__ __forceinline__ int lk_window_lo(int win, int end){ return win > 0 ? (end > win ? end - win : 0) : win < 0 ? (end - 1) / -win * -win : 0; }
// A value soft-capped at `c`, `c·tanh(v / c)` (a score before its softmax, the logits); `v` itself where `c` is 0.
__device__ __forceinline__ float lk_cap(float v, float c){ return c > 0.f ? c * tanhf(v / c) : v; }
__device__ __forceinline__ double lk_cap(double v, double c){ return c > 0.0 ? c * tanh(v / c) : v; }
// The two-word scale of values whose largest magnitude is `a`, and value `v`'s two words under scale `s`, by quotients:
// `v ≈ s·(h + l/128)` (the two-word activation quantizer's); by products with the scale's reciprocal `r` where a kernel
// takes many values under one scale; four words packed into one, the first lowest.
__device__ __forceinline__ float lk_two_scale(float a){ return (a > 1e-30f ? a : 1e-30f) / 127.0f; }
__device__ __forceinline__ void lk_two_words(float v, float s, int& h, int& l){
  h = __float2int_rn(v / s); h = h > 127 ? 127 : (h < -128 ? -128 : h);
  l = __float2int_rn(__fmaf_rn(-(float)h, s, v) / (s * 0.0078125f)); l = l > 127 ? 127 : (l < -128 ? -128 : l);
}
__device__ __forceinline__ void lk_two_words_r(float v, float s, float r, int& h, int& l){
  h = __float2int_rn(v * r); h = h > 127 ? 127 : (h < -128 ? -128 : h);
  l = __float2int_rn(__fmaf_rn(-(float)h, s, v) * r * 128.f); l = l > 127 ? 127 : (l < -128 ? -128 : l);
}
__device__ __forceinline__ int lk_int8x4(int a, int b, int c, int d){
  return (int)((unsigned)(a & 255) | ((unsigned)(b & 255) << 8) | ((unsigned)(c & 255) << 16) | ((unsigned)(d & 255) << 24));
}
// Quantize the 32-value group this warp holds, one value per lane, as group `g`.
template<int Q>
__device__ __forceinline__ void lk_quant_lane(float v, signed char* __restrict__ qh, signed char* __restrict__ ql,
                                              float* __restrict__ qs, long i, long g){
  float a = lk_warp_max(fabsf(v));
  if(Q == 1){
    float s = a > 0.f ? a / 127.0f : 1.0f;
    if((threadIdx.x & 31) == 0) qs[g] = s;
    qh[i] = (signed char)__float2int_rn(v / s);
  } else {
    float s = lk_two_scale(a);
    int h, l;
    lk_two_words(v, s, h, l);
    if((threadIdx.x & 31) == 0) qs[g] = s;
    qh[i] = (signed char)h; ql[i] = (signed char)l;
  }
}
// Quantize a pair of values `i`, `i + 1` of a 32-value group sixteen lanes of a warp hold, a pair a lane, as group
// `i >> 5` — the group's largest magnitude taken over the sixteen lanes on every lane (a lane `on` false holds none of
// it), and each value and the scale then what `lk_quant_lane` makes of the group.
template<int Q>
__device__ __forceinline__ void lk_quant_pair(float x, float y, bool on, signed char* __restrict__ qh, signed char* __restrict__ ql,
                                              float* __restrict__ qs, long i){
  float a = on ? fmaxf(fabsf(x), fabsf(y)) : 0.f;
  #pragma unroll
  for(int o = 8; o > 0; o >>= 1) a = fmaxf(a, __shfl_xor_sync(0xffffffffu, a, o));
  if(!on) return;
  if(Q == 1){
    float s = a > 0.f ? a / 127.0f : 1.0f;
    if((threadIdx.x & 15) == 0) qs[i >> 5] = s;
    qh[i] = (signed char)__float2int_rn(x / s);
    qh[i + 1] = (signed char)__float2int_rn(y / s);
  } else {
    float s = lk_two_scale(a);
    int h, l;
    lk_two_words(x, s, h, l);
    qh[i] = (signed char)h; ql[i] = (signed char)l;
    lk_two_words(y, s, h, l);
    qh[i + 1] = (signed char)h; ql[i + 1] = (signed char)l;
    if((threadIdx.x & 15) == 0) qs[i >> 5] = s;
  }
}
// Where an attention quantizes its output for the projection that reads it: its words, low words and scales.
struct lk_qout { signed char* q; signed char* l; float* s; const float* g; int ga; };
// Values `i`, `i + 1` of an attention's output: gated by `lk_act` (`ga`) of the gate's own values `g[i]`, `g[i + 1]` where
// `g` is given, as a product with the activation computes it; written `f32` where `out` is given; quantized under `QO`
// into `qo` (`lk_quant_pair`) — every lane of the warp calls it, `on` the lanes that hold a pair.
template<int QO>
__device__ __forceinline__ void lk_attn_store(float* __restrict__ out, lk_qout qo, long i, bool on, float x, float y){
  if(on && qo.g){
    const float gx = qo.g[i], gy = qo.g[i + 1];
    x *= qo.ga == 0 ? lk_act<0>(gx) : qo.ga == 1 ? lk_act<1>(gx) : lk_act<2>(gx);
    y *= qo.ga == 0 ? lk_act<0>(gy) : qo.ga == 1 ? lk_act<1>(gy) : lk_act<2>(gy);
  }
  if(on && out) *(float2*)(out + i) = make_float2(x, y);
  if(QO > 0) lk_quant_pair<QO>(x, y, on, qo.q, qo.l, qo.s, i);
}
// One step of a Q8_0 dot's partial sum, as its two factors: the partial `p` becomes `fmaf(a, b, p)` for the int8 dot
// `d` (and, in two words, the low words' `e`) of a block of weight scale `w` and activation scale `s`, each operation
// spelled out, so every kernel that sums as the GEMV sums takes the same steps (`lk_q8_acc` takes one).
template<int Q>
__device__ __forceinline__ float2 lk_q8_term(float w, float s, float d, float e){
  if(Q == 1) return make_float2(__fmul_rn(w, s), d);
  return make_float2(w, __fmaf_rn(s, d, __fmul_rn(__fmul_rn(s, 0.0078125f), e)));
}
template<int Q>
__device__ __forceinline__ float lk_q8_acc(float acc, float w, float s, float d, float e){
  const float2 t = lk_q8_term<Q>(w, s, d, e);
  return __fmaf_rn(t.x, t.y, acc);
}
// The four partials of a quantized Q8_0 dot summed in the one order every kernel that sums as the GEMV sums takes.
__device__ __forceinline__ float lk_q8_sum4(float p0, float p1, float p2, float p3){ return (p0 + p2) + (p1 + p3); }
// The int8 dot of one 32-value block two lanes hold sixteen values each of (`w` and `x` their words), exact, in both.
__device__ __forceinline__ int lk_q8_block_dot(int4 w, int4 x){
  int d = __dp4a(w.x, x.x, 0) + __dp4a(w.y, x.y, 0) + __dp4a(w.z, x.z, 0) + __dp4a(w.w, x.w, 0);
  return d + __shfl_xor_sync(0xffffffffu, d, 1);
}
// The scratch, in `float2`, a warp of a quantized Q8_0 GEMV takes for `U` dots at once over more than one activation
// row: a round's steps of up to eight dots at a time, a dot's sixteen 20 apart, so a half-warp's chain reads fall on
// distinct banks.
template<int U> __host__ __device__ constexpr int lk_q8_scratch(){ return (U < 8 ? U : 8) * 20; }
// Round `c` of rows `o … o + W − 1` dotted with one activation row (`lk_q8_rows_dot`): its first `n` blocks' steps
// into each row's partials in lanes `2q`, chained in order from pairs `q`, `q + 4`, `q + 8`, `q + 12` by `shfl_down`;
// a whole round's sixteen (`WHOLE`) need no test of the blocks past the row's end.
template<int Q, int W, bool WHOLE>
__device__ __forceinline__ void lk_q8_round(float (&acc)[W], const signed char* __restrict__ wq, const __half* __restrict__ ws, long o, int K,
                                            const signed char* __restrict__ xq, const signed char* __restrict__ xl, const float* __restrict__ xs,
                                            int c, int n){
  const int lane = threadIdx.x & 31, j = lane >> 1, nb = K >> 5, blk = c * 16 + j;
  const bool in = WHOLE || j < n;
  const int4 zero = make_int4(0, 0, 0, 0);
  const int4 h = in ? ((const int4*)xq)[c * 32 + lane] : zero;
  const int4 l = Q == 2 && in ? ((const int4*)xl)[c * 32 + lane] : zero;
  const float s = in ? xs[blk] : 0.f;
  #pragma unroll
  for(int r = 0; r < W; ++r){
    const int4 w = in ? ((const int4*)(wq + (o + r) * K))[c * 32 + lane] : zero;
    const float wsc = in ? __half2float(ws[(o + r) * nb + blk]) : 0.f;
    const int d = lk_q8_block_dot(w, h), e = Q == 2 ? lk_q8_block_dot(w, l) : 0;
    const float2 t = lk_q8_term<Q>(wsc, s, (float)d, (float)e);
    if(in) acc[r] = __fmaf_rn(t.x, t.y, acc[r]);
    #pragma unroll
    for(int k = 1; k < 4; ++k){
      const float a = __shfl_down_sync(0xffffffffu, t.x, 8 * k), b = __shfl_down_sync(0xffffffffu, t.y, 8 * k);
      if(WHOLE || j + 4 * k < n) acc[r] = __fmaf_rn(a, b, acc[r]);
    }
  }
}
// Rows `o … o + W − 1` of a Q8_0 matrix (int8 values `wq`, a half scale per 32 in `ws`) dotted with the `M` activation
// rows of `K` values — one warp of a block of at most `WARPS` — row `r`'s dots at `out + r·M`, in every lane. Under the
// precise numerics (Q 0) the activations are `f32` and the sums `f64`, four values a lane at a time. Quantized (int8
// words, two under Q 2), each 32-value block's dot is exact in int32 and its step (`lk_q8_term`) goes into partial
// `b mod 4` of block `b`, a partial's blocks in order; the four partials sum as `lk_q8_sum4` sums them. The row goes
// 512 values (16 blocks) a round, sixteen bytes a lane: lanes `2j`, `2j + 1` block `j` of each (`lk_q8_block_dot`),
// so a round's steps of partial `q` are pairs `q`, `q + 4`, `q + 8` and `q + 12`'s. With one activation row each row's
// partials chain in lanes `2q`, reading the steps by `shfl_down`; with more, the round's steps go through the warp's
// scratch (`lk_q8_scratch`), up to eight dots' a time, and lane `4u + q` chains the time's dot `u`'s partial `q`. The
// blocks past the row's end are none of its partials'.
template<int Q, int M, int W, int WARPS = 8>
__device__ __forceinline__ void lk_q8_rows_dot(const signed char* __restrict__ wq, const __half* __restrict__ ws, long o, int K,
                                               const float* __restrict__ x, const signed char* __restrict__ xq,
                                               const signed char* __restrict__ xl, const float* __restrict__ xs, float* out){
  const int lane = threadIdx.x & 31, nb = K >> 5;
  if constexpr(Q == 0){
    double acc[W][M];
    #pragma unroll
    for(int r = 0; r < W; ++r)
      #pragma unroll
      for(int t = 0; t < M; ++t) acc[r][t] = 0.0;
    for(int g = lane; g < (K >> 2); g += 32){
      char4 w[W];
      double sc[W];
      #pragma unroll
      for(int r = 0; r < W; ++r){ w[r] = ((const char4*)(wq + (o + r) * K))[g]; sc[r] = (double)__half2float(ws[(o + r) * nb + (g >> 3)]); }
      #pragma unroll
      for(int t = 0; t < M; ++t){
        const float* xt = x + (long)t * K + 4 * g;
        float x0 = xt[0], x1 = xt[1], x2 = xt[2], x3 = xt[3];
        #pragma unroll
        for(int r = 0; r < W; ++r) acc[r][t] += sc[r] * ((double)w[r].x * x0 + (double)w[r].y * x1 + (double)w[r].z * x2 + (double)w[r].w * x3);
      }
    }
    #pragma unroll
    for(int r = 0; r < W; ++r)
      #pragma unroll
      for(int t = 0; t < M; ++t) out[r * M + t] = (float)lk_warp_sum_d(acc[r][t]);
  } else if constexpr(M == 1){
    float acc[W];
    #pragma unroll
    for(int r = 0; r < W; ++r) acc[r] = 0.f;
    const int full = nb >> 4, rest = nb & 15;
    for(int c = 0; c < full; ++c) lk_q8_round<Q, W, true>(acc, wq, ws, o, K, xq, xl, xs, c, 16);
    if(rest) lk_q8_round<Q, W, false>(acc, wq, ws, o, K, xq, xl, xs, full, rest);
    #pragma unroll
    for(int r = 0; r < W; ++r){
      float v = acc[r];
      v += __shfl_down_sync(0xffffffffu, v, 4);
      v += __shfl_down_sync(0xffffffffu, v, 2);
      out[r] = __shfl_sync(0xffffffffu, v, 0);
    }
  } else {
    constexpr int U = W * M, G = U < 8 ? U : 8, R = (U + G - 1) / G;
    __shared__ float2 lk_q8s[WARPS][lk_q8_scratch<U>()];
    const int warp = threadIdx.x >> 5, j = lane >> 1, u0 = lane >> 2, q = lane & 3;
    if(warp >= WARPS) __trap();
    float2* sm = lk_q8s[warp];
    float acc[R];
    #pragma unroll
    for(int i = 0; i < R; ++i) acc[i] = 0.f;
    const int4 zero = make_int4(0, 0, 0, 0);
    for(int c = 0; c * 16 < nb; ++c){
      const int blk = c * 16 + j, n = min(16, nb - c * 16);
      const bool in = j < n;
      int4 w[W];
      float wsc[W];
      #pragma unroll
      for(int r = 0; r < W; ++r){
        w[r] = in ? ((const int4*)(wq + (o + r) * K))[c * 32 + lane] : zero;
        wsc[r] = in ? __half2float(ws[(o + r) * nb + blk]) : 0.f;
      }
      float2 st[U];
      #pragma unroll
      for(int t = 0; t < M; ++t){
        const int4 h = in ? ((const int4*)(xq + (long)t * K))[c * 32 + lane] : zero;
        const int4 l = Q == 2 && in ? ((const int4*)(xl + (long)t * K))[c * 32 + lane] : zero;
        const float s = in ? xs[(long)t * nb + blk] : 0.f;
        #pragma unroll
        for(int r = 0; r < W; ++r){
          const int d = lk_q8_block_dot(w[r], h), e = Q == 2 ? lk_q8_block_dot(w[r], l) : 0;
          st[r * M + t] = lk_q8_term<Q>(wsc[r], s, (float)d, (float)e);
        }
      }
      // Up to eight dots' steps a time: pair `j` writes its block's (the even lane an even dot's, the odd an odd's),
      // then lane `4u + q` chains dot `u`'s partial `q` over blocks `q`, `q + 4`, `q + 8`, `q + 12`.
      #pragma unroll
      for(int i = 0; i < R; ++i){
        #pragma unroll
        for(int g = 0; g < G; ++g) if(i * G + g < U && (lane & 1) == (g & 1)) sm[g * 20 + j] = st[i * G + g];
        __syncwarp();
        if(u0 < G && i * G + u0 < U){
          #pragma unroll
          for(int k = 0; k < 4; ++k) if(q + 4 * k < n){ const float2 v = sm[u0 * 20 + q + 4 * k]; acc[i] = __fmaf_rn(v.x, v.y, acc[i]); }
        }
        __syncwarp();
      }
    }
    #pragma unroll
    for(int i = 0; i < R; ++i){
      float v = acc[i];
      v += __shfl_down_sync(0xffffffffu, v, 2);
      v += __shfl_down_sync(0xffffffffu, v, 1);
      #pragma unroll
      for(int g = 0; g < G; ++g) if(i * G + g < U) out[i * G + g] = __shfl_sync(0xffffffffu, v, 4 * g);
    }
  }
}
"#;

/// A stream held back for a span of the global timer. Under `--stagger <seed>` one runs before each launch of a step, its
/// span drawn from the seed, the form and the launch, so launches its streams leave unordered run in another order
/// under each seed.
pub const STALL: &str = r#"
__global__ void lk_stall(unsigned long long ns){
  unsigned long long t0, t;
  asm volatile("mov.u64 %0, %%globaltimer;" : "=l"(t0));
  do { __nanosleep(256); asm volatile("mov.u64 %0, %%globaltimer;" : "=l"(t)); } while(t - t0 < ns);
}
// The stall before launch `i` of the form of `rows` rows under `seed`: under 128 µs, drawn by splitmix64.
static unsigned long long lk_stall_ns(unsigned long long seed, unsigned long long rows, unsigned long long i){
  unsigned long long z = seed * 0x9e3779b97f4a7c15ULL + rows * 0xbf58476d1ce4e5b9ULL + i * 0x94d049bb133111ebULL;
  z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ULL;
  z = (z ^ (z >> 27)) * 0x94d049bb133111ebULL;
  return (z ^ (z >> 31)) % 128000ULL;
}
"#;

/// Two projections of one activation through a gate, `y = act(x·Wg)·(x·Wu)`: a warp `W` consecutive output rows dots
/// their gate and up rows (formats `FG`, `FU`) with each of the `M` activation rows as the plain GEMV of each format dots
/// them (`lk_gemv_dot`), adds their biases, and gates them — `ACT` −1 SwiGLU as `lk_swiglu` computes it, otherwise
/// `u·act(g)` as the gated product `lk_binary` computes it. With a quantized output (`QO`) a block of `32 / W` warps
/// takes 32 outputs, one quantized block a row, quantized as `lk_quant_lane` quantizes it; without, a block of 8 warps
/// takes `8·W`.
const GLU: &str = r#"
template<int FG, int FU, int Q, int M, int ACT, bool OUTF, int QO, int W = 1>
__global__ void __launch_bounds__(QO > 0 ? 1024 / W : 256) lk_gemv_glu(lk_w wg, lk_w wu, const float* __restrict__ x,
    const signed char* __restrict__ xq, const signed char* __restrict__ xl, const float* __restrict__ xs,
    const float* __restrict__ bg, const float* __restrict__ bu, float* __restrict__ y, signed char* __restrict__ yq,
    signed char* __restrict__ yl, float* __restrict__ ys, int K, int N){
  __shared__ float v[M][32];
  int lane = threadIdx.x & 31, warp = threadIdx.x >> 5;
  long o = ((long)blockIdx.x * (blockDim.x >> 5) + warp) * W;
  if(o < N){
    // Each hidden value from its gate and up sums, plus their biases, through the gated activation.
    auto out = [&](int r, int t, float gs, float us){
      float gv = bg ? gs + bg[o + r] : gs, uv = bu ? us + bu[o + r] : us;
      float val = ACT < 0 ? gv / (1.0f + expf(-gv)) * uv : uv * lk_act<(ACT < 0 ? 0 : ACT)>(gv);
      if(QO > 0) v[t][warp * W + r] = val;
      else if(OUTF) y[(long)t * N + o + r] = val;
    };
    if constexpr(FG == FU && lk_h16_lanes<FG, Q>()){
      float cg[lk_h16_tiles<W, M>()][4], cu[lk_h16_tiles<W, M>()][4];
      lk_h16_mma_pair<FG, Q, W, M>(wg, wu, o, K, x, K, cg, cu);
      lk_h16_each<W, M>([&](int r, int t, int n, int e){ out(r, t, cg[n][e], cu[n][e]); });
    } else {
      float g[W * M], u[W * M];
      lk_gemv_dot<FG, Q, M, W, (QO > 0 ? 32 / W : 8)>(wg, o, K, x, xq, xl, xs, g);
      lk_gemv_dot<FU, Q, M, W, (QO > 0 ? 32 / W : 8)>(wu, o, K, x, xq, xl, xs, u);
      for(int i = lane; i < W * M; i += 32) out(i / M, i % M, lk_pick(g, i), lk_pick(u, i));
    }
  }
  if(QO > 0){
    __syncthreads();
    if(warp < M){
      long at = (long)warp * N + (long)blockIdx.x * 32 + lane;
      float val = v[warp][lane];
      if(OUTF) y[at] = val;
      lk_quant_lane<QO>(val, yq, yl, ys, at, at >> 5);
    }
  }
}
"#;

/// The positions the attention scores at a time (`LK_TILE` in its source).
pub const ATTN_TILE: usize = 64;

/// The fewest positions a split of an attention's span takes: below it a split's partial costs more than its scores.
pub const ATTN_MIN_SPLIT: usize = 32;

/// The key and value elements an attention block reads a position, across its heads: four latent-attention heads'
/// worth (`(576 + 512) · 4`), which scored best from a thousand positions on (the 3060's attention measurements),
/// against two heads' at a hundred.
pub const ATTN_BLOCK_VALUES: usize = 4352;

/// The dynamic shared memory an attention block takes: its `g` query heads of `hd` (`f32`), then — in accumulators of
/// `acc` bytes — their scores over a tile and each head's maximum, sum and fold factor over each of `splits` chunks.
pub fn attn_smem_bytes(g: usize, hd: usize, splits: usize, acc: usize) -> usize {
    (g * hd * 4).next_multiple_of(16) + (g * ATTN_TILE + 3 * g * splits + attn_warp_slots(g, hd, acc)) * acc
}

/// The values of a key a lane of a warp-local block takes (`lk_attn_lane_values`): eight, four or two — the most whose
/// `g` heads' weighed values in accumulators of `acc` bytes stay within 32 registers — and never under a warp's share of
/// a head `hd` wide; none where even that share does not fit.
pub fn attn_lane_values(g: usize, hd: usize, acc: usize) -> usize {
    if g * 8 * acc <= 128 {
        8
    } else if g * 4 * acc <= 128 && hd <= 128 {
        4
    } else if g * 2 * acc <= 128 && hd <= 64 {
        2
    } else {
        0
    }
}

/// Whether a block of `g` heads `hd` wide summing in `acc`-byte accumulators takes its chunks warp-local
/// (`lk_attn_warp_heads`, the kernel's `lk_attn_warps` where its values are as wide and its rows start sixteen bytes in
/// step): `f32` sums, at most eight heads, 64, 128 or 256 wide, a lane's share of their weighed values within its
/// registers (`attn_lane_values`) — eight heads 256 wide spilled 1.8 KB, and a decode step took 595 µs on the 3060
/// against the tile chunks' 149. `f64` sums take the tile chunks: every lane of a run repeats its heads' `f64`
/// arithmetic, which on the 3060 took 1.6 to 3.6 times the tile chunks' time.
pub fn attn_warp_heads(g: usize, hd: usize, acc: usize) -> bool {
    acc == 4 && g <= 8 && matches!(hd, 64 | 128 | 256) && attn_lane_values(g, hd, acc) > 0
}

/// The heads whose values a warp-local block's warps combine at once (`lk_attn_warp_pass`): at most 512 values a warp.
pub fn attn_warp_pass(g: usize, hd: usize) -> usize {
    (512 / hd).max(1).min(g)
}

/// The values a warp-local block combines its warps' partials in (`lk_attn_warp_slots`): each of its eight warps' maxima
/// and sums, then each warp's values of a pass of heads.
pub fn attn_warp_slots(g: usize, hd: usize, acc: usize) -> usize {
    if attn_warp_heads(g, hd, acc) { 16 * g + 8 * attn_warp_pass(g, hd) * hd } else { 0 }
}

/// The pairs of each head's values a thread of an attention block (256 threads) accumulates: `lk_attention` weighs at
/// most `g` times as many of its block's `g` heads' value slots a thread.
pub fn attn_values_per_thread(vd: usize) -> usize {
    vd.div_ceil(512)
}

/// The blocks of an attention (`lk_attention`'s `MB`) a multiprocessor holds at once, by the registers it leaves them:
/// three for a launch of more blocks than the chip holds at two (`blocks` over `sms`), which come in waves, so
/// occupancy bounds them — on the 3060, 5.7% faster for Llama-3.2-1B's batch of 64 and 3.4% for Gemma-3-1B's; two for a
/// launch under one wave, whose blocks' latency bounds it, for the `f64` sums (`acc` 8 bytes), which need the
/// registers, and for blocks of `g` heads `hd` wide that take their chunks warp-local (`attn_warp_heads`, values `vd` as
/// wide), whose runs keep their queries and weighed values in registers: at three they spilled 1,408 bytes and a
/// 16-row chunk of Qwen3-8B at depth 8,192 took 7.9 ms on the 3060, at two 0.94 (the tile chunks at three 1.5). A
/// kernel at three takes every chunk a tile of keys at a time (`lk_attention_run`'s `W`).
pub fn attn_blocks_a_multiprocessor(blocks: usize, sms: usize, acc: usize, g: usize, hd: usize, vd: usize) -> usize {
    if acc == 4 && blocks > 2 * sms && !(attn_warp_heads(g, hd, acc) && vd == hd) {
        3
    } else {
        2
    }
}

/// The path an attention launch takes past a row of one chunk (`lk_attention`'s `PATH`, `lk_attn_path`), fixed where it
/// launches so its kernel compiles that path alone and ptxas allots its registers to it: the chunks' partials of a
/// grid of `z` split blocks a row folded by the last to arrive, held side by side by the row's one block (`side`
/// chunks at a time), weighed one at a time by it, or none where a row is one chunk (`splits` 1).
pub fn attn_path(z: usize, side: usize, splits: usize) -> &'static str {
    if z > 1 {
        "LK_ATTN_SPLIT"
    } else if side > 0 {
        "LK_ATTN_HELD"
    } else if splits > 1 {
        "LK_ATTN_CHUNKS"
    } else {
        "LK_ATTN_ONE"
    }
}

/// The lanes a key's dot takes in an attention of heads `hd` wide (`lk_attn_run_width`, `lk_attention`'s `RW`): a
/// half warp for a head of 9 to 16 fours, the whole warp otherwise.
pub fn attn_run_width(hd: usize) -> usize {
    if hd / 4 > 8 && hd / 4 <= 16 {
        16
    } else {
        32
    }
}

/// The most shared memory an attention block (`lk_attention`) keeps its chunks' partials in, so a row it takes whole
/// (a batch's sequence, a chunk's row) weighs each chunk once: a block of 256 threads keeps three to a streaming
/// multiprocessor.
pub const ATTN_HELD_BYTES: usize = 32 * 1024;

/// The bytes of a block's `g` heads' partials over `splits` chunks — each chunk's maximum, sum and `vd` weighed values,
/// in the accumulator's `acc` bytes — and whether the block holds them (`held`): a row cut into chunks, the partials
/// within `ATTN_HELD_BYTES`.
pub fn attn_held_bytes(g: usize, vd: usize, splits: usize, acc: usize) -> usize {
    g * splits * (vd + 2) * acc
}
pub fn attn_holds(g: usize, vd: usize, splits: usize, acc: usize) -> bool {
    splits > 1 && attn_held_bytes(g, vd, splits, acc) <= ATTN_HELD_BYTES
}

/// The chunks of its row a block that holds them weighs side by side (`lk_attn_side`): as many as its threads' value
/// slots hold — each chunk `vd / 2` pairs a head, a thread `attn_values_per_thread` a head — and no more than the row's
/// `splits`; one where a chunk fills them.
pub fn attn_side_chunks(vd: usize, splits: usize) -> usize {
    (512 * attn_values_per_thread(vd) / vd).clamp(1, splits.max(1))
}

/// The shared memory a block weighing `side` chunks side by side takes past `attn_smem_bytes`' one tile of scores:
/// a tile of its `g` heads' scores for each other chunk.
pub fn attn_side_bytes(g: usize, side: usize, acc: usize) -> usize {
    side.saturating_sub(1) * g * ATTN_TILE * acc
}

/// The most chunks an attention cuts a row's span into: enough blocks for two a streaming multiprocessor (`sms`) over
/// the `blocks` the head blocks of one row give, and no more than give each chunk `ATTN_MIN_SPLIT` positions of
/// `span`.
pub fn attn_splits(blocks: usize, span: usize, sms: usize) -> usize {
    (2 * sms).div_ceil(blocks).clamp(1, span.div_ceil(ATTN_MIN_SPLIT).max(1))
}

/// The query heads an attention block takes together: the largest divisor of `group`, at most 16, whose keys and values
/// a position stay within `ATTN_BLOCK_VALUES` (one head whatever its width), whose accumulators (`g` heads' value
/// pairs a thread, twice over in a block that takes every chunk) stay within 32 registers, and whose queries, scores
/// and chunk sums fit `smem` bytes — `None` when not even one head fits.
pub fn attn_heads_per_block(group: usize, hd: usize, vd: usize, span: usize, sms: usize, smem: usize, acc: usize) -> Option<usize> {
    let vpt = attn_values_per_thread(vd);
    (1..=group.min(16)).rev().find(|&g| {
        group % g == 0
            && (g == 1 || g * (hd + vd) <= ATTN_BLOCK_VALUES)
            && g * vpt <= 16
            && attn_smem_bytes(g, hd, attn_splits(1, span, sms), acc) <= smem
    })
}

/// The query vectors a block of the tiled attention takes: heads of one key/value group over consecutive rows.
pub const ATTN_TILE_Q: usize = 32;

/// The heads of one group a tiled attention block takes: the largest power of two dividing `group`, at most 32 — the
/// block's rows `ATTN_TILE_Q` over it.
pub fn attn_tile_heads(group: usize) -> usize {
    1 << group.trailing_zeros().min(5)
}

/// The query vectors a block of the flash attention (`lk_attention_flash`) takes: 64 for heads and values to 128 wide,
/// 32 past them, so a thread's outputs stay 64 registers and the block's shared memory within a block's.
pub fn attn_flash_queries(hd: usize, vd: usize) -> usize {
    if hd <= 128 && vd <= 128 {
        64
    } else {
        32
    }
}

/// The flash attention block's dynamic shared memory: its queries, two stages of a tile's keys 32 values of the head at
/// a time and of its values 16 keys at a time, and the tile's weights — rows padded four values.
pub fn attn_flash_smem_bytes(hd: usize, vd: usize) -> usize {
    let bq = attn_flash_queries(hd, vd);
    (bq * (hd + 4) + 2 * 64 * 36 + 2 * 16 * (vd + 4) + 64 * (bq + 4)) * 4
}

/// The threads of a flash attention block: 256 over 64 queries (a thread 4 queries by 4 keys), 128 over 32 — where the
/// 3060 measured each fastest.
pub fn attn_flash_threads(hd: usize, vd: usize) -> usize {
    if attn_flash_queries(hd, vd) == 64 {
        256
    } else {
        128
    }
}

/// The groups of four values, 128 apart, a thread of the tiled attention accumulates for each of its queries.
pub fn attn_tile_values(vd: usize) -> usize {
    vd.div_ceil(128)
}

/// The tiled attention block's dynamic shared memory: a 32-wide chunk of its queries and of a tile's 64 keys, the
/// tile's scores, and eight keys' values.
pub fn attn_tile_smem_bytes(vd: usize) -> usize {
    (32 * 32 + 32 * 64 + 64 * 32 + 8 * vd) * 4
}

/// The threads of the block that groups a block's expert choices (`lk_moe_group`): a warp a 32 choices, as many warps
/// as 32 and as each warp's count of every one of `experts` experts, beside the running offsets, fits 48 KB.
pub fn moe_group_threads(experts: usize) -> usize {
    32 * (48 * 1024 / (4 * experts)).saturating_sub(1).clamp(1, 32)
}

/// The shared memory of `lk_moe_group` at `threads`: the running offsets and each warp's counts.
pub fn moe_group_smem_bytes(experts: usize, threads: usize) -> usize {
    (1 + threads / 32) * experts * 4
}

/// The bytes of the lane's sums under the numerics of kernel mode `mode` ([`mode`]), and their CUDA type: `f64` for the
/// precise numerics (mode 0) — every sum of every kernel, a norm's, a convolution's, the delta rule's, the router's
/// softmax and the experts' — `f32` for the others.
pub fn accumulator(mode: u32) -> (usize, &'static str) {
    if mode == 0 {
        (8, "double")
    } else {
        (4, "float")
    }
}

/// The template argument that names a weight format in the block library: its GGUF type id.
pub fn format_arg(ty: crate::model::quant::GgmlType) -> u32 {
    ty.id()
}

/// The activation rows a block of `lk_gemv_f32` takes, together (its `LK_F32_ROWS`): few enough that a wide chunk's rows
/// spread over the chip, every row's dots summed as one row's are.
pub const GEMV_F32_ROWS: usize = 4;

/// The outputs a block of the decode step's expert down projection takes (`W` of `lk_moe_down`), and the warps its
/// shared expert's rows take (`SW`): four outputs where `d` divides by four, so each kept expert's warp has four rows'
/// weight loads in flight over one load of the hidden row — and a warp a row of the shared expert's where its rows are
/// wider than an expert's, so no warp holds the block back. Against a block of one output on the 3060, DeepSeek-V2-Lite's
/// down projections (Q8_0 beside a shared Q6_K, Q5_0 beside a shared Q4_K) drop 13–31%, gpt-oss-20b's (MXFP4) 23–36%
/// and Qwen3-30B-A3B's shape (Q4_K, Q6_K) 16–33%; eight outputs lost but in MXFP4.
pub fn moe_down_rows(d: usize, dff: usize, sdff: usize) -> (usize, usize) {
    let w = [4, 2, 1].into_iter().find(|w| d % w == 0).expect("one divides every width");
    (w, if sdff > dff { w } else { 1 })
}

/// The blocks and threads of a decode step's down projection (`lk_moe_down`, `lk_moe_down_norm`) over a row of `d`:
/// `moe_down_rows` outputs a block, a warp each of the `k` kept experts and the shared expert's warps where its
/// intermediate `sdff` is not 0.
pub fn moe_down_launch(d: usize, dff: usize, k: usize, sdff: usize) -> (usize, usize) {
    let (rows, shared) = moe_down_rows(d, dff, sdff);
    (d / rows, 32 * (k + if sdff > 0 { shared } else { 0 }))
}

/// The threads of a block-format projection that norms its row in its last block (`lk_gemv_blk_norm`): eight warps,
/// four where a warp takes `per_warp` rows past one.
pub fn gemv_blk_norm_threads(per_warp: usize) -> usize {
    32 * if per_warp == 1 { 8 } else { 4 }
}

/// Whether a block of `threads` runs a norm of a row of `cols` as the norm's own block does, each of its threads one of
/// the norm's (`norm_threads`). A narrower block plays several of the norm's warps a warp, one after another, and the
/// row's norm runs on that one block alone: DeepSeek-V2-Lite's 320-thread down blocks playing the norm's 1024 decoded at
/// 142.0 tokens a second on the 3060, 146.7 with the norm's tail cut and 146.3 with the norm its own launch.
pub fn block_holds_its_norm(threads: usize, cols: usize) -> bool {
    threads >= norm_threads(cols)
}

/// The most taps a gated delta layer's convolution takes inside its products' launch (`lk_gemv_gdn`, `LK_GDN_TAPS`): its
/// ring's values and taps are copied in by the warp's idle lanes while the row's dot streams. Qwen3.5's and Qwen3.8's are 4.
pub const GDN_TAPS: usize = 4;

/// The fewest rows a batch's Q8_0 projection takes to the tensor cores (`lk_gemv_mma`); fewer run on the GEMV.
pub const GEMV_MMA_MIN_ROWS: usize = 2;
/// A batch's projection over Q4_K, Q6_K, Q4_0, Q4_1, Q5_0 or Q5_1 rows on the tensor cores (`lk_gemv_mma_blk`): at most
/// this many rows (one m-tile), `GEMV_MMA_BLK_OUTPUTS` outputs a block of `GEMV_MMA_BLK_THREADS` threads, each part a whole number of
/// blocks; it takes a batch of at least `GEMV_MMA_BLK_MIN_ROWS` rows, fewer run on the GEMV.
pub const GEMV_MMA_BLK_ROWS: usize = 16;
pub const GEMV_MMA_BLK_OUTPUTS: usize = 64;
pub const GEMV_MMA_BLK_THREADS: usize = 512;
pub const GEMV_MMA_BLK_MIN_ROWS: usize = 2;
/// `lk_gemv_mma_blk<F>`'s shared memory: the warps' 16 level-one sums of the block's 16 rows by 64 outputs, and for
/// Q4_K each warp's table of a unit's sub-block scales.
pub fn gemv_mma_blk_smem_bytes(f: u32) -> usize {
    16 * 16 * GEMV_MMA_BLK_OUTPUTS * 4 + if f == 12 { 16 * 2 * 8 * 4 * 16 } else { 0 }
}
/// The most rows `lk_gemv_mma` takes in one launch: four m-tiles of 16.
pub const GEMV_MMA_ROWS: usize = 64;

/// `lk_gemv_mma`'s m-tiles for `m` rows (at most `GEMV_MMA_ROWS`): 1, 2 or 4.
pub fn gemv_mma_tiles(m: usize) -> usize {
    match m {
        0..=16 => 1,
        17..=32 => 2,
        _ => 4,
    }
}

/// `lk_gemv_mma`'s shape over `mt` m-tiles, `(nt, mw, nw)`: four n-tiles of 8 outputs a block over one m-tile, a warp
/// each; eight over two, a warp each n-tile over both; sixteen over four, a warp each two n-tiles over two m-tiles — so
/// a block stages its activation rows once for `32·mt` outputs, and over four m-tiles each fragment a warp loads feeds
/// two products. On the 3060 at 64 rows the two-by-two warps beat a warp each n-tile over all four m-tiles on
/// Llama-3.2-1B's down projection (8192 by 2048) 117 µs to 102, its output projection 33.8 to 30.9, its query, key and
/// value 35.6 to 32.9 and its gate and up 171.6 to 168.8; at 16 and 32 rows two n-tiles a warp gain nothing.
pub fn gemv_mma_shape(mt: usize) -> (usize, usize, usize) {
    match mt {
        4 => (16, 2, 2),
        _ => (4 * mt, mt, 1),
    }
}

/// `lk_gemv_mma`'s launch over `m` rows (at most `GEMV_MMA_ROWS`) of a projection of `n` outputs and `k` on a chip of
/// `sms` multiprocessors, `(mt, nt, mw, nw)`: the widest tile its rows fill (`gemv_mma_shape`), except that a narrow
/// projection of long rows over four m-tiles — its blocks of 128 outputs leaving more than half the chip idle, each
/// running long (`k` at least 2048) — takes blocks of 64 outputs, twice as many. On the 3060 at 64 rows: Gemma-3-1B's
/// down projection (6912 by 1152) 94 µs to 54 (62 as the 32-row tile in two slabs of grid layers), Qwen3.5-0.8B's (3584
/// by 1024) 54 to 31 (35) and its output projection (2048 by 1024) 33 to 20 (22); wider or shorter projections are
/// faster in blocks of 128.
pub fn gemv_mma_launch(m: usize, n: usize, k: usize, sms: usize) -> (usize, usize, usize, usize) {
    let mt = gemv_mma_tiles(m);
    let (nt, mw, nw) = gemv_mma_shape(mt);
    if mt == 4 && 2 * n.div_ceil(8 * nt) <= sms && k >= 2048 {
        return (mt, nt / 2, mw, nw);
    }
    (mt, nt, mw, nw)
}

/// `lk_gemv_mma`'s threads: a warp each `nw` n-tiles over each `mw` of its `mt` m-tiles.
pub fn gemv_mma_threads(mt: usize, nt: usize, mw: usize, nw: usize) -> usize {
    32 * (nt / nw) * (mt / mw)
}

/// `lk_gemv_mma`'s shared memory (`lk_mma_smem`): its stages (three, two under the two-word numerics), a stage four
/// blocks of each of its `8·nt` matrix rows and `16·mt` activation rows (both words' under two words), rows 144 bytes
/// apart, and the stage's scales.
pub fn gemv_mma_smem_bytes(two: bool, mt: usize, nt: usize) -> usize {
    let stage = 8 * nt * 144 + if two { 2 } else { 1 } * 16 * mt * 144 + 8 * nt * 8 + 16 * mt * 16;
    stage * if two { 2 } else { 3 }
}

/// The output rows a warp of a GEMV takes (`W` of `lk_row_dot`): four in a block format when one activation row meets
/// a matrix of at least 2048 rows of at least 64 sub-blocks — four rows' weight loads in flight over one load of the
/// activation outran one row a warp by 12–20% on the 3060 there, and lost where fewer warps would cover the chip or a
/// row leaves lanes idle — one otherwise. A Q8_0 row already moves sixteen bytes a lane at a time, and so does a 16-bit
/// float row (`lk_h16_dot`).
pub fn rows_a_warp(ty: crate::model::quant::GgmlType, n: usize, k: usize, m: usize) -> usize {
    let four = ty != crate::model::quant::GgmlType::Q8_0 && !sixteen(ty) && m == 1 && n >= 2048 && k / 32 >= 64 && n % 32 == 0;
    if four {
        4
    } else {
        1
    }
}

/// The output rows a warp of a 16-bit float matrix's GEMV of `n` outputs takes, and the rows a group its upload
/// interleaves them in (`lk_h16_group`): under the quantized numerics sixteen — the weights the A operand of each
/// m16n8k16 — or the most of the `n8` tile that divides `n`, eight, four, two or one (`lk_h16_mma_dot`), the matrix's
/// shape and not a schedule a rule may hold back; one, its rows one after another, under the precise numerics
/// (`lk_h16_dot`).
pub fn sixteen_rows_a_warp(activations: Q8MatKind, n: usize) -> usize {
    if activations == Q8MatKind::Dequant {
        return 1;
    }
    [16, 8, 4, 2, 1].into_iter().find(|w| n % w == 0).unwrap_or(1)
}

/// The warps a launch of a 16-bit float matrix's GEMV leaves each multiprocessor at the least where its rows allow
/// (`sixteen_launch_rows`).
pub const SIXTEEN_FILL: usize = 8;

/// The output rows a warp a launch of a 16-bit float matrix's GEMV over `rows` outputs takes on a chip of `sms`
/// multiprocessors, its layout allowing `per_warp` (`sixteen_rows_a_warp`): the most of `per_warp`, halved, that still
/// gives every multiprocessor `SIXTEEN_FILL` warps — so a narrow matrix's few rows spread over the whole chip — and
/// never under eight, where the dot's lanes past its rows load nothing (Qwen3.5-0.8B's 1024-row projections decoded at
/// 191.0 tokens a second at eight rows a warp, 180.3 at two). Rows a warp never change a sum (each output's dot is the
/// same whatever the warp's other rows), so the launch chooses them for the chip alone.
pub fn sixteen_launch_rows(per_warp: usize, rows: usize, sms: usize) -> usize {
    let mut w = per_warp.max(1);
    while w > 8 && rows / w < SIXTEEN_FILL * sms {
        w /= 2;
    }
    w
}

/// The warps a block of a 16-bit float matrix's GEMV on the tensor cores takes, its outputs plain (`lk_gemv_blk`,
/// `lk_gemv_glu`): one. Its dot keeps about 180 registers a thread, so a multiprocessor holds one block of eight warps
/// but eleven of one, and every warp of a launch, a narrow matrix's few too, lands on its own block: Llama-3.2-1B's gated
/// pair decoded 129.3 tokens a second in blocks of one warp, 128.9 in blocks of eight, and Qwen3.5-0.8B's 1024-row
/// projections 191.0 in 128 blocks of one, 189.9 in 64 of four.
pub const SIXTEEN_BLOCK_WARPS: usize = 1;

/// Whether a 16-bit float matrix's product of `k` inputs fits the 16-bit tensor cores under the quantized numerics:
/// whole 32s of `k`, a block of `lk_h16_block`.
pub fn sixteen_fits(k: usize) -> bool {
    k % 32 == 0
}

/// Whether a per-head matmul whose rows are its inputs stages them (`lk_head_axpy_staged`, `AXPY_W` sub-blocks and
/// `AXPY_R` row groups a block): under the quantized numerics in every block format but Q6_K and Q8_0 — 1.7–2.8 times
/// the per-lane kernel on the 3060 there (19.9 µs to 9.7 for DeepSeek-V2-Lite's Q4_K absorption); the per-lane kernel
/// keeps the precise numerics' `f64` sums, Q6_K's heavier decode and Q8_0's one activation row. The choice reads no
/// count of activation rows: a prompt's chunk and a decode step sum alike, so a prompt continues alike in chunks or a
/// token at a time.
pub fn axpy_staged(ty: crate::model::quant::GgmlType, numerics_mode: u32) -> bool {
    use crate::model::quant::GgmlType as T;
    numerics_mode != 0 && ty != T::Q6K && ty != T::Q8_0
}

/// The sub-blocks of outputs and the row groups a block of the staged per-head matmul takes.
pub const AXPY_W: usize = 4;
pub const AXPY_R: usize = 2;

/// The bytes of the staged per-head matmul's own sums in shared memory (`red`): an `f32` for each lane of each of its
/// `AXPY_R · AXPY_W` warps — it stages only under the quantized numerics, which sum in `f32`.
pub const AXPY_SUMS: usize = AXPY_R * AXPY_W * 32 * 4;

/// The bytes of a row the staged per-head matmul holds in shared memory (`lk_axpy_pitch`): the units of `AXPY_W`
/// sub-blocks and one more, 15 bytes of alignment slack, rounded to an odd number of words.
pub fn axpy_pitch(ty: crate::model::quant::GgmlType) -> usize {
    let w = AXPY_W;
    let bytes = match ty {
        crate::model::quant::GgmlType::Q8_0 => w * 34,
        _ => {
            let (unit, subs) = unit(ty);
            (w.div_ceil(subs) + 1) * unit + 15
        }
    };
    let words = bytes.div_ceil(4);
    4 * if words % 2 == 0 { words + 1 } else { words }
}

/// A block format's smallest unit the kernels decode alone (`lk_unit_bytes`, `lk_unit_subs`): its bytes and its 32-weight
/// sub-blocks — the checkpoint's block, but MXFP4's group of [`MXFP4_GROUP`] sub-blocks and Q6_K's group of
/// [`Q6K_GROUP`] blocks as the lane lays them out.
pub fn unit(ty: crate::model::quant::GgmlType) -> (usize, usize) {
    use crate::model::quant::GgmlType as T;
    match ty {
        T::Mxfp4 => (MXFP4_GROUP * 17, MXFP4_GROUP),
        T::Q6K => (Q6K_GROUP * 210, Q6K_GROUP * 8),
        T::Q4_0 | T::Q4_1 | T::Q5_0 | T::Q5_1 | T::Iq4Nl | T::Iq4Xs | T::Q2K | T::Q3K | T::Q4K | T::Q5K => {
            (ty.block_bytes(), ty.block_len() / 32)
        }
        other => panic!("no staged decode of {other:?}"),
    }
}

/// The sub-blocks of an MXFP4 group as the lane lays a row out (`lk_mx_exp`, `lk_mx_nib`): the group's exponents, a
/// byte each, then its nibbles, 16 bytes each.
pub const MXFP4_GROUP: usize = 16;

/// The blocks of a Q6_K group as the lane lays a row out (`lk_q6_blk`, `lk_q6_d`): the group's eight f16 scales, then
/// its blocks' other 208 bytes each.
pub const Q6K_GROUP: usize = 8;

/// The bytes of a row of `cols` weights in `ty` as the device keeps it (`lk_mx_row_bytes`, `lk_q6_row_bytes`): MXFP4's
/// sub-blocks in groups of [`MXFP4_GROUP`], its last group as many as remain, 16 bytes of exponents a group; Q6_K's
/// blocks in groups of [`Q6K_GROUP`], 16 bytes of scales a group — each a whole number of 16-byte words; any other
/// format's blocks as the checkpoint lays them out.
pub fn row_bytes(ty: crate::model::quant::GgmlType, cols: usize) -> usize {
    use crate::model::quant::GgmlType as T;
    let bytes = ty.bytes_for(cols);
    match ty {
        T::Mxfp4 => {
            let nb = bytes / 17;
            nb / MXFP4_GROUP * MXFP4_GROUP * 17 + if nb % MXFP4_GROUP > 0 { 16 + nb % MXFP4_GROUP * 16 } else { 0 }
        }
        T::Q6K => {
            let nb = bytes / 210;
            nb.div_ceil(Q6K_GROUP) * 16 + nb * 208
        }
        _ => bytes,
    }
}

/// The zero bytes an upload keeps past a block-format matrix's rows, so a staged read of a row's last unit stays inside
/// its allocation: 30 bytes past the checkpoint's blocks; past an MXFP4 group as short as one sub-block, the rest of a
/// whole group's 288 bytes.
pub fn tail_bytes(ty: crate::model::quant::GgmlType) -> usize {
    if ty == crate::model::quant::GgmlType::Mxfp4 {
        256
    } else {
        32
    }
}

/// The blocks an arg-max over `n` values spreads across: a run of at least 2048 values a block, at most 64 blocks.
pub fn argmax_blocks(n: usize) -> usize {
    n.div_ceil(2048).clamp(1, 64)
}

/// The warps an expert router's row takes (`WPE` of `lk_moe_route`): four for an `f32` router whose row splits into four
/// parts of whole 16-byte words, so its experts fill four times the blocks; one for a quantized router, whose row is
/// its format's one dot.
pub fn route_warps(f32_router: bool, d: usize) -> usize {
    if f32_router && d % 16 == 0 && d >= 512 {
        4
    } else {
        1
    }
}

/// The widest head a rotation turns: its block holds the head's values in shared memory (`lk_rope_head`'s `hv`).
pub const ROPE_HEAD: usize = 1024;

/// The most partials a norm folds in place of its added operand (`lk_fold`, `allreduce_norm`).
pub const FOLD_PARTIALS: usize = 8;

/// The most parts a row normed by parts takes, and the most it writes (`lk_row_parts`, `lk_normed_parts`).
pub const NORM_PARTS_MAX: usize = 8;

/// The threads of a block that norms a row of `cols` (`lk_rmsnorm`, and the norm inside a rotation): the row's width in
/// whole warps, at most 1024 — each thread then holds at most `cols / 1024` values.
pub fn norm_threads(cols: usize) -> usize {
    cols.next_multiple_of(32).min(1024)
}

/// The block a norm of `rows` rows of `cols` launches and the norm's warps each of its warps plays, `(threads, VW)`
/// (`lk_rmsnorm`): the norm's own threads (`norm_threads`) for one row; over more, the fewest warps' worth at most 256
/// threads that divide the norm's warps, each playing `VW` of them — a multiprocessor holds six blocks of 256 where it
/// holds one of the norm's 1024, so a chunk's rows run in waves six blocks a multiprocessor deep. The norm's threads are
/// the block's played `VW` times over, so every sum is the norm's own block's.
pub fn norm_block(cols: usize, rows: usize) -> (usize, usize) {
    let warps = norm_threads(cols) / 32;
    if rows <= 1 {
        return (32 * warps, 1);
    }
    let vw = (1..=warps).find(|v| warps % v == 0 && warps / v <= 8).unwrap_or(warps);
    (32 * warps / vw, vw)
}

/// Whether `ty` is a 16-bit float format the library reads in place, widening each weight in registers (`lk_h16`): F16
/// and BF16. A product by such a matrix reads its activation `f32` under every numerics.
pub fn sixteen(ty: crate::model::quant::GgmlType) -> bool {
    use crate::model::quant::GgmlType as T;
    matches!(ty, T::F16 | T::Bf16)
}

/// Whether the lane reads a matrix in `ty` as rows in place: Q8_0, a format the block library decodes, or a 16-bit
/// float format.
pub fn reads_in_place(ty: crate::model::quant::GgmlType) -> bool {
    ty == crate::model::quant::GgmlType::Q8_0 || decodes(ty) || sixteen(ty)
}

/// Whether the block library decodes `ty`: the formats with a sub-block decoder below.
pub fn decodes(ty: crate::model::quant::GgmlType) -> bool {
    use crate::model::quant::GgmlType as T;
    matches!(ty, T::Q4_0 | T::Q4_1 | T::Q5_0 | T::Q5_1 | T::Q2K | T::Q3K | T::Q4K | T::Q5K | T::Q6K | T::Mxfp4 | T::Iq4Nl | T::Iq4Xs)
}

/// One library kernel: its name and its source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kernel {
    Blocks,
    GatherQ8,
    GatherBlk,
    GatherF32,
    RmsNorm,
    NormParts,
    GatherNorm,
    Quant,
    GemvQ8,
    GemvQ8Norm,
    GemvBlk,
    GemvBlkNorm,
    GemvParts,
    Mma,
    GemmBlk,
    GemmF32,
    GemvF32,
    Binary,
    Scale,
    Softcap,
    SwiGlu,
    Unary,
    Relu,
    Rope,
    GemvPartsRope,
    SetRow,
    Attention,
    AttentionTiled,
    AttentionShared,
    AttentionFlash,
    AttentionMma,
    AttentionI8,
    ArgMax,
    Conv,
    L2Norm,
    Decay,
    Gates,
    DeltaStep,
    DeltaNorm,
    DeltaStepNorm,
    DeltaChunks,
    Moe,
    MoeNorm,
    Slice,
    Join,
    Heads,
    HeadTiled,
    Glu,
    GemvMma,
    GemvMmaBlk,
    GemvGdn,
    GemmH16,
}

impl Kernel {
    /// The kernel's CUDA name.
    pub fn name(self) -> &'static str {
        match self {
            Kernel::Blocks => "lk_w",
            Kernel::GatherQ8 => "lk_gather_q8",
            Kernel::GatherBlk => "lk_gather_blk",
            Kernel::GatherF32 => "lk_gather_f32",
            Kernel::RmsNorm => "lk_rmsnorm",
            Kernel::NormParts => "lk_norm_by_sums",
            Kernel::GatherNorm => "lk_gather_norm",
            Kernel::Quant => "lk_quant",
            Kernel::GemvQ8 => "lk_gemv_q8",
            Kernel::GemvQ8Norm => "lk_gemv_q8_norm",
            Kernel::GemvBlk => "lk_gemv_blk",
            Kernel::GemvBlkNorm => "lk_gemv_blk_norm",
            Kernel::GemvParts => "lk_gemv_parts",
            Kernel::Mma => "lk_mma_s8",
            Kernel::GemmBlk => "lk_gemm_blk",
            Kernel::GemmF32 => "lk_gemm_f32",
            Kernel::GemvF32 => "lk_gemv_f32",
            Kernel::Binary => "lk_binary",
            Kernel::Scale => "lk_scale",
            Kernel::Softcap => "lk_softcap",
            Kernel::SwiGlu => "lk_swiglu",
            Kernel::Unary => "lk_unary",
            Kernel::Relu => "lk_relu",
            Kernel::Rope => "lk_rope",
            Kernel::GemvPartsRope => "lk_gemv_parts_rope",
            Kernel::SetRow => "lk_set_row",
            Kernel::Attention => "lk_attention",
            Kernel::AttentionTiled => "lk_attention_tiled",
            Kernel::AttentionShared => "lk_attention_shared",
            Kernel::AttentionFlash => "lk_attention_flash",
            Kernel::AttentionMma => "lk_attention_mma",
            Kernel::AttentionI8 => "lk_attention_i8",
            Kernel::ArgMax => "lk_argmax",
            Kernel::Conv => "lk_conv",
            Kernel::L2Norm => "lk_l2norm",
            Kernel::Decay => "lk_decay",
            Kernel::Gates => "lk_gates",
            Kernel::DeltaStep => "lk_delta_step",
            Kernel::DeltaNorm => "lk_delta_norm",
            Kernel::DeltaStepNorm => "lk_delta_step",
            Kernel::DeltaChunks => "lk_delta_state",
            Kernel::Moe => "lk_moe",
            Kernel::MoeNorm => "lk_moe_norm",
            Kernel::Slice => "lk_slice",
            Kernel::Join => "lk_join",
            Kernel::Heads => "lk_head",
            Kernel::HeadTiled => "lk_head_tiled",
            Kernel::Glu => "lk_gemv_glu",
            Kernel::GemvMma => "lk_gemv_mma",
            Kernel::GemvMmaBlk => "lk_gemv_mma_blk",
            Kernel::GemvGdn => "lk_gemv_gdn",
            Kernel::GemmH16 => "lk_gemm_h16",
        }
    }

    /// The kernel's CUDA source.
    pub fn source(self) -> &'static str {
        match self {
            Kernel::Blocks => BLOCKS,
            Kernel::GatherQ8 => GATHER_Q8,
            Kernel::GatherBlk => GATHER_BLK,
            Kernel::GatherF32 => GATHER_F32,
            Kernel::RmsNorm => RMSNORM,
            Kernel::NormParts => NORM_PARTS,
            Kernel::GatherNorm => GATHER_NORM,
            Kernel::Quant => QUANT,
            Kernel::GemvQ8 => GEMV_Q8,
            Kernel::GemvQ8Norm => GEMV_Q8_NORM,
            Kernel::GemvBlk => GEMV_BLK,
            Kernel::GemvBlkNorm => GEMV_BLK_NORM,
            Kernel::GemvParts => GEMV_PARTS,
            Kernel::Mma => MMA,
            Kernel::GemmBlk => GEMM_BLK,
            Kernel::GemmF32 => GEMM_F32,
            Kernel::GemvF32 => GEMV_F32,
            Kernel::Binary => BINARY,
            Kernel::Scale => SCALE,
            Kernel::Softcap => SOFTCAP,
            Kernel::SwiGlu => SWIGLU,
            Kernel::Unary => UNARY,
            Kernel::Relu => RELU,
            Kernel::Rope => ROPE,
            Kernel::GemvPartsRope => GEMV_PARTS_ROPE,
            Kernel::SetRow => SET_ROW,
            Kernel::Attention => ATTENTION,
            Kernel::AttentionTiled => ATTENTION_TILED,
            Kernel::AttentionShared => ATTENTION_SHARED,
            Kernel::AttentionFlash => ATTENTION_FLASH,
            Kernel::AttentionMma => ATTENTION_MMA,
            Kernel::AttentionI8 => ATTENTION_I8,
            Kernel::ArgMax => ARGMAX,
            Kernel::Conv => CONV,
            Kernel::L2Norm => L2NORM,
            Kernel::Decay => DECAY,
            Kernel::Gates => GATES,
            Kernel::DeltaStep => DELTA_STEP,
            Kernel::DeltaNorm => DELTA_NORM,
            Kernel::DeltaStepNorm => DELTA_STEP_NORM,
            Kernel::DeltaChunks => DELTA_CHUNKS,
            Kernel::Moe => MOE,
            Kernel::MoeNorm => MOE_NORM,
            Kernel::Slice => SLICE,
            Kernel::Join => JOIN,
            Kernel::Heads => HEADS,
            Kernel::HeadTiled => HEAD_TILED,
            Kernel::Glu => GLU,
            Kernel::GemvMma => GEMV_MMA,
            Kernel::GemvMmaBlk => GEMV_MMA_BLK,
            Kernel::GemvGdn => GEMV_GDN,
            Kernel::GemmH16 => GEMM_H16,
        }
    }
}

/// A weight matrix as the kernels read it, and the sub-block decoders of every format besides Q8_0 (template argument
/// `F`, the GGUF type id): a Q8_0 matrix is its int8 values `q` and block scales `s`; any other is its blocks `b`, a row
/// every `rb` bytes — an MXFP4 row's blocks regrouped as `upload_mxfp4` lays them out (`lk_mx_exp`, `lk_mx_nib`), a
/// Q6_K row's grouped as `upload_q6k` lays them out (`lk_q6_blk`, `lk_q6_d`), every other format's as the checkpoint
/// lays them out. `lk_sub<F>` decodes sub-block `sb` (weights `32·sb …`) of a row into 32 int8 levels and, per half,
/// a scale and an offset; a weight is `scale·level + offset` rounded as the CPU dequantizer rounds it (a product, then a
/// sum); `lk_level<F>` decodes one element of a sub-block alike. `lk_row_dot<F, Q, M>` is a row's dot with `M` activation
/// rows (`ld` apart) under numerics `Q`, one warp; `lk_dot<F, Q, M>` the same for any format, a Q8_0 matrix's split
/// arrays included; `lk_gemv_dot<F, Q, M>` a row's dot with `M` activation rows of its length as the plain GEMV of its
/// format takes it, the one dot of a format every kernel shares so a fused launch computes what its parts computed.
pub const BLOCKS: &str = r#"
// A matrix: a Q8_0 matrix's split int8 levels and scales (`q`, `s`), or any other format's blocks at `b`, `rb` bytes a row;
// a 16-bit float matrix's rows `g` to a group, interleaved (`lk_h16_at`), 1 where they are kept row after row.
struct lk_w { const signed char* q; const __half* s; const unsigned char* b; long rb; int g = 1; };
// A part's outputs: an `f32` array or a narrower float's table, set from any of their pointers; the host reads an `f32`
// part's back as its pointer, a kernel through `lk_part_out`.
struct lk_part_y {
  void* p = nullptr;
  lk_store t = LK_F32;
  lk_part_y() = default;
  __host__ __device__ lk_part_y(std::nullptr_t) {}
  __host__ __device__ lk_part_y(float* q) : p(q) {}
  __host__ __device__ lk_part_y(__half* q) : p(q), t(LK_F16) {}
  __host__ __device__ lk_part_y(__nv_fp8_e4m3* q) : p(q), t(LK_E4M3) {}
  __host__ operator float*() const { return (float*)p; }
};
// Up to `LK_PARTS` matrices one activation is projected by (the planner's `GEMM_PARTS`), their outputs side by side:
// part `p`'s matrix `w[p]`, outputs `y[p]` (rows from the one-based position `row[p]` names, `pitch[p]` apart, where it
// names one), epilogue `bias[p]` (its bias, or every step a product's epilogue takes: `lk_epi`), and its outputs
// `end[p − 1] … end[p] − 1` of the launch's.
#define LK_PARTS 6
struct lk_parts {
  lk_w w[LK_PARTS];
  lk_part_y y[LK_PARTS];
  lk_epi bias[LK_PARTS];
  int end[LK_PARTS];
  const int* row[LK_PARTS];
  long pitch[LK_PARTS];
};
__device__ __forceinline__ lk_out lk_part_out(const lk_parts& parts, int p){
  const lk_part_y& y = parts.y[p];
  return y.t == LK_F16 ? lk_out((__half*)y.p) : y.t == LK_E4M3 ? lk_out((__nv_fp8_e4m3*)y.p) : lk_out((float*)y.p);
}
// The part whose columns hold a tile's first output `n0` (the parts' ends rise, an unused part's where the last used
// one's does): its matrix, outputs, bias, output rows and pitch, its own count of outputs, and `n0` its own — each part
// read from the launch's parameters at a constant offset and the tile's taken by selects, so the parts stay in the
// parameter bank (indexed by a part computed at run time, they would be copied to every thread's stack).
__device__ __forceinline__ void lk_take_part(const lk_parts& parts, int& n0, lk_w& w, lk_out& y, lk_epi& bias,
                                             const int* __restrict__& yrow, long& ypitch, int& N){
  int start = 0, end = parts.end[0];
  w = parts.w[0]; y = lk_part_out(parts, 0); bias = parts.bias[0]; yrow = parts.row[0]; ypitch = parts.pitch[0];
  #pragma unroll
  for(int q = 1; q < LK_PARTS; ++q)
    if(n0 >= parts.end[q - 1]){
      start = parts.end[q - 1]; end = parts.end[q];
      w = parts.w[q]; y = lk_part_out(parts, q); bias = parts.bias[q]; yrow = parts.row[q]; ypitch = parts.pitch[q];
    }
  N = end - start; n0 -= start;
}
// The little-endian f16 at `p`, as an `f32`.
__device__ __forceinline__ float lk_h(const unsigned char* p){ return __half2float(__ushort_as_half((unsigned short)(p[0] | (p[1] << 8)))); }
__device__ __forceinline__ float lk_e8m0_half(unsigned char x){
  unsigned int bits = x < 2 ? 0x00200000u << x : ((unsigned int)(x - 1)) << 23;
  return __uint_as_float(bits);
}
// An MXFP4 row as the lane lays it out: its sub-blocks in groups of 16, a group's 16 E8M0 exponents then its 16 nibble
// words, the last group as many sub-blocks as remain — so a sub-block's nibbles are one aligned 16-byte word. Sub-block
// `s`'s exponent byte and nibble word, and the bytes of a row of `nb` sub-blocks.
__host__ __device__ constexpr long lk_mx_exp(long s){ return s / 16 * 272 + s % 16; }
__host__ __device__ constexpr long lk_mx_nib(long s){ return s / 16 * 272 + 16 + s % 16 * 16; }
__host__ __device__ constexpr long lk_mx_row_bytes(long nb){ return nb / 16 * 272 + (nb % 16 ? 16 + nb % 16 * 16 : 0); }
// `rows` MXFP4 rows of `nb` sub-blocks from the checkpoint's 17-byte blocks at `src` (an exponent, then 16 bytes of
// nibbles) into the lane's layout at `dst`, `lk_mx_row_bytes(nb)` a row, its pads untouched.
static inline void lk_mx_regroup(const unsigned char* src, long rows, long nb, unsigned char* dst){
  for(long i = 0; i < rows; ++i){
    unsigned char* row = dst + i * lk_mx_row_bytes(nb);
    for(long s = 0; s < nb; ++s){
      const unsigned char* blk = src + (i * nb + s) * 17;
      row[lk_mx_exp(s)] = blk[0];
      for(int j = 0; j < 16; ++j) row[lk_mx_nib(s) + j] = blk[1 + j];
    }
  }
}
// A Q6_K row as the lane lays it out: its 210-byte blocks in groups of eight, a group's eight f16 scales first (16
// bytes), then each block's other 208 bytes (low nibbles, high bits, sub-block scales) as the checkpoint keeps them, the
// last group as many blocks as remain — so every block's levels start on a 16-byte boundary and read in 16-byte words.
// Block `b`'s 208 bytes and its scale, and the bytes of a row of `nb` blocks.
__host__ __device__ constexpr long lk_q6_blk(long b){ return b / 8 * 1680 + 16 + b % 8 * 208; }
__host__ __device__ constexpr long lk_q6_d(long b){ return b / 8 * 1680 + b % 8 * 2; }
__host__ __device__ constexpr long lk_q6_row_bytes(long nb){ return (nb + 7) / 8 * 16 + nb * 208; }
// `rows` Q6_K rows of `nb` blocks from the checkpoint's 210-byte blocks at `src` into the lane's layout at `dst`,
// `lk_q6_row_bytes(nb)` a row, its pads untouched.
static inline void lk_q6_regroup(const unsigned char* src, long rows, long nb, unsigned char* dst){
  for(long i = 0; i < rows; ++i){
    unsigned char* row = dst + i * lk_q6_row_bytes(nb);
    for(long b = 0; b < nb; ++b){
      const unsigned char* blk = src + (i * nb + b) * 210;
      for(int j = 0; j < 208; ++j) row[lk_q6_blk(b) + j] = blk[j];
      row[lk_q6_d(b)] = blk[208];
      row[lk_q6_d(b) + 1] = blk[209];
    }
  }
}
// A 16-bit float matrix read on the tensor cores keeps its rows interleaved `g` to a group: block `b` of 32 values of a
// group is the group's `g` rows' 64 bytes there, one after another, so a warp reading a block of its rows reads one run
// of `64·g` bytes. `lk_h16_group` is the group of a matrix of `rows` rows — sixteen, or the most of 8, 4, 2 and 1 that
// divides them — `lk_h16_at` the 64 bytes of block `b` of row `r`, and `lk_h16_interleave` lays out `rows` rows of
// `cols` values, kept one after another at `src`, in groups of `g` at `dst`.
__host__ __device__ constexpr int lk_h16_group(long rows){
  return rows % 16 == 0 ? 16 : rows % 8 == 0 ? 8 : rows % 4 == 0 ? 4 : rows % 2 == 0 ? 2 : 1;
}
__host__ __device__ __forceinline__ const unsigned char* lk_h16_addr(const unsigned char* base, long rb, int g, long r, int b){
  return base + r / g * g * rb + (long)b * g * 64 + r % g * 64;
}
__device__ __forceinline__ const unsigned char* lk_h16_at(const lk_w& w, long r, int b){ return lk_h16_addr(w.b, w.rb, w.g, r, b); }
static inline void lk_h16_interleave(const unsigned char* src, long rows, long cols, int g, unsigned char* dst){
  const long rb = cols * 2;
  for(long r = 0; r < rows; ++r)
    for(long b = 0; b < cols / 32; ++b)
      for(int j = 0; j < 64; ++j) dst[r / g * g * rb + b * g * 64 + r % g * 64 + j] = src[r * rb + b * 64 + j];
}
__device__ __forceinline__ void lk_scale_min_k4(int j, const unsigned char* q, int& sc, int& m){
  if(j < 4){ sc = q[j] & 63; m = q[j + 4] & 63; }
  else { sc = (q[j + 4] & 0xf) | ((q[j - 4] >> 6) << 4); m = (q[j + 4] >> 4) | ((q[j] >> 6) << 4); }
}
// Word `i` of four, by selects rather than an index, so the four stay in registers.
__device__ __forceinline__ unsigned int lk_pick4(const unsigned int* a, int i){ return i == 0 ? a[0] : i == 1 ? a[1] : i == 2 ? a[2] : a[3]; }
// The IQ4 formats' sixteen non-linear levels (ggml's `kvalues_iq4nl`): one by its nibble, and four to a word.
__device__ __forceinline__ int lk_iq4_level(int n){
  const signed char kv[16] = {-127, -104, -83, -65, -49, -35, -22, -10, 1, 13, 25, 38, 53, 69, 89, 113};
  return kv[n];
}
// The levels of the low nibbles of `q`'s four bytes (`lo`) and of their high nibbles (`hi`), four to a word, from a
// table of sixteen int8 levels four to a word (`t0` levels 0–3 … `t3` 12–15), by byte permutes: each nibble's low three
// bits pick a byte from levels 0–7 and from 8–15, its fourth bit picks between them — no memory read.
__device__ __forceinline__ void lk_table16(unsigned int q, unsigned int t0, unsigned int t1, unsigned int t2, unsigned int t3,
                                           int& lo, int& hi){
  unsigned int t[2], sel = 0x32103210u | ((q & 0x88888888u) >> 1);
  #pragma unroll
  for(int i = 0; i < 2; ++i){
    unsigned int s = q >> (16 * i);
    t[i] = __byte_perm(__byte_perm(t0, t1, s), __byte_perm(t2, t3, s), sel >> (16 * i));
  }
  lo = (int)__byte_perm(t[0], t[1], 0x6420);
  hi = (int)__byte_perm(t[0], t[1], 0x7531);
}
// IQ4's levels and MXFP4's doubled E2M1 values (ggml's `kvalues_mxfp4`), four to a word.
__device__ __forceinline__ void lk_iq4_word(unsigned int q, int& lo, int& hi){
  lk_table16(q, 0xBFAD9881u, 0xF6EADDCFu, 0x26190D01u, 0x71594535u, lo, hi);
}
__device__ __forceinline__ void lk_mxfp4_word(unsigned int q, int& lo, int& hi){
  lk_table16(q, 0x03020100u, 0x0C080604u, 0xFDFEFF00u, 0xF4F8FAFCu, lo, hi);
}
// The `n` bytes from `p`, any alignment, as words: aligned word loads joined by funnel shifts. Reads up to three bytes
// past them, which every block-format upload's 32 trailing bytes cover.
template<int N>
__device__ __forceinline__ void lk_words(const unsigned char* p, unsigned int* q){
  const unsigned int* a = (const unsigned int*)((size_t)p & ~(size_t)3);
  unsigned int sh = 8 * (unsigned int)((size_t)p & 3), w[N + 1];
  #pragma unroll
  for(int i = 0; i <= N; ++i) w[i] = a[i];
  #pragma unroll
  for(int i = 0; i < N; ++i) q[i] = __funnelshift_r(w[i], w[i + 1], sh);
}
// An IQ4_XS sub-block's scale: the super-block's `d` times its 6-bit signed scale (low four bits from `sl`, the byte of
// `scales_l` it shares with its neighbour, high two from `sh`, the super-block's `scales_h`), as the CPU dequantizer
// rounds it.
__device__ __forceinline__ float lk_iq4xs_scale(float d, unsigned int sh, unsigned int sl, int ib){
  int ls = (int)((sl >> (4 * (ib & 1))) & 0xf) | (int)(((sh >> (2 * ib)) & 3) << 4);
  return d * (float)(ls - 32);
}
template<int F>
__device__ __forceinline__ void lk_sub(const unsigned char* __restrict__ row, int sb, signed char* v, float* d, float* c){
  if(F == 2 || F == 3){
    const unsigned char* b = row + (long)sb * (F == 2 ? 18 : 20);
    float dd = lk_h(b), cc = F == 3 ? lk_h(b + 2) : 0.f;
    const unsigned char* qs = b + (F == 2 ? 2 : 4);
    #pragma unroll
    for(int j = 0; j < 16; ++j){ v[j] = (qs[j] & 0xf) - (F == 2 ? 8 : 0); v[j + 16] = (qs[j] >> 4) - (F == 2 ? 8 : 0); }
    d[0] = d[1] = dd; c[0] = c[1] = cc;
  } else if(F == 6 || F == 7){
    const unsigned char* b = row + (long)sb * (F == 6 ? 22 : 24);
    float dd = lk_h(b), cc = F == 7 ? lk_h(b + 2) : 0.f;
    const unsigned char* h = b + (F == 6 ? 2 : 4);
    unsigned int qh = h[0] | (h[1] << 8) | (h[2] << 16) | ((unsigned int)h[3] << 24);
    const unsigned char* qs = h + 4;
    #pragma unroll
    for(int j = 0; j < 16; ++j){
      v[j] = ((qs[j] & 0xf) | (((qh >> j) << 4) & 0x10)) - (F == 6 ? 16 : 0);
      v[j + 16] = ((qs[j] >> 4) | ((qh >> (j + 12)) & 0x10)) - (F == 6 ? 16 : 0);
    }
    d[0] = d[1] = dd; c[0] = c[1] = cc;
  } else if(F == 39){
    const unsigned char* qs = row + lk_mx_nib(sb);
    const signed char kv[16] = {0, 1, 2, 3, 4, 6, 8, 12, 0, -1, -2, -3, -4, -6, -8, -12};
    float dd = lk_e8m0_half(row[lk_mx_exp(sb)]);
    #pragma unroll
    for(int j = 0; j < 16; ++j){ v[j] = kv[qs[j] & 0xf]; v[j + 16] = kv[qs[j] >> 4]; }
    d[0] = d[1] = dd; c[0] = c[1] = 0.f;
  } else if(F == 20 || F == 23){
    const unsigned char* b = F == 20 ? row + (long)sb * 18 : row + (long)(sb >> 3) * 136;
    int ib = sb & 7;
    float dd = F == 20 ? lk_h(b) : lk_iq4xs_scale(lk_h(b), b[2] | (b[3] << 8), b[4 + (ib >> 1)], ib);
    const unsigned char* qs = F == 20 ? b + 2 : b + 8 + 16 * ib;
    #pragma unroll
    for(int j = 0; j < 16; ++j){ v[j] = lk_iq4_level(qs[j] & 0xf); v[j + 16] = lk_iq4_level(qs[j] >> 4); }
    d[0] = d[1] = dd; c[0] = c[1] = 0.f;
  } else if(F == 12 || F == 13){
    const unsigned char* b = row + (long)(sb >> 3) * (F == 12 ? 144 : 176);
    int j = sb & 7, p = j >> 1, hi = j & 1, sc, m;
    lk_scale_min_k4(j, b + 4, sc, m);
    float dd = lk_h(b) * (float)sc, cc = -lk_h(b + 2) * (float)m;
    const unsigned char* ql = b + (F == 12 ? 16 : 48) + p * 32;
    #pragma unroll
    for(int l = 0; l < 32; ++l){
      int lv = hi ? ql[l] >> 4 : ql[l] & 0xf;
      if(F == 13) lv += (b[16 + l] & (1 << (2 * p + hi))) ? 16 : 0;
      v[l] = (signed char)lv;
    }
    d[0] = d[1] = dd; c[0] = c[1] = cc;
  } else if(F == 10){
    const unsigned char* b = row + (long)(sb >> 3) * 84;
    int j = sb & 7, part = j >> 2, k = j & 3;
    const unsigned char* q = b + 16 + part * 32;
    float dd = lk_h(b + 80), dm = lk_h(b + 82);
    #pragma unroll
    for(int l = 0; l < 32; ++l) v[l] = (signed char)((q[(l & 15) + (l >> 4) * 16] >> (2 * k)) & 3);
    #pragma unroll
    for(int h = 0; h < 2; ++h){ int sc = b[part * 8 + k * 2 + h]; d[h] = dd * (float)(sc & 0xf); c[h] = -(dm * (float)(sc >> 4)); }
  } else if(F == 11){
    const unsigned char* b = row + (long)(sb >> 3) * 110;
    int j = sb & 7, part = j >> 2, k = j & 3, mbit = 1 << (part * 4 + k);
    const unsigned char* q = b + 32 + part * 32;
    const unsigned char* p = b + 96;
    unsigned int a0 = p[0] | (p[1] << 8) | (p[2] << 16) | ((unsigned int)p[3] << 24);
    unsigned int a1 = p[4] | (p[5] << 8) | (p[6] << 16) | ((unsigned int)p[7] << 24);
    unsigned int t = p[8] | (p[9] << 8) | (p[10] << 16) | ((unsigned int)p[11] << 24);
    unsigned int aux[4] = {(a0 & 0x0f0f0f0fu) | ((t & 0x03030303u) << 4), (a1 & 0x0f0f0f0fu) | (((t >> 2) & 0x03030303u) << 4),
                           ((a0 >> 4) & 0x0f0f0f0fu) | (((t >> 4) & 0x03030303u) << 4), ((a1 >> 4) & 0x0f0f0f0fu) | (((t >> 6) & 0x03030303u) << 4)};
    float dd = lk_h(b + 108);
    #pragma unroll
    for(int l = 0; l < 32; ++l){
      int at = (l & 15) + (l >> 4) * 16;
      v[l] = (signed char)(((q[at] >> (2 * k)) & 3) - ((b[at] & mbit) ? 0 : 4));
    }
    #pragma unroll
    for(int h = 0; h < 2; ++h){
      int is = part * 8 + k * 2 + h;
      int sc = (signed char)((lk_pick4(aux, is >> 2) >> (8 * (is & 3))) & 0xff);
      d[h] = dd * (float)(sc - 32); c[h] = 0.f;
    }
  } else if(F == 14){
    const unsigned char* b = row + lk_q6_blk(sb >> 3);
    int j = sb & 7, part = j >> 2, k = j & 3;
    const unsigned char* ql = b + part * 64;
    const unsigned char* qh = b + 128 + part * 32;
    const signed char* sc = (const signed char*)(b + 192 + part * 8);
    float dd = lk_h(row + lk_q6_d(sb >> 3));
    #pragma unroll
    for(int l = 0; l < 32; ++l){
      int lo = (k & 1) ? ql[l + 32] : ql[l];
      lo = (k & 2) ? lo >> 4 : lo & 0xf;
      v[l] = (signed char)((lo | (((qh[l] >> (2 * k)) & 3) << 4)) - 32);
    }
    d[0] = dd * (float)sc[2 * k]; d[1] = dd * (float)sc[2 * k + 1]; c[0] = c[1] = 0.f;
  }
}
// Element `l` of sub-block `sb` of a row in format `F`, as `lk_sub` decodes it: its int8 level, and the scale `d` and
// offset `c` of its half — the decoder of one weight a lane reads alone.
template<int F>
__device__ __forceinline__ int lk_level(const unsigned char* __restrict__ row, int sb, int l, float& d, float& c){
  if(F == 2 || F == 3){
    const unsigned char* b = row + (long)sb * (F == 2 ? 18 : 20);
    d = lk_h(b); c = F == 3 ? lk_h(b + 2) : 0.f;
    const unsigned char* qs = b + (F == 2 ? 2 : 4);
    return (l < 16 ? (qs[l] & 0xf) : (qs[l - 16] >> 4)) - (F == 2 ? 8 : 0);
  } else if(F == 6 || F == 7){
    const unsigned char* b = row + (long)sb * (F == 6 ? 22 : 24);
    d = lk_h(b); c = F == 7 ? lk_h(b + 2) : 0.f;
    const unsigned char* h = b + (F == 6 ? 2 : 4);
    unsigned int qh = h[0] | (h[1] << 8) | (h[2] << 16) | ((unsigned int)h[3] << 24);
    const unsigned char* qs = h + 4;
    int q = l < 16 ? ((qs[l] & 0xf) | (((qh >> l) << 4) & 0x10)) : ((qs[l - 16] >> 4) | ((qh >> (l - 4)) & 0x10));
    return q - (F == 6 ? 16 : 0);
  } else if(F == 39){
    const unsigned char* qs = row + lk_mx_nib(sb);
    const signed char kv[16] = {0, 1, 2, 3, 4, 6, 8, 12, 0, -1, -2, -3, -4, -6, -8, -12};
    d = lk_e8m0_half(row[lk_mx_exp(sb)]); c = 0.f;
    return kv[l < 16 ? (qs[l] & 0xf) : (qs[l - 16] >> 4)];
  } else if(F == 20 || F == 23){
    const unsigned char* b = F == 20 ? row + (long)sb * 18 : row + (long)(sb >> 3) * 136;
    int ib = sb & 7;
    d = F == 20 ? lk_h(b) : lk_iq4xs_scale(lk_h(b), b[2] | (b[3] << 8), b[4 + (ib >> 1)], ib); c = 0.f;
    const unsigned char* qs = F == 20 ? b + 2 : b + 8 + 16 * ib;
    return lk_iq4_level(l < 16 ? (qs[l] & 0xf) : (qs[l - 16] >> 4));
  } else if(F == 12 || F == 13){
    const unsigned char* b = row + (long)(sb >> 3) * (F == 12 ? 144 : 176);
    int j = sb & 7, p = j >> 1, hi = j & 1, sc, m;
    lk_scale_min_k4(j, b + 4, sc, m);
    d = lk_h(b) * (float)sc; c = -lk_h(b + 2) * (float)m;
    const unsigned char* ql = b + (F == 12 ? 16 : 48) + p * 32;
    int lv = hi ? ql[l] >> 4 : ql[l] & 0xf;
    if(F == 13) lv += (b[16 + l] & (1 << (2 * p + hi))) ? 16 : 0;
    return lv;
  } else if(F == 10){
    const unsigned char* b = row + (long)(sb >> 3) * 84;
    int j = sb & 7, part = j >> 2, k = j & 3;
    const unsigned char* q = b + 16 + part * 32;
    float dd = lk_h(b + 80), dm = lk_h(b + 82);
    int sc = b[part * 8 + k * 2 + (l >> 4)];
    d = dd * (float)(sc & 0xf); c = -(dm * (float)(sc >> 4));
    return (q[(l & 15) + (l >> 4) * 16] >> (2 * k)) & 3;
  } else if(F == 11){
    const unsigned char* b = row + (long)(sb >> 3) * 110;
    int j = sb & 7, part = j >> 2, k = j & 3, mbit = 1 << (part * 4 + k);
    const unsigned char* q = b + 32 + part * 32;
    const unsigned char* p = b + 96;
    unsigned int a0 = p[0] | (p[1] << 8) | (p[2] << 16) | ((unsigned int)p[3] << 24);
    unsigned int a1 = p[4] | (p[5] << 8) | (p[6] << 16) | ((unsigned int)p[7] << 24);
    unsigned int t = p[8] | (p[9] << 8) | (p[10] << 16) | ((unsigned int)p[11] << 24);
    unsigned int aux[4] = {(a0 & 0x0f0f0f0fu) | ((t & 0x03030303u) << 4), (a1 & 0x0f0f0f0fu) | (((t >> 2) & 0x03030303u) << 4),
                           ((a0 >> 4) & 0x0f0f0f0fu) | (((t >> 4) & 0x03030303u) << 4), ((a1 >> 4) & 0x0f0f0f0fu) | (((t >> 6) & 0x03030303u) << 4)};
    int is = part * 8 + k * 2 + (l >> 4);
    int sc = (signed char)((lk_pick4(aux, is >> 2) >> (8 * (is & 3))) & 0xff);
    d = lk_h(b + 108) * (float)(sc - 32); c = 0.f;
    int at = (l & 15) + (l >> 4) * 16;
    return ((q[at] >> (2 * k)) & 3) - ((b[at] & mbit) ? 0 : 4);
  } else {
    const unsigned char* b = row + lk_q6_blk(sb >> 3);
    int j = sb & 7, part = j >> 2, k = j & 3;
    const unsigned char* ql = b + part * 64;
    const unsigned char* qh = b + 128 + part * 32;
    const signed char* sc = (const signed char*)(b + 192 + part * 8);
    d = lk_h(row + lk_q6_d(sb >> 3)) * (float)sc[2 * k + (l >> 4)]; c = 0.f;
    int lo = (k & 1) ? ql[l + 32] : ql[l];
    lo = (k & 2) ? lo >> 4 : lo & 0xf;
    return (lo | (((qh[l] >> (2 * k)) & 3) << 4)) - 32;
  }
}
__device__ __forceinline__ int lk_pack4(const signed char* v){
  return (v[0] & 0xff) | ((v[1] & 0xff) << 8) | ((v[2] & 0xff) << 16) | ((unsigned int)(v[3] & 0xff) << 24);
}
// A word of four bytes from an address 2-byte aligned, or 4-byte aligned.
__device__ __forceinline__ int lk_w16(const unsigned char* p){
  const unsigned short* h = (const unsigned short*)p;
  return (int)((unsigned int)h[0] | ((unsigned int)h[1] << 16));
}
__device__ __forceinline__ int lk_w32(const unsigned char* p){ return *(const int*)p; }
// The fifth bits of four levels, from four consecutive bits of `qh`, placed at bit 4 of each byte.
__device__ __forceinline__ int lk_bit4(unsigned int b){
  return (int)(((b << 4) & 0x10u) | ((b << 11) & 0x1000u) | ((b << 18) & 0x100000u) | ((b << 25) & 0x10000000u));
}
// Sub-block `sb` of a row in format `F`, as `lk_sub` decodes it, its 32 levels packed four to an int for `__dp4a` (`w[i]`
// the levels 4i … 4i+3) — read a word at a time, never a level at a time; the scale and offset of each half as `lk_sub`.
template<int F>
__device__ __forceinline__ void lk_sub4(const unsigned char* __restrict__ row, int sb, int* w, float* d, float* c){
  if(F == 2 || F == 3){
    const unsigned char* b = row + (long)sb * (F == 2 ? 18 : 20);
    float dd = lk_h(b), cc = F == 3 ? lk_h(b + 2) : 0.f;
    const unsigned char* qs = b + (F == 2 ? 2 : 4);
    #pragma unroll
    for(int i = 0; i < 4; ++i){
      int q = F == 2 ? lk_w16(qs + 4 * i) : lk_w32(qs + 4 * i);
      w[i] = q & 0x0F0F0F0F; w[i + 4] = (q >> 4) & 0x0F0F0F0F;
      if(F == 2){ w[i] = __vsub4(w[i], 0x08080808); w[i + 4] = __vsub4(w[i + 4], 0x08080808); }
    }
    d[0] = d[1] = dd; c[0] = c[1] = cc;
  } else if(F == 6 || F == 7){
    const unsigned char* b = row + (long)sb * (F == 6 ? 22 : 24);
    float dd = lk_h(b), cc = F == 7 ? lk_h(b + 2) : 0.f;
    const unsigned char* h = b + (F == 6 ? 2 : 4);
    unsigned int qh = F == 6 ? (unsigned int)lk_w16(h) : (unsigned int)lk_w32(h);
    const unsigned char* qs = h + 4;
    #pragma unroll
    for(int i = 0; i < 4; ++i){
      int q = F == 6 ? lk_w16(qs + 4 * i) : lk_w32(qs + 4 * i);
      w[i] = (q & 0x0F0F0F0F) | lk_bit4((qh >> (4 * i)) & 0xF);
      w[i + 4] = ((q >> 4) & 0x0F0F0F0F) | lk_bit4((qh >> (16 + 4 * i)) & 0xF);
      if(F == 6){ w[i] = __vsub4(w[i], 0x10101010); w[i + 4] = __vsub4(w[i + 4], 0x10101010); }
    }
    d[0] = d[1] = dd; c[0] = c[1] = cc;
  } else if(F == 12){
    // The super-block's header (d, dmin, the twelve scale bytes) one 16-byte word, the sub-block's nibbles two — the
    // words its pair's other sub-block reads too.
    const unsigned char* b = row + (long)(sb >> 3) * 144;
    int j = sb & 7, p = j >> 1, hi = j & 1, sc, m;
    const uint4* qw = (const uint4*)(b + 16 + p * 32);
    const uint4 hd = *(const uint4*)b, q0 = qw[0], q1 = qw[1];
    // Scale byte `k` of the twelve by shifts, so the header stays in registers.
    const unsigned long long lo = (unsigned long long)hd.y | ((unsigned long long)hd.z << 32);
    auto byte = [&](int k) -> int {
      return k < 8 ? (int)((lo >> (8 * k)) & 0xff) : (int)((hd.w >> (8 * (k - 8))) & 0xff);
    };
    if(j < 4){ sc = byte(j) & 63; m = byte(j + 4) & 63; }
    else { sc = (byte(j + 4) & 0xf) | ((byte(j - 4) >> 6) << 4); m = (byte(j + 4) >> 4) | ((byte(j) >> 6) << 4); }
    float dd = __half2float(__ushort_as_half((unsigned short)(hd.x & 0xffff))) * (float)sc;
    float cc = -__half2float(__ushort_as_half((unsigned short)(hd.x >> 16))) * (float)m;
    const unsigned int q[8] = {q0.x, q0.y, q0.z, q0.w, q1.x, q1.y, q1.z, q1.w};
    #pragma unroll
    for(int i = 0; i < 8; ++i) w[i] = (int)((q[i] >> (4 * hi)) & 0x0F0F0F0Fu);
    d[0] = d[1] = dd; c[0] = c[1] = cc;
  } else if(F == 13){
    const unsigned char* b = row + (long)(sb >> 3) * 176;
    int j = sb & 7, p = j >> 1, hi = j & 1, sc, m;
    lk_scale_min_k4(j, b + 4, sc, m);
    float dd = lk_h(b) * (float)sc, cc = -lk_h(b + 2) * (float)m;
    const unsigned char* ql = b + 48 + p * 32;
    #pragma unroll
    for(int i = 0; i < 8; ++i){
      int q = (lk_w32(ql + 4 * i) >> (4 * hi)) & 0x0F0F0F0F;
      q |= ((lk_w32(b + 16 + 4 * i) >> (2 * p + hi)) & 0x01010101) << 4;
      w[i] = q;
    }
    d[0] = d[1] = dd; c[0] = c[1] = cc;
  } else if(F == 10){
    const unsigned char* b = row + (long)(sb >> 3) * 84;
    int j = sb & 7, part = j >> 2, k = j & 3;
    const unsigned char* q = b + 16 + part * 32;
    float dd = lk_h(b + 80), dm = lk_h(b + 82);
    #pragma unroll
    for(int i = 0; i < 8; ++i) w[i] = (lk_w32(q + 4 * i) >> (2 * k)) & 0x03030303;
    #pragma unroll
    for(int h = 0; h < 2; ++h){ int sc = b[part * 8 + k * 2 + h]; d[h] = dd * (float)(sc & 0xf); c[h] = -(dm * (float)(sc >> 4)); }
  } else if(F == 11){
    const unsigned char* b = row + (long)(sb >> 3) * 110;
    int j = sb & 7, part = j >> 2, k = j & 3, bit = part * 4 + k;
    const unsigned char* q = b + 32 + part * 32;
    const unsigned char* p = b + 96;
    unsigned int a0 = (unsigned int)lk_w16(p), a1 = (unsigned int)lk_w16(p + 4), t = (unsigned int)lk_w16(p + 8);
    unsigned int aux[4] = {(a0 & 0x0f0f0f0fu) | ((t & 0x03030303u) << 4), (a1 & 0x0f0f0f0fu) | (((t >> 2) & 0x03030303u) << 4),
                           ((a0 >> 4) & 0x0f0f0f0fu) | (((t >> 4) & 0x03030303u) << 4), ((a1 >> 4) & 0x0f0f0f0fu) | (((t >> 6) & 0x03030303u) << 4)};
    float dd = lk_h(b + 108);
    #pragma unroll
    for(int i = 0; i < 8; ++i){
      int lo = (lk_w16(q + 4 * i) >> (2 * k)) & 0x03030303;
      int missing = (~(lk_w16(b + 4 * i) >> bit) & 0x01010101) << 2;
      w[i] = __vsub4(lo, missing);
    }
    #pragma unroll
    for(int h = 0; h < 2; ++h){
      int is = part * 8 + k * 2 + h;
      int sc = (signed char)((lk_pick4(aux, is >> 2) >> (8 * (is & 3))) & 0xff);
      d[h] = dd * (float)(sc - 32); c[h] = 0.f;
    }
  } else if(F == 14){
    const unsigned char* b = row + lk_q6_blk(sb >> 3);
    int j = sb & 7, part = j >> 2, k = j & 3;
    const uint4* qlw = (const uint4*)(b + part * 64 + ((k & 1) ? 32 : 0));
    const uint4* qhw = (const uint4*)(b + 128 + part * 32);
    const uint4 l0 = qlw[0], l1 = qlw[1], h0 = qhw[0], h1 = qhw[1];
    const unsigned int ql[8] = {l0.x, l0.y, l0.z, l0.w, l1.x, l1.y, l1.z, l1.w};
    const unsigned int qh[8] = {h0.x, h0.y, h0.z, h0.w, h1.x, h1.y, h1.z, h1.w};
    // The sub-block's two scales, one half-word.
    const unsigned short scw = *(const unsigned short*)(b + 192 + part * 8 + 2 * k);
    float dd = __half2float(__ushort_as_half(*(const unsigned short*)(row + lk_q6_d(sb >> 3))));
    #pragma unroll
    for(int i = 0; i < 8; ++i){
      int lo = ((int)ql[i] >> (4 * (k >> 1))) & 0x0F0F0F0F;
      int hi = (((int)qh[i] >> (2 * k)) & 0x03030303) << 4;
      w[i] = __vsub4(lo | hi, 0x20202020);
    }
    d[0] = dd * (float)(signed char)(scw & 0xff); d[1] = dd * (float)(signed char)(scw >> 8); c[0] = c[1] = 0.f;
  } else if(F == 20 || F == 23){
    const unsigned char* b = F == 20 ? row + (long)sb * 18 : row + (long)(sb >> 3) * 136;
    int ib = sb & 7;
    float dd = F == 20 ? lk_h(b) : lk_iq4xs_scale(lk_h(b), b[2] | (b[3] << 8), b[4 + (ib >> 1)], ib);
    const unsigned char* qs = F == 20 ? b + 2 : b + 8 + 16 * ib;
    #pragma unroll
    for(int i = 0; i < 4; ++i) lk_iq4_word((unsigned int)(F == 20 ? lk_w16(qs + 4 * i) : lk_w32(qs + 4 * i)), w[i], w[i + 4]);
    d[0] = d[1] = dd; c[0] = c[1] = 0.f;
  } else if(F == 39){
    uint4 n = *(const uint4*)(row + lk_mx_nib(sb));
    unsigned int q[4] = {n.x, n.y, n.z, n.w};
    #pragma unroll
    for(int i = 0; i < 4; ++i) lk_mxfp4_word(q[i], w[i], w[i + 4]);
    d[0] = d[1] = lk_e8m0_half(row[lk_mx_exp(sb)]); c[0] = c[1] = 0.f;
  } else {
    signed char v[32];
    lk_sub<F>(row, sb, v, d, c);
    #pragma unroll
    for(int i = 0; i < 8; ++i) w[i] = lk_pack4(v + 4 * i);
  }
}
// A sub-block's 32 weights (its levels `wv`, four to a word, and each half's scale `d` and offset `c`) dotted with an
// activation's 32 int8 words `h` (and its low words `l` under Q 2), before the activation's scale.
template<int Q>
__device__ __forceinline__ float lk_sub_dot(const int* wv, const float* d, const float* c, const int* h, const int* l){
  int dh0 = 0, dh1 = 0, sh0 = 0, sh1 = 0;
  #pragma unroll
  for(int i = 0; i < 4; ++i){ dh0 = __dp4a(wv[i], h[i], dh0); sh0 = __dp4a(0x01010101, h[i], sh0); }
  #pragma unroll
  for(int i = 4; i < 8; ++i){ dh1 = __dp4a(wv[i], h[i], dh1); sh1 = __dp4a(0x01010101, h[i], sh1); }
  float part = d[0] * (float)dh0 + c[0] * (float)sh0 + d[1] * (float)dh1 + c[1] * (float)sh1;
  if(Q == 2){
    int dl0 = 0, dl1 = 0, sl0 = 0, sl1 = 0;
    #pragma unroll
    for(int i = 0; i < 4; ++i){ dl0 = __dp4a(wv[i], l[i], dl0); sl0 = __dp4a(0x01010101, l[i], sl0); }
    #pragma unroll
    for(int i = 4; i < 8; ++i){ dl1 = __dp4a(wv[i], l[i], dl1); sl1 = __dp4a(0x01010101, l[i], sl1); }
    part += 0.0078125f * (d[0] * (float)dl0 + c[0] * (float)sl0 + d[1] * (float)dl1 + c[1] * (float)sl1);
  }
  return part;
}
// A 16-bit float weight (F 1, F16; F 30, BF16) widened to `f32`, exactly.
template<int F>
__device__ __forceinline__ float lk_h16(unsigned short h){
  if constexpr(F == 30) return __uint_as_float((unsigned)h << 16);
  else return __half2float(__ushort_as_half(h));
}
// The 16-bit tensor cores' product of k 16 (`F` 1, F16; 30, BF16): `c` += `a` · `b`, a fragment of 16 × 16 activation
// words by one of 16 × 8 weights, summed in `f32`.
template<int F>
__device__ __forceinline__ void lk_mma_h16(float* c, const unsigned* a, const unsigned* b){
  if constexpr(F == 30)
    asm volatile("mma.sync.aligned.m16n8k16.row.col.f32.bf16.bf16.f32 {%0,%1,%2,%3}, {%4,%5,%6,%7}, {%8,%9}, {%0,%1,%2,%3};\n"
                 : "+f"(c[0]), "+f"(c[1]), "+f"(c[2]), "+f"(c[3]) : "r"(a[0]), "r"(a[1]), "r"(a[2]), "r"(a[3]), "r"(b[0]), "r"(b[1]));
  else
    asm volatile("mma.sync.aligned.m16n8k16.row.col.f32.f16.f16.f32 {%0,%1,%2,%3}, {%4,%5,%6,%7}, {%8,%9}, {%0,%1,%2,%3};\n"
                 : "+f"(c[0]), "+f"(c[1]), "+f"(c[2]), "+f"(c[3]) : "r"(a[0]), "r"(a[1]), "r"(a[2]), "r"(a[3]), "r"(b[0]), "r"(b[1]));
}
// Two activations as a word of `F`'s 16-bit floats, each rounded to the nearest (the first the word's low half), and
// under two words (`Q` 2) what each leaves, rounded so too: the words a 16-bit product multiplies.
template<int F, int Q>
__device__ __forceinline__ void lk_h16_words(float x0, float x1, unsigned& hi, unsigned& lo){
  if constexpr(F == 30){
    const __nv_bfloat162 h = __floats2bfloat162_rn(x0, x1);
    hi = *(const unsigned*)&h;
    if constexpr(Q == 2){ const float2 b = __bfloat1622float2(h); const __nv_bfloat162 l = __floats2bfloat162_rn(x0 - b.x, x1 - b.y); lo = *(const unsigned*)&l; }
  } else {
    const __half2 h = __floats2half2_rn(x0, x1);
    hi = *(const unsigned*)&h;
    if constexpr(Q == 2){ const float2 b = __half22float2(h); const __half2 l = __floats2half2_rn(x0 - b.x, x1 - b.y); lo = *(const unsigned*)&l; }
  }
}
// One block of 32 `k` of a 16-bit product on the tensor cores, as every row count sums it: lane (`g`, `t`) holds its two
// activation rows' values `k0 + 8t … k0 + 8t + 7` (`xa` row g, `xb` row g + 8) and its output column's weights there
// (`w`, four words), and the block is two products of k 16, the lane's weights 0–3 and then 4–7, each the high words'
// product and then, under two words, the low words'. `lk_h16_take` rounds the activation's words once, `lk_h16_mul`
// multiplies them by one column's weights.
struct lk_h16_a { unsigned h[2][4], l[2][4]; };
template<int F, int Q>
__device__ __forceinline__ lk_h16_a lk_h16_take(const float* xa, const float* xb){
  lk_h16_a a;
  #pragma unroll
  for(int s = 0; s < 2; ++s){
    lk_h16_words<F, Q>(xa[4 * s], xa[4 * s + 1], a.h[s][0], a.l[s][0]);
    lk_h16_words<F, Q>(xb[4 * s], xb[4 * s + 1], a.h[s][1], a.l[s][1]);
    lk_h16_words<F, Q>(xa[4 * s + 2], xa[4 * s + 3], a.h[s][2], a.l[s][2]);
    lk_h16_words<F, Q>(xb[4 * s + 2], xb[4 * s + 3], a.h[s][3], a.l[s][3]);
  }
  return a;
}
template<int F, int Q>
__device__ __forceinline__ void lk_h16_mul(float* c, const lk_h16_a& a, const unsigned* w){
  #pragma unroll
  for(int s = 0; s < 2; ++s){
    const unsigned b[2] = {w[2 * s], w[2 * s + 1]};
    lk_mma_h16<F>(c, a.h[s], b);
    if constexpr(Q == 2) lk_mma_h16<F>(c, a.l[s], b);
  }
}
template<int F, int Q>
__device__ __forceinline__ void lk_h16_block(float* c, const float* xa, const float* xb, const unsigned* w){
  lk_h16_mul<F, Q>(c, lk_h16_take<F, Q>(xa, xb), w);
}
// Rows `row … row + W − 1` (`W` 1, 2, 4, 8 or 16, within one of the matrix's groups) of a 16-bit float matrix dotted with
// one activation row on the 16-bit tensor cores, a block of 32 `k` at a time as `lk_h16_block` sums it, the block's
// weights `AHEAD` blocks ahead of the one it multiplies in flight — a warp keeps that many blocks of its rows streaming,
// so the few warps a narrow matrix gives the chip still cover the memory's latency. Each block of the warp's rows is one
// run of `64·W` bytes (`lk_h16_at`), lane `l` its sixteen bytes `16l` on. Under eight rows (`lk_h16_rows_a`) the
// activation is the product's A, its first row of sixteen, and the weights its B, a row a column of eight; sixteen rows
// (`lk_h16_rows_b`) take the weights as A, rows `g` and `g + 8`, and the activation as B's first column — the same
// products at the same `k` of every m16n8k16, which sum alike whichever operand holds them. Every lane gets row `r`'s
// sum in `out[r]`. `K` is whole 32s. Over `M` activation rows (`lk_h16_mma_rows`, `M` at most 16) the rows under eight
// are the activation's rows `g` and `g + 8` of A, and sixteen take them as B's columns, eight to a tile, each output
// summed as the one row's: a lane keeps its own outputs (`lk_h16_each`). The weights stream past L1 (`lk_ld_stream`), so
// the activation the warps read a block at a time stays in it.
#define LK_H16_AHEAD 8
// Sixteen bytes of a matrix read once, left out of L1 (`L1::no_allocate`).
__device__ __forceinline__ uint4 lk_ld_stream(const uint4* p){
  uint4 v;
  asm("ld.global.nc.L1::no_allocate.v4.u32 {%0, %1, %2, %3}, [%4];" : "=r"(v.x), "=r"(v.y), "=r"(v.z), "=r"(v.w) : "l"(p));
  return v;
}
__device__ __forceinline__ void lk_h16_x(const float* __restrict__ xr, int b, bool first, float* xa){
  if(first){
    const float4 a = __ldg((const float4*)(xr + 32 * b)), e = __ldg((const float4*)(xr + 32 * b + 4));
    xa[0] = a.x; xa[1] = a.y; xa[2] = a.z; xa[3] = a.w; xa[4] = e.x; xa[5] = e.y; xa[6] = e.z; xa[7] = e.w;
  } else {
    #pragma unroll
    for(int j = 0; j < 8; ++j) xa[j] = 0.f;
  }
}
// A stage under eight rows: the activation's rows `g` (`xa`, where `ia`) and `g + 8` (`xb`, where `ib`) as A, the lane's
// weight row as B.
template<int F, int Q, int AHEAD>
__device__ __forceinline__ void lk_h16_rows_a(float* c, uint4* ahead, const uint4* wr, long step, const float* xa_row, const float* xb_row, int b, int nb, bool mine, bool ia, bool ib){
  const uint4 raw = *ahead;
  *ahead = mine && b + AHEAD < nb ? lk_ld_stream(wr + step * (b + AHEAD)) : make_uint4(0u, 0u, 0u, 0u);
  const unsigned wv[4] = {raw.x, raw.y, raw.z, raw.w};
  float xa[8], xb[8];
  lk_h16_x(xa_row, b, ia, xa);
  lk_h16_x(xb_row, b, ib, xb);
  lk_h16_block<F, Q>(c, xa, xb, wv);
}
// A stage of sixteen rows: the weights' rows `g` and `g + 8` as A, and in each of `NT` tiles the activation's row
// `8n + g` (`xr[n]`, where `in[n]`) as B's column.
template<int F, int Q, int AHEAD, int NT>
__device__ __forceinline__ void lk_h16_rows_b(float (&c)[NT][4], uint4 (&ahead)[2], const uint4* wr, long step, const float* const (&xr)[NT], const bool (&in)[NT], int b, int nb){
  const uint4 ra = ahead[0], rb = ahead[1];
  ahead[0] = b + AHEAD < nb ? lk_ld_stream(wr + step * (b + AHEAD)) : make_uint4(0u, 0u, 0u, 0u);
  ahead[1] = b + AHEAD < nb ? lk_ld_stream(wr + step * (b + AHEAD) + 32) : make_uint4(0u, 0u, 0u, 0u);
  const unsigned wa[4] = {ra.x, ra.y, ra.z, ra.w}, wb[4] = {rb.x, rb.y, rb.z, rb.w};
  #pragma unroll
  for(int n = 0; n < NT; ++n){
    float xa[8];
    lk_h16_x(xr[n], b, in[n], xa);
    unsigned xh[4] = {0u, 0u, 0u, 0u}, xl[4] = {0u, 0u, 0u, 0u};
    #pragma unroll
    for(int i = 0; i < 4; ++i) lk_h16_words<F, Q>(xa[2 * i], xa[2 * i + 1], xh[i], xl[i]);
    #pragma unroll
    for(int s = 0; s < 2; ++s){
      const unsigned a[4] = {wa[2 * s], wb[2 * s], wa[2 * s + 1], wb[2 * s + 1]};
      const unsigned bh[2] = {xh[2 * s], xh[2 * s + 1]};
      lk_mma_h16<F>(c[n], a, bh);
      if constexpr(Q == 2){ const unsigned bl[2] = {xl[2 * s], xl[2 * s + 1]}; lk_mma_h16<F>(c[n], a, bl); }
    }
  }
}
// The products of rows `row … row + W − 1` with the `M` activation rows `ld` apart, each lane's in `c`: one tile of
// four under eight rows, `⌈M/8⌉` of sixteen rows.
template<int W, int M> __host__ __device__ constexpr int lk_h16_tiles(){ return W == 16 ? (M + 7) / 8 : 1; }
template<int F, int Q, int W, int M, int AHEAD = LK_H16_AHEAD>
__device__ __forceinline__ void lk_h16_mma_rows(lk_w w, long row, int K, const float* __restrict__ x, long ld, float (&c)[lk_h16_tiles<W, M>()][4]){
  static_assert(M <= 16 && (W == 1 || W == 2 || W == 4 || W == 8 || W == 16), "at most sixteen activation rows, sixteen rows a warp");
  constexpr int NT = lk_h16_tiles<W, M>();
  const int lane = threadIdx.x & 31, g = lane >> 2, t = lane & 3, nb = K >> 5;
  const uint4* wr = (const uint4*)lk_h16_at(w, row, 0) + lane;
  const long step = 4L * w.g;
  #pragma unroll
  for(int n = 0; n < NT; ++n)
    #pragma unroll
    for(int e = 0; e < 4; ++e) c[n][e] = 0.f;
  if constexpr(W == 16){
    const float* xr[NT];
    bool in[NT];
    #pragma unroll
    for(int n = 0; n < NT; ++n){ in[n] = 8 * n + g < M; xr[n] = x + (in[n] ? (long)(8 * n + g) * ld : 0) + 8 * t; }
    uint4 ahead[AHEAD][2];
    #pragma unroll
    for(int i = 0; i < AHEAD; ++i){
      ahead[i][0] = i < nb ? lk_ld_stream(wr + step * i) : make_uint4(0u, 0u, 0u, 0u);
      ahead[i][1] = i < nb ? lk_ld_stream(wr + step * i + 32) : make_uint4(0u, 0u, 0u, 0u);
    }
    int b = 0;
    for(; b + AHEAD <= nb; b += AHEAD){
      #pragma unroll
      for(int i = 0; i < AHEAD; ++i) lk_h16_rows_b<F, Q, AHEAD, NT>(c, ahead[i], wr, step, xr, in, b + i, nb);
    }
    #pragma unroll
    for(int i = 0; i < AHEAD; ++i)
      if(b + i < nb) lk_h16_rows_b<F, Q, AHEAD, NT>(c, ahead[i], wr, step, xr, in, b + i, nb);
  } else {
    const bool mine = g < W, ia = g < M, ib = g + 8 < M;
    const float* xa = x + (ia ? (long)g * ld : 0) + 8 * t;
    const float* xb = x + (ib ? (long)(g + 8) * ld : 0) + 8 * t;
    uint4 ahead[AHEAD];
    #pragma unroll
    for(int i = 0; i < AHEAD; ++i) ahead[i] = mine && i < nb ? lk_ld_stream(wr + step * i) : make_uint4(0u, 0u, 0u, 0u);
    int b = 0;
    for(; b + AHEAD <= nb; b += AHEAD){
      #pragma unroll
      for(int i = 0; i < AHEAD; ++i) lk_h16_rows_a<F, Q, AHEAD>(c[0], &ahead[i], wr, step, xa, xb, b + i, nb, mine, ia, ib);
    }
    #pragma unroll
    for(int i = 0; i < AHEAD; ++i)
      if(b + i < nb) lk_h16_rows_a<F, Q, AHEAD>(c[0], &ahead[i], wr, step, xa, xb, b + i, nb, mine, ia, ib);
  }
}
// `lk_h16_mma_rows` over two matrices of one format at once, a gated pair's gate `wg` and up `wu`: each stage the
// activation's words rounded once for both and both matrices' blocks in flight, each matrix's sums as its own row dot
// sums them; sixteen rows a warp keep half as many stages of each in flight, the registers of one matrix's.
template<int W> __host__ __device__ constexpr int lk_h16_pair_ahead(){ return W == 16 ? LK_H16_AHEAD / 2 : LK_H16_AHEAD; }
template<int F, int Q, int W, int M, int AHEAD = lk_h16_pair_ahead<W>()>
__device__ __forceinline__ void lk_h16_mma_pair(lk_w wg, lk_w wu, long row, int K, const float* __restrict__ x, long ld,
                                                float (&cg)[lk_h16_tiles<W, M>()][4], float (&cu)[lk_h16_tiles<W, M>()][4]){
  static_assert(M <= 16 && (W == 1 || W == 2 || W == 4 || W == 8 || W == 16), "at most sixteen activation rows, sixteen rows a warp");
  constexpr int NT = lk_h16_tiles<W, M>();
  const int lane = threadIdx.x & 31, g = lane >> 2, t = lane & 3, nb = K >> 5;
  const uint4* rg = (const uint4*)lk_h16_at(wg, row, 0) + lane;
  const uint4* ru = (const uint4*)lk_h16_at(wu, row, 0) + lane;
  const long sg = 4L * wg.g, su = 4L * wu.g;
  #pragma unroll
  for(int n = 0; n < NT; ++n)
    #pragma unroll
    for(int e = 0; e < 4; ++e){ cg[n][e] = 0.f; cu[n][e] = 0.f; }
  if constexpr(W == 16){
    const float* xr[NT];
    bool in[NT];
    #pragma unroll
    for(int n = 0; n < NT; ++n){ in[n] = 8 * n + g < M; xr[n] = x + (in[n] ? (long)(8 * n + g) * ld : 0) + 8 * t; }
    uint4 ag[AHEAD][2], au[AHEAD][2];
    #pragma unroll
    for(int i = 0; i < AHEAD; ++i){
      ag[i][0] = i < nb ? lk_ld_stream(rg + sg * i) : make_uint4(0u, 0u, 0u, 0u);
      ag[i][1] = i < nb ? lk_ld_stream(rg + sg * i + 32) : make_uint4(0u, 0u, 0u, 0u);
      au[i][0] = i < nb ? lk_ld_stream(ru + su * i) : make_uint4(0u, 0u, 0u, 0u);
      au[i][1] = i < nb ? lk_ld_stream(ru + su * i + 32) : make_uint4(0u, 0u, 0u, 0u);
    }
    int b = 0;
    for(; b + AHEAD <= nb; b += AHEAD){
      #pragma unroll
      for(int i = 0; i < AHEAD; ++i){
        lk_h16_rows_b<F, Q, AHEAD, NT>(cg, ag[i], rg, sg, xr, in, b + i, nb);
        lk_h16_rows_b<F, Q, AHEAD, NT>(cu, au[i], ru, su, xr, in, b + i, nb);
      }
    }
    #pragma unroll
    for(int i = 0; i < AHEAD; ++i)
      if(b + i < nb){
        lk_h16_rows_b<F, Q, AHEAD, NT>(cg, ag[i], rg, sg, xr, in, b + i, nb);
        lk_h16_rows_b<F, Q, AHEAD, NT>(cu, au[i], ru, su, xr, in, b + i, nb);
      }
  } else {
    const bool mine = g < W, ia = g < M, ib = g + 8 < M;
    const float* xa = x + (ia ? (long)g * ld : 0) + 8 * t;
    const float* xb = x + (ib ? (long)(g + 8) * ld : 0) + 8 * t;
    uint4 ag[AHEAD], au[AHEAD];
    #pragma unroll
    for(int i = 0; i < AHEAD; ++i){
      ag[i] = mine && i < nb ? lk_ld_stream(rg + sg * i) : make_uint4(0u, 0u, 0u, 0u);
      au[i] = mine && i < nb ? lk_ld_stream(ru + su * i) : make_uint4(0u, 0u, 0u, 0u);
    }
    int b = 0;
    for(; b + AHEAD <= nb; b += AHEAD){
      #pragma unroll
      for(int i = 0; i < AHEAD; ++i){
        lk_h16_rows_a<F, Q, AHEAD>(cg[0], &ag[i], rg, sg, xa, xb, b + i, nb, mine, ia, ib);
        lk_h16_rows_a<F, Q, AHEAD>(cu[0], &au[i], ru, su, xa, xb, b + i, nb, mine, ia, ib);
      }
    }
    #pragma unroll
    for(int i = 0; i < AHEAD; ++i)
      if(b + i < nb){
        lk_h16_rows_a<F, Q, AHEAD>(cg[0], &ag[i], rg, sg, xa, xb, b + i, nb, mine, ia, ib);
        lk_h16_rows_a<F, Q, AHEAD>(cu[0], &au[i], ru, su, xa, xb, b + i, nb, mine, ia, ib);
      }
  }
}
// Whether a product by a matrix in format `F` under numerics `Q` sums on the 16-bit tensor cores in a GEMV's lanes at
// every row count (`lk_h16_mma_rows`, `lk_h16_mma_pair`): a 16-bit matrix under the quantized numerics.
template<int F, int Q> __host__ __device__ constexpr bool lk_h16_lanes(){ return (F == 1 || F == 30) && (Q == 1 || Q == 2); }
// Whether a product of `M` rows by a matrix in format `F` under numerics `Q` sums on the 16-bit tensor cores in a GEMV's
// lanes (`lk_h16_mma_rows`): a 16-bit matrix under the quantized numerics, more than one row.
template<int F, int Q, int M> __host__ __device__ constexpr bool lk_h16_rows(){ return (F == 1 || F == 30) && (Q == 1 || Q == 2) && M > 1; }
// Each output of `lk_h16_mma_rows` lane `lane` holds, as `f(r, a, n, e)` takes it: weight row `r` of the warp's, activation
// row `a`, at `c[n][e]` — under eight rows lane (`g`, `t`) the activation's rows `g`, `g + 8` by weight rows `2t`, `2t + 1`;
// at sixteen weight rows `g`, `g + 8` by the activation's rows `8n + 2t`, `8n + 2t + 1`.
template<int W, int M, typename Fn>
__device__ __forceinline__ void lk_h16_each(Fn f){
  const int lane = threadIdx.x & 31, g = lane >> 2, t = lane & 3;
  #pragma unroll
  for(int n = 0; n < lk_h16_tiles<W, M>(); ++n)
    #pragma unroll
    for(int e = 0; e < 4; ++e){
      const int r = W == 16 ? g + (e >= 2 ? 8 : 0) : 2 * t + (e & 1);
      const int a = W == 16 ? 8 * n + 2 * t + (e & 1) : g + (e >= 2 ? 8 : 0);
      if(r < W && a < M) f(r, a, n, e);
    }
}
template<int F, int Q, int W, int AHEAD = LK_H16_AHEAD>
__device__ __forceinline__ void lk_h16_mma_dot(lk_w w, long row, int K, const float* __restrict__ x, float* out){
  float c[1][4];
  lk_h16_mma_rows<F, Q, W, 1, AHEAD>(w, row, K, x, 0, c);
  if constexpr(W == 16){
    #pragma unroll
    for(int r = 0; r < 16; ++r) out[r] = __shfl_sync(0xffffffffu, r < 8 ? c[0][0] : c[0][2], 4 * (r & 7));
  } else {
    #pragma unroll
    for(int r = 0; r < W; ++r) out[r] = __shfl_sync(0xffffffffu, (r & 1) ? c[0][1] : c[0][0], r >> 1);
  }
}
// Rows `row … row + W − 1` of a 16-bit float matrix dotted with the `M` activation rows `ld` apart, read `f32` under
// every numerics: a lane takes eight weights at a time, sixteen bytes, widened in registers, and their eight
// activations; each lane sums its products in order in `A` (`f64` under the precise numerics, `f32` otherwise), then
// the warp's sums are added. `K` is whole eights.
template<int F, int Q, int M, int W>
__device__ __forceinline__ void lk_h16_dot(lk_w w, long row, int K, const float* __restrict__ x, long ld, float* out){
  typedef typename lk_acc<Q == 3 ? 1 : Q>::t A;
  const int lane = threadIdx.x & 31;
  A acc[W][M];
  #pragma unroll
  for(int r = 0; r < W; ++r)
    #pragma unroll
    for(int t = 0; t < M; ++t) acc[r][t] = 0;
  #pragma unroll 4
  for(int k = lane * 8; k < K; k += 256){
    #pragma unroll
    for(int r = 0; r < W; ++r){
      const uint4 raw = *(const uint4*)(w.b + (row + r) * w.rb + (long)k * 2);
      const unsigned int word[4] = {raw.x, raw.y, raw.z, raw.w};
      float v[8];
      #pragma unroll
      for(int i = 0; i < 4; ++i){ v[2 * i] = lk_h16<F>((unsigned short)word[i]); v[2 * i + 1] = lk_h16<F>((unsigned short)(word[i] >> 16)); }
      #pragma unroll
      for(int t = 0; t < M; ++t){
        const float4 a = *(const float4*)(x + (long)t * ld + k), b = *(const float4*)(x + (long)t * ld + k + 4);
        const float xv[8] = {a.x, a.y, a.z, a.w, b.x, b.y, b.z, b.w};
        #pragma unroll
        for(int i = 0; i < 8; ++i) acc[r][t] += (A)v[i] * (A)xv[i];
      }
    }
  }
  #pragma unroll
  for(int r = 0; r < W; ++r)
    #pragma unroll
    for(int t = 0; t < M; ++t) out[r * M + t] = (float)lk_warp_sum(acc[r][t]);
}
// Rows `row … row + W − 1` of `w` dotted with the `M` activation rows `ld` apart (f32 `x` under Q 0 and 3, the int8
// words `xq` (and `xl`) with a scale per 32 in `xs` otherwise; a 16-bit float matrix's always `f32`, `lk_h16_dot`),
// row `r`'s dot with activation row `t` in `out[r·M + t]` —
// one warp, every lane's sums reduced. Each row's dot is the same whatever `W`: the rows only share the activation's
// loads. Under Q 0 each weight is dequantized as the CPU dequantizer rounds it and summed in f64; under Q 3 a lane
// takes half a sub-block, its levels a word at a time and its inputs sixteen bytes at a time, and sums each half's
// `d·Σ level·x + c·Σ x` in f32.
template<int F, int M, int W>
__device__ __forceinline__ void lk_row_dot_f32(lk_w w, long row, int K, const float* __restrict__ x, long ld, float* out){
  int lane = threadIdx.x & 31, nb = K >> 5;
  const unsigned char* r0 = w.b + row * w.rb;
  float acc[W][M];
  #pragma unroll
  for(int r = 0; r < W; ++r)
    #pragma unroll
    for(int t = 0; t < M; ++t) acc[r][t] = 0.f;
  for(int u = lane; u < 2 * nb; u += 32){
    int sb = u >> 1, hf = u & 1;
    #pragma unroll
    for(int r = 0; r < W; ++r){
      int wv[8]; float d[2], c[2];
      lk_sub4<F>(r0 + r * w.rb, sb, wv, d, c);
      #pragma unroll
      for(int t = 0; t < M; ++t){
        const float4* xt = (const float4*)(x + (long)t * ld + sb * 32 + 16 * hf);
        float sd = 0.f, sx = 0.f;
        #pragma unroll
        for(int k = 0; k < 4; ++k){
          float4 xv = xt[k];
          int lv = wv[4 * hf + k];
          sd = fmaf((float)(signed char)lv, xv.x, sd); sd = fmaf((float)(signed char)(lv >> 8), xv.y, sd);
          sd = fmaf((float)(signed char)(lv >> 16), xv.z, sd); sd = fmaf((float)(signed char)(lv >> 24), xv.w, sd);
          sx += (xv.x + xv.y) + (xv.z + xv.w);
        }
        acc[r][t] = fmaf(d[hf], sd, fmaf(c[hf], sx, acc[r][t]));
      }
    }
  }
  #pragma unroll
  for(int r = 0; r < W; ++r)
    #pragma unroll
    for(int t = 0; t < M; ++t) out[r * M + t] = lk_warp_sum(acc[r][t]);
}
// Sub-blocks `sb` and `sb + 1` (`sb` even) of a Q4_K row, each as `lk_sub4` decodes it, from one read of the words they
// share: the super-block's header (d, dmin, the twelve scale bytes) and the pair's 32 bytes of nibbles, the first
// sub-block their low nibbles and the second their high; each sub-block's six-bit scale and offset by shifts, so the
// header stays in registers.
// A lane's activation words of a pair of sub-blocks (64 bytes) and of a quarter (128), in 16-byte words.
__device__ __forceinline__ void lk_ld64(int (&h)[16], const signed char* p){
  const uint4* q = (const uint4*)p;
  #pragma unroll
  for(int i = 0; i < 4; ++i){ uint4 v = __ldg(q + i); h[4 * i] = (int)v.x; h[4 * i + 1] = (int)v.y; h[4 * i + 2] = (int)v.z; h[4 * i + 3] = (int)v.w; }
}
__device__ __forceinline__ void lk_ld128(int (&h)[32], const signed char* p){
  const uint4* q = (const uint4*)p;
  #pragma unroll
  for(int i = 0; i < 8; ++i){ uint4 v = __ldg(q + i); h[4 * i] = (int)v.x; h[4 * i + 1] = (int)v.y; h[4 * i + 2] = (int)v.z; h[4 * i + 3] = (int)v.w; }
}
__device__ __forceinline__ void lk_pair4_q4k(const unsigned char* __restrict__ row, int sb, int (&w)[2][8],
                                             float (&d)[2][2], float (&c)[2][2]){
  const unsigned char* b = row + (long)(sb >> 3) * 144;
  const int j = sb & 7, p = j >> 1;
  const uint4* qw = (const uint4*)(b + 16 + p * 32);
  const uint4 hd = *(const uint4*)b, q0 = qw[0], q1 = qw[1];
  const unsigned long long lo = (unsigned long long)hd.y | ((unsigned long long)hd.z << 32);
  const float hdd = __half2float(__ushort_as_half((unsigned short)(hd.x & 0xffff)));
  const float hdm = __half2float(__ushort_as_half((unsigned short)(hd.x >> 16)));
  #pragma unroll
  for(int e = 0; e < 2; ++e){
    const int k = j + e;
    const int b0 = (int)((lo >> (8 * (k & 7))) & 0xff), bm4 = (int)((lo >> (8 * ((k - 4) & 7))) & 0xff);
    const int b4 = k + 4 < 8 ? (int)((lo >> (8 * (k + 4))) & 0xff) : (int)((hd.w >> (8 * (k - 4))) & 0xff);
    const int sc = k < 4 ? (b0 & 63) : ((b4 & 0xf) | ((bm4 >> 6) << 4));
    const int m = k < 4 ? (b4 & 63) : ((b4 >> 4) | ((b0 >> 6) << 4));
    d[e][0] = d[e][1] = hdd * (float)sc; c[e][0] = c[e][1] = -hdm * (float)m;
  }
  const unsigned int q[8] = {q0.x, q0.y, q0.z, q0.w, q1.x, q1.y, q1.z, q1.w};
  #pragma unroll
  for(int i = 0; i < 8; ++i){ w[0][i] = (int)(q[i] & 0x0F0F0F0Fu); w[1][i] = (int)((q[i] >> 4) & 0x0F0F0F0Fu); }
}
// Sub-blocks `sb` … `sb + 3` (`sb` a multiple of four: a quarter) of a grouped Q6_K row, each as `lk_sub4` decodes it,
// from one read of the words they share: the quarter's 64 bytes of low nibbles, its 32 of high bits, its eight sub-block
// scales and its block's scale.
__device__ __forceinline__ void lk_quarter4_q6k(const unsigned char* __restrict__ row, int sb, int (&w)[4][8],
                                                float (&d)[4][2]){
  const unsigned char* b = row + lk_q6_blk(sb >> 3);
  const int part = (sb & 7) >> 2;
  const uint4* qlw = (const uint4*)(b + part * 64);
  const uint4* qhw = (const uint4*)(b + 128 + part * 32);
  const uint4 l0 = qlw[0], l1 = qlw[1], l2 = qlw[2], l3 = qlw[3], h0 = qhw[0], h1 = qhw[1];
  const uint2 scw = *(const uint2*)(b + 192 + part * 8);
  const float dd = __half2float(__ushort_as_half(*(const unsigned short*)(row + lk_q6_d(sb >> 3))));
  const unsigned int ql[16] = {l0.x, l0.y, l0.z, l0.w, l1.x, l1.y, l1.z, l1.w,
                               l2.x, l2.y, l2.z, l2.w, l3.x, l3.y, l3.z, l3.w};
  const unsigned int qh[8] = {h0.x, h0.y, h0.z, h0.w, h1.x, h1.y, h1.z, h1.w};
  #pragma unroll
  for(int k = 0; k < 4; ++k){
    #pragma unroll
    for(int i = 0; i < 8; ++i){
      const int lo = ((int)ql[(k & 1) * 8 + i] >> (4 * (k >> 1))) & 0x0F0F0F0F;
      const int hi = (((int)qh[i] >> (2 * k)) & 0x03030303) << 4;
      w[k][i] = __vsub4(lo | hi, 0x20202020);
    }
    const unsigned int sw = k < 2 ? scw.x : scw.y;
    d[k][0] = dd * (float)(signed char)((sw >> (16 * (k & 1))) & 0xff);
    d[k][1] = dd * (float)(signed char)((sw >> (16 * (k & 1) + 8)) & 0xff);
  }
}
template<int F, int Q, int M, int W = 1>
__device__ __forceinline__ void lk_row_dot_blk(lk_w w, long row, int K, const float* __restrict__ x,
                                               const signed char* __restrict__ xq, const signed char* __restrict__ xl,
                                               const float* __restrict__ xs, long ld, float* out){
  int lane = threadIdx.x & 31, nb = K >> 5;
  const unsigned char* r0 = w.b + row * w.rb;
  if(Q == 3){
    lk_row_dot_f32<F, M, W>(w, row, K, x, ld, out);
  } else if(Q == 0){
    typedef typename lk_acc<Q>::t A;
    A acc[W][M];
    #pragma unroll
    for(int r = 0; r < W; ++r)
      #pragma unroll
      for(int t = 0; t < M; ++t) acc[r][t] = 0;
    for(int sb = lane; sb < nb; sb += 32){
      #pragma unroll
      for(int r = 0; r < W; ++r){
        signed char v[32]; float d[2], c[2];
        lk_sub<F>(r0 + r * w.rb, sb, v, d, c);
        #pragma unroll
        for(int t = 0; t < M; ++t){
          const float* xt = x + (long)t * ld + sb * 32;
          #pragma unroll
          for(int i = 0; i < 32; ++i) acc[r][t] += (A)__fadd_rn(__fmul_rn(d[i >> 4], (float)v[i]), c[i >> 4]) * (A)xt[i];
        }
      }
    }
    #pragma unroll
    for(int r = 0; r < W; ++r)
      #pragma unroll
      for(int t = 0; t < M; ++t) out[r * M + t] = (float)lk_warp_sum(acc[r][t]);
  } else {
    float acc[W][M];
    #pragma unroll
    for(int r = 0; r < W; ++r)
      #pragma unroll
      for(int t = 0; t < M; ++t) acc[r][t] = 0.f;
    static_assert(W == 1 || M == 1, "several rows a warp meet one activation row");
    for(int sb = lane; sb < nb; sb += 32){
      if constexpr(F == 12){
        // A lane a pair of sub-blocks, `2·sb` and the next, decoded from the words they share, each dotted as
        // `lk_sub_dot` dots it — the decode step's rows a warp and a chunk's rows alike.
        const int s2 = 2 * sb;
        if(s2 >= nb) continue;
        if(W == 1){
          int wv[2][8]; float d[2][2], c[2][2];
          lk_pair4_q4k(r0, s2, wv, d, c);
          #pragma unroll
          for(int t = 0; t < M; ++t){
            int h[16], l[16];
            lk_ld64(h, xq + (long)t * ld + s2 * 32);
            if(Q == 2) lk_ld64(l, xl + (long)t * ld + s2 * 32);
            const float* s = xs + (long)t * (ld >> 5) + s2;
            acc[0][t] += s[0] * lk_sub_dot<Q>(wv[0], d[0], c[0], h, Q == 2 ? l : nullptr);
            acc[0][t] += s[1] * lk_sub_dot<Q>(wv[1], d[1], c[1], h + 8, Q == 2 ? l + 8 : nullptr);
          }
        } else {
          int h[16], l[16];
          lk_ld64(h, xq + s2 * 32);
          if(Q == 2) lk_ld64(l, xl + s2 * 32);
          const float x0 = xs[s2], x1 = xs[s2 + 1];
          #pragma unroll
          for(int r = 0; r < W; ++r){
            int wv[2][8]; float d[2][2], c[2][2];
            lk_pair4_q4k(r0 + r * w.rb, s2, wv, d, c);
            acc[r][0] += x0 * lk_sub_dot<Q>(wv[0], d[0], c[0], h, Q == 2 ? l : nullptr);
            acc[r][0] += x1 * lk_sub_dot<Q>(wv[1], d[1], c[1], h + 8, Q == 2 ? l + 8 : nullptr);
          }
        }
      } else
      if constexpr(F == 14){
        // A lane a quarter of a block, sub-blocks `4·sb` … `4·sb + 3`, decoded from the words they share, each dotted as
        // `lk_sub_dot` dots it — the decode step's rows a warp and a chunk's rows alike.
        const int s4 = 4 * sb;
        if(s4 >= nb) continue;
        const float zero[2] = {0.f, 0.f};
        if(W == 1){
          int wv[4][8]; float d[4][2];
          lk_quarter4_q6k(r0, s4, wv, d);
          #pragma unroll
          for(int t = 0; t < M; ++t){
            int h[32], l[32];
            lk_ld128(h, xq + (long)t * ld + s4 * 32);
            if(Q == 2) lk_ld128(l, xl + (long)t * ld + s4 * 32);
            const float* s = xs + (long)t * (ld >> 5) + s4;
            #pragma unroll
            for(int e = 0; e < 4; ++e)
              acc[0][t] += s[e] * lk_sub_dot<Q>(wv[e], d[e], zero, h + 8 * e, Q == 2 ? l + 8 * e : nullptr);
          }
        } else {
          int h[32], l[32];
          lk_ld128(h, xq + s4 * 32);
          if(Q == 2) lk_ld128(l, xl + s4 * 32);
          float x[4];
          #pragma unroll
          for(int e = 0; e < 4; ++e) x[e] = xs[s4 + e];
          #pragma unroll
          for(int r = 0; r < W; ++r){
            int wv[4][8]; float d[4][2];
            lk_quarter4_q6k(r0 + r * w.rb, s4, wv, d);
            #pragma unroll
            for(int e = 0; e < 4; ++e)
              acc[r][0] += x[e] * lk_sub_dot<Q>(wv[e], d[e], zero, h + 8 * e, Q == 2 ? l + 8 * e : nullptr);
          }
        }
      } else
      if(W == 1){
        // The weight row decoded once, dotted with each activation row in turn.
        int wv[8]; float d[2], c[2];
        lk_sub4<F>(r0, sb, wv, d, c);
        #pragma unroll
        for(int t = 0; t < M; ++t){
          const int* l = Q == 2 ? (const int*)(xl + (long)t * ld) + sb * 8 : nullptr;
          acc[0][t] += xs[(long)t * (ld >> 5) + sb] * lk_sub_dot<Q>(wv, d, c, (const int*)(xq + (long)t * ld) + sb * 8, l);
        }
      } else {
        // The activation's words loaded once, each weight row decoded and dotted in turn.
        const int* hp = (const int*)xq + sb * 8;
        const int* lp = Q == 2 ? (const int*)xl + sb * 8 : nullptr;
        int h[8], l[8];
        #pragma unroll
        for(int i = 0; i < 8; ++i){ h[i] = hp[i]; l[i] = Q == 2 ? lp[i] : 0; }
        float s = xs[sb];
        #pragma unroll
        for(int r = 0; r < W; ++r){
          int wv[8]; float d[2], c[2];
          lk_sub4<F>(r0 + r * w.rb, sb, wv, d, c);
          acc[r][0] += s * lk_sub_dot<Q>(wv, d, c, h, Q == 2 ? l : nullptr);
        }
      }
    }
    #pragma unroll
    for(int r = 0; r < W; ++r)
      #pragma unroll
      for(int t = 0; t < M; ++t) out[r * M + t] = lk_warp_sum(acc[r][t]);
  }
}
template<int F, int Q, int M, int W = 1, int AHEAD = LK_H16_AHEAD>
__device__ __forceinline__ void lk_row_dot(lk_w w, long row, int K, const float* __restrict__ x,
                                           const signed char* __restrict__ xq, const signed char* __restrict__ xl,
                                           const float* __restrict__ xs, long ld, float* out){
  if constexpr((F == 1 || F == 30) && (Q == 1 || Q == 2)){
    static_assert(M == 1 && (W == 1 || W == 2 || W == 4 || W == 8 || W == 16), "a 16-bit product under the quantized numerics is one row on the tensor cores, at most sixteen rows a warp");
    lk_h16_mma_dot<F, Q, W, AHEAD>(w, row, K, x, out);
  } else if constexpr(F == 1 || F == 30) lk_h16_dot<F, Q, M, W>(w, row, K, x, ld, out);
  else lk_row_dot_blk<F, Q, M, W>(w, row, K, x, xq, xl, xs, ld, out);
}
template<int F, int Q, int M>
__device__ __forceinline__ void lk_dot(lk_w w, long row, int K, const float* __restrict__ x,
                                       const signed char* __restrict__ xq, const signed char* __restrict__ xl,
                                       const float* __restrict__ xs, long ld, float* out){
  if(F != 8){ lk_row_dot<F, Q, M>(w, row, K, x, xq, xl, xs, ld, out); return; }
  int lane = threadIdx.x & 31;
  const signed char* wq = w.q + row * K;
  const __half* ws = w.s + row * (K >> 5);
  if(Q == 3){
    // A lane four levels at a time, each block's `d·Σ level·x` in f32.
    float acc[M];
    #pragma unroll
    for(int t = 0; t < M; ++t) acc[t] = 0.f;
    const int* w4 = (const int*)wq;
    for(int g = lane; g < (K >> 2); g += 32){
      int lv = w4[g];
      float d = __half2float(ws[g >> 3]);
      #pragma unroll
      for(int t = 0; t < M; ++t){
        float4 xv = *(const float4*)(x + (long)t * ld + 4 * g);
        float sd = (float)(signed char)lv * xv.x;
        sd = fmaf((float)(signed char)(lv >> 8), xv.y, sd); sd = fmaf((float)(signed char)(lv >> 16), xv.z, sd);
        sd = fmaf((float)(signed char)(lv >> 24), xv.w, sd);
        acc[t] = fmaf(d, sd, acc[t]);
      }
    }
    #pragma unroll
    for(int t = 0; t < M; ++t) out[t] = lk_warp_sum(acc[t]);
  } else if(Q == 0){
    typedef typename lk_acc<Q>::t A;
    A acc[M];
    #pragma unroll
    for(int t = 0; t < M; ++t) acc[t] = 0;
    for(int k = lane; k < K; k += 32){
      A wv = (A)__half2float(ws[k >> 5]) * (A)wq[k];
      #pragma unroll
      for(int t = 0; t < M; ++t) acc[t] += wv * (A)x[(long)t * ld + k];
    }
    #pragma unroll
    for(int t = 0; t < M; ++t) out[t] = (float)lk_warp_sum(acc[t]);
  } else {
    float acc[M];
    #pragma unroll
    for(int t = 0; t < M; ++t) acc[t] = 0.f;
    const int* w4 = (const int*)wq;
    for(int g = lane; g < (K >> 2); g += 32){
      int blk = g >> 3, wv = w4[g];
      float wsc = __half2float(ws[blk]);
      #pragma unroll
      for(int t = 0; t < M; ++t){
        float s = xs[(long)t * (ld >> 5) + blk];
        int d = __dp4a(wv, ((const int*)(xq + (long)t * ld))[g], 0);
        if(Q == 1) acc[t] += (float)d * (wsc * s);
        else acc[t] += wsc * (s * (float)d + s * 0.0078125f * (float)__dp4a(wv, ((const int*)(xl + (long)t * ld))[g], 0));
      }
    }
    #pragma unroll
    for(int t = 0; t < M; ++t) out[t] = lk_warp_sum(acc[t]);
  }
}
// Rows `o … o + W − 1` of a matrix in format `F` (8: Q8_0, split) dotted with the `M` activation rows of `K` values as
// the plain GEMV of that format dots them, row `r`'s dots at `out + r·M` — one warp of a block of at most `WARPS`, a
// 16-bit matrix's weights `AHEAD` blocks ahead in flight (fewer in a kernel whose blocks leave a thread few registers).
template<int F, int Q, int M, int W = 1, int WARPS = 8, int AHEAD = LK_H16_AHEAD>
__device__ __forceinline__ void lk_gemv_dot(lk_w w, long o, int K, const float* __restrict__ x,
                                            const signed char* __restrict__ xq, const signed char* __restrict__ xl,
                                            const float* __restrict__ xs, float* out){
  if constexpr(F == 8) lk_q8_rows_dot<Q, M, W, WARPS>(w.q, w.s, o, K, x, xq, xl, xs, out);
  else lk_row_dot<F, Q, M, W, AHEAD>(w, o, K, x, xq, xl, xs, K, out);
}
// A weight sub-block's raw words, fetched ahead of their decoding so their loads are in flight while the kernel works
// on what it fetched before: the bytes `lk_sub4` reads for it, by the widest loads the format's alignment allows (16
// bytes in Q4_K, Q5_K, Q8_0's levels and MXFP4's nibbles, 4 in Q4_1, Q5_1, Q2_K and IQ4_XS, 2 in Q4_0, Q5_0, Q3_K, Q6_K
// and IQ4_NL, 1 in MXFP4's exponent).
template<int F> struct lk_raw { };
template<> struct lk_raw<2> { unsigned int q[4]; unsigned short d; };
template<> struct lk_raw<3> { unsigned int q[4], dm; };
template<> struct lk_raw<6> { unsigned int q[4], qh; unsigned short d; };
template<> struct lk_raw<7> { unsigned int q[4], qh, dm; };
template<> struct lk_raw<8> { uint4 q0, q1; unsigned short d; };
template<> struct lk_raw<10> { unsigned int q[8], dm; unsigned short sc; };
template<> struct lk_raw<11> { unsigned int q[8], hm[8], a0, a1, t; unsigned short d; };
template<> struct lk_raw<12> { uint4 h, q0, q1; };
template<> struct lk_raw<13> { uint4 h, qh0, qh1, q0, q1; };
template<> struct lk_raw<14> { uint4 l0, l1, h0, h1; unsigned short sc, d; };
// Two sub-blocks of a stage that read the same words, fetched as one (`lk_fetch_pair`): Q6_K's `j` and `j + 2` of a
// 128-value half, whose low nibbles are the same 32 bytes and whose high bits the half's 32 — the first's raw words
// and the second's own scales; `lk_pair_stride` is the second's distance, 0 where a format's sub-blocks share none.
template<int F> __host__ __device__ constexpr int lk_pair_stride(){ return F == 14 ? 2 : 0; }
template<int F> struct lk_raw_pair { lk_raw<F> a; unsigned short sc2; };
template<> struct lk_raw<39> { unsigned int q[4]; unsigned char e; };
template<> struct lk_raw<20> { unsigned int q[4]; unsigned short d; };
template<> struct lk_raw<23> { unsigned int q[4]; unsigned short d, sh; unsigned char sl; };
__device__ __forceinline__ unsigned short lk_u16(const unsigned char* p){ return *(const unsigned short*)p; }
template<int F> __device__ __forceinline__ void lk_fetch(lk_raw<F>& raw, lk_w w, long row, int K, int nb, int ks);
// Sub-blocks `ks` and `ks + lk_pair_stride<F>()` of matrix row `row`, the pair's raw words.
template<int F> __device__ __forceinline__ void lk_fetch_pair(lk_raw_pair<F>& p, lk_w w, long row, int K, int nb, int ks){
  lk_fetch<F>(p.a, w, row, K, nb, ks);
  if constexpr(F == 14){
    const int j = (ks + 2) & 7;
    p.sc2 = lk_u16(w.b + row * w.rb + lk_q6_blk((ks + 2) >> 3) + 192 + (j >> 2) * 8 + 2 * (j & 3));
  }
}
// Sub-block `ks` of matrix row `row` (`K` values, `nb` sub-blocks), its raw words.
template<int F> __device__ __forceinline__ void lk_fetch(lk_raw<F>& raw, lk_w w, long row, int K, int nb, int ks){
  const unsigned char* r = w.b + row * w.rb;
  int j = ks & 7, part = j >> 2, k = j & 3;
  if constexpr(F == 2 || F == 6){
    const unsigned char* b = r + (long)ks * (F == 2 ? 18 : 22);
    raw.d = lk_u16(b);
    if constexpr(F == 6) raw.qh = (unsigned int)lk_w16(b + 2);
    #pragma unroll
    for(int i = 0; i < 4; ++i) raw.q[i] = (unsigned int)lk_w16(b + (F == 2 ? 2 : 6) + 4 * i);
  } else if constexpr(F == 3 || F == 7){
    const unsigned char* b = r + (long)ks * (F == 3 ? 20 : 24);
    raw.dm = (unsigned int)lk_w32(b);
    if constexpr(F == 7) raw.qh = (unsigned int)lk_w32(b + 4);
    #pragma unroll
    for(int i = 0; i < 4; ++i) raw.q[i] = (unsigned int)lk_w32(b + (F == 3 ? 4 : 8) + 4 * i);
  } else if constexpr(F == 8){
    const uint4* q = (const uint4*)(w.q + row * K + (long)ks * 32);
    raw.q0 = __ldg(q); raw.q1 = __ldg(q + 1);
    raw.d = __half_as_ushort(w.s[row * nb + ks]);
  } else if constexpr(F == 10){
    const unsigned char* b = r + (long)(ks >> 3) * 84;
    #pragma unroll
    for(int i = 0; i < 8; ++i) raw.q[i] = (unsigned int)lk_w32(b + 16 + part * 32 + 4 * i);
    raw.sc = lk_u16(b + part * 8 + k * 2);
    raw.dm = (unsigned int)lk_w32(b + 80);
  } else if constexpr(F == 11){
    const unsigned char* b = r + (long)(ks >> 3) * 110;
    #pragma unroll
    for(int i = 0; i < 8; ++i){ raw.q[i] = (unsigned int)lk_w16(b + 32 + part * 32 + 4 * i); raw.hm[i] = (unsigned int)lk_w16(b + 4 * i); }
    raw.a0 = (unsigned int)lk_w16(b + 96); raw.a1 = (unsigned int)lk_w16(b + 100); raw.t = (unsigned int)lk_w16(b + 104);
    raw.d = lk_u16(b + 108);
  } else if constexpr(F == 12 || F == 13){
    const unsigned char* b = r + (long)(ks >> 3) * (F == 12 ? 144 : 176);
    const uint4* q = (const uint4*)(b + (F == 12 ? 16 : 48) + (j >> 1) * 32);
    raw.h = __ldg((const uint4*)b); raw.q0 = __ldg(q); raw.q1 = __ldg(q + 1);
    if constexpr(F == 13){ raw.qh0 = __ldg((const uint4*)(b + 16)); raw.qh1 = __ldg((const uint4*)(b + 32)); }
  } else if constexpr(F == 14){
    const unsigned char* b = r + lk_q6_blk(ks >> 3);
    const uint4* ql = (const uint4*)(b + part * 64 + ((k & 1) ? 32 : 0));
    const uint4* qh = (const uint4*)(b + 128 + part * 32);
    raw.l0 = __ldg(ql); raw.l1 = __ldg(ql + 1); raw.h0 = __ldg(qh); raw.h1 = __ldg(qh + 1);
    raw.sc = lk_u16(b + 192 + part * 8 + 2 * k);
    raw.d = lk_u16(r + lk_q6_d(ks >> 3));
  } else if constexpr(F == 39){
    uint4 n = __ldg((const uint4*)(r + lk_mx_nib(ks)));
    raw.q[0] = n.x; raw.q[1] = n.y; raw.q[2] = n.z; raw.q[3] = n.w;
    raw.e = __ldg(r + lk_mx_exp(ks));
  } else if constexpr(F == 20){
    const unsigned char* b = r + (long)ks * 18;
    raw.d = lk_u16(b);
    #pragma unroll
    for(int i = 0; i < 4; ++i) raw.q[i] = (unsigned int)lk_w16(b + 2 + 4 * i);
  } else if constexpr(F == 23){
    const unsigned char* b = r + (long)(ks >> 3) * 136;
    raw.d = lk_u16(b); raw.sh = lk_u16(b + 2); raw.sl = b[4 + (j >> 1)];
    #pragma unroll
    for(int i = 0; i < 4; ++i) raw.q[i] = (unsigned int)lk_w32(b + 8 + 16 * j + 4 * i);
  } else static_assert(F != F, "a block format with no raw words");
}
__device__ __forceinline__ float lk_hf(unsigned int bits){ return __half2float(__ushort_as_half((unsigned short)(bits & 0xFFFF))); }
// The sub-block from its fetched words — `lk_sub4`'s levels, scales and offsets, operation for operation.
// The formats whose halves scale by integers under the block's one scale (Q3_K, Q6_K): the block GEMM weighs the
// halves' dots by them in integers (`lk_unpack_ints`).
template<int F> __host__ __device__ constexpr bool lk_int_halves(){ return F == 11 || F == 14; }
// Sub-block `ks` of a format whose halves scale by integers (Q3_K, Q6_K, `lk_int_halves`): its int8 levels, the
// block's scale `dd` and the halves' integer scales as two int8 in `sp`'s low bytes, the first half's lowest (Q3_K's
// six-bit scales less 32, Q6_K's int8 scales as the block keeps them), so a half's value is `dd · scale · level`.
template<int F> __device__ __forceinline__ void lk_unpack_ints(const lk_raw<F>& raw, int ks, int* w, float& dd, int& sp){
  int j = ks & 7, part = j >> 2, k = j & 3;
  if constexpr(F == 11){
    int bit = part * 4 + k;
    unsigned int a0 = raw.a0, a1 = raw.a1, t = raw.t;
    unsigned int aux[4] = {(a0 & 0x0f0f0f0fu) | ((t & 0x03030303u) << 4), (a1 & 0x0f0f0f0fu) | (((t >> 2) & 0x03030303u) << 4),
                           ((a0 >> 4) & 0x0f0f0f0fu) | (((t >> 4) & 0x03030303u) << 4), ((a1 >> 4) & 0x0f0f0f0fu) | (((t >> 6) & 0x03030303u) << 4)};
    dd = lk_hf(raw.d);
    #pragma unroll
    for(int i = 0; i < 8; ++i){
      int lo = ((int)raw.q[i] >> (2 * k)) & 0x03030303;
      int missing = (~((int)raw.hm[i] >> bit) & 0x01010101) << 2;
      w[i] = __vsub4(lo, missing);
    }
    sp = 0;
    #pragma unroll
    for(int h = 0; h < 2; ++h){
      int is = part * 8 + k * 2 + h;
      sp |= ((((lk_pick4(aux, is >> 2) >> (8 * (is & 3))) & 0xff) - 32) & 0xff) << (8 * h);
    }
  } else {
    dd = lk_hf(raw.d);
    const unsigned int ql[8] = {raw.l0.x, raw.l0.y, raw.l0.z, raw.l0.w, raw.l1.x, raw.l1.y, raw.l1.z, raw.l1.w};
    const unsigned int qh[8] = {raw.h0.x, raw.h0.y, raw.h0.z, raw.h0.w, raw.h1.x, raw.h1.y, raw.h1.z, raw.h1.w};
    #pragma unroll
    for(int i = 0; i < 8; ++i){
      int lo = ((int)ql[i] >> (4 * (k >> 1))) & 0x0F0F0F0F;
      int hi = (((int)qh[i] >> (2 * k)) & 0x03030303) << 4;
      w[i] = __vsub4(lo | hi, 0x20202020);
    }
    sp = raw.sc;
  }
}
template<int F> __device__ __forceinline__ void lk_unpack(const lk_raw<F>& raw, int ks, int* w, float* d, float* c){
  int j = ks & 7, part = j >> 2, k = j & 3;
  if constexpr(F == 2 || F == 3){
    float dd, cc = 0.f;
    if constexpr(F == 2) dd = lk_hf(raw.d);
    else { dd = lk_hf(raw.dm); cc = lk_hf(raw.dm >> 16); }
    #pragma unroll
    for(int i = 0; i < 4; ++i){
      int q = (int)raw.q[i];
      w[i] = q & 0x0F0F0F0F; w[i + 4] = (q >> 4) & 0x0F0F0F0F;
      if(F == 2){ w[i] = __vsub4(w[i], 0x08080808); w[i + 4] = __vsub4(w[i + 4], 0x08080808); }
    }
    d[0] = d[1] = dd; c[0] = c[1] = cc;
  } else if constexpr(F == 6 || F == 7){
    float dd, cc = 0.f;
    if constexpr(F == 6) dd = lk_hf(raw.d);
    else { dd = lk_hf(raw.dm); cc = lk_hf(raw.dm >> 16); }
    unsigned int qh = raw.qh;
    #pragma unroll
    for(int i = 0; i < 4; ++i){
      int q = (int)raw.q[i];
      w[i] = (q & 0x0F0F0F0F) | lk_bit4((qh >> (4 * i)) & 0xF);
      w[i + 4] = ((q >> 4) & 0x0F0F0F0F) | lk_bit4((qh >> (16 + 4 * i)) & 0xF);
      if(F == 6){ w[i] = __vsub4(w[i], 0x10101010); w[i + 4] = __vsub4(w[i + 4], 0x10101010); }
    }
    d[0] = d[1] = dd; c[0] = c[1] = cc;
  } else if constexpr(F == 8){
    w[0] = raw.q0.x; w[1] = raw.q0.y; w[2] = raw.q0.z; w[3] = raw.q0.w; w[4] = raw.q1.x; w[5] = raw.q1.y; w[6] = raw.q1.z; w[7] = raw.q1.w;
    d[0] = d[1] = lk_hf(raw.d); c[0] = c[1] = 0.f;
  } else if constexpr(F == 10){
    float dd = lk_hf(raw.dm), dm = lk_hf(raw.dm >> 16);
    #pragma unroll
    for(int i = 0; i < 8; ++i) w[i] = ((int)raw.q[i] >> (2 * k)) & 0x03030303;
    #pragma unroll
    for(int h = 0; h < 2; ++h){ int sc = (raw.sc >> (8 * h)) & 0xFF; d[h] = dd * (float)(sc & 0xf); c[h] = -(dm * (float)(sc >> 4)); }
  } else if constexpr(lk_int_halves<F>()){
    float dd; int sp;
    lk_unpack_ints<F>(raw, ks, w, dd, sp);
    d[0] = dd * (float)(signed char)(sp & 0xFF); d[1] = dd * (float)(signed char)(sp >> 8); c[0] = c[1] = 0.f;
  } else if constexpr(F == 12 || F == 13){
    int hi = j & 1, pp = j >> 1;
    // Scale byte `i` of the head's twelve.
    auto sb = [&](int i){ unsigned int wd = i < 4 ? raw.h.y : i < 8 ? raw.h.z : raw.h.w; return (int)((wd >> (8 * (i & 3))) & 0xFF); };
    int sc, m;
    if(j < 4){ sc = sb(j) & 63; m = sb(j + 4) & 63; }
    else { sc = (sb(j + 4) & 0xF) | ((sb(j - 4) >> 6) << 4); m = (sb(j + 4) >> 4) | ((sb(j) >> 6) << 4); }
    d[0] = d[1] = lk_hf(raw.h.x) * (float)sc; c[0] = c[1] = -lk_hf(raw.h.x >> 16) * (float)m;
    unsigned int q[8] = {raw.q0.x, raw.q0.y, raw.q0.z, raw.q0.w, raw.q1.x, raw.q1.y, raw.q1.z, raw.q1.w};
    #pragma unroll
    for(int i = 0; i < 8; ++i){
      int v = (int)((q[i] >> (4 * hi)) & 0x0F0F0F0Fu);
      if constexpr(F == 13){
        unsigned int qh[8] = {raw.qh0.x, raw.qh0.y, raw.qh0.z, raw.qh0.w, raw.qh1.x, raw.qh1.y, raw.qh1.z, raw.qh1.w};
        v |= (int)(((qh[i] >> (2 * pp + hi)) & 0x01010101u) << 4);
      }
      w[i] = v;
    }
  } else if constexpr(F == 39){
    #pragma unroll
    for(int i = 0; i < 4; ++i) lk_mxfp4_word(raw.q[i], w[i], w[i + 4]);
    d[0] = d[1] = lk_e8m0_half(raw.e); c[0] = c[1] = 0.f;
  } else if constexpr(F == 20 || F == 23){
    float dd;
    if constexpr(F == 20) dd = lk_hf(raw.d);
    else dd = lk_iq4xs_scale(lk_hf(raw.d), raw.sh, raw.sl, j);
    #pragma unroll
    for(int i = 0; i < 4; ++i) lk_iq4_word(raw.q[i], w[i], w[i + 4]);
    d[0] = d[1] = dd; c[0] = c[1] = 0.f;
  } else static_assert(F != F, "a block format with no raw words");
}
"#;

/// `Y[t,o] = Σ_k X[t,k]·W[o,k]` for the `M` rows of `X` (`M ≤ 16`) over the `N` rows of a matrix in block format `F`,
/// `W` consecutive output rows a warp ([`BLOCKS`]' row dot), plus the bias when there is one. With a quantized output
/// (`QO`) a block of `32 / W` warps takes 32 outputs, one quantized block a row, quantized as `lk_quant_lane` quantizes
/// it, and `y` (when not null) takes the `f32` result too.
const GEMV_BLK: &str = r#"
template<int F, int Q, int M, int W, int QO>
__device__ __forceinline__ void lk_gemv_blk_rows(lk_w w, const float* __restrict__ x, const signed char* __restrict__ xq,
                            const signed char* __restrict__ xl, const float* __restrict__ xs,
                            lk_epi bias, lk_out y, const int* __restrict__ yrow,
                            long ypitch, int K, int N, signed char* __restrict__ yq, signed char* __restrict__ yl,
                            float* __restrict__ ys){
  __shared__ float v[QO > 0 ? M : 1][32];
  int lane = threadIdx.x & 31, warp = threadIdx.x >> 5;
  long o = (((long)blockIdx.x * blockDim.x + threadIdx.x) >> 5) * W;
  if(o < N){
    lk_out yo = y ? LK_ROW(y, yrow, ypitch) : lk_out();
    auto out = [&](int r, int t, float d){
      float val = bias(d, t, o + r, N);
      if(yo) yo[(long)t * N + o + r] = val;
      if(QO > 0) v[t][warp * W + r] = val;
    };
    if constexpr(lk_h16_rows<F, Q, M>()){
      float c[lk_h16_tiles<W, M>()][4];
      lk_h16_mma_rows<F, Q, W, M>(w, o, K, x, K, c);
      lk_h16_each<W, M>([&](int r, int t, int n, int e){ out(r, t, c[n][e]); });
    } else {
      float d[W * M];
      lk_row_dot<F, Q, M, W>(w, o, K, x, xq, xl, xs, K, d);
      for(int i = lane; i < W * M; i += 32) out(i / M, i % M, lk_pick(d, i));
    }
  }
  if(QO > 0){
    __syncthreads();
    if(warp < M){
      long at = (long)warp * N + (long)blockIdx.x * 32 + lane;
      lk_quant_lane<QO>(v[warp][lane], yq, yl, ys, at, at >> 5);
    }
  }
}
template<int F, int Q, int M, int W = 1, int QO = 0>
__global__ void lk_gemv_blk(lk_w w, const float* __restrict__ x, const signed char* __restrict__ xq,
                            const signed char* __restrict__ xl, const float* __restrict__ xs,
                            lk_epi bias, lk_out y, const int* __restrict__ yrow,
                            long ypitch, int K, int N, signed char* __restrict__ yq, signed char* __restrict__ yl,
                            float* __restrict__ ys){
  lk_gemv_blk_rows<F, Q, M, W, QO>(w, x, xq, xl, xs, bias, y, yrow, ypitch, K, N, yq, yl, ys);
}
"#;

/// `lk_gemv_blk` over a step's one row, then, in its last-arriving block, `lk_rmsnorm` over the row it wrote: a
/// projection in a block format with the residual add and the next norm in its epilogue. Its blocks compute and write
/// their outputs as `lk_gemv_blk` does — `W` rows a warp — and count in at `cnt`; the last reads the row back through
/// L2 and norms it as `lk_rmsnorm`'s block of `vt` threads does, its warps playing the norm's (`VW` each), then resets
/// the count.
const GEMV_BLK_NORM: &str = r#"
template<int F, int Q, int W, bool ADD, bool SUM, bool OUTF, int QO, int PER, typename A, bool PRE, int VW>
__global__ void lk_gemv_blk_norm(lk_w w, const float* __restrict__ x, const signed char* __restrict__ xq,
                                 const signed char* __restrict__ xl, const float* __restrict__ xs,
                                 const float* __restrict__ bias, float* y, int K, int N, lk_norm_of nm, int vt,
                                 int* __restrict__ cnt){
  __shared__ A red[32];
  __shared__ int last;
  lk_gemv_blk_rows<F, Q, 1, W, 0>(w, x, xq, xl, xs, bias, y, nullptr, 0, K, N, nullptr, nullptr, nullptr);
  __threadfence();
  __syncthreads();
  if(threadIdx.x == 0) last = atomicAdd(cnt, 1) == gridDim.x - 1;
  __syncthreads();
  if(!last) return;
  __threadfence();
  lk_out yr = nm.y ? LK_ROW(nm.y, nm.yrow, nm.ypitch) : lk_out();
  lk_rmsnorm_row<ADD, SUM, OUTF, QO, PER, A, PRE, true, VW>(nm.x, nm.r, nm.s, nm.w, yr, nm.yq, nm.yl, nm.ys, N, nm.eps, 0, 0,
                                                            nm.pw, nm.peps, red, vt);
  if(threadIdx.x == 0) *cnt = 0;
}
"#;

/// The projections of one activation's `M` rows (`M ≤ 16`) by up to `GEMM_PARTS` matrices in format `F` (8: Q8_0's
/// split arrays), their `N` outputs side by side as `parts` lays them out: warp `o` dots `W` consecutive rows of the
/// part holding output `o` (a part's outputs a whole number of warps' rows) as the part's own GEMV dots them
/// (`lk_gemv_dot`), and writes each, plus the part's bias, where the part's outputs go.
const GEMV_PARTS: &str = r#"
template<int F, int Q, int M, int W = 1>
__global__ void lk_gemv_parts(const float* __restrict__ x, const signed char* __restrict__ xq, const signed char* __restrict__ xl,
                              const float* __restrict__ xs, int K, int N, lk_parts parts){
  long o = (((long)blockIdx.x * blockDim.x + threadIdx.x) >> 5) * W;
  if(o >= N) return;
  int p = 0;
  while(p < LK_PARTS - 1 && o >= parts.end[p]) ++p;
  long start = p ? parts.end[p - 1] : 0, r = o - start;
  long n = parts.end[p] - start;
  lk_out yo = LK_ROW(lk_part_out(parts, p), parts.row[p], parts.pitch[p]);
  const lk_epi b = parts.bias[p];
  if constexpr(lk_h16_rows<F, Q, M>()){
    float c[lk_h16_tiles<W, M>()][4];
    lk_h16_mma_rows<F, Q, W, M>(parts.w[p], r, K, x, K, c);
    lk_h16_each<W, M>([&](int i, int t, int k, int e){ yo[(long)t * n + r + i] = b(c[k][e], t, r + i, n); });
  } else {
    float d[W * M];
    lk_gemv_dot<F, Q, M, W>(parts.w[p], r, K, x, xq, xl, xs, d);
    for(int j = threadIdx.x & 31; j < W * M; j += 32) yo[(long)j % M * n + r + j / M] = b(lk_pick(d, j), j % M, r + j / M, n);
  }
}
"#;

/// A batch's GEMV on the int8 tensor cores (`lk_gemv_mma`): the `M` activation rows (at most `16·MT`, `MT` 1, 2 or 4)
/// by up to four Q8_0 matrices side by side (`lk_parts`, each part a whole number of 8-output tiles), every (row,
/// output) summed exactly as `lk_q8_rows_dot` sums it — so a batch's sequence sums as the decode step's one row does:
/// each 32-value block's int8 dot exact (an int32 `mma` of k 32), its step (`lk_q8_acc`) into partial `b mod 4` of
/// block `b`, a partial's blocks in order, and the four partials as `lk_q8_sum4` sums them. A block takes `NT` n-tiles
/// of 8 outputs over all `MT` m-tiles, a warp `NW` of the n-tiles over `MW` of the m-tiles — each fragment it loads, an
/// m-tile's activations or an n-tile's matrix rows, feeding `NW` or `MW` products — staging four blocks of every matrix
/// row and activation row at a time in shared memory by `cp.async` (`lk_mma_stages` stages in flight), and each stage's
/// scales by plain loads a stage ahead; a stage's block `j` goes into partial `j`, and a warp's four partials of each
/// of its elements stay in registers.
const GEMV_MMA: &str = r#"
template<int Q> __host__ __device__ constexpr int lk_mma_stages(){ return Q == 2 ? 2 : 3; }

// A stage's row in shared memory: four blocks (128 values) and 16 bytes of padding, so the eight rows an `mma` reads
// fall on distinct banks.
#define LK_MP 144
// A stage: four blocks of each of the block's `8·NT` matrix rows and `16·MT` activation rows (both words' under two
// words), and their scales.
template<int Q, int MT, int NT> __host__ __device__ constexpr int lk_mma_stage_bytes(){
  return 8 * NT * LK_MP + (Q == 2 ? 2 : 1) * 16 * MT * LK_MP + 8 * NT * 4 * 2 + 16 * MT * 4 * 4;
}
template<int Q, int MT, int NT> __host__ __device__ constexpr int lk_mma_smem(){ return lk_mma_stages<Q>() * lk_mma_stage_bytes<Q, MT, NT>(); }
// One stage's four blocks (from block `blk0`, `nb` the row's), block `j` into partial `j` of `acc`: `b` the stage's
// buffer, `wld` this lane's address of its first n-tile's matrix rows for `ldmatrix` and `ald` of its first m-tile's
// activation rows, `wsr` its first n-tile's two outputs' scales, `row` its first activation row.
template<int Q, int MW, int NW, int AOFF, int LOFF, int XSOFF>
__device__ __forceinline__ void lk_mma_blocks(float (&acc)[MW][NW][4][4], const unsigned char* b, int wld, int ald, const __half* wsr,
                                              int row0, int blk0, int nb){
  #pragma unroll
  for(int j = 0; j < 4; ++j){
    if(blk0 + j < nb){
      int bw[NW][2];
      float w0[NW], w1[NW];
      #pragma unroll
      for(int n = 0; n < NW; ++n){
        lk_ldsm2_s8(bw[n], b + wld + n * 8 * LK_MP + 32 * j);
        w0[n] = __half2float(wsr[n * 32 + j]); w1[n] = __half2float(wsr[n * 32 + 4 + j]);
      }
      #pragma unroll
      for(int m = 0; m < MW; ++m){
        const int row = row0 + m * 16;
        int a[4], l[4];
        lk_ldsm_s8(a, b + AOFF + ald + m * 16 * LK_MP + 32 * j);
        if(Q == 2) lk_ldsm_s8(l, b + LOFF + ald + m * 16 * LK_MP + 32 * j);
        const float* xsr = (const float*)(b + XSOFF);
        const float s0 = xsr[row * 4 + j], s1 = xsr[(row + 8) * 4 + j];
        #pragma unroll
        for(int n = 0; n < NW; ++n){
          // One word reads its sums by the conversion, two by the subtraction, as the block path reads them.
          constexpr bool I = Q == 1;
          int c[4], cl[4] = {lk_dot0<I>(), lk_dot0<I>(), lk_dot0<I>(), lk_dot0<I>()};
          lk_mma_s8_from(c, a, bw[n], lk_dot0<I>());
          if(Q == 2) lk_mma_s8_from(cl, l, bw[n], lk_dot0<I>());
          float (&p)[4] = acc[m][n][j];
          p[0] = lk_q8_acc<Q>(p[0], w0[n], s0, lk_dot_f<I>(c[0]), lk_dot_f<I>(cl[0]));
          p[1] = lk_q8_acc<Q>(p[1], w1[n], s0, lk_dot_f<I>(c[1]), lk_dot_f<I>(cl[1]));
          p[2] = lk_q8_acc<Q>(p[2], w0[n], s1, lk_dot_f<I>(c[2]), lk_dot_f<I>(cl[2]));
          p[3] = lk_q8_acc<Q>(p[3], w1[n], s1, lk_dot_f<I>(c[3]), lk_dot_f<I>(cl[3]));
        }
      }
    }
  }
}
template<int Q, int MT, int NT, int MW = 1, int NW = 1>
__global__ void __launch_bounds__(32 * (NT / NW) * (MT / MW), MW == 1 && NW == 1 ? 512 / (32 * NT * MT) : 1) lk_gemv_mma(
    const signed char* __restrict__ xq, const signed char* __restrict__ xl, const float* __restrict__ xs, int K, int N, int M,
    lk_parts parts){
  constexpr int NB = 8 * NT, MS = MT / MW, ST = lk_mma_stages<Q>(), T = 32 * (NT / NW) * MS;
  constexpr int SB = lk_mma_stage_bytes<Q, MT, NT>(), RA = 16 * MT;
  constexpr int AOFF = NB * LK_MP, LOFF = AOFF + RA * LK_MP, WSOFF = AOFF + (Q == 2 ? 2 : 1) * RA * LK_MP, XSOFF = WSOFF + NB * 8;
  extern __shared__ __align__(16) unsigned char lk_ms[];
  const int tid = threadIdx.x, warp = tid >> 5, lane = tid & 31, g = lane >> 2, q4 = lane & 3;
  // This warp's first n-tile and first m-tile.
  const int ntile = (warp / MS) * NW, mbase = (warp % MS) * MW;
  const long o0 = (long)blockIdx.x * NB;
  const int nb = K >> 5, stages = (nb + 3) >> 2;
  auto part_of = [&](long o){ int p = 0; while(p < LK_PARTS - 1 && o >= parts.end[p]) ++p; return p; };
  auto row_in = [&](long o, int p){ return o - (p ? parts.end[p - 1] : 0); };
  // This thread's copies each stage: 16-byte chunks `tid + T·i` of the block's matrix rows (eight a row) and of its
  // activation rows; and its scales of the stage's, entries `tid + T·i` — matrix row `e / 4`'s block `e mod 4`, and
  // activation row `e / 4`'s.
  constexpr int WC = (NB * 8 + T - 1) / T, AC = (RA * 8 + T - 1) / T, WSC = (NB * 4 + T - 1) / T, XC = (RA * 4 + T - 1) / T;
  const signed char* wsrc[WC];
  bool win[WC];
  #pragma unroll
  for(int i = 0; i < WC; ++i){
    const int c = tid + T * i;
    const long o = o0 + (c >> 3);
    win[i] = c < NB * 8 && o < N;
    const int p = part_of(win[i] ? o : 0);
    wsrc[i] = parts.w[p].q + (win[i] ? row_in(o, p) : 0) * K + 16 * (c & 7);
  }
  bool sin_[WSC];
  const __half* wsc[WSC];
  #pragma unroll
  for(int i = 0; i < WSC; ++i){
    const int e = tid + T * i;
    const long o = o0 + (e >> 2);
    sin_[i] = e < NB * 4 && o < N;
    const int p = part_of(sin_[i] ? o : 0);
    wsc[i] = parts.w[p].s + (sin_[i] ? row_in(o, p) : 0) * nb + (e & 3);
  }
  bool xin[XC];
  const float* xsc[XC];
  #pragma unroll
  for(int i = 0; i < XC; ++i){
    const int e = tid + T * i;
    xin[i] = e < RA * 4 && (e >> 2) < M;
    xsc[i] = xs + (long)(xin[i] ? e >> 2 : 0) * nb + (e & 3);
  }
  auto copy = [&](int st){
    unsigned char* b = lk_ms + (st % ST) * SB;
    const int kb = st * 128;
    #pragma unroll
    for(int i = 0; i < WC; ++i){
      const int c = tid + T * i;
      if(c < NB * 8) lk_cp16(b + (c >> 3) * LK_MP + 16 * (c & 7), wsrc[i] + kb, win[i] && kb + 16 * (c & 7) < K);
    }
    #pragma unroll
    for(int i = 0; i < AC; ++i){
      const int c = tid + T * i;
      if(c < RA * 8){
        const int t = c >> 3;
        const bool in = t < M && kb + 16 * (c & 7) < K;
        const long off = (long)(in ? t : 0) * K + kb + 16 * (c & 7);
        lk_cp16(b + AOFF + t * LK_MP + 16 * (c & 7), xq + (in ? off : 0), in);
        if(Q == 2) lk_cp16(b + LOFF + t * LK_MP + 16 * (c & 7), xl + (in ? off : 0), in);
      }
    }
  };
  // A stage's scales by plain loads, a stage ahead of their use; set into the stage's buffer before its wait.
  __half wnext[WSC];
  float xnext[XC];
  auto load_scales = [&](int st){
    const int blk = st * 4 + (tid & 3);
    #pragma unroll
    for(int i = 0; i < WSC; ++i) if(tid + T * i < NB * 4) wnext[i] = sin_[i] && blk < nb ? wsc[i][st * 4] : __float2half(0.f);
    #pragma unroll
    for(int i = 0; i < XC; ++i) if(tid + T * i < RA * 4) xnext[i] = xin[i] && blk < nb ? xsc[i][st * 4] : 0.f;
  };
  auto set_scales = [&](int st){
    unsigned char* b = lk_ms + (st % ST) * SB;
    #pragma unroll
    for(int i = 0; i < WSC; ++i) if(tid + T * i < NB * 4) ((__half*)(b + WSOFF))[tid + T * i] = wnext[i];
    #pragma unroll
    for(int i = 0; i < XC; ++i) if(tid + T * i < RA * 4) ((float*)(b + XSOFF))[tid + T * i] = xnext[i];
  };
  float acc[MW][NW][4][4];
  #pragma unroll
  for(int m = 0; m < MW; ++m)
    #pragma unroll
    for(int n = 0; n < NW; ++n)
      #pragma unroll
      for(int p = 0; p < 4; ++p)
        #pragma unroll
        for(int e = 0; e < 4; ++e) acc[m][n][p][e] = 0.f;
  #pragma unroll
  for(int i = 0; i < ST - 1; ++i){ if(i < stages) copy(i); asm volatile("cp.async.commit_group;\n"); }
  if(stages > 0) load_scales(0);
  // Stage `st`: its scales set, its copies landed for every thread, the copy `ST − 1` stages on started into the buffer
  // every warp has finished reading; its buffer.
  auto ready = [&](int st){
    set_scales(st);
    asm volatile("cp.async.wait_group %0;\n" :: "n"(ST - 2));
    __syncthreads();
    if(st + ST - 1 < stages) copy(st + ST - 1);
    asm volatile("cp.async.commit_group;\n");
    if(st + 1 < stages) load_scales(st + 1);
    return lk_ms + (st % ST) * SB;
  };
  // `ldmatrix`'s row addresses: of the first n-tile's matrix rows, lane `l` row `l mod 8`'s bytes `16·(l / 8 mod 2)` on;
  // of an m-tile's activation rows, lane `l` row `l mod 8 + 8·(l / 8 mod 2)`'s bytes `16·(l / 16)` on.
  const int wld = (ntile * 8 + (lane & 7)) * LK_MP + 16 * ((lane >> 3) & 1);
  const int ald = (mbase * 16 + (lane & 7) + 8 * ((lane >> 3) & 1)) * LK_MP + 16 * (lane >> 4);
  const int row0 = mbase * 16 + g;
  const int wsoff = WSOFF + (ntile * 8 + 2 * q4) * 4 * 2;
  for(int st = 0; st < stages; ++st){
    const unsigned char* b = ready(st);
    lk_mma_blocks<Q, MW, NW, AOFF, LOFF, XSOFF>(acc, b, wld, ald, (const __half*)(b + wsoff), row0, 4 * st, nb);
  }
  asm volatile("cp.async.wait_group 0;\n");
  // Each element's four partials summed as `lk_q8_sum4` sums them, written as `lk_gemv_parts` writes it.
  #pragma unroll
  for(int m = 0; m < MW; ++m)
    #pragma unroll
    for(int n = 0; n < NW; ++n){
      float vs[4];
      #pragma unroll
      for(int e = 0; e < 4; ++e) vs[e] = lk_q8_sum4(acc[m][n][0][e], acc[m][n][1][e], acc[m][n][2][e], acc[m][n][3][e]);
      #pragma unroll
      for(int e = 0; e < 4; ++e){
        const float v = vs[e];
        const long o = o0 + (ntile + n) * 8 + 2 * q4 + (e & 1);
        const int t = (mbase + m) * 16 + g + 8 * (e >> 1);
        if(o < N && t < M){
          const int p = part_of(o);
          const long start = p ? parts.end[p - 1] : 0, rr = o - start, np = parts.end[p] - start;
          lk_out yo = LK_ROW(lk_part_out(parts, p), parts.row[p], parts.pitch[p]);
          const lk_epi bias = parts.bias[p];
          yo[(long)t * np + rr] = bias(v, t, rr, np);
        }
      }
    }
}
"#;

/// A batch's GEMV over Q4_K, Q6_K, Q4_0, Q4_1, Q5_0 or Q5_1 rows on the int8 tensor cores (`lk_gemv_mma_blk<F>`, F 12,
/// 14, 2, 3, 6 or 7), every (row, output) summed exactly as the decode step's GEMV sums its one row (`lk_row_dot_blk`):
/// the GEMV's lane L takes the units L, L + 32, … of a row — a Q4_K pair of sub-blocks, a Q6_K quarter, a 32-value
/// block (`lk_sub4`'s levels) — each sub-block's halves dotted exactly and
/// its term `lk_sub_dot`'s, `acc += s·term` in order, and `lk_warp_sum` folds the 32 lanes' sums. Here each half
/// sub-block's int8 dot is one m16n8k16 `mma` (exact) for 16 activation rows by 8 outputs, the codes' sums by `dp4a`
/// across the quad, and a block of 16 warps takes 64 outputs of the M ≤ 16 rows: warp w the lanes w and w + 16, each
/// lane's units in order over 8 n-tiles sharing each fragment of the activation, the tree's first level (lane w with
/// w + 16) in the warp, its other four over shared memory. A Q4_K or 32-value-block warp holds both lanes' sums at
/// once; a Q6_K quarter's fragments leave registers for one, so its warp sums lane w, parks it in the level-one table
/// and adds lane w + 16's. A Q4_K unit's scales are decoded by the warp into shared memory, the others' read where they
/// are taken. The block's outputs lie in one part (`lk_take_part`, its outputs taken after the sums), written as
/// `lk_gemv_mma` writes them. Rows past M write nothing (a Q6_K block reads row M − 1 for them, the others no
/// activation). One word a block: its arguments are `lk_gemv_mma`'s, the low words `xl` none.
const GEMV_MMA_BLK: &str = r#"
__device__ __forceinline__ void lk_mb_mma16(int (&c)[4], int a0, int a1, int b0){
  asm volatile("mma.sync.aligned.m16n8k16.row.col.s32.s8.s8.s32 {%0,%1,%2,%3}, {%4,%5}, {%6}, {%0,%1,%2,%3};\n"
               : "+r"(c[0]), "+r"(c[1]), "+r"(c[2]), "+r"(c[3]) : "r"(a0), "r"(a1), "r"(b0));
}
// A sub-block's term before its activation scale from its halves' exact dots and code sums, as the decode GEMV's
// `lk_sub_dot` takes it for format `F`. Q4_K's is `lk_sub_dot`'s expression, which nvcc fuses here as it fuses it there
// (its offset a negated product it folds) — spelled out, its sums spill. The others' fuse the offset's product onto
// the dot, then the second half's dot and product in order, each spelled out: left to nvcc, the first pair fuses
// the other way here.
template<int F>
__device__ __forceinline__ float lk_mb_part(const float* d, const float* c, int dh0, int sh0, int dh1, int sh1){
  if constexpr(F == 12){
    float part = d[0] * (float)dh0 + c[0] * (float)sh0 + d[1] * (float)dh1 + c[1] * (float)sh1;
    return part;
  } else {
    return __fmaf_rn(c[1], (float)sh1, __fmaf_rn(d[1], (float)dh1, __fmaf_rn(c[0], (float)sh0, __fmul_rn(d[0], (float)dh0))));
  }
}
// Sub-block `k` (0 … 7) of a Q4_K super-block's header `hd`: its scale and offset, as `lk_pair4_q4k` takes them.
__device__ __forceinline__ void lk_mb_q4k_scale(uint4 hd, int k, float& d, float& c){
  const unsigned long long lo = (unsigned long long)hd.y | ((unsigned long long)hd.z << 32);
  const float hdd = __half2float(__ushort_as_half((unsigned short)(hd.x & 0xffff)));
  const float hdm = __half2float(__ushort_as_half((unsigned short)(hd.x >> 16)));
  const int b0 = (int)((lo >> (8 * (k & 7))) & 0xff), bm4 = (int)((lo >> (8 * ((k - 4) & 7))) & 0xff);
  const int b4 = k + 4 < 8 ? (int)((lo >> (8 * (k + 4))) & 0xff) : (int)((hd.w >> (8 * (k - 4))) & 0xff);
  const int sc = k < 4 ? (b0 & 63) : ((b4 & 0xf) | ((bm4 >> 6) << 4));
  const int m = k < 4 ? (b4 & 63) : ((b4 >> 4) | ((b0 >> 6) << 4));
  d = hdd * (float)sc;
  c = -hdm * (float)m;
}
// Half `hf` of sub-block `e`'s fragments for rows g and g + 8 (codes `a`, sums `sh`) at value `kk` of the activation;
// a row not `in` reads nothing and gives zeros.
__device__ __forceinline__ void lk_mb_act(const signed char* __restrict__ xr0, const signed char* __restrict__ xr1, bool in0, bool in1,
                                          int kk, int (&a)[2], int (&sh)[2]){
  a[0] = in0 ? __ldg((const int*)(xr0 + kk)) : 0; a[1] = in1 ? __ldg((const int*)(xr1 + kk)) : 0;
  int s0 = __dp4a(0x01010101, a[0], 0), s1 = __dp4a(0x01010101, a[1], 0);
  s0 += __shfl_xor_sync(0xffffffffu, s0, 1); s0 += __shfl_xor_sync(0xffffffffu, s0, 2);
  s1 += __shfl_xor_sync(0xffffffffu, s1, 1); s1 += __shfl_xor_sync(0xffffffffu, s1, 2);
  sh[0] = s0; sh[1] = s1;
}
template<int F>
__global__ void __launch_bounds__(512, 1) lk_gemv_mma_blk(const signed char* __restrict__ xq, const signed char* __restrict__ xl,
                                                         const float* __restrict__ xs, int K, int N, int M, lk_parts parts){
  static_assert(F == 12 || F == 14 || F == 2 || F == 3 || F == 6 || F == 7, "a batch's tensor-core GEMV of Q4_K, Q6_K, Q4_0, Q4_1, Q5_0 or Q5_1 rows");
  constexpr int NW = 8, NO = 64, WARPS = 16;
  extern __shared__ __align__(16) unsigned char lk_mbs[];
  float (*part)[16][NO] = (float (*)[16][NO])lk_mbs;
  float4 (*sc)[2][NW][4] = (float4 (*)[2][NW][4])(lk_mbs + 16 * 16 * NO * 4);
  const int lane = threadIdx.x & 31, warp = threadIdx.x >> 5, g = lane >> 2, q4 = lane & 3;
  // The part's matrix; its outputs, bias and rows are taken after the sums, so none holds a register through them.
  int nl = blockIdx.x * NO, np = N;
  lk_w w;
  {
    lk_out y; lk_epi bias; const int* __restrict__ yrow; long ypitch;
    lk_take_part(parts, nl, w, y, bias, yrow, ypitch, np);
  }
  const long rb = w.rb;
  const unsigned char* __restrict__ wk = w.b + (long)nl * rb;
  const int nb = K >> 5, nu = F == 12 ? K >> 6 : F == 14 ? K >> 7 : K >> 5, rounds = (nu + 31) >> 5;
  // Warp w sums leaf w and leaf w + 16, each over its units L, L + 32, … in order, and the tree's first level adds the
  // two into the table of level-one sums.
  if constexpr(F != 14){
    // A pair's or a block's fragments leave registers for both leaves' sums at once. Rows past M read nothing: the
    // predicates also keep the next unit's loads behind this one's sums, which is what holds both leaves' sums in the
    // registers.
    const bool in0 = g < M, in1 = g + 8 < M;
    const signed char* xr0 = xq + (long)g * K;
    const signed char* xr1 = xq + (long)(g + 8) * K;
    float acc[2][NW][4];
    #pragma unroll
    for(int i = 0; i < 2; ++i)
      #pragma unroll
      for(int n = 0; n < NW; ++n)
        #pragma unroll
        for(int e = 0; e < 4; ++e) acc[i][n][e] = 0.f;
    #pragma unroll
    for(int i = 0; i < 2; ++i){
      const int L = warp + 16 * i;
      for(int rd = 0; rd < rounds; ++rd){
        const int u = rd * 32 + L;
        if(u >= nu) break;
        if constexpr(F == 12){
          // A pair: sub-blocks 2u and 2u + 1, super-block u / 4; each lane decodes outputs lane and lane + 32 into the
          // warp's table of the pair's scales.
          const int s2 = 2 * u, sbk = u >> 2, k0 = s2 & 7;
          #pragma unroll
          for(int t = 0; t < 2; ++t){
            const int col = t * 32 + lane;
            float d0, c0, d1, c1;
            const uint4 hd = __ldg((const uint4*)(wk + (long)col * rb + (long)sbk * 144));
            lk_mb_q4k_scale(hd, k0, d0, c0);
            lk_mb_q4k_scale(hd, k0 + 1, d1, c1);
            const int n = col >> 3, qq = (col & 7) >> 1, o = col & 1;
            float* f0 = (float*)&sc[warp][0][n][qq];
            float* f1 = (float*)&sc[warp][1][n][qq];
            f0[2 * o] = d0; f0[2 * o + 1] = c0; f1[2 * o] = d1; f1[2 * o + 1] = c1;
          }
          __syncwarp();
          unsigned int wq[NW][2];
          #pragma unroll
          for(int n = 0; n < NW; ++n){
            const unsigned char* qb = wk + (long)(n * 8 + g) * rb + (long)sbk * 144 + 16 + (u & 3) * 32;
            wq[n][0] = __ldg((const unsigned int*)(qb + 4 * q4));
            wq[n][1] = __ldg((const unsigned int*)(qb + 16 + 4 * q4));
          }
          #pragma unroll
          for(int e = 0; e < 2; ++e){
            int a[2][2], sh[2][2];
            #pragma unroll
            for(int hf = 0; hf < 2; ++hf) lk_mb_act(xr0, xr1, in0, in1, (s2 + e) * 32 + hf * 16 + 4 * q4, a[hf], sh[hf]);
            const float x0 = in0 ? xs[(long)g * nb + s2 + e] : 0.f, x1 = in1 ? xs[(long)(g + 8) * nb + s2 + e] : 0.f;
            #pragma unroll
            for(int n = 0; n < NW; ++n){
              int dh[2][4];
              #pragma unroll
              for(int hf = 0; hf < 2; ++hf){
                const int b0 = (int)((e ? wq[n][hf] >> 4 : wq[n][hf]) & 0x0F0F0F0Fu);
                dh[hf][0] = dh[hf][1] = dh[hf][2] = dh[hf][3] = 0;
                lk_mb_mma16(dh[hf], a[hf][0], a[hf][1], b0);
              }
              const float4 fs = sc[warp][e][n][q4];
              #pragma unroll
              for(int o = 0; o < 2; ++o){
                const float d1 = o ? fs.z : fs.x, c1 = o ? fs.w : fs.y;
                const float d[2] = {d1, d1}, c[2] = {c1, c1};
                #pragma unroll
                for(int r = 0; r < 2; ++r){
                  const int el = r * 2 + o;
                  acc[i][n][el] = __fmaf_rn(r ? x1 : x0, lk_mb_part<F>(d, c, dh[0][el], sh[0][r], dh[1][el], sh[1][r]), acc[i][n][el]);
                }
              }
            }
          }
          __syncwarp();
        } else {
          // A sub-block: unit u, 32 values of each row, its block `BB` bytes (`lk_sub4`'s layout and levels).
          constexpr int BB = F == 2 ? 18 : F == 3 ? 20 : F == 6 ? 22 : 24, QS = F == 2 ? 2 : F == 3 ? 4 : F == 6 ? 6 : 8;
          int a[2][2], sh[2][2];
          #pragma unroll
          for(int hf = 0; hf < 2; ++hf) lk_mb_act(xr0, xr1, in0, in1, u * 32 + hf * 16 + 4 * q4, a[hf], sh[hf]);
          const float x0 = in0 ? xs[(long)g * nb + u] : 0.f, x1 = in1 ? xs[(long)(g + 8) * nb + u] : 0.f;
          #pragma unroll
          for(int n = 0; n < NW; ++n){
            const unsigned char* bg = wk + (long)(n * 8 + g) * rb + (long)u * BB;
            const int q = F == 2 || F == 6 ? lk_w16(bg + QS + 4 * q4) : lk_w32(bg + QS + 4 * q4);
            int w0 = q & 0x0F0F0F0F, w1 = (q >> 4) & 0x0F0F0F0F;
            if constexpr(F == 6 || F == 7){
              const unsigned int qh = F == 6 ? (unsigned int)lk_w16(bg + 2) : (unsigned int)lk_w32(bg + 4);
              w0 |= lk_bit4((qh >> (4 * q4)) & 0xF); w1 |= lk_bit4((qh >> (16 + 4 * q4)) & 0xF);
            }
            if constexpr(F == 2){ w0 = __vsub4(w0, 0x08080808); w1 = __vsub4(w1, 0x08080808); }
            if constexpr(F == 6){ w0 = __vsub4(w0, 0x10101010); w1 = __vsub4(w1, 0x10101010); }
            int dh[2][4];
            dh[0][0] = dh[0][1] = dh[0][2] = dh[0][3] = 0;
            dh[1][0] = dh[1][1] = dh[1][2] = dh[1][3] = 0;
            lk_mb_mma16(dh[0], a[0][0], a[0][1], w0);
            lk_mb_mma16(dh[1], a[1][0], a[1][1], w1);
            #pragma unroll
            for(int o = 0; o < 2; ++o){
              const unsigned char* bo = wk + (long)(n * 8 + 2 * q4 + o) * rb + (long)u * BB;
              const float dd = lk_h(bo), cc = F == 3 || F == 7 ? lk_h(bo + 2) : 0.f;
              const float d[2] = {dd, dd}, c[2] = {cc, cc};
              #pragma unroll
              for(int r = 0; r < 2; ++r){
                const int el = r * 2 + o;
                acc[i][n][el] = __fmaf_rn(r ? x1 : x0, lk_mb_part<F>(d, c, dh[0][el], sh[0][r], dh[1][el], sh[1][r]), acc[i][n][el]);
              }
            }
          }
        }
      }
    }
    #pragma unroll
    for(int n = 0; n < NW; ++n)
      #pragma unroll
      for(int el = 0; el < 4; ++el) part[warp][(el >> 1) * 8 + g][n * 8 + 2 * q4 + (el & 1)] = acc[0][n][el] + acc[1][n][el];
  } else {
    // A quarter's fragments leave registers for one leaf's sums: the first leaf's are parked in the table. Rows past M
    // read row M − 1 and their sums are never stored, so the fragments every n-tile waits on load unpredicated.
    const int c0 = min(g, M - 1), c1 = min(g + 8, M - 1);
    const signed char* xc0 = xq + (long)c0 * K;
    const signed char* xc1 = xq + (long)c1 * K;
    #pragma unroll 1
    for(int i = 0; i < 2; ++i){
      const int L = warp + 16 * i;
      float acc[NW][4];
      #pragma unroll
      for(int n = 0; n < NW; ++n)
        #pragma unroll
        for(int e = 0; e < 4; ++e) acc[n][e] = 0.f;
      for(int rd = 0; rd < rounds; ++rd){
        const int u = rd * 32 + L;
        if(u >= nu) break;
        // A quarter: sub-blocks 4u … 4u + 3, half `prt` of block u / 2 (the lane's grouped layout).
        const int s4 = 4 * u, blk = u >> 1, prt = u & 1;
        int a[4][2][2], sh[4][2][2];
        float xv[4][2];
        #pragma unroll
        for(int k = 0; k < 4; ++k){
          #pragma unroll
          for(int hf = 0; hf < 2; ++hf) lk_mb_act(xc0, xc1, true, true, (s4 + k) * 32 + hf * 16 + 4 * q4, a[k][hf], sh[k][hf]);
          xv[k][0] = xs[(long)c0 * nb + s4 + k]; xv[k][1] = xs[(long)c1 * nb + s4 + k];
        }
        #pragma unroll
        for(int n = 0; n < NW; ++n){
          const unsigned char* b = wk + (long)(n * 8 + g) * rb + lk_q6_blk(blk);
          unsigned int ql[4], qh[2];
          #pragma unroll
          for(int j = 0; j < 4; ++j) ql[j] = __ldg((const unsigned int*)(b + prt * 64 + 4 * (4 * j + q4)));
          #pragma unroll
          for(int j = 0; j < 2; ++j) qh[j] = __ldg((const unsigned int*)(b + 128 + prt * 32 + 4 * (4 * j + q4)));
          float dd[2];
          uint2 sw[2];
          #pragma unroll
          for(int o = 0; o < 2; ++o){
            const unsigned char* row = wk + (long)(n * 8 + 2 * q4 + o) * rb;
            dd[o] = __half2float(__ushort_as_half(__ldg((const unsigned short*)(row + lk_q6_d(blk)))));
            sw[o] = __ldg((const uint2*)(row + lk_q6_blk(blk) + 192 + prt * 8));
          }
          #pragma unroll
          for(int k = 0; k < 4; ++k){
            int dh[2][4];
            #pragma unroll
            for(int hf = 0; hf < 2; ++hf){
              const int lo = ((int)ql[(k & 1) * 2 + hf] >> (4 * (k >> 1))) & 0x0F0F0F0F;
              const int hi = (((int)qh[hf] >> (2 * k)) & 0x03030303) << 4;
              dh[hf][0] = dh[hf][1] = dh[hf][2] = dh[hf][3] = 0;
              lk_mb_mma16(dh[hf], a[k][hf][0], a[k][hf][1], (int)__vsub4(lo | hi, 0x20202020));
            }
            #pragma unroll
            for(int o = 0; o < 2; ++o){
              const unsigned int wv = k < 2 ? sw[o].x : sw[o].y;
              const float d[2] = {dd[o] * (float)(signed char)((wv >> (16 * (k & 1))) & 0xff), dd[o] * (float)(signed char)((wv >> (16 * (k & 1) + 8)) & 0xff)};
              const float c[2] = {0.f, 0.f};
              #pragma unroll
              for(int r = 0; r < 2; ++r){
                const int el = r * 2 + o;
                acc[n][el] = __fmaf_rn(xv[k][r], lk_mb_part<F>(d, c, dh[0][el], sh[k][0][r], dh[1][el], sh[k][1][r]), acc[n][el]);
              }
            }
          }
        }
      }
      #pragma unroll
      for(int n = 0; n < NW; ++n)
        #pragma unroll
        for(int el = 0; el < 4; ++el){
          float& v = part[warp][(el >> 1) * 8 + g][n * 8 + 2 * q4 + (el & 1)];
          v = i ? v + acc[n][el] : acc[n][el];
        }
    }
  }
  __syncthreads();
  nl = blockIdx.x * NO; np = N;
  lk_out y; lk_epi bias; const int* __restrict__ yrow; long ypitch;
  {
    lk_w pw;
    lk_take_part(parts, nl, pw, y, bias, yrow, ypitch, np);
  }
  lk_out yo = LK_ROW(y, yrow, ypitch);
  for(int i = threadIdx.x; i < 16 * NO; i += 32 * WARPS){
    const int t = i / NO, o = i % NO;
    if(t >= M) continue;
    float v[16];
    #pragma unroll
    for(int L = 0; L < 16; ++L) v[L] = part[L][t][o];
    #pragma unroll
    for(int L = 0; L < 8; ++L) v[L] += v[L + 8];
    #pragma unroll
    for(int L = 0; L < 4; ++L) v[L] += v[L + 4];
    v[0] += v[2]; v[1] += v[3];
    v[0] += v[1];
    yo[(long)t * np + nl + o] = bias(v[0], t, nl + o, np);
  }
}
"#;

/// Rows `idx[0], idx[1], …` (1-based) of a table in block format `F`, decoded to `f32` and scaled by `scale`: one
/// thread per 32-weight sub-block of the output.
const GATHER_BLK: &str = r#"
// Sub-block `sb` of row `r` of a table in block format `F`, its 32 values scaled into `o`.
template<int F>
__device__ __forceinline__ void lk_gather_sub(lk_w w, long r, int sb, float* __restrict__ o, float scale){
  if constexpr(F == 1 || F == 30){
    const unsigned short* h = (const unsigned short*)lk_h16_at(w, r, sb);
    #pragma unroll
    for(int i = 0; i < 32; ++i) o[i] = lk_h16<F>(h[i]) * scale;
  } else {
    signed char v[32]; float d[2], c[2];
    lk_sub<F>(w.b + r * w.rb, sb, v, d, c);
    #pragma unroll
    for(int i = 0; i < 32; ++i) o[i] = __fadd_rn(__fmul_rn(d[i >> 4], (float)v[i]), c[i >> 4]) * scale;
  }
}
template<int F>
__global__ void lk_gather_blk(lk_w w, const int* __restrict__ idx, float* __restrict__ out, int cols, float scale){
  int sb = blockIdx.x * blockDim.x + threadIdx.x; if(sb >= (cols >> 5)) return;
  lk_gather_sub<F>(w, (long)(idx[blockIdx.y] - 1), sb, out + (long)blockIdx.y * cols + sb * 32, scale);
}
"#;

/// Rows `idx[0], idx[1], …` (1-based) of a Q8_0 table, scaled to `f32` and by `scale`, one row of the output per
/// index (`gridDim.y` of them).
const GATHER_Q8: &str = r#"
// Value `c` of row `r` of a Q8_0 table, scaled.
__device__ __forceinline__ float lk_gather_q8_at(const signed char* __restrict__ wq, const __half* __restrict__ ws, long r, int c,
                                                 int cols, float scale){
  return (float)wq[r * cols + c] * __half2float(ws[r * (cols >> 5) + (c >> 5)]) * scale;
}
__global__ void lk_gather_q8(const signed char* __restrict__ wq, const __half* __restrict__ ws,
                             const int* __restrict__ idx, float* __restrict__ out, int cols, float scale){
  int c = blockIdx.x * blockDim.x + threadIdx.x; if(c >= cols) return;
  out[(long)blockIdx.y * cols + c] = lk_gather_q8_at(wq, ws, (long)(idx[blockIdx.y] - 1), c, cols, scale);
}
"#;

/// Rows `idx[0], idx[1], …` (1-based) of an `f32` table, scaled by `scale`, one row of the output per index (`gridDim.y`
/// of them).
const GATHER_F32: &str = r#"
// Value `c` of row `r` of an `f32` table, scaled.
__device__ __forceinline__ float lk_gather_f32_at(const float* __restrict__ t, long r, int c, int cols, float scale){
  return t[r * cols + c] * scale;
}
__global__ void lk_gather_f32(const float* __restrict__ t, const int* __restrict__ idx, float* __restrict__ out, int cols,
                              float scale){
  int c = blockIdx.x * blockDim.x + threadIdx.x; if(c >= cols) return;
  out[(long)blockIdx.y * cols + c] = lk_gather_f32_at(t, (long)(idx[blockIdx.y] - 1), c, cols, scale);
}
"#;

/// Per-row RMSNorm, one block per row: `y = v / sqrt(mean(v²) + eps) · w` (no `w` for the weightless sugar) with `v = x (+ r)`. `ADD` adds the
/// residual first (and `SUM` writes the sum out, the residual stream the next block reads); `OUTF` writes `y`;
/// `Q` quantizes `y` for the GEMVs that read it (`cols` a multiple of 32). Row `r` of `x` is read `r·xp + xo` in (a part
/// of every row of a wider table), and `y`'s written `r·yp + yo` into the rows of a table from the row `yrow` names. The
/// norm's `norm_threads(cols)` threads read the row once, thread `t` keeping its `PER` values `t, t + norm_threads, …`
/// in registers from the sum of squares to the scaling; a block of them all, or of a `VW`-th of them, each of its warps
/// playing `VW` of the norm's (`norm_block`), so a chunk's rows run in blocks a multiprocessor holds several of. The row's work is `lk_rmsnorm_row`, which a projection's
/// last-arriving block also runs over the rows the projection wrote (`L2`: read past the L1, where other blocks of the
/// launch wrote them), so the two compute the same norm. A block of fewer threads than the norm's own plays them:
/// `vt` the norm's threads, each warp of the block up to `VW` of the norm's warps (`lk_warps_sum`). A factor `c` other
/// than 1 scales `y` once it is rounded to `f32`, before it is stored or quantized, as `lk_scale` scales the stored row
/// (`norm_scale`); the sum is written unscaled. With the partials of an all-reduce (`f`, `allreduce_norm`), the added
/// operand is their left fold, each partial read where it lives and summed in order as the all-reduce's adds sum them.
const RMSNORM: &str = r#"
// The partials whose left fold a norm adds in place of one operand: `p[0] + p[1] + …`, the first `N` (`L2`: read past
// the L1). `N` is the kernel's own, so every partial's pointer is a register, never an array indexed at run time.
#define LK_FOLD 8
struct lk_fold {
  const float* p[LK_FOLD];
  template<int N, bool L2> __device__ __forceinline__ float at(long i) const {
    float a = L2 ? __ldcg(p[0] + i) : p[0][i];
    #pragma unroll
    for(int k = 1; k < N; ++k) a = a + (L2 ? __ldcg(p[k] + i) : p[k][i]);
    return a;
  }
};
// The norm a fold runs over the row its first part finished: its operands `x` and `r` (one of them that result), the
// sum it writes, its weight and outputs (`y` at the row `yrow` names), the norm of its added operand first (`pw`,
// `peps`).
struct lk_norm_of {
  const float* x; const float* r; float* s; const float* w; lk_out y; const int* yrow; long ypitch;
  signed char* yq; signed char* yl; float* ys; float eps; const float* pw; float peps;
};
template<bool ADD, bool SUM, bool OUTF, int Q, int PER, typename A, bool PRE, bool L2, int VW = 1, int FN = 0>
__device__ __forceinline__ void lk_rmsnorm_row(const float* x, const float* r, float* __restrict__ s,
                           const float* __restrict__ w, lk_out yr, signed char* __restrict__ yq,
                           signed char* __restrict__ yl, float* __restrict__ ys, int cols, float eps, long base, long xb,
                           const float* __restrict__ pw, float peps, A* red, int vt, float c = 1.f, lk_fold f = {}){
  auto ld = [](const float* p){ return L2 ? __ldcg(p) : *p; };
  // The added operand at `i`: its own array's value, or the fold of the all-reduce's `FN` partials.
  auto added = [&](long i){
    if constexpr(FN > 0) return f.template at<FN, L2>(i);
    else return ld(r + i);
  };
  const int nw = blockDim.x >> 5, vw = vt >> 5, warp = threadIdx.x >> 5, lane = threadIdx.x & 31;
  // The norm's thread this thread plays `j`-th, and whether its warp is one of the norm's.
  auto u = [&](int j){ return (warp + j * nw) * 32 + lane; };
  auto on = [&](int j){ return warp + j * nw < vw; };
  float v[VW][PER];
  // Under PRE the added operand is first normed here (by `pw`, `peps`), each value as a norm of its own computes it.
  float p[VW][PRE ? PER : 1];
  if(PRE){
    A pacc[VW];
    #pragma unroll
    for(int j = 0; j < VW; ++j){
      pacc[j] = 0;
      #pragma unroll
      for(int k = 0; k < PER; ++k){
        int i = u(j) + k * vt;
        p[j][k] = on(j) && i < cols ? added(base + i) : 0.f;
        pacc[j] += (A)p[j][k] * (A)p[j][k];
      }
    }
    A pden = sqrt(lk_warps_sum<VW>(pacc, vw, red) / (A)cols + (A)peps);
    #pragma unroll
    for(int j = 0; j < VW; ++j){
      #pragma unroll
      for(int k = 0; k < PER; ++k){
        int i = u(j) + k * vt;
        if(on(j) && i < cols) p[j][k] = pw ? (float)((A)p[j][k] / pden * (A)pw[i]) : (float)((A)p[j][k] / pden);
      }
    }
  }
  A acc[VW];
  #pragma unroll
  for(int j = 0; j < VW; ++j){
    acc[j] = 0;
    #pragma unroll
    for(int k = 0; k < PER; ++k){
      int i = u(j) + k * vt;
      v[j][k] = 0.f;
      if(on(j) && i < cols){
        v[j][k] = ld(x + xb + i);
        if(ADD){ v[j][k] += PRE ? p[j][k] : added(base + i); if(SUM) s[base + i] = v[j][k]; }
      }
      acc[j] += (A)v[j][k] * (A)v[j][k];
    }
  }
  A den = sqrt(lk_warps_sum<VW>(acc, vw, red) / (A)cols + (A)eps);
  #pragma unroll
  for(int j = 0; j < VW; ++j){
    if(!on(j)) continue;
    #pragma unroll
    for(int k = 0; k < PER; ++k){
      int i = u(j) + k * vt;
      // A warp holds a whole 32-value group: the quantizer's maximum is the warp's.
      if(Q == 0 ? i < cols : (i & ~31) < cols){
        float o = w ? (float)((A)v[j][k] / den * (A)w[i]) : (float)((A)v[j][k] / den);
        if(c != 1.f) o = o * c;
        if(Q == 0 || OUTF) yr[i] = o;
        if(Q > 0) lk_quant_lane<Q>(o, yq, yl, ys, base + i, (base + i) >> 5);
      }
    }
  }
}
template<bool ADD, bool SUM, bool OUTF, int Q, int PER, typename A, bool PRE = false, int FN = 0, int VW = 1>
__global__ void __launch_bounds__(1024 / VW) lk_rmsnorm(const float* __restrict__ x, const float* __restrict__ r,
                           float* __restrict__ s, const float* __restrict__ w, lk_out y,
                           signed char* __restrict__ yq, signed char* __restrict__ yl, float* __restrict__ ys, int cols,
                           float eps, long xp, long xo, const int* __restrict__ yrow, long ypitch, long yp, long yo,
                           const float* __restrict__ pw = nullptr, float peps = 0.f, float c = 1.f, lk_fold f = {}){
  __shared__ A red[32];
  long base = (long)blockIdx.x * cols, xb = (long)blockIdx.x * xp + xo;
  lk_out yr = y ? LK_ROW(y, yrow, ypitch) + (long)blockIdx.x * yp + yo : lk_out();
  lk_rmsnorm_row<ADD, SUM, OUTF, Q, PER, A, PRE, false, VW, FN>(x, r, s, w, yr, yq, yl, ys, cols, eps, base, xb, pw, peps, red,
                                                                VW * blockDim.x, c, f);
}
"#;

/// A row normed by parts (`Launch::NormParts`), a block a row and part: `lk_sumsq` sums a part's squares in the
/// accumulator `A` as `lk_rmsnorm`'s block over a row of that part alone sums them — `PER` values a thread, the threads
/// those of the widest part's norm — into `s[r·parts + k]`, the one value a row the part sends the others; then
/// `lk_norm_by_sums` adds row `r`'s sums in part order and scales each part it writes as `lk_rmsnorm` scales a row by
/// its sum, the part's weights from its place in the row on.
const NORM_PARTS: &str = r#"
#define LK_NORM_PARTS 8
// The parts of the row: part `k`'s row `r` read from `x[k] + r·xp[k] + xo[k]`, `cols[k]` wide, its weights from `wo[k]`
// on of the norm's.
struct lk_row_parts {
  const float* x[LK_NORM_PARTS]; long xp[LK_NORM_PARTS]; long xo[LK_NORM_PARTS]; int cols[LK_NORM_PARTS]; long wo[LK_NORM_PARTS];
};
// The parts it writes: output `o` part `part[o]` normed, row `r` written at `y[o]` (rows from the one-based position
// `row[o]` names, `pitch[o]` apart, where it names one) from `r·yp[o] + yo[o]`.
struct lk_normed_parts {
  lk_out y[LK_NORM_PARTS]; const int* row[LK_NORM_PARTS]; long pitch[LK_NORM_PARTS]; long yp[LK_NORM_PARTS];
  long yo[LK_NORM_PARTS]; int part[LK_NORM_PARTS];
};
template<int PER, typename A>
__global__ void __launch_bounds__(1024) lk_sumsq(lk_row_parts in, A* __restrict__ s, int parts){
  __shared__ A red[32];
  const int k = blockIdx.y;
  const float* x = in.x[k] + (long)blockIdx.x * in.xp[k] + in.xo[k];
  A acc[1] = {0};
  #pragma unroll
  for(int j = 0; j < PER; ++j){
    int i = threadIdx.x + j * blockDim.x;
    float v = i < in.cols[k] ? x[i] : 0.f;
    acc[0] += (A)v * (A)v;
  }
  A t = lk_warps_sum<1>(acc, blockDim.x >> 5, red);
  if(threadIdx.x == 0) s[(long)blockIdx.x * parts + k] = t;
}
template<int PER, typename A>
__global__ void __launch_bounds__(1024) lk_norm_by_sums(lk_row_parts in, lk_normed_parts out, const A* __restrict__ s,
                           const float* __restrict__ w, int parts, int n, float eps){
  const long r = blockIdx.x;
  const int o = blockIdx.y, k = out.part[o];
  A t = s[r * parts];
  for(int q = 1; q < parts; ++q) t = t + s[r * parts + q];
  A den = sqrt(t / (A)n + (A)eps);
  const float* x = in.x[k] + r * in.xp[k] + in.xo[k];
  lk_out y = LK_ROW(out.y[o], out.row[o], out.pitch[o]) + r * out.yp[o] + out.yo[o];
  #pragma unroll
  for(int j = 0; j < PER; ++j){
    int i = threadIdx.x + j * blockDim.x;
    if(i < in.cols[k]) y[i] = w ? (float)((A)x[i] / den * (A)w[in.wo[k] + i]) : (float)((A)x[i] / den);
  }
}
"#;

/// A gather, then `lk_rmsnorm` over the row it gathered, a block a row: the embedding's rows and the first norm
/// (`embed_norm`). A block of the norm's threads, or of a `VW`-th of them each warp playing `VW` of the norm's
/// (`norm_block`), gathers its row into `out` — the raw row its later readers take — as the gather of the table's
/// format computes it (`F` 0: an `f32` table `t`; 8: Q8_0's split arrays; a block format otherwise), then norms it as
/// `lk_rmsnorm`'s block for that row does.
const GATHER_NORM: &str = r#"
template<int F, bool OUTF, int Q, int PER, typename A, int VW = 1>
__global__ void __launch_bounds__(1024 / VW) lk_gather_norm(lk_w w, const float* __restrict__ t, const int* __restrict__ idx,
                           float* __restrict__ out, int cols, float scale, const float* __restrict__ nw, lk_out y,
                           signed char* __restrict__ yq, signed char* __restrict__ yl, float* __restrict__ ys, float eps,
                           const int* __restrict__ yrow, long ypitch){
  __shared__ A red[32];
  const long r = (long)(idx[blockIdx.x] - 1), base = (long)blockIdx.x * cols;
  if constexpr(F == 0){
    for(int c = threadIdx.x; c < cols; c += blockDim.x) out[base + c] = lk_gather_f32_at(t, r, c, cols, scale);
  } else if constexpr(F == 8){
    for(int c = threadIdx.x; c < cols; c += blockDim.x) out[base + c] = lk_gather_q8_at(w.q, w.s, r, c, cols, scale);
  } else {
    for(int sb = threadIdx.x; sb < (cols >> 5); sb += blockDim.x) lk_gather_sub<F>(w, r, sb, out + base + sb * 32, scale);
  }
  __syncthreads();
  lk_out yr = y ? LK_ROW(y, yrow, ypitch) + base : lk_out();
  lk_rmsnorm_row<false, false, OUTF, Q, PER, A, false, false, VW>(out, nullptr, nullptr, nw, yr, yq, yl, ys, cols, eps, base, base,
                                                                  nullptr, 0.f, red, VW * blockDim.x);
}
"#;

/// `lk_gemv_q8`, then, in its last-arriving block, `lk_rmsnorm` over the rows it wrote: a projection with the residual
/// add and the next norm in its epilogue. A block of `norm_threads(N)` threads, a warp an output row, computes each row
/// as `lk_gemv_q8` does and writes it to `y`; every block counts in at `cnt` once its rows are out, and the last reads
/// `y` back through L2 and norms each of the `M` rows as `lk_rmsnorm`'s block for that row does — the norm's `x` and
/// `r` are its operands, one of them `y` — then resets the count.
const GEMV_Q8_NORM: &str = r#"
template<int Q, int M, bool ADD, bool SUM, bool OUTF, int QO, int PER, typename A, bool PRE>
__global__ void __launch_bounds__(1024) lk_gemv_q8_norm(const signed char* __restrict__ wq, const __half* __restrict__ ws,
                           const float* __restrict__ ax, const signed char* __restrict__ xq,
                           const signed char* __restrict__ xl, const float* __restrict__ xs,
                           const float* __restrict__ bias, float* y, int K, int N,
                           const float* x, const float* r, float* __restrict__ s, const float* __restrict__ w, lk_out yo,
                           const int* __restrict__ yrow, long ypitch, signed char* __restrict__ yq,
                           signed char* __restrict__ yl, float* __restrict__ ys, float eps, int* __restrict__ cnt,
                           const float* __restrict__ pw = nullptr, float peps = 0.f){
  __shared__ A red[32];
  __shared__ int last;
  int o = (blockIdx.x * blockDim.x + threadIdx.x) >> 5;
  if(o < N){
    float v[M];
    lk_q8_rows_dot<Q, M, 1>(wq, ws, o, K, ax, xq, xl, xs, v);
    for(int t = threadIdx.x & 31; t < M; t += 32){
      const float vt = lk_pick(v, t);
      y[(long)t * N + o] = bias ? vt + bias[o] : vt;
    }
  }
  __threadfence();
  __syncthreads();
  if(threadIdx.x == 0) last = atomicAdd(cnt, 1) == gridDim.x - 1;
  __syncthreads();
  if(!last) return;
  __threadfence();
  lk_out yr = yo ? LK_ROW(yo, yrow, ypitch) : lk_out();
  for(int t = 0; t < M; ++t){
    long base = (long)t * N;
    lk_rmsnorm_row<ADD, SUM, OUTF, QO, PER, A, PRE, true>(x, r, s, w, yo ? yr + base : lk_out(), yq, yl, ys, N, eps, base, base, pw, peps, red, blockDim.x);
  }
  if(threadIdx.x == 0) *cnt = 0;
}
"#;

/// Quantize an activation of `n` values (a multiple of 32), one warp per group.
const QUANT: &str = r#"
template<int Q>
__global__ void lk_quant(const float* __restrict__ x, signed char* __restrict__ xq, signed char* __restrict__ xl,
                         float* __restrict__ xs, int n){
  long i = (long)blockIdx.x * blockDim.x + threadIdx.x; if(i >= n) return;
  lk_quant_lane<Q>(x[i], xq, xl, xs, i, i >> 5);
}
"#;

/// `Y[t,o] = Σ_k X[t,k]·W[o,k]` for the `M` rows of `X` (`M ≤ 16`) over the `N` rows of a Q8_0 matrix (int8 rows of `K`,
/// an f16 scale per 32), one warp per output row: the weight row is read once for every row of `X`. `Q` is the
/// activations' numerics; the result may land in the rows of a cache a position names.
const GEMV_Q8: &str = r#"
template<int Q, int M>
__global__ void lk_gemv_q8(const signed char* __restrict__ wq, const __half* __restrict__ ws,
                           const float* __restrict__ x, const signed char* __restrict__ xq,
                           const signed char* __restrict__ xl, const float* __restrict__ xs,
                           lk_epi bias, lk_out y, const int* __restrict__ yrow,
                           long ypitch, int K, int N){
  int o = (blockIdx.x * blockDim.x + threadIdx.x) >> 5; if(o >= N) return;
  lk_out yo = LK_ROW(y, yrow, ypitch);
  float v[M];
  lk_q8_rows_dot<Q, M, 1>(wq, ws, o, K, x, xq, xl, xs, v);
  for(int t = threadIdx.x & 31; t < M; t += 32) yo[(long)t * N + o] = bias(lk_pick(v, t), t, o, N);
}
"#;

/// The tensor-core GEMMs' helpers: a 16-byte `cp.async` that zero-fills when `pred` is false, the int8 `mma`s of k 32
/// and k 16, and the read of an `mma`'s int32 sum as a float.
const MMA: &str = r#"
// An int8 `mma`'s int32 sum read as a float, exactly (a block's dot of 32 int8 products is under 2¹⁹). Read by the
// conversion where a kernel asks it (`I`) and it issues on the float pipe (I2FP, sm_86 on): the sum begins at zero (the
// zero register: no start value moved into an `mma`'s accumulators), `lk_dot0`. Otherwise it begins at the bits of
// 1.5·2²³, ends as the float 1.5·2²³ + sum while |sum| < 2²², and one float subtraction reads it — before sm_86 the
// conversion runs at a quarter of the float rate. The two readings give the same float.
#if defined(__CUDA_ARCH__) && __CUDA_ARCH__ >= 860
#define LK_I2FP true
#else
#define LK_I2FP false
#endif
template<bool I> __host__ __device__ constexpr int lk_dot0(){ return I && LK_I2FP ? 0 : 0x4B400000; }
template<bool I> __device__ __forceinline__ float lk_dot_f(int d){
  if constexpr(I && LK_I2FP) return (float)d;
  else return __int_as_float(d) - 12582912.f;
}
__device__ __forceinline__ void lk_cp16(void* smem, const void* gmem, bool pred){
  unsigned s = (unsigned)__cvta_generic_to_shared(smem);
  int n = pred ? 16 : 0;
  asm volatile("cp.async.cg.shared.global [%0], [%1], 16, %2;\n" :: "r"(s), "l"(gmem), "r"(n));
}
// Four `f32` of a staged tile from four elements at `g` (zeros where `pred` is false): an `f32` array's by `cp.async`,
// landing with the copies in flight beside it; a binary16 array's widened as it is read, there at once.
__device__ __forceinline__ void lk_stage4(float* smem, const float* g, bool pred){ lk_cp16(smem, g, pred); }
__device__ __forceinline__ void lk_stage4(float* smem, const __nv_fp8_e4m3* g, bool pred){
  *(float4*)smem = pred ? lk_from4(g) : make_float4(0.f, 0.f, 0.f, 0.f);
}
__device__ __forceinline__ void lk_stage4(float* smem, const __half* g, bool pred){
  *(float4*)smem = pred ? lk_from4(g) : make_float4(0.f, 0.f, 0.f, 0.f);
}
__device__ __forceinline__ void lk_mma_s8(int* c, const int* a, const int* b){
  asm volatile("mma.sync.aligned.m16n8k32.row.col.s32.s8.s8.s32 {%0,%1,%2,%3}, {%4,%5,%6,%7}, {%8,%9}, {%0,%1,%2,%3};\n"
               : "+r"(c[0]), "+r"(c[1]), "+r"(c[2]), "+r"(c[3])
               : "r"(a[0]), "r"(a[1]), "r"(a[2]), "r"(a[3]), "r"(b[0]), "r"(b[1]));
}
__device__ __forceinline__ void lk_mma_s8_k16(int* c, const int* a, int b){
  asm volatile("mma.sync.aligned.m16n8k16.row.col.s32.s8.s8.s32 {%0,%1,%2,%3}, {%4,%5}, {%6}, {%0,%1,%2,%3};\n"
               : "+r"(c[0]), "+r"(c[1]), "+r"(c[2]), "+r"(c[3]) : "r"(a[0]), "r"(a[1]), "r"(b));
}
// An int8 `mma` whose sums start at `c`: not volatile, so the scheduler interleaves it with the float work around it.
__device__ __forceinline__ void lk_mma_s8_from(int* d, const int* a, const int* b, int c){
  asm("mma.sync.aligned.m16n8k32.row.col.s32.s8.s8.s32 {%0,%1,%2,%3}, {%4,%5,%6,%7}, {%8,%9}, {%10,%10,%10,%10};\n"
      : "=r"(d[0]), "=r"(d[1]), "=r"(d[2]), "=r"(d[3]) : "r"(a[0]), "r"(a[1]), "r"(a[2]), "r"(a[3]), "r"(b[0]), "r"(b[1]), "r"(c));
}
__device__ __forceinline__ void lk_mma_s8_k16_from(int* d, const int* a, int b, int c){
  asm("mma.sync.aligned.m16n8k16.row.col.s32.s8.s8.s32 {%0,%1,%2,%3}, {%4,%5}, {%6}, {%7,%7,%7,%7};\n"
      : "=r"(d[0]), "=r"(d[1]), "=r"(d[2]), "=r"(d[3]) : "r"(a[0]), "r"(a[1]), "r"(b), "r"(c));
}
// A `cp.async` of `B` bytes (4, 8 or 16) copying `n` of them and zero-filling the rest.
template<int B>
__device__ __forceinline__ void lk_cpn(void* smem, const void* gmem, int n){
  unsigned s = (unsigned)__cvta_generic_to_shared(smem);
  if constexpr(B == 16) asm volatile("cp.async.cg.shared.global [%0], [%1], 16, %2;\n" :: "r"(s), "l"(gmem), "r"(n));
  else asm volatile("cp.async.ca.shared.global [%0], [%1], %2, %3;\n" :: "r"(s), "l"(gmem), "n"(B), "r"(n));
}
// Four 8 × 8 matrices of 16-bit words from shared memory, a lane's row address each, as `mma` fragments.
__device__ __forceinline__ void lk_ldsm_s8(int* r, const void* p){
  unsigned s = (unsigned)__cvta_generic_to_shared(p);
  asm volatile("ldmatrix.sync.aligned.m8n8.x4.shared.b16 {%0,%1,%2,%3}, [%4];\n" : "=r"(r[0]), "=r"(r[1]), "=r"(r[2]), "=r"(r[3]) : "r"(s));
}
// Two, lanes 0 to 15 giving the rows.
__device__ __forceinline__ void lk_ldsm2_s8(int* r, const void* p){
  unsigned s = (unsigned)__cvta_generic_to_shared(p);
  asm volatile("ldmatrix.sync.aligned.m8n8.x2.shared.b16 {%0,%1}, [%2];\n" : "=r"(r[0]), "=r"(r[1]) : "r"(s));
}
"#;

/// `Y[m, n] = Σ_k X[m, k]·W[n, k]` for `M` rows of int8 activations (a scale per 32, and the low words under `Q` 2) over
/// the `N` rows of a matrix in block format `F`, on the int8 tensor cores: a block computes a `BM` × 128 tile over
/// stages of 128 `k` (four sub-blocks). The activation rows land in shared memory by `cp.async`, the next stage's in a
/// second buffer while this one multiplies where both fit; the weight rows' sub-blocks are fetched a stage ahead as
/// their raw words (`lk_fetch`) and decoded from them (`lk_unpack`, as `lk_sub4` decodes) into int8 levels and each
/// half's scale and offset. Where two decoded tiles fit beside the two activation buffers and a block holds its
/// multiprocessor alone (`lk_stage_over`), the next stage's activations are copied, its weights decoded into the other
/// tile and the stage after's words fetched between this stage's sub-blocks, so one barrier at a stage's start frees
/// the last one's buffers: Qwen3-8B's paired gate and up over a 512-row prompt took 3.32 ms on the 3060 where a stage
/// waiting on its decode took 4.15 (its Q6_K down 2.32 where 2.52). Each sub-block's activation sum is taken once a
/// stage, scaled by its row's scale, for the offsets. A warp multiplies 32 × 32, one
/// `m16n8k32` `mma` a sub-block — two `m16n8k16` for a format whose halves scale apart (Q2_K, Q3_K, Q6_K) — each int32
/// dot scaled as the sub-block's dot is, `s·(d₀·dot₀ + c₀·Σ₀ + d₁·dot₁ + c₁·Σ₁)`; where the halves scale by integers
/// under the block's one scale `d` (Q3_K, Q6_K, `lk_int_halves`), their dots are weighed by those integers exactly and
/// converted once, `s·d·(sc₀·dot₀ + sc₁·dot₁)` — a third of the epilogue's arithmetic gone: Qwen3-8B's Q6_K down at
/// 512 rows on the 3060 2.34 ms → 1.94, and nearer the exact product (8.2e-7 of its largest against 1.1e-6). Rows past `M` and outputs past `N`
/// are zero and never stored. A Q8_0 matrix (`F` 8) is read as its levels and one scale a sub-block, and under two words
/// a sub-block's high and low dots fold into one exact integer, `s/128·d·(128·dot + dotₗ)`. Over a `k` of whole
/// 128-value scale groups a Q8_0 matrix streams (`lk_blk_streamed`): a stage is 32 of `k`, `LK_STREAM_STAGES` of them in
/// flight by `cp.async`, each landing on a barrier of its own and freed when every warp has read it (no barrier of the
/// whole block in the loop), its rows swizzled so a `ldmatrix` reads every bank once; each scale group (four stages' row
/// and column scales) lands with its first stage, two held; paired, a warp's 32 outputs are 16 gate rows and the same 16
/// up rows, so each hidden value meets its gate and up sums in one thread. Either path sums each output alike, and cuts
/// the same tiles of a last wave.
///
/// Grouped (`G` 1, 2), the GEMM multiplies an expert mixture's choices: grid layer `z` is expert `z`, its `ecount[z]`
/// rows the gathered activation rows from `eoff[z]`, its matrix rows `z·N … z·N + N − 1` of the stacked experts, its
/// outputs the same rows of `y` (`yrow` and `ypitch` unread); `G` 2 adds the expert's bias and weighs each row by its
/// choice's weight (`wt[list[row]]`), as `lk_moe_down_rows` weighs it. In parts (`G` 4), the GEMM multiplies one
/// activation by up to `GEMM_PARTS` matrices side by side (`parts`): its `N` outputs part `p`'s from `end[p − 1]` to
/// `end[p]`, each part a whole number of tiles, its matrix `w[p]`, its bias `bias[p]` and its outputs `y[p]` — rows from
/// the one-based position `row[p]` names, `pitch[p]` apart, where it names one. Paired
/// and dense (`G` 5), it multiplies a gated pair's gate (`w`, `bias`) and up (`w2`, `bias2`) over the activation's rows,
/// a tile `LK_BN / 2` of each, and writes each of the `N` hidden values through the gated activation — `ACT` −1 the
/// SwiGLU `g·σ(g)·u`, otherwise `u·act(g)` (`lk_act<ACT>`), as `lk_gemv_glu` computes it — into `y` where given and
/// quantized (`hq`, `hl`, `hs`) where given. A line of blocks cuts pair tiles `2i` and `2i + 1` as the gate's own
/// GEMM cuts its tile `i`.
const GEMM_BLK: &str = r#"
#define LK_BN 128
template<int F> __host__ __device__ constexpr bool lk_halves_apart(){ return F == 10 || F == 11 || F == 14; }
// The formats whose sub-blocks carry an offset (a minimum): Q4_1, Q5_1, Q2_K, Q4_K, Q5_K.
template<int F> __host__ __device__ constexpr bool lk_has_offset(){ return F == 3 || F == 7 || F == 10 || F == 12 || F == 13; }
// A stage's activations, in bytes: the activation tile (and its low words) and each row's scale for each of the
// stage's four sub-blocks.
template<int Q, int BM> __host__ __device__ constexpr int lk_stage_acts(){ return (Q == 2 ? 2 : 1) * BM * 144 + BM * 4 * 4; }
// A stage's decoded weights, in bytes: the weight tile's levels, then each sub-block's scales (two halves where they
// scale apart) and offsets.
template<int F> __host__ __device__ constexpr int lk_stage_wt(){
  return LK_BN * 144 + LK_BN * 4 * (lk_halves_apart<F>() ? 2 : 1) * (lk_has_offset<F>() ? 2 : 1) * 4;
}
// The activation's sums a stage, in bytes, where the format has offsets: each row's over each sub-block (each half where
// the halves scale apart), scaled by the row's scale, a float each — and the low words' under two.
template<int F, int Q, int BM> __host__ __device__ constexpr int lk_stage_sums(){
  return lk_has_offset<F>() ? (Q == 2 ? 2 : 1) * BM * 4 * (lk_halves_apart<F>() ? 2 : 1) * 4 : 0;
}
// The rest of a stage, in bytes: its decoded weights and the activation's sums.
template<int F, int Q, int BM> __host__ __device__ constexpr int lk_stage_rest(){ return lk_stage_wt<F>() + lk_stage_sums<F, Q, BM>(); }
// Whether the activations take two buffers — the next stage's landing while this one multiplies — within a block's
// 99 KB of shared memory.
template<int F, int Q, int BM> __host__ __device__ constexpr bool lk_stage_pipelined(){
  return 2 * lk_stage_acts<Q, BM>() + lk_stage_rest<F, Q, BM>() <= 101376;
}
// One stage's weights: the `SB` sub-blocks from `k0` of the `LK_BN` matrix rows from `wbase + n0`, their levels as int8
// words at `Bs` (rows `BP` bytes apart, zero past `N` and `K`) and each sub-block's scales (`DH`), then its offsets
// (`CH`), at `Bd`, a sub-block's rows side by side — so a column pair's are one vector: a thread a sub-block, the `U`
// of each of the `NT` threads from the words `raw` fetched for them — or, two a thread where a format's two share
// their words (`lk_stage_raw`), one pair's.
// A paired tile's (`PAIR`) weight rows are its `LK_BN / 2` gate rows, then the same up rows.
template<bool PAIR> __device__ __forceinline__ int lk_tile_row(int r){ return PAIR ? r & (LK_BN / 2 - 1) : r; }
// One piece of a stage's weights, sub-block `sb` of tile row `r` from its raw words: its levels at `Bs`, its scales
// and offsets at `Bd`.
template<int F, int SB, int BP, int DH, int CH, bool PAIR>
__device__ __forceinline__ void lk_stage_piece(const lk_raw<F>& raw, int n0, int N, int nb, int k0, int r, int sb, unsigned char* Bs,
                                               float* Bd){
  const int ks = (k0 >> 5) + sb;
  int wv[8]; float d[2], o[2];
  bool live = n0 + lk_tile_row<PAIR>(r) < N && ks < nb;
  if constexpr(lk_int_halves<F>()){
    // The block's scale, then the halves' integer scales, packed, in the second's bits (`lk_int_halves`).
    int sp = 0;
    d[0] = 0.f; o[0] = o[1] = 0.f;
    if(live) lk_unpack_ints<F>(raw, ks, wv, d[0], sp);
    else for(int i = 0; i < 8; ++i) wv[i] = 0;
    d[1] = __int_as_float(sp);
  } else if(live) lk_unpack<F>(raw, ks, wv, d, o);
  else { for(int i = 0; i < 8; ++i) wv[i] = 0; d[0] = d[1] = o[0] = o[1] = 0.f; }
  uint4* dst = (uint4*)(Bs + r * BP + sb * 32);
  dst[0] = make_uint4(wv[0], wv[1], wv[2], wv[3]); dst[1] = make_uint4(wv[4], wv[5], wv[6], wv[7]);
  #pragma unroll
  for(int h = 0; h < DH; ++h){
    Bd[(sb * LK_BN + r) * (DH + CH) + h] = d[h];
    if(CH) Bd[(sb * LK_BN + r) * (DH + CH) + DH + h] = o[h];
  }
}
template<int F, int SB, int BP, int DH, int CH, int NT, int U, bool PAIR>
__device__ __forceinline__ void lk_stage_weights(const lk_raw<F> (&raw)[U], int n0, int N, int nb, int k0, unsigned char* Bs,
                                                 float* Bd, int tid){
  #pragma unroll
  for(int u = 0; u < U; ++u){
    int c = tid + u * NT;
    lk_stage_piece<F, SB, BP, DH, CH, PAIR>(raw[u], n0, N, nb, k0, c / SB, c % SB, Bs, Bd);
  }
}
// A thread's pair of a stage's sub-blocks that share their words (`lk_raw_pair`): tile row `tid / (SB / 2)`, its
// sub-blocks `sb` and `sb + lk_pair_stride<F>()`.
template<int F, int SB> __device__ __forceinline__ int lk_pair_sb(int tid){
  constexpr int S = lk_pair_stride<F>();
  const int q = tid % (SB / 2);
  return q / S * 2 * S + q % S;
}
template<int F, int SB, int BP, int DH, int CH, int NT, int U, bool PAIR>
__device__ __forceinline__ void lk_stage_weights(const lk_raw_pair<F> (&raw)[1], int n0, int N, int nb, int k0, unsigned char* Bs,
                                                 float* Bd, int tid){
  const int r = tid / (SB / 2), sb = lk_pair_sb<F, SB>(tid);
  lk_stage_piece<F, SB, BP, DH, CH, PAIR>(raw[0].a, n0, N, nb, k0, r, sb, Bs, Bd);
  lk_raw<F> second = raw[0].a;
  second.sc = raw[0].sc2;
  lk_stage_piece<F, SB, BP, DH, CH, PAIR>(second, n0, N, nb, k0, r, sb + lk_pair_stride<F>(), Bs, Bd);
}
// The next stage's raw weight words for this thread's `U` sub-blocks: of `w`, or of `w2` in a paired tile's up rows.
template<int F, int SB, int NT, int U, bool PAIR>
__device__ __forceinline__ void lk_stage_fetch(lk_raw<F> (&raw)[U], lk_w w, lk_w w2, long wbase, int n0, int N, int K, int nb, int k0,
                                               int tid){
  #pragma unroll
  for(int u = 0; u < U; ++u){
    int c = tid + u * NT, r = c / SB, ks = (k0 >> 5) + c % SB, row = lk_tile_row<PAIR>(r);
    if(n0 + row < N && ks < nb) lk_fetch<F>(raw[u], PAIR && r >= LK_BN / 2 ? w2 : w, wbase + n0 + row, K, nb, ks);
  }
}
template<int F, int SB, int NT, int U, bool PAIR>
__device__ __forceinline__ void lk_stage_fetch(lk_raw_pair<F> (&raw)[1], lk_w w, lk_w w2, long wbase, int n0, int N, int K, int nb,
                                               int k0, int tid){
  const int r = tid / (SB / 2), ks = (k0 >> 5) + lk_pair_sb<F, SB>(tid), row = lk_tile_row<PAIR>(r);
  if(n0 + row < N && ks < nb) lk_fetch_pair<F>(raw[0], PAIR && r >= LK_BN / 2 ? w2 : w, wbase + n0 + row, K, nb, ks);
}
// The raw words a thread holds for its `U` sub-blocks of a stage: one pair's where a format's two share their words
// and the stage gives each thread two, each sub-block's own otherwise.
template<int F, int U> __host__ __device__ constexpr bool lk_stage_pairs(){ return U == 2 && lk_pair_stride<F>() > 0; }
template<bool B, class T, class E> struct lk_pick_type { using t = T; };
template<class T, class E> struct lk_pick_type<false, T, E> { using t = E; };
template<int F, int U> using lk_stage_raw = typename lk_pick_type<lk_stage_pairs<F, U>(), lk_raw_pair<F>, lk_raw<F>>::t;
template<int F, int U> __host__ __device__ constexpr int lk_stage_raws(){ return lk_stage_pairs<F, U>() ? 1 : U; }
// `BM` activation rows a block, `BM / 32 · 4` warps: 128 for a dense product, 64 for an expert's few rows. Paired (`G`
// 3), a block takes `LK_BN / 2` hidden columns of an expert's gate (`w`, `bias`) and up (`w2`, `bias2`) projections
// together and writes each hidden value through the gated activation `ACT` (`limit`, `slope`), as `lk_moe_glu` computes
// it, quantized as `lk_quant_lane` quantizes it (`hq`, `hl`, `hs`). A dense product with `part` given runs on a line
// of blocks: block `b < tail` the whole tile `b` (tiles row by row of column tiles), each later block one of `split`
// parts of a remaining tile's stages, its sums parked in `part`; the last part of a tile to arrive (`cnt`) sums the
// tile's parts in order and writes the tile.
#define LK_STREAM_STAGES 4
template<int Q, int BM> __host__ __device__ constexpr int lk_q8_stage(){ return (Q == 2 ? 2 : 1) * BM * 32 + LK_BN * 32; }
template<int BM> __host__ __device__ constexpr int lk_q8_group(){ return BM * 16 + LK_BN * 8; }
// Byte `16·c` of row `r` of a tile of 32-byte rows: the row's halves swapped on every other run of four rows.
__device__ __forceinline__ int lk_q8_swz(int r, int c){ return r * 32 + ((c ^ ((r >> 2) & 1)) << 4); }
__device__ __forceinline__ void lk_mbar_init(unsigned a, unsigned n){ asm volatile("mbarrier.init.shared.b64 [%0], %1;\n" :: "r"(a), "r"(n) : "memory"); }
__device__ __forceinline__ void lk_mbar_arrive(unsigned a){ asm volatile("{\n .reg .b64 st;\n mbarrier.arrive.shared.b64 st, [%0];\n}\n" :: "r"(a) : "memory"); }
// The barrier counts this thread's arrival once every `cp.async` it issued before has landed.
__device__ __forceinline__ void lk_mbar_copies(unsigned a){ asm volatile("cp.async.mbarrier.arrive.noinc.shared.b64 [%0];\n" :: "r"(a) : "memory"); }
__device__ __forceinline__ void lk_mbar_wait(unsigned a, unsigned parity){
  asm volatile("{\n .reg .pred p;\n LK_WAIT_%=:\n mbarrier.test_wait.parity.shared.b64 p, [%0], %1;\n @!p bra LK_WAIT_%=;\n}\n" :: "r"(a), "r"(parity) : "memory");
}
__device__ __forceinline__ void lk_cp16s(unsigned s, const void* g, bool pred){
  asm volatile("cp.async.cg.shared.global [%0], [%1], 16, %2;\n" :: "r"(s), "l"(g), "r"(pred ? 16 : 0));
}
__device__ __forceinline__ void lk_cp8s(unsigned s, const void* g, bool pred){
  asm volatile("cp.async.ca.shared.global [%0], [%1], 8, %2;\n" :: "r"(s), "l"(g), "r"(pred ? 8 : 0));
}
__device__ __forceinline__ void lk_ldsm_s(int* r, unsigned s){
  asm volatile("ldmatrix.sync.aligned.m8n8.x4.shared.b16 {%0,%1,%2,%3}, [%4];\n" : "=r"(r[0]), "=r"(r[1]), "=r"(r[2]), "=r"(r[3]) : "r"(s));
}
// Row `rw` of a tile's matrix rows: paired, row `rw` of warp `rw / 32`'s 32 is gate row `16·(rw / 32) + rw % 16` below
// 16 and the same up row above; otherwise row `rw`.
template<bool PAIR> __device__ __forceinline__ int lk_q8_row(int rw){ return PAIR ? (rw >> 5) * 16 + (rw & 15) : rw; }
// The streamed path of `lk_gemm_blk<8>` over a `K` of whole 128-value scale groups, each warp `WM` rows by 32 outputs
// of the tile: `lk_gemm_blk`'s arguments, its block's dynamic shared memory at `lk_sm`.
template<int Q, int G, int BM, int ACT, int WM, bool PLAIN>
__device__ __forceinline__ void lk_blk_streamed(unsigned char* lk_sm, lk_w w, const signed char* __restrict__ xq,
    const signed char* __restrict__ xl, const float* __restrict__ xs, lk_epi bias, lk_out y,
    const int* __restrict__ yrow, long ypitch, int K, int N, int M, const int* __restrict__ ecount,
    const int* __restrict__ eoff, const int* __restrict__ list, const float* __restrict__ wt,
    lk_w w2, const float* __restrict__ bias2, signed char* __restrict__ hq,
    signed char* __restrict__ hl, float* __restrict__ hs, float limit, float slope,
    int tail, int split, float* __restrict__ part, int* __restrict__ cnt, lk_parts parts){
  constexpr bool PAIR = G == 3 || G == 5, GROUPED = G >= 1 && G <= 3, DENSE = !GROUPED;
  constexpr int TW = PAIR ? LK_BN / 2 : LK_BN;
  constexpr int MT = WM / 16, NT = BM / WM * 128, AW = Q == 2 ? 2 : 1;
  constexpr int A_BYTES = BM * 32, STAGE = lk_q8_stage<Q, BM>(), GROUP = lk_q8_group<BM>(), RING = LK_STREAM_STAGES * STAGE;
  // The grid's blocks in launch order take the row tiles of a column tile one after another, so the blocks a column
  // tile's weights serve run together and read them once from memory.
  const int lin = blockIdx.x + blockIdx.y * gridDim.x;
  int m0 = (lin % gridDim.y) * BM, n0 = (lin / gridDim.y) * TW;
  // On a line of blocks: this block's tile, and the scale groups of its part of the tile (all of them for a whole tile).
  bool parted = DENSE && part && (int)blockIdx.x >= tail;
  if(DENSE && part){
    // The whole tiles are the first `tail` of the tiles row by row, and the cut ones the rest, as the block path cuts
    // them; the line takes the whole ones a column tile at a time — every column tile `rows` of them, the first `wide`
    // one more — so the blocks a column tile's weights serve run together.
    int b = blockIdx.x, nbx = (N + TW - 1) / TW, tile;
    if(b < tail){
      int rows = tail / nbx, wide = tail % nbx, c, r;
      if(b < (rows + 1) * wide){ c = b / (rows + 1); r = b % (rows + 1); }
      else { int e = b - (rows + 1) * wide; c = wide + e / rows; r = e % rows; }
      tile = r * nbx + c;
    } else tile = tail + (b - tail) / split;
    m0 = (tile / nbx) * BM; n0 = (tile % nbx) * TW;
  }
  if(G == 4) lk_take_part(parts, n0, w, y, bias, yrow, ypitch, N);
  long first = 0, wbase = 0;
  if(GROUPED){
    M = ecount[blockIdx.z];
    if(m0 >= M) return;
    first = eoff[blockIdx.z];
    wbase = (long)blockIdx.z * N;
    xq += first * K;
    if(Q == 2) xl += first * K;
    xs += first * (K >> 5);
  }
  const int tid = threadIdx.x, lane = tid & 31, warp = tid >> 5;
  const int nb = K >> 5;
  int g_lo = 0, g_hi = nb >> 2;
  if(parted){
    int piece = ((int)blockIdx.x - tail) % split, groups = nb >> 2;
    g_lo = piece * groups / split; g_hi = (piece + 1) * groups / split;
  }
  const int kb_lo = 4 * g_lo, n = 4 * (g_hi - g_lo);
  const unsigned base = (unsigned)__cvta_generic_to_shared(lk_sm);
  const unsigned full = base + RING + 2 * GROUP, empty = full + 8 * LK_STREAM_STAGES;
  if(tid == 0) for(int s = 0; s < LK_STREAM_STAGES; ++s){ lk_mbar_init(full + 8 * s, NT); lk_mbar_init(empty + 8 * s, NT / 32); }
  __syncthreads();
  // Each thread's copies, the same every stage: activation chunks, matrix chunks, and a row's scale group.
  constexpr int A_IT = (BM * 2 + NT - 1) / NT;
  const signed char* a_src[A_IT]; int a_dst[A_IT]; bool a_ok[A_IT], a_on[A_IT];
  #pragma unroll
  for(int it = 0; it < A_IT; ++it){
    int c = tid + it * NT, r = c >> 1, ch = c & 1;
    a_on[it] = c < BM * 2;
    a_ok[it] = a_on[it] && m0 + r < M;
    a_src[it] = xq + (long)(a_ok[it] ? m0 + r : 0) * K + ch * 16;
    a_dst[it] = lk_q8_swz(r, ch);
  }
  const long a_lo = Q == 2 ? (long)(xl - xq) : 0;
  // Two threads a matrix row: 256 copy the tile's rows, any beyond them none.
  const int w_r = tid >> 1, w_ch = tid & 1, w_row = lk_q8_row<PAIR>(w_r);
  const bool w_on = tid < 2 * LK_BN, w_up = PAIR && (w_r & 31) >= 16, w_ok = w_on && n0 + w_row < N;
  const signed char* w_src = (w_up ? w2.q : w.q) + (w_ok ? wbase + n0 + w_row : 0) * (long)K + w_ch * 16;
  const int w_dst = AW * A_BYTES + lk_q8_swz(w_r, w_ch);
  // Scale groups: threads below `BM` copy their activation row's four scales (16 bytes), the next 128 their matrix row's
  // four (8 bytes).
  const bool sa_on = tid < BM, s_on = tid < BM + LK_BN;
  const int s_r = sa_on ? tid : tid - BM, s_row = sa_on ? s_r : lk_q8_row<PAIR>(s_r);
  const bool s_up = PAIR && !sa_on && (s_r & 31) >= 16;
  const bool s_ok = s_on && (sa_on ? m0 + s_r < M : n0 + s_row < N);
  const char* s_src = sa_on ? (const char*)(xs + (long)(s_ok ? m0 + s_r : 0) * nb)
                            : (const char*)((s_up ? w2.s : w.s) + (s_ok ? wbase + n0 + s_row : 0) * (long)nb);
  const int s_step = sa_on ? 4 : 2, s_dst = sa_on ? s_r * 16 : BM * 16 + s_r * 8;
  // Stage `i` into buffer `b` (`i`'s place in the ring), and with it scale group `i / 4` when `i` starts one.
  auto load = [&](int i, int b, bool group){
    const int kb = kb_lo + i;
    const unsigned st = base + b * STAGE;
    #pragma unroll
    for(int it = 0; it < A_IT; ++it){
      if(!a_on[it]) continue;
      lk_cp16s(st + a_dst[it], a_src[it] + kb * 32, a_ok[it]);
      if(Q == 2) lk_cp16s(st + A_BYTES + a_dst[it], a_src[it] + a_lo + kb * 32, a_ok[it]);
    }
    if(w_on) lk_cp16s(st + w_dst, w_src + kb * 32, w_ok);
    if(group && s_on){
      const unsigned sg = base + RING + ((kb >> 2) & 1) * GROUP + s_dst;
      if(sa_on) lk_cp16s(sg, s_src + kb * s_step, s_ok);
      else lk_cp8s(sg, s_src + kb * s_step, s_ok);
    }
    lk_mbar_copies(full + 8 * b);
  };
  const int wm = (warp >> 2) * WM, wn = (warp & 3) * 32;
  const int g = lane >> 2, t4 = lane & 3;
  float acc[MT][4][4];
  #pragma unroll
  for(int i = 0; i < MT; ++i)
    #pragma unroll
    for(int j = 0; j < 4; ++j)
      #pragma unroll
      for(int e = 0; e < 4; ++e) acc[i][j][e] = 0.f;
  // This lane's `ldmatrix` rows: the activation's 16 by its chunk, and the matrix's for two n-tiles at a time.
  const int a_row = wm + (lane & 15), a_ch = lane >> 4;
  const int b_row = wn + (lane >> 4) * 8 + (lane & 7), b_ch = (lane >> 3) & 1;
  const bool live = m0 + wm < M;
  // The stages run four at a time, a scale group each, so every stage's buffer, barrier phase and scale stage is fixed.
  if(n > 0){
    #pragma unroll
    for(int i = 0; i < LK_STREAM_STAGES - 1; ++i) load(i, i, i == 0);
  }
  for(int q = 0; q < n / 4; ++q){
    const unsigned char* ring = lk_sm + RING + ((g_lo + q) & 1) * GROUP;
    const float* sa = (const float*)ring;
    const __half* sw = (const __half*)(ring + BM * 16);
    #pragma unroll
    for(int j = 0; j < 4; ++j){
      const int i = 4 * q + j;
      lk_mbar_wait(full + 8 * j, q & 1);
      const unsigned st = base + j * STAGE;
      // A warp whose rows all lie past the tile's last row leaves the multiply to the others.
      if(live){
        int bf[4][2];
        #pragma unroll
        for(int p = 0; p < 2; ++p){
          int r4[4];
          lk_ldsm_s(r4, st + AW * A_BYTES + lk_q8_swz(b_row + p * 16, b_ch));
          bf[2 * p][0] = r4[0]; bf[2 * p][1] = r4[1]; bf[2 * p + 1][0] = r4[2]; bf[2 * p + 1][1] = r4[3];
        }
        float dcol[4][2];
        #pragma unroll
        for(int nt = 0; nt < 4; ++nt)
          #pragma unroll
          for(int cc = 0; cc < 2; ++cc) dcol[nt][cc] = __half2float(sw[(wn + nt * 8 + t4 * 2 + cc) * 4 + j]);
        #pragma unroll
        for(int mt = 0; mt < MT; ++mt){
          int af[4], al[4];
          lk_ldsm_s(af, st + lk_q8_swz(a_row + mt * 16, a_ch));
          if(Q == 2) lk_ldsm_s(al, st + A_BYTES + lk_q8_swz(a_row + mt * 16, a_ch));
          const float sa0 = sa[(wm + mt * 16 + g) * 4 + j], sa1 = sa[(wm + mt * 16 + g + 8) * 4 + j];
          #pragma unroll
          for(int nt = 0; nt < 4; ++nt){
            int dot[4], dl[4];
            lk_mma_s8_from(dot, af, bf[nt], lk_dot0<true>());
            if(Q == 2) lk_mma_s8_from(dl, al, bf[nt], lk_dot0<true>());
            #pragma unroll
            for(int e = 0; e < 4; ++e){
              const float s0 = e < 2 ? sa0 : sa1;
              acc[mt][nt][e] = Q == 2 ? fmaf(s0 * 0.0078125f * dcol[nt][e & 1], fmaf(lk_dot_f<true>(dot[e]), 128.f, lk_dot_f<true>(dl[e])), acc[mt][nt][e])
                                      : fmaf(s0 * dcol[nt][e & 1], lk_dot_f<true>(dot[e]), acc[mt][nt][e]);
            }
          }
        }
      }
      __syncwarp();
      if(lane == 0) lk_mbar_arrive(empty + 8 * j);
      // The buffer the stage before this one read takes the stage three ahead, once every warp is past it; the stage
      // three ahead starts a scale group when this one is a group's second.
      if(i + LK_STREAM_STAGES - 1 < n){
        if(i >= 1) lk_mbar_wait(empty + 8 * ((j + 3) & 3), j == 0 ? (q - 1) & 1 : q & 1);
        load(i + LK_STREAM_STAGES - 1, (j + 3) & 3, j == 1);
      }
    }
  }
  asm volatile("cp.async.wait_all;\n" ::: "memory");
  __syncthreads();
  if(parted){
    // A part parks its sums; the tile's last part to arrive sums every part in order, its own read back too.
    __shared__ bool lk_last;
    int tile = tail + ((int)blockIdx.x - tail) / split;
    float4* mine = (float4*)(part + ((long)(blockIdx.x - tail) * NT + tid) * (MT * 16));
    #pragma unroll
    for(int mt = 0; mt < MT; ++mt)
      #pragma unroll
      for(int nt = 0; nt < 4; ++nt) mine[mt * 4 + nt] = make_float4(acc[mt][nt][0], acc[mt][nt][1], acc[mt][nt][2], acc[mt][nt][3]);
    __threadfence();
    __syncthreads();
    if(tid == 0) lk_last = atomicAdd(cnt + (tile - tail), 1) == split - 1;
    __syncthreads();
    if(!lk_last) return;
    #pragma unroll
    for(int mt = 0; mt < MT; ++mt)
      #pragma unroll
      for(int nt = 0; nt < 4; ++nt)
        #pragma unroll
        for(int e = 0; e < 4; ++e) acc[mt][nt][e] = 0.f;
    for(int pc = 0; pc < split; ++pc){
      const float4* src = (const float4*)(part + ((long)((tile - tail) * split + pc) * NT + tid) * (MT * 16));
      #pragma unroll
      for(int mt = 0; mt < MT; ++mt)
        #pragma unroll
        for(int nt = 0; nt < 4; ++nt){
          float4 v = __ldcg(src + mt * 4 + nt);
          acc[mt][nt][0] += v.x; acc[mt][nt][1] += v.y; acc[mt][nt][2] += v.z; acc[mt][nt][3] += v.w;
        }
    }
    if(tid == 0) cnt[tile - tail] = 0;
  }
  if(PAIR){
    // Each hidden value through the gated activation from its gate and up sums (plus their biases) in this thread,
    // staged by row, then quantized a 32-value block a warp at a time.
    float* H = (float*)lk_sm;
    #pragma unroll
    for(int mt = 0; mt < MT; ++mt)
      #pragma unroll
      for(int nt = 0; nt < 2; ++nt)
        #pragma unroll
        for(int e = 0; e < 4; ++e){
          int r = wm + mt * 16 + g + (e >= 2 ? 8 : 0), h = (wn >> 1) + nt * 8 + t4 * 2 + (e & 1), col = n0 + h;
          float gv = acc[mt][nt][e], uv = acc[mt][nt + 2][e];
          if(bias.b && col < N) gv = gv + bias.b[wbase + col];
          if(bias2 && col < N) uv = uv + bias2[wbase + col];
          float hv;
          if(G == 5) hv = ACT < 0 ? gv / (1.0f + expf(-gv)) * uv : uv * lk_act<(ACT < 0 ? 0 : ACT)>(gv);
          else if(ACT == 0) hv = gv / (1.0f + expf(-gv)) * uv;
          else { float gc = fminf(gv, limit); hv = gc / (1.0f + expf(-slope * gc)) * (fminf(fmaxf(uv, -limit), limit) + 1.0f); }
          H[r * TW + h] = hv;
        }
    __syncthreads();
    for(int r = warp; r < BM; r += NT / 32){
      if(m0 + r >= M) break;
      #pragma unroll
      for(int j = 0; j < TW / 32; ++j){
        int c = 32 * j + lane;
        if(n0 + 32 * j >= N) break;
        float hv = H[r * TW + c];
        long i = (first + m0 + r) * N + n0 + c;
        if(G == 5 && y) y[i] = hv;
        if(G == 3 || hq) lk_quant_lane<Q>(hv, hq, hl, hs, i, i >> 5);
      }
    }
    return;
  }
  // An expert mixture's outputs are its `f32` scratch, stored as floats; any other product's through its array's type.
  lk_out yo = GROUPED ? lk_out() : LK_ROW(y, yrow, ypitch);
  float* yg = GROUPED ? (float*)y.p + first * N : nullptr;
  if constexpr(PLAIN && !GROUPED){
    // Every output its sum as it is, into an `f32` array (`PLAIN`): no epilogue step, no conversion.
    float* const yf = (float*)yo.p;
    #pragma unroll
    for(int mt = 0; mt < MT; ++mt)
      #pragma unroll
      for(int nt = 0; nt < 4; ++nt)
        #pragma unroll
        for(int e = 0; e < 4; ++e){
          const int row = m0 + wm + mt * 16 + g + (e >= 2 ? 8 : 0), col = n0 + wn + nt * 8 + t4 * 2 + (e & 1);
          if(row < M && col < N) yf[(long)row * N + col] = acc[mt][nt][e];
        }
    return;
  }
  #pragma unroll
  for(int mt = 0; mt < MT; ++mt)
    #pragma unroll
    for(int nt = 0; nt < 4; ++nt)
      #pragma unroll
      for(int e = 0; e < 4; ++e){
        int row = m0 + wm + mt * 16 + g + (e >= 2 ? 8 : 0);
        int col = n0 + wn + nt * 8 + t4 * 2 + (e & 1);
        if(row >= M || col >= N) continue;
        float v = bias(acc[mt][nt][e], row, wbase + col, N);
        // G 2 weighs each choice's result by its weight, as the expert mixture's down projection does.
        if constexpr(GROUPED) yg[(long)row * N + col] = G == 2 ? v * wt[list[first + row]] : v;
        else yo[(long)row * N + col] = v;
      }
}

// The blocks of a block-format GEMM a multiprocessor holds at once: two of a 64-row tile — a Q8_0 matrix's streamed path
// under either numerics, any other format's under one word, whose two blocks' stages fit the multiprocessor's shared
// memory — but one of Q2_K, Q3_K and Q5_K, whose halves' scales and offsets beside the multiply need more registers
// than two blocks leave a thread; one of any wider tile.
template<int F, int Q, int BM> __host__ __device__ constexpr int lk_gemm_blk_blocks(){
  return BM <= 64 && (F == 8 || Q != 2) && F != 10 && F != 11 && F != 13 ? 2 : 1;
}
// Whether a block's stages take two decoded weight tiles — the next stage's decoded while this one multiplies — beside
// its two activation buffers: where they fit a block's 99 KB and the block holds its multiprocessor alone (two blocks'
// stages, the second tile each, would not fit the multiprocessor's).
template<int F, int Q, int BM> __host__ __device__ constexpr bool lk_stage_over(){
  return lk_stage_pipelined<F, Q, BM>() && lk_gemm_blk_blocks<F, Q, BM>() == 1
      && 2 * lk_stage_acts<Q, BM>() + 2 * lk_stage_wt<F>() + lk_stage_sums<F, Q, BM>() <= 101376;
}
template<int F, int Q, int G = 0, int BM = (G >= 1 && G <= 3 ? 64 : 128), int ACT = 0, bool PLAIN = false>
__global__ void __launch_bounds__(BM * 4, lk_gemm_blk_blocks<F, Q, BM>()) lk_gemm_blk(lk_w w, const signed char* __restrict__ xq,
    const signed char* __restrict__ xl, const float* __restrict__ xs, lk_epi bias, lk_out y,
    const int* __restrict__ yrow, long ypitch, int K, int N, int M, const int* __restrict__ ecount = nullptr,
    const int* __restrict__ eoff = nullptr, const int* __restrict__ list = nullptr, const float* __restrict__ wt = nullptr,
    lk_w w2 = {}, const float* __restrict__ bias2 = nullptr, signed char* __restrict__ hq = nullptr,
    signed char* __restrict__ hl = nullptr, float* __restrict__ hs = nullptr, float limit = 0.f, float slope = 0.f,
    int tail = 0, int split = 1, float* __restrict__ part = nullptr, int* __restrict__ cnt = nullptr, lk_parts parts = {}){
  extern __shared__ __align__(128) unsigned char lk_sm[];
  if constexpr(F == 8){
    if((K & 127) == 0){
      // Launched as eight warps over 128 rows, each warp takes 64 of them; otherwise 32.
      if constexpr(BM == 128){
        if(blockDim.x == 256){
          lk_blk_streamed<Q, G, BM, ACT, 64, PLAIN>(lk_sm, w, xq, xl, xs, bias, y, yrow, ypitch, K, N, M, ecount, eoff, list, wt, w2, bias2,
                                             hq, hl, hs, limit, slope, tail, split, part, cnt, parts);
          return;
        }
      }
      lk_blk_streamed<Q, G, BM, ACT, 32, PLAIN>(lk_sm, w, xq, xl, xs, bias, y, yrow, ypitch, K, N, M, ecount, eoff, list, wt, w2, bias2,
                                         hq, hl, hs, limit, slope, tail, split, part, cnt, parts);
      return;
    }
  }
  constexpr bool PAIR = G == 3 || G == 5, GROUPED = G >= 1 && G <= 3, DENSE = !GROUPED;
  constexpr int TW = PAIR ? LK_BN / 2 : LK_BN;
  constexpr int SB = 4, BK = 32 * SB, BP = BK + 16, AW = Q == 2 ? 2 : 1;
  constexpr int DH = lk_halves_apart<F>() ? 2 : 1, CH = lk_has_offset<F>() ? DH : 0;
  constexpr int A_BYTES = BM * BP;
  int m0 = blockIdx.y * BM, n0 = blockIdx.x * (PAIR ? LK_BN / 2 : LK_BN);
  // On a line of blocks: this block's tile, and the stages of its part of the tile (all of them for a whole tile).
  bool parted = DENSE && part && (int)blockIdx.x >= tail;
  if(DENSE && part){
    int b = blockIdx.x, nbx = (N + TW - 1) / TW, tile = b < tail ? b : tail + (b - tail) / split;
    m0 = (tile / nbx) * BM; n0 = (tile % nbx) * TW;
  }
  // The part whose columns hold the tile: its matrix, outputs and bias, the tile's columns its own.
  if(G == 4) lk_take_part(parts, n0, w, y, bias, yrow, ypitch, N);
  long first = 0, wbase = 0;
  if(GROUPED){
    // Grid layer `z` is an expert: its rows the gathered rows of its choices, its matrix rows `z·N …` of the stack.
    M = ecount[blockIdx.z];
    if(m0 >= M) return;
    first = eoff[blockIdx.z];
    wbase = (long)blockIdx.z * N;
    xq += first * K;
    if(Q == 2) xl += first * K;
    xs += first * (K >> 5);
  }
  int tid = threadIdx.x, lane = tid & 31, warp = tid >> 5;
  int nb = K >> 5, stages = (K + BK - 1) / BK;
  int s_lo = 0, s_hi = stages;
  if(parted){
    int piece = ((int)blockIdx.x - tail) % split;
    s_lo = piece * stages / split; s_hi = (piece + 1) * stages / split;
  }
  // Each warp multiplies 32 × 32 of the tile.
  int wm = (warp >> 2) * 32, wn = (warp & 3) * 32;
  int g = lane >> 2, t4 = lane & 3;
  float acc[2][4][4];
  #pragma unroll
  for(int i = 0; i < 2; ++i)
    #pragma unroll
    for(int j = 0; j < 4; ++j)
      #pragma unroll
      for(int e = 0; e < 4; ++e) acc[i][j][e] = 0.f;
  constexpr bool PIPE = lk_stage_pipelined<F, Q, BM>(), OVER = lk_stage_over<F, Q, BM>();
  constexpr int ACTS = lk_stage_acts<Q, BM>(), WB = lk_stage_wt<F>();
  // The decoded weight tiles after the activation buffers (tile `b` at `W0 + b·WB`, its scales and offsets after its
  // levels), then the activation's sums.
  unsigned char* const W0 = lk_sm + (PIPE ? 2 : 1) * ACTS;
  float* Xf = (float*)(W0 + (OVER ? 2 : 1) * WB);
  // Stage `st`'s activation rows into buffer `b` by `cp.async`, and each row's scales: one 16-byte copy when the
  // stage is whole and the row's scales fall on 16 bytes.
  // Where the stage copies while it multiplies (`OVER`), each thread's 16-byte pieces of a stage's activation rows, the
  // same in every stage — piece `tid + i·BM·4` of the stage's `SB · 2` a row: row `xr + i·XR`, from byte `16·xh` of the
  // stage's `k` — and the first `BM` threads' rows' scales, a thread a row.
  constexpr int XP = SB * 2 / 4, XR = BM * 4 / (SB * 2);
  const int xr = tid / (SB * 2), xh = tid % (SB * 2);
  const long xoff = (long)(m0 + xr) * K + xh * 16;
  auto acts = [&](int st, int b){
    int k0 = st * BK;
    unsigned char* As = lk_sm + b * ACTS;
    float* Sa = (float*)(As + AW * A_BYTES);
    if constexpr(OVER){
      bool kin = k0 + xh * 16 < K;
      for(int i = 0; i < XP; ++i){
        bool ok = m0 + xr + i * XR < M && kin;
        long off = ok ? xoff + (long)i * XR * K + k0 : 0;
        lk_cp16(As + (xr + i * XR) * BP + xh * 16, xq + off, ok);
        if(Q == 2) lk_cp16(As + A_BYTES + (xr + i * XR) * BP + xh * 16, xl + off, ok);
      }
      if(tid < BM){
        long base = (long)(m0 + tid) * nb + (k0 >> 5);
        if((nb & 3) == 0 && m0 + tid < M && (k0 >> 5) + SB <= nb) lk_cp16(Sa + tid * SB, xs + base, true);
        else for(int j = 0; j < SB; ++j) Sa[tid * SB + j] = m0 + tid < M && (k0 >> 5) + j < nb ? xs[base + j] : 0.f;
      }
    } else {
      for(int c = tid; c < BM * SB * 2; c += (BM * 4)){
        // A stage that copies before it multiplies takes its pieces a row at a time: the pieces' rows and offsets held
        // through the loop cost a two-block expert GEMM its last registers.
        int r = c / (SB * 2), h = c % (SB * 2);
        bool ok = m0 + r < M && k0 + h * 16 < K;
        long off = ok ? (long)(m0 + r) * K + k0 + h * 16 : 0;
        lk_cp16(As + r * BP + h * 16, xq + off, ok);
        if(Q == 2) lk_cp16(As + A_BYTES + r * BP + h * 16, xl + off, ok);
        if(h == 0){
          long base = (long)(m0 + r) * nb + (k0 >> 5);
          if((nb & 3) == 0 && m0 + r < M && (k0 >> 5) + SB <= nb) lk_cp16(Sa + r * SB, xs + base, true);
          else for(int j = 0; j < SB; ++j) Sa[r * SB + j] = m0 + r < M && (k0 >> 5) + j < nb ? xs[base + j] : 0.f;
        }
      }
    }
    asm volatile("cp.async.commit_group;\n");
  };
  // Each thread's sub-blocks of a stage, their raw words fetched a stage ahead: a Q6_K pair's once for both
  // (`lk_stage_raw`), where a stage gives each thread two.
  constexpr int U = LK_BN * SB / (BM * 4);
  lk_stage_raw<F, U> raw[lk_stage_raws<F, U>()];
  lk_stage_fetch<F, SB, BM * 4, U, PAIR>(raw, w, w2, wbase, n0, N, K, nb, s_lo * BK, tid);
  // The first stage lands in the buffer its parity names, so each stage's buffer is `st & 1` wherever the part starts.
  acts(s_lo, PIPE ? s_lo & 1 : 0);
  if constexpr(OVER){
    // The first stage's weights decoded ahead of the loop, and the second's fetched.
    unsigned char* Bs = W0 + (s_lo & 1) * WB;
    lk_stage_weights<F, SB, BP, DH, CH, BM * 4, U, PAIR>(raw, n0, N, nb, s_lo * BK, Bs, (float*)(Bs + LK_BN * BP), tid);
    if(s_lo + 1 < s_hi) lk_stage_fetch<F, SB, BM * 4, U, PAIR>(raw, w, w2, wbase, n0, N, K, nb, (s_lo + 1) * BK, tid);
  }
  // A block-format matrix's stages lean (`LEAN`): each thread's sums-phase offsets and each lane's fragment and scale
  // offsets taken once a launch, and a dense tile of a format without offsets multiplying its edges as the zeros they are
  // staged as. A Q8_0 matrix's block path (a `k` of no whole 128-value groups) shares its kernel with the streamed path,
  // whose registers move with any change here, and keeps the offsets where they are used. The block formats read their
  // sums by the conversion under one word (`RD`); under two, whose sub-blocks convert two dots an output, by the
  // subtraction, which measures faster there.
  constexpr bool LEAN = F != 8, RD = LEAN && Q == 1;
  // A thread's activation sums a stage (`SUMS`), sum `it` at fixed offsets from its first: piece `tid + it·BM·4` of the
  // stage's `AW·BM·SB·DH`, word `it / DH`'s rows `(it % DH)·BM / DH` on.
  constexpr int SUMS = lk_has_offset<F>() ? AW * DH : 0;
  const int sum_r = tid / (SB * DH), sum_h = tid % (SB * DH), sum_b = sum_h / DH;
  const int sum_a = sum_r * BP + sum_b * 32 + (DH == 2 ? (sum_h & 1) * 16 : 0), sum_s = sum_r * SB + sum_b;
  const int sum_x = sum_r * SB * DH + sum_h;
  // This lane's offsets, the same every stage: its fragments' rows in the activation and weight tiles, its rows' and
  // columns' scales.
  const unsigned sm0 = (unsigned)__cvta_generic_to_shared(lk_sm);
  const int a_t = (wm + (lane & 7) + ((lane >> 3) & 1) * 8) * BP + (lane >> 4) * 16;
  const int b_t = (int)(W0 - lk_sm) + (wn + (lane >> 4) * 8 + (lane & 7)) * BP + ((lane >> 3) & 1) * 16;
  const int r_t = wm + g, c_t = wn + t4 * 2;
  const bool rows_live = m0 + wm < M;
  for(int st = s_lo; st < s_hi; ++st){
    int k0 = st * BK, b = PIPE ? st & 1 : 0;
    const unsigned char* As = lk_sm + b * ACTS;
    const float* Sa = (const float*)(As + AW * A_BYTES);
    const unsigned char* Bs = W0 + (OVER ? b : 0) * WB;
    const float* Bd = (const float*)(Bs + LK_BN * BP);
    if constexpr(!OVER){
      lk_stage_weights<F, SB, BP, DH, CH, BM * 4, U, PAIR>(raw, n0, N, nb, k0, W0, (float*)(W0 + LK_BN * BP), tid);
      // The next stage's weights are fetched while this one multiplies.
      if(st + 1 < s_hi) lk_stage_fetch<F, SB, BM * 4, U, PAIR>(raw, w, w2, wbase, n0, N, K, nb, k0 + BK, tid);
    }
    asm volatile("cp.async.wait_group 0;\n");
    __syncthreads();
    // The activation's sums over each sub-block (each half, where the halves scale apart), for the offsets, each
    // scaled by its row's scale (the low words' at a 128th) as the offsets take it.
    #pragma unroll
    for(int it = 0; it < SUMS; ++it){
      const int u = it / DH, rd = (it % DH) * (BM / DH);
      const int* a = (const int*)(As + sum_a + u * A_BYTES + rd * BP);
      int sum = 0;
      #pragma unroll
      for(int i = 0; i < 8 / DH; ++i) sum = __dp4a(0x01010101, a[i], sum);
      float s0 = Sa[sum_s + rd * SB];
      Xf[sum_x + (u * BM + rd) * SB * DH] = (u ? s0 * 0.0078125f : s0) * (float)sum;
    }
    __syncthreads();
    // The next stage's activations land in the other buffer while this one multiplies.
    if(!OVER && PIPE && st + 1 < s_hi) acts(st + 1, b ^ 1);
    // Sub-block `sb` of the stage multiplied. A warp whose 32 rows all lie past the tile's last row (an expert with few
    // rows) leaves the multiply to the others, and a sub-block past `K` is skipped. A lean dense tile of a format without
    // offsets skips neither: it multiplies its rows past `M` and sub-blocks past `K` as the zeros they are staged as,
    // which leave each sum as it was (no sum is ever −0), and with no branch between its sub-blocks the scheduler runs
    // one's loads under another's float work.
    auto mul = [&](int sb){
      if(!LEAN ? m0 + wm >= M || (k0 >> 5) + sb >= nb : GROUPED || lk_has_offset<F>() ? !rows_live || (sb > 0 && (k0 >> 5) + sb >= nb) : false) return;
      int ko = sb * 32 + t4 * 4;
      int af[2][4], al[2][4], bf[4][2];
      // This lane's columns' scales and offsets (two columns an n-tile, a value a half where the halves scale apart),
      // and its rows' scales and their sums scaled by them (the low words' at a 128th), read once a sub-block.
      constexpr int OH = CH ? CH : 1;
      float dcol[4][2][DH], ocol[4][2][OH], sa[2][2], sx[2][2][DH], sxl[2][2][DH];
      #pragma unroll
      for(int nt = 0; nt < 4; ++nt){
        constexpr int E = DH + CH;
        const float* v = Bd + (sb * LK_BN + (LEAN ? c_t : wn + t4 * 2) + nt * 8) * E;
        float e[2 * E];
        if constexpr(E == 1){ float2 p = *(const float2*)v; e[0] = p.x; e[1] = p.y; }
        else {
          #pragma unroll
          for(int i = 0; i < 2 * E; i += 4){ float4 p = *(const float4*)(v + i); e[i] = p.x; e[i + 1] = p.y; e[i + 2] = p.z; e[i + 3] = p.w; }
        }
        #pragma unroll
        for(int cc = 0; cc < 2; ++cc)
          #pragma unroll
          for(int h = 0; h < DH; ++h){
            dcol[nt][cc][h] = e[cc * E + h];
            if(CH) ocol[nt][cc][h] = e[cc * E + DH + h];
          }
      }
      #pragma unroll
      for(int mt = 0; mt < 2; ++mt)
        #pragma unroll
        for(int hh = 0; hh < 2; ++hh){
          int r = (LEAN ? r_t : wm + g) + mt * 16 + hh * 8;
          float s0 = Sa[r * SB + sb];
          sa[mt][hh] = s0;
          if(lk_has_offset<F>())
            #pragma unroll
            for(int h = 0; h < DH; ++h){
              sx[mt][hh][h] = Xf[(r * SB + sb) * DH + h];
              if(Q == 2) sxl[mt][hh][h] = Xf[((BM + r) * SB + sb) * DH + h];
            }
        }
      #pragma unroll
      for(int mt = 0; mt < 2; ++mt){
        // Matrix i of the x4 is rows 8·(i & 1) on, bytes 16·(i >> 1) on: this lane's a[i] as the fragment takes it. A
        // format whose halves scale apart reads its rows a word at a time (on the 3060 the x4 cost its GEMM 11%).
        if constexpr(lk_halves_apart<F>()){
          int r = wm + mt * 16 + g;
          af[mt][0] = *(const int*)(As + r * BP + ko);
          af[mt][1] = *(const int*)(As + (r + 8) * BP + ko);
          af[mt][2] = *(const int*)(As + r * BP + ko + 16);
          af[mt][3] = *(const int*)(As + (r + 8) * BP + ko + 16);
          if(Q == 2){
            al[mt][0] = *(const int*)(As + A_BYTES + r * BP + ko);
            al[mt][1] = *(const int*)(As + A_BYTES + (r + 8) * BP + ko);
            al[mt][2] = *(const int*)(As + A_BYTES + r * BP + ko + 16);
            al[mt][3] = *(const int*)(As + A_BYTES + (r + 8) * BP + ko + 16);
          }
        } else {
          unsigned at = LEAN ? sm0 + b * ACTS + a_t + mt * 16 * BP + sb * 32
                             : (unsigned)__cvta_generic_to_shared(As + (wm + mt * 16 + (lane & 7) + ((lane >> 3) & 1) * 8) * BP + sb * 32 + (lane >> 4) * 16);
          lk_ldsm_s(af[mt], at);
          if(Q == 2) lk_ldsm_s(al[mt], at + A_BYTES);
        }
      }
      #pragma unroll
      for(int np = 0; np < 2; ++np){
        // Matrix i of the x4 is n-tile 2·np + (i >> 1)'s columns, bytes 16·(i & 1) on.
        int q[4];
        lk_ldsm_s(q, LEAN ? sm0 + (OVER ? b : 0) * WB + b_t + np * 16 * BP + sb * 32
                          : (unsigned)__cvta_generic_to_shared(Bs + (wn + (2 * np + (lane >> 4)) * 8 + (lane & 7)) * BP + sb * 32 + ((lane >> 3) & 1) * 16));
        bf[2 * np][0] = q[0]; bf[2 * np][1] = q[1]; bf[2 * np + 1][0] = q[2]; bf[2 * np + 1][1] = q[3];
      }
      #pragma unroll
      for(int mt = 0; mt < 2; ++mt)
        #pragma unroll
        for(int nt = 0; nt < 4; ++nt){
          // The accumulator's elements: rows g, g + 8 of the m-tile by columns 2·t4, 2·t4 + 1 of the n-tile.
          int dot[2][4], dl[2][4];
          if(lk_halves_apart<F>()){
            int a0[2] = {af[mt][0], af[mt][1]}, a1[2] = {af[mt][2], af[mt][3]};
            // Integer-scaled halves' dots start at zero, weighed by their scales as integers below.
            constexpr int Z = lk_int_halves<F>() ? 0 : lk_dot0<RD>();
            lk_mma_s8_k16_from(dot[0], a0, bf[nt][0], Z);
            lk_mma_s8_k16_from(dot[1], a1, bf[nt][1], Z);
            if(Q == 2){
              int l0[2] = {al[mt][0], al[mt][1]}, l1[2] = {al[mt][2], al[mt][3]};
              lk_mma_s8_k16_from(dl[0], l0, bf[nt][0], Z);
              lk_mma_s8_k16_from(dl[1], l1, bf[nt][1], Z);
            }
          } else {
            lk_mma_s8_from(dot[0], af[mt], bf[nt], lk_dot0<RD>());
            if(Q == 2) lk_mma_s8_from(dl[0], al[mt], bf[nt], lk_dot0<RD>());
          }
          #pragma unroll
          for(int e = 0; e < 4; ++e){
            int hh = e >> 1, cc = e & 1;
            float s0 = sa[mt][hh], a = acc[mt][nt][e];
            if constexpr(F == 8 && Q == 2){
              acc[mt][nt][e] = fmaf(s0 * 0.0078125f * dcol[nt][cc][0], fmaf(lk_dot_f<RD>(dot[0][e]), 128.f, lk_dot_f<RD>(dl[0][e])), a);
              continue;
            }
            if constexpr(lk_int_halves<F>()){
              // The halves' dots weighed by their integer scales, exactly (|sum| < 2²⁴), then by the row's and the
              // block's scales; the low words' alike at a 128th.
              const int sp = __float_as_int(dcol[nt][cc][1]), sc0 = (signed char)(sp & 0xFF), sc1 = (signed char)(sp >> 8);
              const float sd = s0 * dcol[nt][cc][0];
              a = fmaf(sd, (float)(dot[0][e] * sc0 + dot[1][e] * sc1), a);
              if(Q == 2) a = fmaf(sd * 0.0078125f, (float)(dl[0][e] * sc0 + dl[1][e] * sc1), a);
              acc[mt][nt][e] = a;
              continue;
            }
            #pragma unroll
            for(int h = 0; h < DH; ++h) a = fmaf(s0 * dcol[nt][cc][h], lk_dot_f<RD>(dot[h][e]), a);
            if(lk_has_offset<F>())
              #pragma unroll
              for(int h = 0; h < DH; ++h) a = fmaf(ocol[nt][cc][h], sx[mt][hh][h], a);
            if(Q == 2){
              float sl = s0 * 0.0078125f;
              #pragma unroll
              for(int h = 0; h < DH; ++h) a = fmaf(sl * dcol[nt][cc][h], lk_dot_f<RD>(dl[h][e]), a);
              if(lk_has_offset<F>())
                #pragma unroll
                for(int h = 0; h < DH; ++h) a = fmaf(ocol[nt][cc][h], sxl[mt][hh][h], a);
            }
            acc[mt][nt][e] = a;
          }
        }
    };
    if constexpr(OVER){
      // The next stage's activations copied after the first sub-block and its weights decoded into the other tile after
      // the second, the stage after's weights fetched: the next stage's barrier frees this one's buffers.
      mul(0);
      if(st + 1 < s_hi) acts(st + 1, b ^ 1);
      mul(1);
      if(st + 1 < s_hi){
        unsigned char* Bn = W0 + (b ^ 1) * WB;
        lk_stage_weights<F, SB, BP, DH, CH, BM * 4, U, PAIR>(raw, n0, N, nb, k0 + BK, Bn, (float*)(Bn + LK_BN * BP), tid);
        if(st + 2 < s_hi) lk_stage_fetch<F, SB, BM * 4, U, PAIR>(raw, w, w2, wbase, n0, N, K, nb, k0 + 2 * BK, tid);
      }
      mul(2);
      mul(3);
    } else {
      #pragma unroll 1
      for(int sb = 0; sb < SB; ++sb) mul(sb);
      __syncthreads();
      if(!PIPE && st + 1 < s_hi) acts(st + 1, 0);
    }
  }
  // The last stage's buffers are free once every warp has multiplied it: the paired tile's epilogue stages its sums over
  // them.
  if constexpr(OVER) __syncthreads();
  if(parted){
    // A part parks its sums; the tile's last part to arrive sums every part in order, its own read back too.
    __shared__ bool lk_last;
    int tile = tail + ((int)blockIdx.x - tail) / split;
    float4* mine = (float4*)(part + ((long)(blockIdx.x - tail) * (BM * 4) + tid) * 32);
    #pragma unroll
    for(int mt = 0; mt < 2; ++mt)
      #pragma unroll
      for(int nt = 0; nt < 4; ++nt) mine[mt * 4 + nt] = make_float4(acc[mt][nt][0], acc[mt][nt][1], acc[mt][nt][2], acc[mt][nt][3]);
    __threadfence();
    __syncthreads();
    if(tid == 0) lk_last = atomicAdd(cnt + (tile - tail), 1) == split - 1;
    __syncthreads();
    if(!lk_last) return;
    #pragma unroll
    for(int mt = 0; mt < 2; ++mt)
      #pragma unroll
      for(int nt = 0; nt < 4; ++nt)
        #pragma unroll
        for(int e = 0; e < 4; ++e) acc[mt][nt][e] = 0.f;
    for(int pc = 0; pc < split; ++pc){
      const float4* src = (const float4*)(part + ((long)((tile - tail) * split + pc) * (BM * 4) + tid) * 32);
      #pragma unroll
      for(int mt = 0; mt < 2; ++mt)
        #pragma unroll
        for(int nt = 0; nt < 4; ++nt){
          float4 v = __ldcg(src + mt * 4 + nt);
          acc[mt][nt][0] += v.x; acc[mt][nt][1] += v.y; acc[mt][nt][2] += v.z; acc[mt][nt][3] += v.w;
        }
    }
    if(tid == 0) cnt[tile - tail] = 0;
  }
  if(PAIR){
    // Each hidden value's gate and up sums (plus their biases) staged by column, then through the gated activation
    // and quantized a 32-value block a warp at a time.
    float* E = (float*)lk_sm;
    #pragma unroll
    for(int mt = 0; mt < 2; ++mt)
      #pragma unroll
      for(int nt = 0; nt < 4; ++nt)
        #pragma unroll
        for(int e = 0; e < 4; ++e){
          int r = wm + mt * 16 + g + (e >= 2 ? 8 : 0), c = wn + nt * 8 + t4 * 2 + (e & 1), col = n0 + lk_tile_row<true>(c);
          const float* b = c < LK_BN / 2 ? bias.b : bias2;
          float v = acc[mt][nt][e];
          E[r * LK_BN + c] = b && col < N ? v + b[wbase + col] : v;
        }
    __syncthreads();
    for(int r = warp; r < BM; r += BM / 8){
      if(m0 + r >= M) break;
      #pragma unroll
      for(int j = 0; j < LK_BN / 64; ++j){
        int c = 32 * j + lane;
        if(n0 + 32 * j >= N) break;
        float gv = E[r * LK_BN + c], uv = E[r * LK_BN + LK_BN / 2 + c], hv;
        if(G == 5) hv = ACT < 0 ? gv / (1.0f + expf(-gv)) * uv : uv * lk_act<(ACT < 0 ? 0 : ACT)>(gv);
        else if(ACT == 0) hv = gv / (1.0f + expf(-gv)) * uv;
        else { float gc = fminf(gv, limit); hv = gc / (1.0f + expf(-slope * gc)) * (fminf(fmaxf(uv, -limit), limit) + 1.0f); }
        long i = (first + m0 + r) * N + n0 + c;
        if(G == 5 && y) y[i] = hv;
        if(G == 3 || hq) lk_quant_lane<Q>(hv, hq, hl, hs, i, i >> 5);
      }
    }
    return;
  }
  // An expert mixture's outputs are its `f32` scratch, stored as floats; any other product's through its array's type.
  lk_out yo = GROUPED ? lk_out() : LK_ROW(y, yrow, ypitch);
  float* yg = GROUPED ? (float*)y.p + first * N : nullptr;
  if constexpr(PLAIN && !GROUPED){
    float* const yf = (float*)yo.p;
    #pragma unroll
    for(int mt = 0; mt < 2; ++mt)
      #pragma unroll
      for(int nt = 0; nt < 4; ++nt)
        #pragma unroll
        for(int e = 0; e < 4; ++e){
          const int row = m0 + wm + mt * 16 + g + (e >= 2 ? 8 : 0), col = n0 + wn + nt * 8 + t4 * 2 + (e & 1);
          if(row < M && col < N) yf[(long)row * N + col] = acc[mt][nt][e];
        }
    return;
  }
  #pragma unroll
  for(int mt = 0; mt < 2; ++mt)
    #pragma unroll
    for(int nt = 0; nt < 4; ++nt)
      #pragma unroll
      for(int e = 0; e < 4; ++e){
        int row = m0 + wm + mt * 16 + g + (e >= 2 ? 8 : 0);
        int col = n0 + wn + nt * 8 + t4 * 2 + (e & 1);
        if(row >= M || col >= N) continue;
        float v = bias(acc[mt][nt][e], row, wbase + col, N);
        // G 2 weighs each choice's result by its weight, as the expert mixture's down projection does.
        if constexpr(GROUPED) yg[(long)row * N + col] = G == 2 ? v * wt[list[first + row]] : v;
        else yo[(long)row * N + col] = v;
      }
}
"#;

/// A 16-bit float matrix's GEMM on the 16-bit tensor cores (`F` 1, F16; 30, BF16; `Q` 1 one activation word, 2 two):
/// `Y[t,o] = Σ_k X[t,k]·W[o,k]` for the `M` rows of `X`, read `f32`, over the `N` rows of `W`. A block takes
/// `LK_H16_BM` rows by `TN` outputs (128, 96 or 64, `gemm_h16_launch_tile`) over stages of 32 `k`, `LK_H16_STAGES` in a
/// ring (three in tiles of 96):
/// the weight rows' 32 values each by `cp.async` three stages ahead, the activation rows' `f32` values read a stage
/// ahead and rounded once a block into the slot's words (`lk_h16_words`, a plane a word, laid so a lane's sixteen bytes
/// at a row are its fragment's, every quarter-warp's reads and writes on distinct banks), one barrier a stage. A warp
/// multiplies 32 outputs of 32 rows in tiles of 64 and of 64 rows in tiles of 128 (Llama-3.2-1B's 512-row projections
/// on the 3060 in one-word tiles of 64 at 253.6 to 951.6 µs, against 265.4 to 1010.2 at 64 rows by 16 outputs a warp),
/// each block of 32 `k` as `lk_h16_block` sums it, so every output sums as the decode step's row dot (`lk_h16_mma_dot`)
/// sums it, at either width, and no product splits `k`. In parts (`G` 4) it
/// multiplies one activation by the matrices of `parts` side by side, as `lk_gemm_blk`'s parts do; paired (`G` 5) a
/// block takes `TN / 2` outputs of the gate (`w`, `bias`) and the same of up (`w2`, `bias2`) and writes each hidden
/// value through the gated activation `ACT`, as `lk_gemv_glu` computes it. At 64 outputs and one word two blocks share a
/// multiprocessor, each one's epilogue and first stages under the other's products.
const GEMM_H16: &str = r#"
#define LK_H16_BM 128
#define LK_H16_STAGES 4
#define LK_H16_BP 32
template<int F, int Q, int G = 0, int ACT = 0, int TN = 128>
__global__ void __launch_bounds__(256, Q == 1 && TN <= 96 ? 2 : 1) lk_gemm_h16(lk_w w, const float* __restrict__ x, lk_epi bias, lk_out y,
    const int* __restrict__ yrow, long ypitch, int K, int N, int M, lk_w w2 = {}, const float* __restrict__ bias2 = nullptr,
    lk_parts parts = {}){
  extern __shared__ __align__(16) unsigned char lk_sm[];
  constexpr bool PAIR = G == 5;
  // A stage's slot: the activation rows' words, `Q` planes of `LK_H16_BM` rows by 32 values, then the tile's `TN`
  // weight rows' 32 values (a pair's `TW` gate rows, then the same up rows); a warp's outputs are `NT` tiles of eight
  // over `WMT` tiles of 16 rows: half the tile's outputs of 32 rows in tiles of 64 and 96 (the warps four down the
  // rows, two across), 32 outputs of 64 rows in tiles of 128 (two down, four across); `NS` stages in the ring, three
  // in tiles of 96 so two blocks' fit a multiprocessor.
  static_assert(TN == 128 || TN == 96 || TN == 64, "a tile of 128 outputs, 96 or 64");
  constexpr int WMT = TN == 128 ? 4 : 2, ACROSS = 8 * 16 * WMT / LK_H16_BM, WN = TN / ACROSS, NS = TN == 96 ? 3 : LK_H16_STAGES;
  constexpr int TW = PAIR ? TN / 2 : TN, NT = WN / 8, A_PL = LK_H16_BM * 32 * 2, A_ST = Q * A_PL, B_ST = TN * LK_H16_BP * 2, ST = A_ST + B_ST;
  const int tid = threadIdx.x, lane = tid & 31, warp = tid >> 5, g = lane >> 2, t = lane & 3;
  const int m0 = blockIdx.y * LK_H16_BM;
  int n0 = blockIdx.x * TW;
  // The part whose columns hold the tile: its matrix, outputs and bias, the tile's columns its own.
  if(G == 4) lk_take_part(parts, n0, w, y, bias, yrow, ypitch, N);
  // The warp's `16·WMT` rows by `WN` outputs of the tile.
  const int wm = warp / ACROSS * 16 * WMT, wn = warp % ACROSS * WN, stages = K >> 5;
  float acc[WMT][NT][4];
  #pragma unroll
  for(int i = 0; i < WMT; ++i)
    #pragma unroll
    for(int j = 0; j < NT; ++j)
      #pragma unroll
      for(int e = 0; e < 4; ++e) acc[i][j][e] = 0.f;
  // Stage `st`'s weights: each of the tile's `TN` weight rows' 32 values, four 16-byte copies — a pair's gate rows and
  // then the same up rows, the matrices' fields held by value and selected one by one, so no matrix is selected whole
  // at run time and no parameter's address is taken.
  const unsigned char* const w1b = w.b;
  const unsigned char* const w2b = w2.b;
  const long w1rb = w.rb, w2rb = w2.rb;
  const int w1g = w.g, w2g = w2.g;
  const float* const xs = x;
  auto copy_w = [&](int st){
    unsigned char* Bs = lk_sm + (st % NS) * ST + A_ST;
    for(int c = tid; c < TN * 4; c += 256){
      const int r = c >> 2, h = c & 3, col = PAIR && r >= TW ? r - TW : r;
      const bool up = PAIR && r >= TW, ok = n0 + col < N;
      const unsigned char* mb = up ? w2b : w1b;
      const unsigned char* src = ok ? lk_h16_addr(mb, up ? w2rb : w1rb, up ? w2g : w1g, n0 + col, st) + 16 * h : mb;
      lk_cp16(Bs + r * LK_H16_BP * 2 + 16 * h, src, ok);
    }
    asm volatile("cp.async.commit_group;\n");
  };
  // Stage `st`'s activation: a thread's sixteen values, four 16-byte reads (row `c >> 3`, values `4(c & 7)` … of each
  // `c = tid + 256i`), read a stage ahead and rounded once a block into the slot's words — the words `lk_h16_take`
  // rounds, laid so a lane's sixteen bytes at a row are its fragment's.
  float4 xr[4];
  auto load_x = [&](int st){
    const int k0 = st * 32;
    #pragma unroll
    for(int i = 0; i < 4; ++i){
      const int c = tid + 256 * i, r = c >> 3, h = c & 7;
      xr[i] = st < stages && m0 + r < M ? __ldg((const float4*)(xs + (long)(m0 + r) * K + k0 + 4 * h)) : make_float4(0.f, 0.f, 0.f, 0.f);
    }
  };
  auto store_x = [&](int st){
    unsigned char* As = lk_sm + (st % NS) * ST;
    #pragma unroll
    for(int i = 0; i < 4; ++i){
      const int c = tid + 256 * i, r = c >> 3, h = c & 7;
      unsigned h0, l0, h1, l1;
      lk_h16_words<F, Q>(xr[i].x, xr[i].y, h0, l0);
      lk_h16_words<F, Q>(xr[i].z, xr[i].w, h1, l1);
      *(uint2*)(As + r * 64 + 8 * h) = make_uint2(h0, h1);
      if constexpr(Q == 2) *(uint2*)(As + A_PL + r * 64 + 8 * h) = make_uint2(l0, l1);
    }
  };
  #pragma unroll
  for(int st = 0; st < NS - 1; ++st){
    if(st < stages) copy_w(st);
    else asm volatile("cp.async.commit_group;\n");
  }
  load_x(0);
  store_x(0);
  load_x(1);
  // One barrier a stage: once stage `st`'s weights have landed and every warp is past stage `st − 1`, that stage's
  // slot takes the weights of stage `st + NS − 1`, and the next stage's slot its activation's words.
  for(int st = 0; st < stages; ++st){
    asm volatile("cp.async.wait_group %0;\n" :: "n"(NS - 2));
    __syncthreads();
    if(st + NS - 1 < stages) copy_w(st + NS - 1);
    else asm volatile("cp.async.commit_group;\n");
    if(st + 1 < stages){
      store_x(st + 1);
      load_x(st + 2);
    }
    const unsigned char* sb = lk_sm + (st % NS) * ST;
    const unsigned char* Bs = sb + A_ST;
    unsigned bw[NT][4];
    #pragma unroll
    for(int n = 0; n < NT; ++n){
      const uint4 v = *(const uint4*)(Bs + (wn + 8 * n + g) * LK_H16_BP * 2 + 16 * t);
      bw[n][0] = v.x; bw[n][1] = v.y; bw[n][2] = v.z; bw[n][3] = v.w;
    }
    #pragma unroll
    for(int m = 0; m < WMT; ++m){
      lk_h16_a a;
      #pragma unroll
      for(int p = 0; p < Q; ++p){
        const unsigned char* ra = sb + p * A_PL + (wm + 16 * m + g) * 64 + 16 * t;
        const uint4 u = *(const uint4*)ra, v = *(const uint4*)(ra + 8 * 64);
        unsigned (&o)[2][4] = p ? a.l : a.h;
        o[0][0] = u.x; o[0][2] = u.y; o[1][0] = u.z; o[1][2] = u.w;
        o[0][1] = v.x; o[0][3] = v.y; o[1][1] = v.z; o[1][3] = v.w;
      }
      #pragma unroll
      for(int n = 0; n < NT; ++n) lk_h16_mul<F, Q>(acc[m][n], a, bw[n]);
    }
  }
  asm volatile("cp.async.wait_group 0;\n");
  __syncthreads();
  if(PAIR){
    // Each hidden value's gate and up sums (plus their biases) staged by column, then through the gated activation.
    float* E = (float*)lk_sm;
    #pragma unroll
    for(int m = 0; m < WMT; ++m)
      #pragma unroll
      for(int n = 0; n < NT; ++n)
        #pragma unroll
        for(int e = 0; e < 4; ++e){
          const int r = wm + 16 * m + g + (e >= 2 ? 8 : 0), c = wn + 8 * n + 2 * t + (e & 1), col = n0 + (c < TW ? c : c - TW);
          const float* b = c < TW ? bias.b : bias2;
          const float v = acc[m][n][e];
          E[r * TN + c] = b && col < N ? v + b[col] : v;
        }
    __syncthreads();
    lk_out yo = LK_ROW(y, yrow, ypitch);
    for(int r = warp; r < LK_H16_BM; r += 8){
      if(m0 + r >= M) break;
      for(int c = lane; c < TW; c += 32){
        if(n0 + c >= N) continue;
        const float gv = E[r * TN + c], uv = E[r * TN + TW + c];
        const float hv = ACT < 0 ? gv / (1.0f + expf(-gv)) * uv : uv * lk_act<(ACT < 0 ? 0 : ACT)>(gv);
        yo[(long)(m0 + r) * N + n0 + c] = hv;
      }
    }
    return;
  }
  // The tile's sums staged in shared memory by their fragments, then each output through the epilogue, a row a warp
  // and a column a lane, stored as its array's type.
  float* E = (float*)lk_sm;
  #pragma unroll
  for(int m = 0; m < WMT; ++m)
    #pragma unroll
    for(int n = 0; n < NT; ++n)
      #pragma unroll
      for(int e = 0; e < 4; ++e) E[(wm + 16 * m + g + (e >= 2 ? 8 : 0)) * TN + wn + 8 * n + 2 * t + (e & 1)] = acc[m][n][e];
  __syncthreads();
  lk_out yo = LK_ROW(y, yrow, ypitch);
  const lk_epi epi = bias;
  float* const yf = yo.t == LK_F32 ? (float*)yo.p : nullptr;
  for(int r = warp; r < LK_H16_BM; r += 8){
    const int row = m0 + r;
    if(row >= M) break;
    for(int c = lane; c < TN; c += 32){
      const int col = n0 + c;
      if(col >= N) break;
      const float v = epi(E[r * TN + c], row, col, N);
      const long i = (long)row * N + col;
      if(yf) yf[i] = v;
      else yo[i] = v;
    }
  }
}
"#;

/// The dynamic shared memory of a block of the 16-bit GEMM (`lk_gemm_h16`) in tiles of `tn` outputs, its activation in
/// one word or two (`two`): its four stages' activation words (32 values a row a word) and weight rows (32 values each,
/// `tn` of them), and at least the tile's staged sums, a float for each of its outputs.
pub fn gemm_h16_smem_bytes(two: bool, tn: usize) -> usize {
    let stage = if two { 2 } else { 1 } * GEMM_H16_ROWS * 32 * 2 + tn * 32 * 2;
    (gemm_h16_stages(tn) * stage).max(GEMM_H16_ROWS * tn * 4)
}

/// The stages in the ring of a block of the 16-bit GEMM in tiles of `tn` outputs (`lk_gemm_h16`'s `NS`): three in tiles
/// of 96, so two blocks' stages and staged sums fit a multiprocessor's 100 KB; four otherwise.
pub fn gemm_h16_stages(tn: usize) -> usize {
    if tn == 96 {
        3
    } else {
        4
    }
}

/// The outputs a tile of one launch of the 16-bit GEMM takes over `outputs` outputs (a paired launch's gate and up
/// together) of `rows` activation rows on a chip of `sms` multiprocessors: under one word 96 where its tiles still
/// fill four waves of two blocks a multiprocessor — fewer, wider tiles, each multiplying more outputs a stage — and the
/// tiles of `gemm_h16_tile_n` under fewer, whose wave's tail costs the narrower tile less; under two words theirs. A
/// launch in parts takes `gemm_h16_tile_n`'s, whole tiles a part. On the 3060 over Llama-3.2-1B's 512-row chunk the
/// paired gate and up took 1,716 µs in tiles of 96 against 1,807 in tiles of 64; its 2048-output projections, two
/// waves of 96-wide tiles, 9% longer in them.
pub fn gemm_h16_launch_tile(two: bool, outputs: usize, rows: usize, sms: usize) -> usize {
    let wide = outputs.div_ceil(96) * rows.div_ceil(GEMM_H16_ROWS) >= 4 * 2 * sms;
    if !two && wide {
        96
    } else {
        gemm_h16_tile_n(two)
    }
}

/// The outputs a tile of the 16-bit GEMM takes (`lk_gemm_h16`'s `TN`), its activation in one word or two (`two`): under
/// one word 64, two blocks a multiprocessor — Llama-3.2-1B's projections of a 512-row prompt at 16.4 to 18.3 TFLOPS on
/// the 3060 where tiles of 128, a block a multiprocessor, took 12.3 to 14.5; under two 128, a block a multiprocessor at
/// either width, the wider tile's weights staged once for twice the outputs (7.5 to 9.1 TFLOPS, 7.5 to 8.1 at 64).
pub fn gemm_h16_tile_n(two: bool) -> usize {
    if two {
        128
    } else {
        64
    }
}

/// The activation rows a block of the 16-bit GEMM takes (`LK_H16_BM`).
pub const GEMM_H16_ROWS: usize = 128;

/// The dynamic shared memory a block-format tensor-core GEMM block in format `f` (its GGUF type id) over `bm` activation
/// rows may take, whichever path a launch takes ([`gemm_blk_launch_smem`] over every `k`): what its kernel is allowed.
pub fn gemm_blk_smem_bytes(f: u32, two_pass: bool, bm: usize) -> usize {
    let block = gemm_blk_block_bytes(f, two_pass, bm);
    if f == 8 {
        block.max(gemm_blk_stream_bytes(two_pass, bm))
    } else {
        block
    }
}

/// The dynamic shared memory one launch of the block-format tensor-core GEMM over `k` takes: a Q8_0 matrix's streamed
/// path over a `k` of whole 128-value scale groups, the block path otherwise — so the streamed path's blocks are not
/// held to the block path's larger stages, and two of them share a multiprocessor where its registers allow.
pub fn gemm_blk_launch_smem(f: u32, two_pass: bool, bm: usize, k: usize) -> usize {
    if f == 8 && k % 128 == 0 {
        gemm_blk_stream_bytes(two_pass, bm)
    } else {
        gemm_blk_block_bytes(f, two_pass, bm)
    }
}

/// The block path's shared memory over `bm` activation rows in format `f`: its activation tile (twice under two words)
/// and each row's scales — two of them where they fit — its decoded weight tile with each weight sub-block's scales
/// (both halves where they scale apart) and offsets — two of them where they fit beside the two activation buffers and
/// a block holds its multiprocessor alone — and the activation's scaled sums (where the format has offsets), as
/// `lk_stage_acts`, `lk_stage_wt`, `lk_stage_sums` and `lk_stage_over` lay them out; and at least the paired
/// epilogue's gate and up sums, a float for each of the tile's outputs.
fn gemm_blk_block_bytes(f: u32, two_pass: bool, bm: usize) -> usize {
    let n = GEMM_TILE_N;
    let words = if two_pass { 2 } else { 1 };
    let halves = if matches!(f, 10 | 11 | 14) { 2 } else { 1 };
    let offset = matches!(f, 3 | 7 | 10 | 12 | 13);
    let acts = words * bm * 144 + bm * 4 * 4;
    let weights = n * 144 + n * 4 * halves * if offset { 2 } else { 1 } * 4;
    let sums = if offset { words * bm * 4 * halves * 4 } else { 0 };
    let piped = 2 * acts + weights + sums <= 101_376;
    let over = piped && gemm_blk_blocks(f, two_pass, bm) == 1 && 2 * acts + 2 * weights + sums <= 101_376;
    let stages = match (piped, over) {
        (_, true) => 2 * acts + 2 * weights + sums,
        (true, false) => 2 * acts + weights + sums,
        (false, _) => acts + weights + sums,
    };
    stages.max(bm * n * 4)
}

/// The blocks of the block-format tensor-core GEMM over `bm` activation rows in format `f` a multiprocessor holds at
/// once (`lk_gemm_blk_blocks`): two of a 64-row tile but in Q2_K, Q3_K and Q5_K, and under two words only in Q8_0; one
/// of a wider tile.
fn gemm_blk_blocks(f: u32, two_pass: bool, bm: usize) -> usize {
    if bm <= 64 && (f == 8 || !two_pass) && !matches!(f, 10 | 11 | 13) {
        2
    } else {
        1
    }
}

/// A Q8_0 matrix's streamed path's shared memory over `bm` activation rows: its `STREAM_STAGES` stages (the activation's
/// words, twice under two words, and the matrix's levels, 32 of `k` a row), the two scale groups it holds (four stages'
/// scales of each activation row and each matrix row) and its stages' two barriers each — and at least the paired
/// epilogue's hidden values, a float for each of the pair tile's `bm` × 64.
fn gemm_blk_stream_bytes(two_pass: bool, bm: usize) -> usize {
    let n = GEMM_TILE_N;
    let words = if two_pass { 2 } else { 1 };
    (STREAM_STAGES * (words * bm * 32 + n * 32) + 2 * (bm * 16 + n * 8) + 2 * STREAM_STAGES * 8).max(bm * n / 2 * 4)
}

/// The stages of `k`, 32 each, a Q8_0 matrix's streamed path keeps in flight (`LK_STREAM_STAGES`).
const STREAM_STAGES: usize = 4;

/// The activation rows of a grouped GEMM block over an expert mixture's choices: 128 when the average expert has more
/// than 64 rows to multiply (`choices / experts`), 64 otherwise — a taller tile reads each expert's decoded weights for
/// more rows, a shorter one leaves fewer of its rows empty.
pub fn gemm_blk_expert_rows(choices: usize, experts: usize) -> usize {
    if choices > 64 * experts {
        128
    } else {
        64
    }
}

/// The parts an `f32` router GEMM (`lk_gemm_f32`) of `tiles` 64 × 64 tiles cuts each tile's `chunks` runs of 32 `k`
/// into: as many as give the chip's `sms` two blocks each, whole runs of chunks every one.
pub fn gemm_f32_splits(tiles: usize, sms: usize, chunks: usize) -> usize {
    (2 * sms / tiles.max(1)).clamp(1, chunks.max(1))
}

/// How a dense block GEMM of `tiles` tiles and `stages` stages of `k` fills its last wave of `workers` blocks:
/// `(whole, split)` — the first `whole` tiles a block each, every later tile cut into `split` parts of its stages, as
/// many as fill the wave. `split` 1 when the tiles fill whole waves, or a wave's blocks cannot cut a remaining tile in
/// two.
pub fn gemm_blk_tail(tiles: usize, workers: usize, stages: usize) -> (usize, usize) {
    let rest = tiles % workers;
    let split = if rest == 0 { 1 } else { (workers / rest).min(stages) };
    if split < 2 {
        (tiles, 1)
    } else {
        (tiles - rest, split)
    }
}

/// The threads of a block-format tensor-core GEMM block over `bm` activation rows: a warp each 32 × 32 of the tile.
pub fn gemm_blk_threads(bm: usize) -> usize {
    bm * 4
}

/// The threads one launch of the block-format tensor-core GEMM over `k` takes: a Q8_0 matrix's streamed path over 128
/// rows, eight warps of 64 rows by 32 outputs — two blocks of them, their registers and stages, share a multiprocessor —
/// and a warp each 32 × 32 of the tile otherwise ([`gemm_blk_threads`]).
pub fn gemm_blk_launch_threads(f: u32, bm: usize, k: usize) -> usize {
    if f == 8 && k % 128 == 0 && bm == GEMM_BLK_TILE_M {
        256
    } else {
        gemm_blk_threads(bm)
    }
}

/// The activation rows of a dense block-format tensor-core GEMM block's tile (its outputs: `GEMM_TILE_N`).
pub const GEMM_BLK_TILE_M: usize = 128;

/// The activation rows a dense tensor-core GEMM of a matrix in format `f` (its GGUF type id) takes a block: 64 for a
/// Q8_0 matrix under two words, so two blocks of its streamed path, both words of each stage, share a multiprocessor;
/// `GEMM_BLK_TILE_M` otherwise.
pub fn gemm_blk_rows(f: u32, two_pass: bool) -> usize {
    if f == 8 && two_pass {
        64
    } else {
        GEMM_BLK_TILE_M
    }
}

/// The matrices a launch in parts (`lk_parts`: `lk_gemv_parts`, `lk_gemv_mma`, `lk_gemm_blk`'s `G` 4) multiplies one
/// activation by at most — the kernels' `LK_PARTS`: six, so a recurrent layer's query, key, value, output gate and two
/// gates project its activation in one launch, the gates no launch of their own (at Qwen3.5-0.8B's batch of 64 a
/// single block on the 3060, 16.8 µs a layer).
pub const GEMM_PARTS: usize = 6;

/// The rows of a tiled per-head matmul block (`lk_head_tiled`), each block 64 of a head's outputs.
pub const HEAD_TILE_M: usize = 128;

/// The outputs a block of the tensor-core block GEMM computes (`lk_gemm_blk`'s tile).
pub const GEMM_TILE_N: usize = 128;

/// `Y[t,o] = Σ_k X[t,k]·W[o,k]` for the `M` rows of `X` over an `f32` matrix stored a row per output, `f64`
/// accumulation: `parts` warps an output row (`gemv_f32_parts`), each summing one run of `k` a lane at a time, the runs'
/// sums added in order — a block of eight warps `8 / parts` output rows. A block takes its rows `LK_F32_ROWS` at a time,
/// each weight value read once for all of them, every row summed as a row alone sums.
const GEMV_F32: &str = r#"
#define LK_F32_ROWS 4
__global__ void __launch_bounds__(256) lk_gemv_f32(const float* __restrict__ w, const float* __restrict__ x,
    lk_epi bias, lk_out y, const int* __restrict__ yrow, long ypitch, int K, int N, int M,
    int parts = 1){
  __shared__ double run[LK_F32_ROWS][8];
  int warp = threadIdx.x >> 5, lane = threadIdx.x & 31;
  int o = blockIdx.x * (8 / parts) + warp / parts, p = warp % parts;
  bool live = o < N;
  int span = (K + parts - 1) / parts, k0 = p * span, k1 = min(K, k0 + span);
  const float* wr = w + (long)(live ? o : 0) * K;
  lk_out yo = LK_ROW(y, yrow, ypitch);
  int rb = (M + gridDim.y - 1) / gridDim.y, t1 = min(M, (int)(blockIdx.y + 1) * rb);
  for(int t0 = blockIdx.y * rb; t0 < t1; t0 += LK_F32_ROWS){
    double acc[LK_F32_ROWS];
    #pragma unroll
    for(int r = 0; r < LK_F32_ROWS; ++r) acc[r] = 0.0;
    if(live)
      for(int k = k0 + lane; k < k1; k += 32){
        const double wv = (double)wr[k];
        #pragma unroll
        for(int r = 0; r < LK_F32_ROWS; ++r) if(t0 + r < t1) acc[r] += wv * (double)x[(long)(t0 + r) * K + k];
      }
    #pragma unroll
    for(int r = 0; r < LK_F32_ROWS; ++r) acc[r] = lk_warp_sum_d(acc[r]);
    if(parts == 1){
      #pragma unroll
      for(int r = 0; r < LK_F32_ROWS; ++r)
        if(live && lane == 0 && t0 + r < t1) yo[(long)(t0 + r) * N + o] = bias((float)acc[r], t0 + r, o, N);
      continue;
    }
    if(lane == 0)
      #pragma unroll
      for(int r = 0; r < LK_F32_ROWS; ++r) run[r][warp] = acc[r];
    __syncthreads();
    if(live && p == 0 && lane == 0)
      #pragma unroll
      for(int r = 0; r < LK_F32_ROWS; ++r)
        if(t0 + r < t1){
          double s = 0.0;
          for(int i = 0; i < parts; ++i) s += run[r][warp + i];
          yo[(long)(t0 + r) * N + o] = bias((float)s, t0 + r, o, N);
        }
    __syncthreads();
  }
}
"#;

/// The warps an `f32` GEMV (`lk_gemv_f32`) cuts each output row's `k` into: the most, to eight, that keep 128 values a
/// warp and give its `n` outputs no more than 16 warps a streaming multiprocessor of the chip's `sms`.
pub fn gemv_f32_parts(n: usize, k: usize, sms: usize) -> usize {
    [8, 4, 2].into_iter().find(|&p| k >= 128 * p && n * p <= 16 * sms).unwrap_or(1)
}

/// `c = a ⊕ b` elementwise (`OP` 0 add, 1 multiply), `b` passed through the activation `ACT` first when it names one
/// (a gate; −1 none); `b` holds `nb` elements read at `i mod nb` — all `n`, or one row combined with every row. `Q`
/// quantizes `c` (`n` a multiple of 32) and `OUTF` writes it.
/// `Y[m, n] = Σ_k X[m, k]·W[n, k]`, plus `bias[n]` when there is one, in `f32` over a wide chunk's `M` rows: 64 rows by 64
/// outputs a block over chunks of 32 `k` staged in shared memory, a thread 4 by 4 — a wide chunk's `f32` router. With
/// `split` parts (grid layer `z` a part, [`gemm_f32_splits`]) each tile's `k` is cut into that many runs of whole
/// chunks; each part parks its sums in `part`, and the tile's last part to arrive (`cnt`) sums them in order.
const GEMM_F32: &str = r#"
__global__ void __launch_bounds__(256) lk_gemm_f32(const float* __restrict__ x, const float* __restrict__ w,
                                                   lk_epi bias, lk_out y, int M, int N, int K,
                                                   int split = 1, float* __restrict__ part = nullptr, int* __restrict__ cnt = nullptr){
  __shared__ __align__(16) float Xs[32][64];
  __shared__ __align__(16) float Ws[32][64];
  __shared__ bool lk_last;
  int m0 = blockIdx.y * 64, n0 = blockIdx.x * 64, tid = threadIdx.x, tr = tid >> 4, tn = tid & 15;
  int chunks = (K + 31) / 32, run = (chunks + split - 1) / split, k_lo = blockIdx.z * run * 32, k_hi = min(K, k_lo + run * 32);
  float acc[4][4];
  #pragma unroll
  for(int r = 0; r < 4; ++r)
    #pragma unroll
    for(int c = 0; c < 4; ++c) acc[r][c] = 0.f;
  for(int k0 = k_lo; k0 < k_hi; k0 += 32){
    for(int c = tid; c < 64 * 8; c += 256){
      int r = c >> 3, k = (c & 7) * 4;
      float4 v = m0 + r < M && k0 + k < k_hi ? *(const float4*)(x + (long)(m0 + r) * K + k0 + k) : make_float4(0.f, 0.f, 0.f, 0.f);
      Xs[k][r] = v.x; Xs[k + 1][r] = v.y; Xs[k + 2][r] = v.z; Xs[k + 3][r] = v.w;
      float4 u = n0 + r < N && k0 + k < k_hi ? *(const float4*)(w + (long)(n0 + r) * K + k0 + k) : make_float4(0.f, 0.f, 0.f, 0.f);
      Ws[k][r] = u.x; Ws[k + 1][r] = u.y; Ws[k + 2][r] = u.z; Ws[k + 3][r] = u.w;
    }
    __syncthreads();
    #pragma unroll 8
    for(int k = 0; k < 32; ++k){
      float4 a = *(const float4*)&Xs[k][4 * tr];
      float4 b = *(const float4*)&Ws[k][4 * tn];
      float ar[4] = {a.x, a.y, a.z, a.w}, br[4] = {b.x, b.y, b.z, b.w};
      #pragma unroll
      for(int r = 0; r < 4; ++r)
        #pragma unroll
        for(int c = 0; c < 4; ++c) acc[r][c] = fmaf(ar[r], br[c], acc[r][c]);
    }
    __syncthreads();
  }
  if(split > 1){
    int tile = blockIdx.y * gridDim.x + blockIdx.x;
    float4* mine = (float4*)(part + ((long)(tile * split + blockIdx.z) * 256 + tid) * 16);
    #pragma unroll
    for(int r = 0; r < 4; ++r) mine[r] = make_float4(acc[r][0], acc[r][1], acc[r][2], acc[r][3]);
    __threadfence();
    __syncthreads();
    if(tid == 0) lk_last = atomicAdd(cnt + tile, 1) == split - 1;
    __syncthreads();
    if(!lk_last) return;
    #pragma unroll
    for(int r = 0; r < 4; ++r)
      #pragma unroll
      for(int c = 0; c < 4; ++c) acc[r][c] = 0.f;
    for(int pc = 0; pc < split; ++pc){
      const float4* src = (const float4*)(part + ((long)(tile * split + pc) * 256 + tid) * 16);
      #pragma unroll
      for(int r = 0; r < 4; ++r){
        float4 v = __ldcg(src + r);
        acc[r][0] += v.x; acc[r][1] += v.y; acc[r][2] += v.z; acc[r][3] += v.w;
      }
    }
    if(tid == 0) cnt[tile] = 0;
  }
  #pragma unroll
  for(int r = 0; r < 4; ++r){
    int m = m0 + 4 * tr + r;
    if(m >= M) continue;
    #pragma unroll
    for(int c = 0; c < 4; ++c){
      int n = n0 + 4 * tn + c;
      if(n < N) y[(long)m * N + n] = bias(acc[r][c], m, n, N);
    }
  }
}
"#;

const BINARY: &str = r#"
template<int OP, int ACT, bool OUTF, int Q>
__global__ void lk_binary(const float* __restrict__ a, const float* __restrict__ b, float* __restrict__ c,
                          signed char* __restrict__ cq, signed char* __restrict__ cl, float* __restrict__ cs, long n,
                          long nb){
  long i = (long)blockIdx.x * blockDim.x + threadIdx.x; if(i >= n) return;
  float bv = ACT < 0 ? b[i % nb] : lk_act<ACT>(b[i % nb]);
  float v = OP == 0 ? a[i] + bv : a[i] * bv;
  if(OUTF) c[i] = v;
  if(Q > 0) lk_quant_lane<Q>(v, cq, cl, cs, i, i >> 5);
}
"#;

/// `y = c·tanh(x / c)` elementwise (`softcap`), as `lk_cap`.
const SOFTCAP: &str = r#"
__global__ void lk_softcap(const float* __restrict__ x, float* __restrict__ y, long n, float c){
  long i = (long)blockIdx.x * blockDim.x + threadIdx.x; if(i >= n) return;
  y[i] = lk_cap(x[i], c);
}
"#;

/// `y = x·c` elementwise.
const SCALE: &str = r#"
__global__ void lk_scale(const float* __restrict__ x, float* __restrict__ y, long n, float c){
  long i = (long)blockIdx.x * blockDim.x + threadIdx.x; if(i >= n) return;
  y[i] = x[i] * c;
}
"#;

/// `y = silu(g)·u` elementwise, `silu(g) = g / (1 + e^-g)`; `Q` quantizes `y`, `OUTF` writes it.
const SWIGLU: &str = r#"
template<bool OUTF, int Q>
__global__ void lk_swiglu(const float* __restrict__ g, const float* __restrict__ u, float* __restrict__ y,
                          signed char* __restrict__ yq, signed char* __restrict__ yl, float* __restrict__ ys, long n){
  long i = (long)blockIdx.x * blockDim.x + threadIdx.x; if(i >= n) return;
  float gv = g[i];
  float v = gv / (1.0f + expf(-gv)) * u[i];
  if(OUTF) y[i] = v;
  if(Q > 0) lk_quant_lane<Q>(v, yq, yl, ys, i, i >> 5);
}
"#;

/// An elementwise activation, `lk_act<ACT>`.
const UNARY: &str = r#"
template<int ACT>
__global__ void lk_unary(const float* __restrict__ x, float* __restrict__ y, long n){
  long i = (long)blockIdx.x * blockDim.x + threadIdx.x; if(i >= n) return;
  y[i] = lk_act<ACT>(x[i]);
}
"#;

/// `y = max(x, 0)`, squared when `SQ` (`relu_sqr`): a NaN and either zero give +0, as the language's `max` does. `Q`
/// quantizes `y`, `OUTF` writes it.
const RELU: &str = r#"
template<bool SQ, bool OUTF, int Q>
__global__ void lk_relu(const float* __restrict__ x, float* __restrict__ y, signed char* __restrict__ yq,
                        signed char* __restrict__ yl, float* __restrict__ ys, long n){
  long i = (long)blockIdx.x * blockDim.x + threadIdx.x; if(i >= n) return;
  float r = x[i] > 0.f ? x[i] : 0.f;
  float v = SQ ? r * r : r;
  if(OUTF) y[i] = v;
  if(Q > 0) lk_quant_lane<Q>(v, yq, yl, ys, i, i >> 5);
}
"#;

/// RoPE-NEOX over one head per block: each of the head's first `half` dims paired with the one `half` after it and
/// turned by the angles of one table row, the dims past `2·half` passed through. The heads come `per_row` to a row of
/// the input; row `r` reads table row `*pos + r` (1-based; row `r + 1` with no position). `NORM` RMSNorms the head by
/// `nw` first (QK-norm). The rows may land in the rows of a cache a position names. Head `h` is read `h·xp + xo` into
/// `x` (a part of every head of a wider row) and written `h·yp + yo` into its destination (a part of every head of one).
/// A head's work is `lk_rope_head`, which a projection's block that finishes a head also runs (`lk_gemv_parts_rope`), so
/// the two turn it alike.
const ROPE: &str = r#"
// One pair of a head turned by its angle's cosine `c` and sine `sv`: every rotation's arithmetic.
__device__ __forceinline__ void lk_rope_pair(float av, float bv, float c, float sv, float& ya, float& yb){
  ya = av * c - bv * sv;
  yb = av * sv + bv * c;
}
// One rotation: the arguments that differ between a layer's query and key rotations.
struct lk_rope_side { const float* x; const float* nw; lk_out y; const int* yrow; long ypitch; float eps; long xp, xo, yp, yo; int heads, per_row; long yz; };
// Head `h` of side `s`. A normed head's block is at least the head wide, so thread `t` takes its value `t` alone and
// the threads past the head add nothing: its sum of squares is the same tree whatever the block's width. `L2` reads the
// head past the L1, where other blocks of the launch wrote it. With `seqs`, a side's rows are a batch's sequences: row
// `r` turns from its own position, `pos[r]`, into its own table (`yz` values on from the last's) at its own row.
template<bool NORM, typename A, bool L2>
__device__ __forceinline__ void lk_rope_head(const lk_rope_side& s, int h, const float* __restrict__ cs,
                                             const float* __restrict__ sn, const int* __restrict__ pos, int hd, int half,
                                             int seqs, A* red, float* hv){
  auto ld = [](const float* p){ return L2 ? __ldcg(p) : *p; };
  const int r = h / s.per_row, t = threadIdx.x, rt = blockDim.x;
  const float* xh = s.x + (long)h * s.xp + s.xo;
  lk_out yh = seqs && s.yrow ? s.y + r * s.yz + (long)(s.yrow[r] - 1) * s.ypitch + (long)(h - r * s.per_row) * s.yp + s.yo
                             : LK_ROW(s.y, s.yrow, s.ypitch) + (long)h * s.yp + s.yo;
  A den = 1;
  if(NORM){
    A acc = 0;
    for(int i = t; i < hd; i += rt) acc += (A)ld(xh + i) * (A)ld(xh + i);
    den = sqrt(lk_block_sum(acc, red) / (A)hd + (A)s.eps);
  }
  for(int i = t; i < hd; i += rt) hv[i] = NORM ? (float)((A)ld(xh + i) / den * (A)s.nw[i]) : ld(xh + i);
  __syncthreads();
  long row = (pos ? (long)((seqs ? pos[r] : *pos + r) - 1) : (long)r) * half;
  for(int i = t; i < half; i += rt){
    float ya, yb;
    lk_rope_pair(hv[i], hv[i + half], cs[row + i], sn[row + i], ya, yb);
    yh[i] = ya;
    yh[i + half] = yb;
  }
  for(int i = 2 * half + t; i < hd; i += rt) yh[i] = hv[i];
}
template<bool NORM, typename A>
__global__ void lk_rope(const float* __restrict__ x, const float* __restrict__ nw, const float* __restrict__ cs,
                        const float* __restrict__ sn, const int* __restrict__ pos, lk_out y,
                        const int* __restrict__ yrow, long ypitch, int hd, int half, int per_row, float eps,
                        long xp, long xo, long yp, long yo){
  __shared__ A red[32];
  __shared__ float hv[1024];
  const lk_rope_side s{x, nw, y, yrow, ypitch, eps, xp, xo, yp, yo, (int)gridDim.x, per_row, 0};
  lk_rope_head<NORM, A, false>(s, (int)blockIdx.x, cs, sn, pos, hd, half, 0, red, hv);
}
// The unnormed rotation of `heads` heads a thread a slot of a head — its `half` pairs, then the values past `2·half` it
// passes through — each pair turned as `lk_rope_head` turns it (`lk_rope_pair`), so a chunk's rows spread over the
// chip's threads rather than a block a head.
__global__ void lk_rope_pairs(const float* __restrict__ x, const float* __restrict__ cs, const float* __restrict__ sn,
                              const int* __restrict__ pos, lk_out y, const int* __restrict__ yrow, long ypitch, int hd,
                              int half, int per_row, long heads, long xp, long xo, long yp, long yo){
  const int slots = hd - half;
  const long at = (long)blockIdx.x * blockDim.x + threadIdx.x;
  if(at >= heads * slots) return;
  const long h = at / slots;
  const int j = (int)(at % slots), r = (int)(h / per_row);
  const float* xh = x + h * xp + xo;
  lk_out yh = LK_ROW(y, yrow, ypitch) + h * yp + yo;
  if(j < half){
    const long row = (pos ? (long)(*pos + r - 1) : (long)r) * half;
    float ya, yb;
    lk_rope_pair(xh[j], xh[j + half], cs[row + j], sn[row + j], ya, yb);
    yh[j] = ya;
    yh[j + half] = yb;
  } else {
    yh[half + j] = xh[half + j];
  }
}
// Two rotations by one table at one position in one launch, `a`'s heads then `b`'s, each head as `lk_rope<NORM, A>`
// computes it.
template<bool NORM, typename A>
__global__ void lk_rope_parts(lk_rope_side a, lk_rope_side b, const float* __restrict__ cs, const float* __restrict__ sn,
                              const int* __restrict__ pos, int hd, int half, int seqs = 0){
  __shared__ A red[32];
  __shared__ float hv[1024];
  // Each side by its own call, as `lk_gemv_parts_rope` turns them: a side chosen at run time puts the by-value
  // `lk_rope_side` in a local frame.
  if((int)blockIdx.x < a.heads) lk_rope_head<NORM, A, false>(a, (int)blockIdx.x, cs, sn, pos, hd, half, seqs, red, hv);
  else lk_rope_head<NORM, A, false>(b, (int)blockIdx.x - a.heads, cs, sn, pos, hd, half, seqs, red, hv);
}
"#;

/// `lk_gemv_parts`, then each head of a rotation that reads one of its parts turned by the block that finishes the head:
/// a step's projections of one activation with their heads' norms and rotations in the epilogue (`qkv_head_epilogue`).
/// Each warp computes its `W` rows as `lk_gemv_parts` does and writes them; each block then counts in, at the head's
/// count in `cnt`, the rows it wrote of each head a rotation reads — side `a` reads part `pa`, `b` part `pb` (−1: no
/// side) — and the block whose rows complete a head reads it back through L2 and turns it as `lk_rope_parts` turns it,
/// then resets the count. A block is at least a normed head wide, as the rotation's own block is.
const GEMV_PARTS_ROPE: &str = r#"
// A 16-bit float matrix's launch names `NT`, the threads its blocks launch with: the registers a thread may take, and so
// how many blocks of its weights the dots keep in flight; every other format's blocks take up to 1024.
template<int F, int Q, int M, int W, bool NORM, typename A, int NT = 1024>
__global__ void __launch_bounds__((F == 1 || F == 30) ? NT : 1024) lk_gemv_parts_rope(const float* __restrict__ x, const signed char* __restrict__ xq,
                              const signed char* __restrict__ xl, const float* __restrict__ xs, int K, int N,
                              lk_parts parts, lk_rope_side a, lk_rope_side b, int pa, int pb,
                              const float* __restrict__ cs, const float* __restrict__ sn, const int* __restrict__ pos,
                              int hd, int half, int* __restrict__ cnt){
  __shared__ A red[32];
  __shared__ float hv[1024];
  __shared__ int done;
  const long rows = (long)(blockDim.x >> 5) * W, first = (long)blockIdx.x * rows, last = min(first + rows, (long)N);
  const long o = first + (long)(threadIdx.x >> 5) * W;
  if(o < N){
    int p = 0;
    while(p < LK_PARTS - 1 && o >= parts.end[p]) ++p;
    long start = p ? parts.end[p - 1] : 0, r = o - start;
    long n = parts.end[p] - start;
    float d[W * M];
    lk_gemv_dot<F, Q, M, W, 8, (NT <= 256 ? LK_H16_AHEAD : 2)>(parts.w[p], r, K, x, xq, xl, xs, d);
    lk_out yo = LK_ROW(lk_part_out(parts, p), parts.row[p], parts.pitch[p]);
    const lk_epi b = parts.bias[p];
    for(int j = threadIdx.x & 31; j < W * M; j += 32){
      const int i = j / M, t = j % M;
      yo[(long)t * n + r + i] = b(lk_pick(d, j), t, r + i, n);
    }
  }
  __threadfence();
  __syncthreads();
  for(int side = 0; side < 2; ++side){
    const int pp = side ? pb : pa;
    if(pp < 0) continue;
    const long start = pp ? parts.end[pp - 1] : 0, lo = max(first, start), hi = min(last, (long)parts.end[pp]);
    for(long at = lo; at < hi;){
      const int h = (int)((at - start) / hd);
      const long to = min(hi, start + (long)(h + 1) * hd);
      const int head = (side ? a.heads : 0) + h, took = (int)(to - at);
      if(threadIdx.x == 0) done = atomicAdd(cnt + head, took) + took == hd;
      __syncthreads();
      if(done){
        __threadfence();
        // Each side by its own call: a side chosen at run time puts the by-value `lk_rope_side` in a local frame,
        // and every field the head's loops read from it goes to local memory.
        if(side) lk_rope_head<NORM, A, true>(b, h, cs, sn, pos, hd, half, 0, red, hv);
        else lk_rope_head<NORM, A, true>(a, h, cs, sn, pos, hd, half, 0, red, hv);
        if(threadIdx.x == 0) cnt[head] = 0;
      }
      __syncthreads();
      at = to;
    }
  }
}
"#;

/// `lk_gemv_parts` over a step's one row with a gated delta layer's inputs in its epilogue (`gdn_input_epilogue`):
/// each warp computes and writes its `W` rows as `lk_gemv_parts` does, each row then carried on by a lane of its own — a
/// row of a convolution's part through its taps and the SiLU into the convolution's output (as `lk_conv_tap` sums
/// them, the ring advanced in place), a row of a gate's part into its gate (`lk_act<1>`, `lk_forget`), the operands
/// read in flight while the rows' dots stream — at `LK_GDN_OWN` rows a warp and more into each row's lane's registers,
/// below them into shared memory by the warp's idle lanes; each block counts in, at the head's count
/// in `cnt`, the rows it wrote of each head a normed convolution makes, and the block whose rows complete a head reads
/// it back through L2 and norms it (`lk_l2_unit`, a block at least the head wide), then resets the count. Gates
/// projected from the `f32` activation instead take the launch's last `gb` blocks, each one block of `lk_gate_proj`
/// (`lk_gate_heads`). Every value is computed as the launches apart compute it.
const GEMV_GDN: &str = r#"
// Four bytes from global memory into shared memory, in flight until the copies are waited on.
__device__ __forceinline__ void lk_cp4(void* smem, const void* gmem){
  unsigned s = (unsigned)__cvta_generic_to_shared(smem);
  asm volatile("cp.async.ca.shared.global [%0], [%1], 4;\n" :: "r"(s), "l"(gmem));
}
// The most taps a convolution of `lk_gemv_gdn` takes, and the operands a row's epilogue reads: its ring's `T − 1`
// values, its `T` taps, its decay's `dt` and `al`.
#define LK_GDN_TAPS 4
#define LK_GDN_OPS (2 * LK_GDN_TAPS + 1)
// The rows a warp of `lk_gemv_gdn` from which each row's own lane holds its operands in registers; below them the
// warp's idle lanes copy every row's into shared memory.
#define LK_GDN_OWN 8
// A convolution of `lk_gemv_gdn`: the part of the GEMV it reads (−1: none), its weights, ring and output, and whether
// each head of its output is normed.
struct lk_gdn_conv { int part; const float* wc; float* cs; float* y; int l2; };
// A gated delta layer's inputs past its projections: its convolutions (`T` taps, normed heads `hd` wide with `eps`),
// and its `heads` gates — from parts `pb` and `pa` of the GEMV, or (`pb` −1) from the `f32` activation `xf` by the
// projections `wb` and `wa` of `K`, `parts` warps a head, in the launch's last `gb` blocks.
struct lk_gdn {
  lk_gdn_conv c[3];
  int T, hd;
  float eps;
  int pb, pa, gb, K, parts, heads;
  const float* wb; const float* wa; const float* xf; const float* dt; const float* al; float* beta; float* decay;
};
// `NT` as `lk_gemv_parts_rope`'s.
template<int F, int Q, int W, typename A, int NT = 1024>
__global__ void __launch_bounds__((F == 1 || F == 30) ? NT : 1024) lk_gemv_gdn(const float* __restrict__ x, const signed char* __restrict__ xq,
                              const signed char* __restrict__ xl, const float* __restrict__ xs, int K, int N,
                              lk_parts parts, lk_gdn g, const int* __restrict__ pos, int* __restrict__ cnt){
  __shared__ A red[32];
  __shared__ int done;
  constexpr bool OWN = W >= LK_GDN_OWN;
  // Each warp's rows' operands, copied by its idle lanes.
  __shared__ float lk_ops[OWN ? 1 : 32][OWN ? 1 : W][LK_GDN_OPS];
  const int gemv = (int)gridDim.x - g.gb;
  if((int)blockIdx.x >= gemv){
    lk_gate_heads((int)blockIdx.x - gemv, g.wb, g.wa, g.xf, g.dt, g.al, g.beta, g.decay, g.K, g.heads, g.parts);
    return;
  }
  const int q = *pos - 1;
  const long rows = (long)(blockDim.x >> 5) * W, first = (long)blockIdx.x * rows, last = min(first + rows, (long)N);
  const long o = first + (long)(threadIdx.x >> 5) * W;
  if(o < N){
    int p = 0;
    while(p < LK_PARTS - 1 && o >= parts.end[p]) ++p;
    const long start = p ? parts.end[p - 1] : 0, r = o - start;
    const int C = (int)(parts.end[p] - start);
    // Lane `i` carries the warp's row `i`: its convolution's ring values before this position and its taps, or its
    // decay's `dt` and `al`, read while the rows' dots stream — into its registers at `LK_GDN_OWN` rows a warp and
    // more, below them into shared memory by the warp's lanes 1–3 (the ring), 4–7 (the taps), 8 and 9 (`dt`, `al`).
    const int i = threadIdx.x & 31, c = (int)r + i, warp = threadIdx.x >> 5;
    const bool mine = i < W;
    int conv = -1;
    #pragma unroll
    for(int j = 0; j < 3; ++j) if(g.c[j].part == p) conv = j;
    float ring[LK_GDN_TAPS - 1], tap[LK_GDN_TAPS], dt = 0.f, al = 0.f;
    if constexpr(OWN){
      #pragma unroll
      for(int t = 0; t < LK_GDN_TAPS - 1; ++t)
        ring[t] = mine && conv >= 0 && t < g.T - 1 ? g.c[conv].cs[(long)lk_slot(q - (g.T - 1) + t, g.T) * C + c] : 0.f;
      #pragma unroll
      for(int t = 0; t < LK_GDN_TAPS; ++t) tap[t] = mine && conv >= 0 && t < g.T ? g.c[conv].wc[(long)t * C + c] : 0.f;
      if(mine && p == g.pa){ dt = g.dt[c]; al = g.al[c]; }
    } else {
      #pragma unroll
      for(int w = 0; w < W; ++w){
        const int cw = (int)r + w;
        const float* src = nullptr;
        int at = 0;
        if(conv >= 0 && i >= 1 && i < g.T){ src = g.c[conv].cs + (long)lk_slot(q - (g.T - 1) + (i - 1), g.T) * C + cw; at = i - 1; }
        else if(conv >= 0 && i >= LK_GDN_TAPS && i < LK_GDN_TAPS + g.T){ src = g.c[conv].wc + (long)(i - LK_GDN_TAPS) * C + cw; at = i - 1; }
        else if(p == g.pa && i == 2 * LK_GDN_TAPS){ src = g.dt + cw; at = 2 * LK_GDN_TAPS - 1; }
        else if(p == g.pa && i == 2 * LK_GDN_TAPS + 1){ src = g.al + cw; at = 2 * LK_GDN_TAPS; }
        if(src) lk_cp4(&lk_ops[warp][w][at], src);
      }
      asm volatile("cp.async.commit_group;\n");
    }
    float d[W];
    lk_gemv_dot<F, Q, 1, W, 8, (NT <= 256 ? LK_H16_AHEAD : 2)>(parts.w[p], r, K, x, xq, xl, xs, d);
    if constexpr(!OWN){
      asm volatile("cp.async.wait_all;\n");
      __syncwarp();
      if(mine){
        const float* op = lk_ops[warp][i];
        #pragma unroll
        for(int t = 0; t < LK_GDN_TAPS - 1; ++t) ring[t] = op[t];
        #pragma unroll
        for(int t = 0; t < LK_GDN_TAPS; ++t) tap[t] = op[LK_GDN_TAPS - 1 + t];
        dt = op[2 * LK_GDN_TAPS - 1]; al = op[2 * LK_GDN_TAPS];
      }
    }
    if(mine){
      lk_out yo = LK_ROW(lk_part_out(parts, p), parts.row[p], parts.pitch[p]);
      const float* b = parts.bias[p].b;
      const float di = lk_pick(d, i), v = b ? di + b[c] : di;
      yo[c] = v;
      if(conv >= 0){
        // The taps before this position, oldest first, then this position's, summed as `lk_conv_tap` sums them.
        g.c[conv].cs[(long)lk_slot(q, g.T) * C + c] = v;
        A sum = 0;
        #pragma unroll
        for(int t = 0; t < LK_GDN_TAPS - 1; ++t) if(t < g.T - 1) sum = lk_fma((A)ring[t], (A)tap[t], sum);
        sum = lk_fma((A)v, (A)lk_pick(tap, g.T - 1), sum);
        g.c[conv].y[c] = lk_act<0>((float)sum);
      }
      if(p == g.pb) g.beta[c] = lk_act<1>(v);
      if(p == g.pa) g.decay[c] = lk_forget(v, dt, al);
    }
  }
  // Only a block holding rows of a normed head goes on to the heads' count: its writers' rows made visible first.
  bool normed = false;
  #pragma unroll
  for(int j = 0; j < 3; ++j){
    if(!g.c[j].l2) continue;
    const int pp = g.c[j].part;
    const long start = pp ? parts.end[pp - 1] : 0;
    normed = normed || max(first, start) < min(last, (long)parts.end[pp]);
  }
  if(!normed) return;
  if((threadIdx.x & 31) == 0) __threadfence();
  __syncthreads();
  int head = 0;
  #pragma unroll
  for(int j = 0; j < 3; ++j){
    if(!g.c[j].l2) continue;
    const int pp = g.c[j].part;
    const long start = pp ? parts.end[pp - 1] : 0, end = parts.end[pp], lo = max(first, start), hi = min(last, end);
    for(long at = lo; at < hi;){
      const int h = (int)((at - start) / g.hd);
      const long to = min(hi, start + (long)(h + 1) * g.hd);
      if(threadIdx.x == 0) done = atomicAdd(cnt + head + h, (int)(to - at)) + (int)(to - at) == g.hd;
      __syncthreads();
      if(done){
        __threadfence();
        float* yh = g.c[j].y + (long)h * g.hd;
        const bool live = (int)threadIdx.x < g.hd;
        const float v = lk_l2_unit<A>(live ? __ldcg(yh + threadIdx.x) : 0.f, live, g.eps, red);
        if(live) yh[threadIdx.x] = v;
        if(threadIdx.x == 0) cnt[head + h] = 0;
      }
      __syncthreads();
      at = to;
    }
    head += (int)((end - start) / g.hd);
  }
}
"#;

/// Rows `*idx … *idx + count − 1` (1-based; from row `fixed` with no index) of a table of rows `cols` wide, overwritten
/// by the `count` rows of `src`.
const SET_ROW: &str = r#"
__global__ void lk_set_row(lk_out t, const int* __restrict__ idx, int fixed, const float* __restrict__ src,
                           long n, int cols, long tz = 0){
  long c = (long)blockIdx.x * blockDim.x + threadIdx.x; if(c >= n) return;
  // A batch's rows (`tz` > 0): row `r` of `src` into its own table, `tz` values on from the last's, at its own index.
  if(tz){ long r = c / cols; t[r * tz + (long)(idx[r] - 1) * cols + c % cols] = src[c]; return; }
  t[(long)((idx ? *idx : fixed) - 1) * cols + c] = src[c];
}
// The rows' quantized form, as `lk_set_row` sets their values: each value's words — its low word too under two words —
// and each block's scale, from its first value. Rows of whole 32-value blocks, so a block's place is its own.
__global__ void lk_set_row_q(signed char* __restrict__ tq, signed char* __restrict__ tl, float* __restrict__ ts,
                             const int* __restrict__ idx, int fixed, const signed char* __restrict__ sq,
                             const signed char* __restrict__ sl, const float* __restrict__ ss, long n, int cols, long tz = 0){
  long c = (long)blockIdx.x * blockDim.x + threadIdx.x; if(c >= n) return;
  long d = tz ? (c / cols) * tz + (long)(idx[c / cols] - 1) * cols + c % cols : (long)((idx ? *idx : fixed) - 1) * cols + c;
  tq[d] = sq[c];
  if(tl) tl[d] = sl[c];
  if((c & 31) == 0) ts[d >> 5] = ss[c >> 5];
}
"#;

/// GQA attention split over the attended span. A block takes `G` query heads of one key/value head together (`h0 / group`,
/// so every key and value row it reads serves all `G`) and one query row: query row `r` attends the first `*pos + r`
/// cached rows (all `len` with no position), or under a window `win` the last `win` of them, cut into `pe` chunks of
/// at least `minc` positions and no more than `splits` — a function of the row's span alone, so a row's arithmetic is
/// the same in every form. A chunk scores its positions `LK_TILE` at a time, a warp `KB` keys at once (`(q·k)·scale`,
/// soft-capped at `cap` where it is not 0 — `lk_cap`, as every attention kernel caps its scores — keys read four values
/// at a time), with a running maximum and sum, each tile's values weighed in: a thread weighs the
/// value slots `tid + 256·k` of the block's `G·vd/2` (head `s / (vd/2)`'s pair `s mod (vd/2)` for slot `s`, at most
/// `G·VPT` a thread), a head's values read `vw` apart from `voff` — a latent cache's latent where it lies. With a grid
/// of `splits` blocks a row (the decode step), each chunk is a block that writes its heads' maxima, sums and weighed
/// values to `part`, and the last to arrive (`cnt`, reset by it) folds them (`lk_attn_fold`, then
/// `lk_attn_fold_values`, each thread folding the slots it weighed); with one (a batch's sequence or a prefill chunk,
/// whose rows already fill the chip), the block takes every chunk of its row and folds them alike — each once, kept in
/// its shared memory as a split block keeps it in `part`, when the launch gives it room for them (`held`,
/// `attn_holds`); otherwise their maxima and sums first, then their values. One chunk's
/// heads are written at once; head `h`'s sink `sinks[h]` joins the maximum and the denominator when there are sinks. A
/// key row is `kvw` wide, a head `hd`. Every sum — the scores' dots, the softmax's sums, the weighed values, the chunks' partials and their
/// fold — runs in the accumulator type `A`: `double` under the precise numerics, whose contract is `f32` values with
/// `f64` sums, `float` otherwise.
const ATTENTION: &str = r#"
#define LK_TILE 64
// The attention's sums in the accumulator type `A`: `float`, or `double` for the precise numerics.
template<typename A> struct lk_pair { A x, y; };
__device__ __forceinline__ float lk_exp(float v){ return expf(v); }
__device__ __forceinline__ double lk_exp(double v){ return exp(v); }
__device__ __forceinline__ float lk_wsum(float v){ return lk_warp_sum(v); }
__device__ __forceinline__ double lk_wsum(double v){ return lk_warp_sum_d(v); }
__device__ __forceinline__ float lk_wmax(float v){ return lk_warp_max(v); }
__device__ __forceinline__ double lk_wmax(double v){
  for(int o = 16; o > 0; o >>= 1) v = fmax(v, __shfl_down_sync(0xffffffffu, v, o));
  return __shfl_sync(0xffffffffu, v, 0);
}
// The `V` values (a power of two) each lane holds, each summed over a run of `D·2` lanes as `lk_wsum` sums a value over
// the warp — the lanes paired across `D`, then `D/2`, … then 1: while a lane holds more than one value it keeps one half
// (the upper where its bit `D` is set) and sends the other, then the one left is summed with its partner's. Value
// `base + i` lands at `v[i]`, `base` the halves a lane kept.
template<int D, int V, typename A, int N>
__device__ __forceinline__ void lk_tree_halves(A (&v)[N], int lane, int& base){
  if constexpr(D >= 1){
    if constexpr(V > 1){
      const bool up = lane & D;
      #pragma unroll
      for(int i = 0; i < V / 2; ++i){
        const A send = up ? v[i] : v[i + V / 2], keep = up ? v[i + V / 2] : v[i];
        v[i] = keep + __shfl_xor_sync(0xffffffffu, send, D);
      }
      if(up) base += V / 2;
      lk_tree_halves<D / 2, V / 2>(v, lane, base);
    } else {
      v[0] = v[0] + __shfl_xor_sync(0xffffffffu, v[0], D);
      lk_tree_halves<D / 2, 1>(v, lane, base);
    }
  }
}
template<int V> __host__ __device__ constexpr int lk_pow2_at_least(){ int p = 1; while(p < V) p *= 2; return p; }
// A tile's `n` keys scored for a block's `G` heads, run `R` lanes a key: a warp `KB` keys a run at once, `32 / R` runs a
// warp. A lane sums its share of the head (fours `lane mod R`, `+ R`, …), the run's lanes past the head zeros; the lanes
// past the run add as zeros, then each run sums its keys' dots as `lk_tree_halves` sums them — the operations
// `lk_wsum` takes over a key's dot with the head in lanes `0 … R − 1`. Key `j` reads cache row `row(j)` (−1: none); its
// head `g`'s sum goes to `emit(j, g, s)`, once. Which of a run's keys read a row is a mask (`ok`): the keys a lane
// emits are known only at run time, and an array indexed by them would live in local memory.
template<int G, typename A, int R, typename KE, typename Row, typename Emit>
__device__ __forceinline__ void lk_attn_scores_in(const float* qs, const KE* __restrict__ kc, int n, int hd, int kvw, long koff,
                                                  Row row, Emit emit){
  constexpr int KB = G <= 2 ? 4 : (G <= 4 ? 2 : 1), S = KB * (32 / R), NV = KB * G, V = lk_pow2_at_least<NV>();
  constexpr int H = V < R ? V : R;
  const int lane = threadIdx.x & 31, warp = threadIdx.x >> 5, nw = blockDim.x >> 5, h4 = hd >> 2;
  const int run = lane / R, at = lane % R;
  const float4* q4 = (const float4*)qs;
  for(int i = warp; i < n; i += nw * S){
    int rw[KB], ok = 0;
    #pragma unroll
    for(int b = 0; b < KB; ++b){ rw[b] = row(i + (run * KB + b) * nw); ok |= (rw[b] >= 0) << b; }
    A d[V];
    #pragma unroll
    for(int v = 0; v < V; ++v) d[v] = 0;
    for(int e = at; e < h4; e += R){
      float4 k[KB];
      #pragma unroll
      for(int b = 0; b < KB; ++b) k[b] = rw[b] >= 0 ? lk_from4(kc + (long)rw[b] * kvw + koff + 4 * e) : make_float4(0.f, 0.f, 0.f, 0.f);
      #pragma unroll
      for(int g = 0; g < G; ++g){
        float4 a = q4[g * h4 + e];
        #pragma unroll
        for(int b = 0; b < KB; ++b){
          A& s = d[b * G + g];
          s = lk_fma((A)a.x, (A)k[b].x, s); s = lk_fma((A)a.y, (A)k[b].y, s);
          s = lk_fma((A)a.z, (A)k[b].z, s); s = lk_fma((A)a.w, (A)k[b].w, s);
        }
      }
    }
    if constexpr(R < 32){
      #pragma unroll
      for(int v = 0; v < V; ++v) d[v] = lk_add(d[v], (A)0);
    }
    int base = 0;
    lk_tree_halves<R / 2, V>(d, lane, base);
    // A run's lanes below the last distance halved at hold every value; the first of them emits it.
    constexpr int LAST = R / H;
    if(at % LAST == 0){
      #pragma unroll
      for(int v = 0; v < V / H; ++v){
        const int idx = base + v;
        if(idx < NV){
          const int b = idx / G, g = idx - b * G;
          if(ok >> b & 1) emit(i + (run * KB + b) * nw, g, d[v]);
        }
      }
    }
  }
}
// The lanes a key's dot takes (`lk_attn_scores_in`'s `R`) for a head of `hd`: a half warp for a head of 9 to 16 fours
// — two keys a run at once — and the whole warp otherwise. A launch takes it once (`lk_attention`), so each run's body
// keeps its registers.
__host__ __device__ constexpr int lk_attn_run_width(int hd){ return hd / 4 > 8 && hd / 4 <= 16 ? 16 : 32; }
template<int G, typename A, typename KE, typename Row, typename Emit>
__device__ __forceinline__ void lk_attn_scores(const float* qs, const KE* __restrict__ kc, int n, int hd, int kvw, long koff,
                                               Row row, Emit emit){
  if(lk_attn_run_width(hd) == 16) lk_attn_scores_in<G, A, 16>(qs, kc, n, hd, kvw, koff, row, emit);
  else lk_attn_scores_in<G, A, 32>(qs, kc, n, hd, kvw, koff, row, emit);
}
// Eight consecutive elements from `p` as `f32`: one sixteen-byte load of eight binary16 values, two of four otherwise.
template<typename E> __device__ __forceinline__ void lk_from8(const E* p, float* r){
  const float4 a = lk_from4(p), b = lk_from4(p + 4);
  r[0] = a.x; r[1] = a.y; r[2] = a.z; r[3] = a.w; r[4] = b.x; r[5] = b.y; r[6] = b.z; r[7] = b.w;
}
__device__ __forceinline__ void lk_from8(const __half* p, float* r){
  const uint4 u = *(const uint4*)p;
  const float2 a = __half22float2(*(const __half2*)&u.x), b = __half22float2(*(const __half2*)&u.y);
  const float2 c = __half22float2(*(const __half2*)&u.z), d = __half22float2(*(const __half2*)&u.w);
  r[0] = a.x; r[1] = a.y; r[2] = b.x; r[3] = b.y; r[4] = c.x; r[5] = c.y; r[6] = d.x; r[7] = d.y;
}
// `DK` values of a row at `p` as floats: a lane's share of a key's or a value's row.
template<int DK, typename E> __device__ __forceinline__ void lk_lane_row(const E* __restrict__ p, float (&r)[DK]){
  if constexpr(DK == 2){
    const float2 v = lk_from2(p); r[0] = v.x; r[1] = v.y;
  } else if constexpr(DK == 4){
    const float4 v = lk_from4(p); r[0] = v.x; r[1] = v.y; r[2] = v.z; r[3] = v.w;
  } else {
    #pragma unroll
    for(int i = 0; i < DK / 8; ++i) lk_from8(p + 8 * i, r + 8 * i);
  }
}
// The values of a key a lane of a warp-local block takes (`lk_attn_warps`): eight — one sixteen-byte load of a
// binary16 row — or four or two, the most whose `g` heads' weighed values in accumulators of `acc` bytes stay within 32
// registers, and never under a warp's share of a head `hd` wide; none where even that share does not fit.
__host__ __device__ constexpr int lk_attn_lane_values(int g, int hd, int acc){
  return g * 8 * acc <= 128 ? 8 : g * 4 * acc <= 128 && hd <= 128 ? 4 : g * 2 * acc <= 128 && hd <= 64 ? 2 : 0;
}
// Whether a block of `g` heads `hd` wide summing in `acc`-byte accumulators takes its chunks warp-local
// (`lk_attn_warps`): `f32` sums, at most eight heads, 64, 128 or 256 wide, a lane's share of their weighed values within
// its registers (`lk_attn_lane_values`). `f64` sums take the tile chunks: every lane of a run repeats its heads' `f64`
// arithmetic, which on a chip of a few `f64` units a multiprocessor took 1.6 to 3.6 times the tile chunks' time.
__host__ __device__ constexpr bool lk_attn_warp_heads(int g, int hd, int acc){
  return acc == 4 && g <= 8 && (hd == 64 || hd == 128 || hd == 256) && lk_attn_lane_values(g, hd, acc) > 0;
}
// Whether the block's chunks over these caches take that path: its values as wide as its keys, on caches whose every
// row it reads starts in step with a lane's widest load — eight elements, the cache's start at the bytes of eight
// elements up to sixteen (`lk_lane_row_align`), so a batch's sequences, their caches a multiple of eight elements
// apart, take their chunks as their decode steps do.
template<typename E> __host__ __device__ constexpr int lk_lane_row_align(){ return 8 * (int)sizeof(E) < 16 ? 8 * (int)sizeof(E) : 16; }
template<typename A, typename KE, typename VE>
__device__ __forceinline__ bool lk_attn_by_warps(int g, int hd, int vd, const KE* kc, const VE* vc, int kvw, int vw, long koff, long vo){
  return lk_attn_warp_heads(g, hd, (int)sizeof(A)) && vd == hd && ((kvw | vw | koff | vo) & 7) == 0
      && ((size_t)kc & (lk_lane_row_align<KE>() - 1)) == 0 && ((size_t)vc & (lk_lane_row_align<VE>() - 1)) == 0;
}
// The heads whose values a warp-local block's warps combine at once (`lk_attn_warps`): at most 512 values a warp.
__host__ __device__ constexpr int lk_attn_warp_pass(int g, int hd){ return 512 / hd < 1 ? 1 : 512 / hd < g ? 512 / hd : g; }
// The values a warp-local block combines its warps' partials in: each of its eight warps' maxima and sums, then each
// warp's values of a pass of heads.
__host__ __device__ constexpr int lk_attn_warp_slots(int g, int hd, int acc){
  return lk_attn_warp_heads(g, hd, acc) ? 16 * g + 8 * lk_attn_warp_pass(g, hd) * hd : 0;
}
// One chunk `[j0, j1)` of a block's `G` heads `HD` wide, warp-local: each run of `HD / DK` lanes takes a contiguous
// run of the chunk's keys, `KB` a step, each lane `DK` consecutive values of a key (`lk_attn_lane_values`) — so a warp
// takes `32 · DK / HD` keys at once. A key's dot sums across its run by a butterfly — every lane of the run the same
// sum, IEEE addition commuting — scaled and capped as the tile chunk takes it; each run's running maximum, sum and
// weighed values stay in registers, no barrier in the loop. A warp's runs fold by a butterfly over the runs; then the
// warps' partials fold in warp order through `wb`. The chunk's maxima and sums go to `mx` and `sum` (head `g`'s at
// `g · ms`), its values to each thread's slots `acc` as `lk_attn_chunk` leaves them. Every form a row takes computes
// its chunks so.
template<int G, int VPT, typename A, bool VALUES, int HD, typename KE, typename VE>
__device__ __forceinline__ void lk_attn_warps(const float* qs, A* wb, A* mx, A* sum, int ms, const KE* __restrict__ kc,
                                              const VE* __restrict__ vc, int j0, int j1, int kvw, int vw, long koff,
                                              long vo, A scale, A cap, lk_pair<A> (&acc)[G * VPT]){
  constexpr int DK = lk_attn_lane_values(G, HD, (int)sizeof(A)), R = HD / DK, S = 32 / R, KB = (DK >= 8 ? 2 : 4) * G <= 16 ? (DK >= 8 ? 2 : 4) : 16 / G < 1 ? 1 : 16 / G;
  const int lane = threadIdx.x & 31, warp = threadIdx.x >> 5, nw = blockDim.x >> 5, sub = lane % R;
  const int runs = nw * S, per = (j1 - j0 + runs - 1) / runs;
  const int kb = min(j0 + (warp * S + lane / R) * per, j1), ke = min(kb + per, j1);
  // The block's queries are in place (`qs`, staged by every thread before the chunk).
  __syncthreads();
  float q[G][DK];
  #pragma unroll
  for(int g = 0; g < G; ++g){
    #pragma unroll
    for(int d = 0; d < DK; ++d) q[g][d] = qs[g * HD + sub * DK + d];
  }
  A m[G], l[G], a[G][DK];
  #pragma unroll
  for(int g = 0; g < G; ++g){
    m[g] = -INFINITY; l[g] = 0;
    #pragma unroll
    for(int d = 0; d < DK; ++d) a[g][d] = 0;
  }
  // Every run of a warp takes as many steps, a run's keys past its own masked: its lanes shuffle together.
  for(int t = 0; t < per; t += KB){
    const int j = kb + t;
    float kf[KB][DK], vf[VALUES ? KB : 1][DK];
    #pragma unroll
    for(int b = 0; b < KB; ++b){
      const long row = j + b < ke ? j + b : j0;
      lk_lane_row<DK>(kc + row * kvw + koff + sub * DK, kf[b]);
      if constexpr(VALUES) lk_lane_row<DK>(vc + row * vw + vo + sub * DK, vf[b]);
    }
    A sc[KB][G];
    #pragma unroll
    for(int b = 0; b < KB; ++b){
      #pragma unroll
      for(int g = 0; g < G; ++g){
        A x = 0;
        #pragma unroll
        for(int d = 0; d < DK; ++d) x = lk_fma((A)q[g][d], (A)kf[b][d], x);
        sc[b][g] = x;
      }
    }
    #pragma unroll
    for(int o = R / 2; o > 0; o >>= 1){
      #pragma unroll
      for(int b = 0; b < KB; ++b){
        #pragma unroll
        for(int g = 0; g < G; ++g) sc[b][g] = lk_add(sc[b][g], __shfl_xor_sync(0xffffffffu, sc[b][g], o));
      }
    }
    #pragma unroll
    for(int g = 0; g < G; ++g){
      A mn = m[g], p[KB];
      #pragma unroll
      for(int b = 0; b < KB; ++b){
        sc[b][g] = j + b < ke ? lk_cap(sc[b][g] * scale, cap) : (A)-INFINITY;
        mn = fmax(mn, sc[b][g]);
      }
      const A c = m[g] == -INFINITY ? (A)0 : lk_exp(m[g] - mn);
      A ps = 0;
      #pragma unroll
      for(int b = 0; b < KB; ++b){
        p[b] = sc[b][g] == -INFINITY ? (A)0 : lk_exp(sc[b][g] - mn);
        ps = lk_add(ps, p[b]);
      }
      l[g] = lk_fma(l[g], c, ps); m[g] = mn;
      if constexpr(VALUES){
        #pragma unroll
        for(int d = 0; d < DK; ++d){
          A v = lk_mul(a[g][d], c);
          #pragma unroll
          for(int b = 0; b < KB; ++b) v = lk_fma(p[b], (A)vf[b][d], v);
          a[g][d] = v;
        }
      }
    }
  }
  // A warp's runs folded: each head's maximum over them, then each run's sum and values under it summed across them.
  #pragma unroll
  for(int g = 0; g < G; ++g){
    A mw = m[g];
    #pragma unroll
    for(int o = R; o < 32; o <<= 1) mw = fmax(mw, __shfl_xor_sync(0xffffffffu, mw, o));
    const A c = m[g] == -INFINITY ? (A)0 : lk_exp(m[g] - mw);
    A lw = lk_mul(l[g], c);
    #pragma unroll
    for(int o = R; o < 32; o <<= 1) lw = lk_add(lw, __shfl_xor_sync(0xffffffffu, lw, o));
    if constexpr(VALUES){
      #pragma unroll
      for(int d = 0; d < DK; ++d){
        A v = lk_mul(a[g][d], c);
        #pragma unroll
        for(int o = R; o < 32; o <<= 1) v = lk_add(v, __shfl_xor_sync(0xffffffffu, v, o));
        a[g][d] = v;
      }
    }
    m[g] = mw; l[g] = lw;
  }
  // The warps' partials side by side — warp `w`'s maxima and sums at `w · G` — and their values a pass of `GP` heads at
  // a time, warp `w`'s head `g` at `(w · GP + g mod GP) · HD`: each thread's value pairs folded over the warps in warp
  // order under the heads' maxima.
  constexpr int GP = lk_attn_warp_pass(G, HD), NP = (G + GP - 1) / GP, ph = HD >> 1;
  A* wm = wb;
  A* wl = wb + 8 * G;
  A* wv = wb + 16 * G;
  if(lane == 0){
    #pragma unroll
    for(int g = 0; g < G; ++g){ wm[warp * G + g] = m[g]; wl[warp * G + g] = l[g]; }
  }
  if constexpr(VALUES){
    #pragma unroll
    for(int k = 0; k < G * VPT; ++k) acc[k] = lk_pair<A>{0, 0};
  }
  #pragma unroll
  for(int pass = 0; pass < NP; ++pass){
    if constexpr(VALUES){
      if(lane < R){
        #pragma unroll
        for(int g = pass * GP; g < (pass + 1) * GP && g < G; ++g){
          #pragma unroll
          for(int d = 0; d < DK; ++d) wv[(warp * GP + g - pass * GP) * HD + sub * DK + d] = a[g][d];
        }
      }
    }
    __syncthreads();
    if(pass == 0 && threadIdx.x < G){
      const int g = threadIdx.x;
      A M = -INFINITY, total = 0;
      for(int w = 0; w < nw; ++w) M = fmax(M, wm[w * G + g]);
      for(int w = 0; w < nw; ++w){
        if(wm[w * G + g] > -INFINITY) total = lk_fma(wl[w * G + g], lk_exp(wm[w * G + g] - M), total);
      }
      mx[g * ms] = M; sum[g * ms] = total;
    }
    if constexpr(VALUES){
      #pragma unroll
      for(int k = 0; k < G * VPT; ++k){
        const int s = threadIdx.x + k * blockDim.x, g = s / ph, e = s - g * ph;
        if(s < G * ph && g / GP == pass){
          A M = -INFINITY;
          for(int w = 0; w < nw; ++w) M = fmax(M, wm[w * G + g]);
          lk_pair<A> x{0, 0};
          for(int w = 0; w < nw; ++w){
            const A c = wm[w * G + g] == -INFINITY ? (A)0 : lk_exp(wm[w * G + g] - M);
            const A* v = wv + (w * GP + g - pass * GP) * HD + 2 * e;
            x = w == 0 ? lk_pair<A>{lk_mul(v[0], c), lk_mul(v[1], c)} : lk_pair<A>{lk_fma(v[0], c, x.x), lk_fma(v[1], c, x.y)};
          }
          acc[k] = x;
        }
      }
    }
    __syncthreads();
  }
}
// A chunk warp-local at its head width (`lk_attn_by_warps`): 64 at a key's run of 16 lanes (`lk_attn_run_width`), 128
// or 256 at 32 — so a launch compiles only the widths its run can be, and of those only the ones its heads' lanes hold
// (`lk_attn_warp_heads`).
template<int G, int VPT, typename A, bool VALUES, int R, typename KE, typename VE>
__device__ __forceinline__ void lk_attn_warps_at(const float* qs, A* wb, A* mx, A* sum, int ms, const KE* __restrict__ kc,
                                                 const VE* __restrict__ vc, int j0, int j1, int hd, int kvw, int vw, long koff,
                                                 long vo, A scale, A cap, lk_pair<A> (&acc)[G * VPT]){
  constexpr int AB = (int)sizeof(A);
  if constexpr(R == 16){
    if constexpr(lk_attn_warp_heads(G, 64, AB)) lk_attn_warps<G, VPT, A, VALUES, 64>(qs, wb, mx, sum, ms, kc, vc, j0, j1, kvw, vw, koff, vo, scale, cap, acc);
  } else {
    if(hd == 128){
      if constexpr(lk_attn_warp_heads(G, 128, AB)) lk_attn_warps<G, VPT, A, VALUES, 128>(qs, wb, mx, sum, ms, kc, vc, j0, j1, kvw, vw, koff, vo, scale, cap, acc);
    } else {
      if constexpr(lk_attn_warp_heads(G, 256, AB)) lk_attn_warps<G, VPT, A, VALUES, 256>(qs, wb, mx, sum, ms, kc, vc, j0, j1, kvw, vw, koff, vo, scale, cap, acc);
    }
  }
}
// One chunk `[j0, j1)` of a block's `G` heads: each head's running maximum `mx` and sum `sum`, and with `VALUES` each
// thread's value slots weighed in `acc` — warp-local where the heads fit (`lk_attn_by_warps`, through `wb`) in a kernel
// that takes them (`W`), else a tile of keys at a time.
template<int G, int VPT, typename A, bool VALUES, int R, bool W, typename KE, typename VE>
__device__ __forceinline__ void lk_attn_chunk(const float* qs, A* sc, A* wb, A* mx, A* sum, A* alpha,
                                              const KE* __restrict__ kc, const VE* __restrict__ vc, int j0, int j1,
                                              int hd, int vd, int kvw, int vw, long koff, long vo, A scale, A cap,
                                              lk_pair<A> (&acc)[G * VPT]){
  if constexpr(W && G <= 8){
    if(lk_attn_by_warps<A>(G, hd, vd, kc, vc, kvw, vw, koff, vo)){
      lk_attn_warps_at<G, VPT, A, VALUES, R>(qs, wb, mx, sum, 1, kc, vc, j0, j1, hd, kvw, vw, koff, vo, scale, cap, acc);
      return;
    }
  }
  int lane = threadIdx.x & 31, warp = threadIdx.x >> 5, nw = blockDim.x >> 5;
  if(threadIdx.x < G){ mx[threadIdx.x] = -INFINITY; sum[threadIdx.x] = 0; }
  #pragma unroll
  for(int k = 0; k < G * VPT; ++k) acc[k] = lk_pair<A>{0, 0};
  __syncthreads();
  for(int jt = j0; jt < j1; jt += LK_TILE){
    int nt = min(LK_TILE, j1 - jt);
    lk_attn_scores_in<G, A, R>(qs, kc, nt, hd, kvw, koff, [&](int j){ return j < nt ? jt + j : -1; },
                               [&](int j, int g, A s){ sc[g * LK_TILE + j] = lk_cap(s * scale, cap); });
    __syncthreads();
    for(int g = warp; g < G; g += nw){
      A m = mx[g];
      for(int j = lane; j < nt; j += 32) m = fmax(m, sc[g * LK_TILE + j]);
      m = lk_wmax(m);
      A s = 0;
      for(int j = lane; j < nt; j += 32){ A p = lk_exp(sc[g * LK_TILE + j] - m); sc[g * LK_TILE + j] = p; s += p; }
      s = lk_wsum(s);
      if(lane == 0){ A a = lk_exp(mx[g] - m); alpha[g] = a; sum[g] = lk_fma(sum[g], a, s); mx[g] = m; }
    }
    __syncthreads();
    if(VALUES){
      const int ph = vd >> 1;
      #pragma unroll
      for(int k = 0; k < G * VPT; ++k){
        const int s = threadIdx.x + k * blockDim.x;
        if(s < G * ph){
          const int g = s / ph, e = s - g * ph;
          const A* w = sc + g * LK_TILE;
          lk_pair<A> a{acc[k].x * alpha[g], acc[k].y * alpha[g]};
          #pragma unroll 8
          for(int j = 0; j < nt; ++j){
            float2 v = lk_from2(vc + (long)(jt + j) * vw + vo + 2 * e);
            a.x = lk_fma(w[j], (A)v.x, a.x); a.y = lk_fma(w[j], (A)v.y, a.y);
          }
          acc[k] = a;
        }
      }
    }
    __syncthreads();
  }
}
// Chunk `u`'s heads' maxima and sums (thread `g` head `g`'s, the thread that resets them for a next chunk) and weighed
// values (each thread its value slots) into the block's partials `p0` — head `g`'s chunk `u` at `(g·splits + u)·stride`
// — as every form folds them.
template<int G, int VPT, typename A>
__device__ __forceinline__ void lk_attn_keep(A* __restrict__ p0, const A* mx, const A* sum, const lk_pair<A> (&acc)[G * VPT],
                                             int u, int splits, long stride, int vd){
  const int ph = vd >> 1;
  if(threadIdx.x < G){ A* p = p0 + (threadIdx.x * splits + u) * stride; p[0] = mx[threadIdx.x]; p[1] = sum[threadIdx.x]; }
  #pragma unroll
  for(int k = 0; k < G * VPT; ++k){
    const int s = threadIdx.x + k * blockDim.x;
    if(s < G * ph){
      const int g = s / ph, e = s - g * ph;
      A* p = p0 + (g * splits + u) * stride;
      p[2 + 2 * e] = acc[k].x; p[3 + 2 * e] = acc[k].y;
    }
  }
}
// Chunks `u0 … u0 + w − 1` of a row's span `[lo, end)`, cut every `chunk` positions, weighed side by side: step `t0`
// takes tile `t0` of every chunk at once — its keys scored, each (head, chunk)'s running maximum and sum taken over it,
// its values weighed — each chunk in exactly the operations `lk_attn_chunk` takes it in alone. Head `g`'s chunk `u`
// keeps its maximum, sum and last factor at `g·splits + u` of `mx`, `sum` and `alpha`, and its weighed values in the
// partials `p0` as `lk_attn_keep` keeps them; a thread's value slots run over the chunks, slot `s` chunk
// `s / (G·vd/2)`'s, so `w` chunks' slots fill at most the `G·VPT` a thread holds.
template<int G, int VPT, int R, bool W, typename A, typename KE, typename VE>
__device__ __forceinline__ void lk_attn_side(const float* qs, A* sc, A* wb, A* mx, A* sum, A* alpha,
                                             const KE* __restrict__ kc, const VE* __restrict__ vc, int lo, int chunk,
                                             int end, int u0, int w, int splits, int hd, int vd, int kvw, int vw,
                                             long koff, long vo, A scale, A cap, A* __restrict__ p0, long stride){
  const int ph = vd >> 1, slots = G * ph;
  // Warp-local heads take their chunks one after another, each as a split block takes it alone.
  if constexpr(W && G <= 8){
    if(lk_attn_by_warps<A>(G, hd, vd, kc, vc, kvw, vw, koff, vo)){
      for(int ul = 0; ul < w; ++ul){
        const int u = u0 + ul, j0 = min(lo + u * chunk, end), j1 = min(j0 + chunk, end);
        lk_pair<A> one[G * VPT];
        lk_attn_warps_at<G, VPT, A, true, R>(qs, wb, mx + u, sum + u, splits, kc, vc, j0, j1, hd, kvw, vw, koff, vo, scale, cap, one);
        #pragma unroll
        for(int k = 0; k < G * VPT; ++k){
          const int s = threadIdx.x + k * blockDim.x;
          if(s < slots){
            const int g = s / ph, e = s - g * ph;
            A* p = p0 + (g * splits + u) * stride;
            p[2 + 2 * e] = one[k].x; p[3 + 2 * e] = one[k].y;
          }
        }
      }
      return;
    }
  }
  int lane = threadIdx.x & 31, warp = threadIdx.x >> 5, nw = blockDim.x >> 5;
  // Chunk `u`'s first position and its keys in the tile from `t0`.
  auto first = [&](int u){ return min(lo + u * chunk, end); };
  auto in_tile = [&](int u, int t0){ int j0 = first(u); return max(0, min(LK_TILE, min(j0 + chunk, end) - j0 - t0)); };
  for(int i = threadIdx.x; i < G * w; i += blockDim.x){ int at = (i % G) * splits + u0 + i / G; mx[at] = -INFINITY; sum[at] = 0; }
  lk_pair<A> acc[G * VPT];
  #pragma unroll
  for(int k = 0; k < G * VPT; ++k) acc[k] = lk_pair<A>{0, 0};
  __syncthreads();
  const int longest = min(first(u0) + chunk, end) - first(u0);
  for(int t0 = 0; t0 < longest; t0 += LK_TILE){
    // The step's keys of every chunk, `full` a chunk (the last's maybe fewer), each scored as `lk_attn_chunk` scores it.
    const int full = min(LK_TILE, longest - t0);
    lk_attn_scores_in<G, A, R>(qs, kc, w * full, hd, kvw, koff,
                               [&](int idx){ int ul = idx / full, j = idx - ul * full; return ul < w && j < in_tile(u0 + ul, t0) ? first(u0 + ul) + t0 + j : -1; },
                               [&](int idx, int g, A s){ int ul = idx / full; sc[(ul * G + g) * LK_TILE + idx - ul * full] = lk_cap(s * scale, cap); });
    __syncthreads();
    // Each (head, chunk)'s running maximum and sum over the step, a warp a pair.
    for(int pi = warp; pi < G * w; pi += nw){
      int g = pi % G, ul = pi / G, nt = in_tile(u0 + ul, t0), at = g * splits + u0 + ul;
      if(nt == 0) continue;
      A* srow = sc + (ul * G + g) * LK_TILE;
      A m = mx[at];
      for(int j = lane; j < nt; j += 32) m = fmax(m, srow[j]);
      m = lk_wmax(m);
      A s = 0;
      for(int j = lane; j < nt; j += 32){ A p = lk_exp(srow[j] - m); srow[j] = p; s += p; }
      s = lk_wsum(s);
      if(lane == 0){ A a = lk_exp(mx[at] - m); alpha[at] = a; sum[at] = lk_fma(sum[at], a, s); mx[at] = m; }
    }
    __syncthreads();
    // Every chunk's values weighed in, a thread its slots over the chunks.
    #pragma unroll
    for(int k = 0; k < G * VPT; ++k){
      const int s = threadIdx.x + k * blockDim.x;
      if(s < w * slots){
        const int ul = s / slots, rest = s - ul * slots, g = rest / ph, e = rest - g * ph, nt = in_tile(u0 + ul, t0);
        if(nt > 0){
          const A* wt = sc + (ul * G + g) * LK_TILE;
          const A al = alpha[g * splits + u0 + ul];
          const long j0 = first(u0 + ul) + t0;
          lk_pair<A> a{acc[k].x * al, acc[k].y * al};
          #pragma unroll 8
          for(int j = 0; j < nt; ++j){
            float2 v = lk_from2(vc + (j0 + j) * vw + vo + 2 * e);
            a.x = lk_fma(wt[j], (A)v.x, a.x); a.y = lk_fma(wt[j], (A)v.y, a.y);
          }
          acc[k] = a;
        }
      }
    }
    __syncthreads();
  }
  #pragma unroll
  for(int k = 0; k < G * VPT; ++k){
    const int s = threadIdx.x + k * blockDim.x;
    if(s < w * slots){
      const int ul = s / slots, rest = s - ul * slots, g = rest / ph, e = rest - g * ph;
      A* p = p0 + (g * splits + u0 + ul) * stride;
      p[2 + 2 * e] = acc[k].x; p[3 + 2 * e] = acc[k].y;
    }
  }
}
// A pair of a chunk's weighed values, read past the cache from the partials the other blocks wrote.
__device__ __forceinline__ lk_pair<float> lk_part_pair(const float* p){ float2 v = __ldcg((const float2*)p); return {v.x, v.y}; }
__device__ __forceinline__ lk_pair<double> lk_part_pair(const double* p){ double2 v = __ldcg((const double2*)p); return {v.x, v.y}; }
// Each head's maximum (with its sink) and denominator over its `pe` chunks' maxima `ms` and sums `ls`, and each
// chunk's factor `ff` in the fold — a function of the chunks alone, the same wherever the chunks ran — by the whole
// block: a head's maximum a thread, then a chunk's factor a thread, then each head's denominator over its chunks in
// order. Every thread of the block calls it.
template<int G, typename A>
__device__ __forceinline__ void lk_attn_fold(const A* ms, const A* ls, A* ff, A* fm, A* fl, const float* __restrict__ sinks,
                                             int h0, int pe, int splits){
  if(threadIdx.x < G){
    const int g = threadIdx.x;
    A m = sinks ? (A)sinks[h0 + g] : (A)-INFINITY;
    #pragma unroll 8
    for(int u = 0; u < pe; ++u) m = fmax(m, ms[g * splits + u]);
    fm[g] = m;
  }
  __syncthreads();
  for(int i = threadIdx.x; i < G * pe; i += blockDim.x){
    const int g = i / pe, u = i - g * pe;
    const A mu = ms[g * splits + u];
    ff[g * splits + u] = mu > -INFINITY ? lk_exp(mu - fm[g]) : (A)0;
  }
  __syncthreads();
  if(threadIdx.x < G){
    const int g = threadIdx.x;
    A l = sinks ? lk_exp((A)sinks[h0 + g] - fm[g]) : (A)0;
    #pragma unroll 8
    for(int u = 0; u < pe; ++u){
      if(ms[g * splits + u] > -INFINITY) l = lk_fma(ls[g * splits + u], ff[g * splits + u], l);
    }
    fl[g] = l;
  }
}
// The fold of a block's `G` heads' weighed values over their `pe` chunks' partials `p0` (as `lk_attn_keep` keeps them,
// in shared memory under `SHARED`, else read past the cache from what the other blocks wrote, `U` chunks' a batch):
// each thread folds the value slots it weighed, chunk by chunk in order. Each head's values divided by its denominator
// `fl` into `out`.
template<int G, int VPT, typename A, bool SHARED = false, int QO = 0, int U = 1>
__device__ __forceinline__ void lk_attn_fold_values(const A* __restrict__ p0, const A* ms, const A* ff, const A* fl,
                                                    float* __restrict__ out, long head0, int pe, int splits, long stride, int vd,
                                                    lk_qout qo = {}){
  const int ph = vd >> 1;
  lk_pair<A> a[G * VPT];
  #pragma unroll
  for(int k = 0; k < G * VPT; ++k) a[k] = lk_pair<A>{0, 0};
  // A batch of `U` chunks' pairs at a time — every read of a batch in flight at once, each
  // chunk's written, an empty chunk's too — then folded in order.
  for(int u0 = 0; u0 < pe; u0 += U){
    lk_pair<A> v[U][G * VPT];
    #pragma unroll
    for(int du = 0; du < U; ++du){
      #pragma unroll
      for(int k = 0; k < G * VPT; ++k){
        const int s = threadIdx.x + k * blockDim.x, g = s / ph, e = s - g * ph;
        const A* p = p0 + (g * splits + u0 + du) * stride + 2 + 2 * e;
        v[du][k] = u0 + du < pe && s < G * ph ? (SHARED ? lk_pair<A>{p[0], p[1]} : lk_part_pair(p)) : lk_pair<A>{0, 0};
      }
    }
    #pragma unroll
    for(int du = 0; du < U; ++du){
      if(u0 + du < pe){
        #pragma unroll
        for(int k = 0; k < G * VPT; ++k){
          const int s = threadIdx.x + k * blockDim.x;
          if(s < G * ph){
            const int g = s / ph, u = u0 + du;
            if(ms[g * splits + u] > -INFINITY){
              const A f = ff[g * splits + u];
              a[k].x = lk_fma(v[du][k].x, f, a[k].x); a[k].y = lk_fma(v[du][k].y, f, a[k].y);
            }
          }
        }
      }
    }
  }
  #pragma unroll
  for(int k = 0; k < G * VPT; ++k){
    const int s = threadIdx.x + k * blockDim.x;
    const bool on = s < G * ph;
    const int g = on ? s / ph : 0, e = s - g * ph;
    lk_attn_store<QO>(out, qo, (head0 + g) * vd + 2 * e, on, on ? (float)(a[k].x / fl[g]) : 0.f, on ? (float)(a[k].y / fl[g]) : 0.f);
  }
}
// The path an attention launch takes past a row of one chunk (`lk_attention`'s `PATH`), fixed where it launches: its
// chunks' partials folded by the last split block to arrive, held side by side by the one block of the row, or weighed
// one at a time by it; a launch of one chunk a row takes none. `LK_ATTN_ANY` takes whichever its grid and its
// arguments name.
enum lk_attn_path { LK_ATTN_ANY, LK_ATTN_SPLIT, LK_ATTN_HELD, LK_ATTN_CHUNKS, LK_ATTN_ONE };
// The attention's body at a key's run of `R` lanes (`lk_attn_run_width`), on path `PATH`, its chunks warp-local where
// they fit in a kernel that holds two blocks a multiprocessor (`W`: at three their registers spill), with the block's
// heads' running maxima, sums, factors and folded maxima and denominators, and the flag of the last split block to
// arrive.
template<int R, int G, int VPT, typename A, typename KE, typename VE, int QO, int PATH, bool W>
__device__ __forceinline__ void lk_attention_run(const float* __restrict__ q, const KE* __restrict__ kc,
    const VE* __restrict__ vc, const int* __restrict__ pos, const float* __restrict__ sinks, float* __restrict__ out,
    A* __restrict__ part, int* __restrict__ cnt, int len, int win, int hd, int vd, int group, int heads, int kvw,
    int vw, int voff, float scale, int minc, int splits, long kz, int held, A* mx, A* sum, A* alpha, A* fm, A* fl,
    int& last, lk_qout qo, float cap){
  extern __shared__ __align__(16) unsigned char lk_smem[];
  float* qs = (float*)lk_smem;
  A* sc = (A*)(lk_smem + ((G * hd * 4 + 15) & ~15));
  A* ms = sc + G * LK_TILE * max(held, 1);
  A* ls = ms + G * splits;
  A* ff = ls + G * splits;
  A* wb = ff + G * splits;
  A* hp = wb + lk_attn_warp_slots(G, hd, (int)sizeof(A));
  int hb = blockIdx.x, r = blockIdx.y, t = blockIdx.z;
  int h0 = hb * G, kvh = h0 / group;
  // A batch's rows (`kz` > 0) are its sequences: row `r` attends its own caches, `kz` values on from the last's, up
  // to its own position; a chunk's rows share the caches, at consecutive positions.
  if(kz){ kc += r * kz; vc += r * kz; }
  int end = pos ? (kz ? pos[r] : *pos + r) : len;
  int lo = lk_window_lo(win, end), n = end - lo;
  int pe = max(1, min(splits, (n + minc - 1) / minc)), chunk = (n + pe - 1) / pe;
  if(t >= pe) return;
  const float* qb = q + ((long)r * heads + h0) * hd;
  for(int i = threadIdx.x; i < G * hd; i += blockDim.x) qs[i] = qb[i];
  long koff = (long)kvh * hd, vo = voff + (long)kvh * vd, head0 = (long)r * heads + h0;
  const int ph = vd >> 1;
  lk_pair<A> acc[G * VPT];
  if(PATH == LK_ATTN_ONE || pe == 1){
    lk_attn_chunk<G, VPT, A, true, R, W>(qs, sc, wb, mx, sum, alpha, kc, vc, lo, end, hd, vd, kvw, vw, koff, vo, (A)scale, (A)cap, acc);
    #pragma unroll
    for(int k = 0; k < G * VPT; ++k){
      const int s = threadIdx.x + k * blockDim.x;
      const bool on = s < G * ph;
      const int g = on ? s / ph : 0, e = s - g * ph;
      float x = 0.f, y = 0.f;
      if(on){
        A sk = sinks ? (A)sinks[h0 + g] : (A)-INFINITY;
        A m = fmax(sk, mx[g]), f = lk_exp(mx[g] - m);
        A l = lk_fma(sum[g], f, sinks ? lk_exp(sk - m) : (A)0);
        x = (float)(acc[k].x * f / l);
        y = (float)(acc[k].y * f / l);
      }
      lk_attn_store<QO>(out, qo, (head0 + g) * vd + 2 * e, on, x, y);
    }
    return;
  }
  long stride = (long)vd + 2;
  if(PATH == LK_ATTN_ANY ? gridDim.z > 1 : PATH == LK_ATTN_SPLIT){
    int j0 = min(lo + t * chunk, end), j1 = min(j0 + chunk, end);
    lk_attn_chunk<G, VPT, A, true, R, W>(qs, sc, wb, mx, sum, alpha, kc, vc, j0, j1, hd, vd, kvw, vw, koff, vo, (A)scale, (A)cap, acc);
    lk_attn_keep<G, VPT>(part + head0 * splits * stride, mx, sum, acc, t, splits, stride, vd);
    __threadfence();
    __syncthreads();
    int* c = cnt + (long)r * gridDim.x + hb;
    if(threadIdx.x == 0) last = atomicAdd(c, 1) == pe - 1;
    __syncthreads();
    if(!last) return;
    __threadfence();
    for(int i = threadIdx.x; i < G * pe; i += blockDim.x){
      int g = i / pe, u = i - g * pe;
      const A* p = part + ((head0 + g) * splits + u) * stride;
      ms[g * splits + u] = __ldcg(p); ls[g * splits + u] = __ldcg(p + 1);
    }
    __syncthreads();
    lk_attn_fold<G>(ms, ls, ff, fm, fl, sinks, h0, pe, splits);
    __syncthreads();
    lk_attn_fold_values<G, VPT, A, false, QO, W ? 8 : 1>(part + head0 * splits * stride, ms, ff, fl, out, head0, pe, splits, stride, vd, qo);
    if(threadIdx.x == 0) *c = 0;
    return;
  }
  // One block takes every chunk of its row, in the arithmetic the split blocks use. With room for their partials
  // (`held`, after the fold's factors), each chunk once, `held` side by side, kept as a split block keeps it, then
  // folded alike; otherwise each chunk's maximum and sum first, then each chunk's values weighed in, chunk by chunk.
  if(PATH == LK_ATTN_ANY ? held != 0 : PATH == LK_ATTN_HELD){
    for(int u0 = 0; u0 < pe; u0 += held)
      lk_attn_side<G, VPT, R, W>(qs, sc, wb, ms, ls, ff, kc, vc, lo, chunk, end, u0, min(held, pe - u0), splits, hd, vd, kvw, vw, koff, vo,
                           (A)scale, (A)cap, hp, stride);
    __syncthreads();
    lk_attn_fold<G>(ms, ls, ff, fm, fl, sinks, h0, pe, splits);
    __syncthreads();
    lk_attn_fold_values<G, VPT, A, true, QO>(hp, ms, ff, fl, out, head0, pe, splits, stride, vd, qo);
    return;
  }
  for(int u = 0; u < pe; ++u){
    int j0 = min(lo + u * chunk, end), j1 = min(j0 + chunk, end);
    lk_attn_chunk<G, VPT, A, false, R, W>(qs, sc, wb, mx, sum, alpha, kc, vc, j0, j1, hd, vd, kvw, vw, koff, vo, (A)scale, (A)cap, acc);
    if(threadIdx.x < G){ ms[threadIdx.x * splits + u] = mx[threadIdx.x]; ls[threadIdx.x * splits + u] = sum[threadIdx.x]; }
  }
  __syncthreads();
  lk_attn_fold<G>(ms, ls, ff, fm, fl, sinks, h0, pe, splits);
  lk_pair<A> a[G * VPT];
  #pragma unroll
  for(int k = 0; k < G * VPT; ++k) a[k] = lk_pair<A>{0, 0};
  for(int u = 0; u < pe; ++u){
    int j0 = min(lo + u * chunk, end), j1 = min(j0 + chunk, end);
    lk_attn_chunk<G, VPT, A, true, R, W>(qs, sc, wb, mx, sum, alpha, kc, vc, j0, j1, hd, vd, kvw, vw, koff, vo, (A)scale, (A)cap, acc);
    #pragma unroll
    for(int k = 0; k < G * VPT; ++k){
      const int s = threadIdx.x + k * blockDim.x;
      if(s < G * ph){
        const int g = s / ph;
        if(ms[g * splits + u] > -INFINITY){
          a[k].x = lk_fma(acc[k].x, ff[g * splits + u], a[k].x);
          a[k].y = lk_fma(acc[k].y, ff[g * splits + u], a[k].y);
        }
      }
    }
  }
  #pragma unroll
  for(int k = 0; k < G * VPT; ++k){
    const int s = threadIdx.x + k * blockDim.x;
    const bool on = s < G * ph;
    const int g = on ? s / ph : 0, e = s - g * ph;
    lk_attn_store<QO>(out, qo, (head0 + g) * vd + 2 * e, on, on ? (float)(a[k].x / fl[g]) : 0.f, on ? (float)(a[k].y / fl[g]) : 0.f);
  }
}
template<int G, int VPT, typename A, int MB = 2, typename KE = float, typename VE = float, int QO = 0, int PATH = LK_ATTN_ANY,
         int RW = 0>
__global__ void __launch_bounds__(256, MB) lk_attention(const float* __restrict__ q, const KE* __restrict__ kc,
    const VE* __restrict__ vc, const int* __restrict__ pos, const float* __restrict__ sinks, float* __restrict__ out,
    A* __restrict__ part, int* __restrict__ cnt, int len, int win, int hd, int vd, int group, int heads, int kvw,
    int vw, int voff, float scale, int minc, int splits, long kz = 0, int held = 0, lk_qout qo = {}, float cap = 0.f){
  __shared__ A mx[G], sum[G], alpha[G], fm[G], fl[G];
  __shared__ int last;
  if(RW == 0 ? lk_attn_run_width(hd) == 16 : RW == 16)
    lk_attention_run<16, G, VPT, A, KE, VE, QO, PATH, MB <= 2>(q, kc, vc, pos, sinks, out, part, cnt, len, win, hd, vd, group, heads, kvw, vw, voff,
                                                scale, minc, splits, kz, held, mx, sum, alpha, fm, fl, last, qo, cap);
  else
    lk_attention_run<32, G, VPT, A, KE, VE, QO, PATH, MB <= 2>(q, kc, vc, pos, sinks, out, part, cnt, len, win, hd, vd, group, heads, kvw, vw, voff,
                                                scale, minc, splits, kz, held, mx, sum, alpha, fm, fl, last, qo, cap);
}
"#;

/// An attention whose values are its key rows' own part (a latent cache's latent, `voff + vd ≤ hd`), one key/value
/// group's `G` heads a block, each cached row read once for every head's score and value — the values read from the
/// staged key rows (`VK`), or from rows of their own `vw` apart where a launch sliced them out of the key rows (the
/// same values, so the same results bit for bit). A row's span is cut into `pe` chunks
/// (`lk_shared_chunks`: at most `splits`, each at least `minc` positions and a whole number of `LK_ST`-row steps but the
/// last), a block each. A block holds its heads' queries in registers — a lane `hd / 32` of the width of four heads —
/// and stages its chunk's rows `LK_ST` at a time in shared memory by `cp.async`, the next step's rows landing while these
/// multiply. A warp scores four heads against `2·⌈G/4⌉` keys over its lanes' parts of the width, the lanes' sums
/// exchanged so that each lane ends with one head's score of one key (`lk_lane_sums`); each head keeps a running maximum
/// and sum (the online softmax); and a thread weighs `VPT` pairs of every head's values, the step's weights read four
/// heads at a time. A row of one chunk writes its outputs; otherwise each chunk writes its heads' maxima, sums and
/// weighed values to `part`, and `lk_attention_fold` folds them. Head `h`'s sink `sinks[h]` joins the maximum and the
/// denominator when there are sinks. A head is `H4` fours wide, a key row `kvw` wide. Every sum is `f32`.
const ATTENTION_SHARED: &str = r#"
#define LK_ST 16
// The chunks a row's `n` positions are cut into: at most `splits`, each at least `minc` positions, every one but the
// last `chunk` positions — a whole number of `LK_ST`-row steps.
__device__ __forceinline__ int lk_shared_chunks(int n, int splits, int minc, int& chunk){
  int pe = max(1, min(splits, (n + minc - 1) / minc));
  chunk = ((n + pe - 1) / pe + LK_ST - 1) / LK_ST * LK_ST;
  return max(1, (n + chunk - 1) / chunk);
}
template<int G, int VPT, int H4, bool VK, typename KE = float, typename VE = float, int QO = 0>
__global__ void __launch_bounds__(256) lk_attention_shared(const float* __restrict__ q, const KE* __restrict__ kc,
    const VE* __restrict__ vc, const int* __restrict__ pos, const float* __restrict__ sinks, float* __restrict__ out,
    float* __restrict__ part, int len, int win, int vd, int heads, int kvw, int vw, int voff, float scale, int minc,
    int splits, long kz = 0, lk_qout qo = {}, float cap = 0.f){
  constexpr int HD = 4 * H4, HQ = G <= 4 ? 1 : (G <= 8 ? 2 : 4), GP = 4 * HQ, TN = LK_ST * HQ / 8, NV = 4 * TN;
  constexpr int M2 = (2 * H4 + 31) / 32, RF = (LK_ST * H4 + 255) / 256;
  extern __shared__ __align__(16) float lk_ss[];
  float* ks = lk_ss;                          // [2][LK_ST][HD]: two steps' rows
  float* sc = ks + 2 * LK_ST * HD;            // [LK_ST][GP]: a step's scores, then weights, a key's heads together
  __shared__ float mx[G], sum[G], alpha[G];
  int hb = blockIdx.x, r = blockIdx.y, t = blockIdx.z, tid = threadIdx.x, lane = tid & 31, warp = tid >> 5;
  // A batch's rows (`kz` > 0) are its sequences, as `lk_attention` takes them.
  if(kz){ kc += r * kz; vc += r * kz; }
  int end = pos ? (kz ? pos[r] : *pos + r) : len;
  int lo = lk_window_lo(win, end), chunk;
  int pe = lk_shared_chunks(end - lo, splits, minc, chunk);
  if(t >= pe) return;
  int j0 = lo + t * chunk, j1 = min(j0 + chunk, end);
  const KE* kb = kc + (long)hb * HD;
  const VE* vb = vc + (long)hb * vd + voff;
  long head0 = (long)r * heads + (long)hb * G;
  // A warp's four heads and keys; a lane's part of the width: float pairs `lane + 32·m`.
  int hq = warp % HQ, kg = warp / HQ;
  float2 qr[4][M2];
  #pragma unroll
  for(int h = 0; h < 4; ++h)
    #pragma unroll
    for(int m = 0; m < M2; ++m){
      int c = lane + 32 * m, hh = 4 * hq + h;
      qr[h][m] = hh < G && c < 2 * H4 ? ((const float2*)(q + (head0 + hh) * HD))[c] : make_float2(0.f, 0.f);
    }
  if(tid < G){ mx[tid] = -INFINITY; sum[tid] = 0.f; }
  float2 acc[G][VPT];
  #pragma unroll
  for(int g = 0; g < G; ++g)
    #pragma unroll
    for(int k = 0; k < VPT; ++k) acc[g][k] = make_float2(0.f, 0.f);
  // Step `s`'s rows into buffer `s & 1`, the rows past the chunk zero; every step a commit group.
  auto issue = [&](int s){
    int js = j0 + s * LK_ST;
    if(js < j1){
      float* dst = ks + (s & 1) * LK_ST * HD;
      #pragma unroll
      for(int u = 0; u < RF; ++u){
        int c = tid + u * 256;
        if(c < LK_ST * H4){
          int j = c / H4, e = c - j * H4;
          bool in = js + j < j1;
          lk_stage4(dst + j * HD + 4 * e, kb + (long)(in ? js + j : js) * kvw + 4 * e, in);
        }
      }
    }
    asm volatile("cp.async.commit_group;\n");
  };
  int steps = (j1 - j0 + LK_ST - 1) / LK_ST;
  issue(0);
  issue(1);
  for(int s = 0; s < steps; ++s){
    asm volatile("cp.async.wait_group 1;\n");
    __syncthreads();
    const float* kst = ks + (s & 1) * LK_ST * HD;
    int nt = min(LK_ST, j1 - (j0 + s * LK_ST));
    float a[NV];
    #pragma unroll
    for(int i = 0; i < NV; ++i) a[i] = 0.f;
    #pragma unroll
    for(int m = 0; m < M2; ++m){
      int c = lane + 32 * m;
      if(c < 2 * H4){
        #pragma unroll
        for(int n = 0; n < TN; ++n){
          float2 k = ((const float2*)(kst + (kg * TN + n) * HD))[c];
          #pragma unroll
          for(int h = 0; h < 4; ++h){
            a[h * TN + n] = fmaf(qr[h][m].x, k.x, a[h * TN + n]);
            a[h * TN + n] = fmaf(qr[h][m].y, k.y, a[h * TN + n]);
          }
        }
      }
    }
    float tot = lk_lane_sums<NV>(a);
    {
      int i = lane & (NV - 1), hh = 4 * hq + i / TN, j = kg * TN + i % TN;
      if(lane < NV && hh < G) sc[j * GP + hh] = j < nt ? lk_cap(tot * scale, cap) : -INFINITY;
    }
    __syncthreads();
    // Each head's running maximum and sum over the step: a half-warp a head, a lane a key.
    {
      int g = warp + 8 * (lane >> 4), j = lane & 15;
      bool live = g < G;
      float s0 = live ? sc[j * GP + g] : -INFINITY, mo = live ? mx[g] : -INFINITY, m = s0;
      #pragma unroll
      for(int o = 8; o > 0; o >>= 1) m = fmaxf(m, __shfl_xor_sync(0xffffffffu, m, o));
      m = fmaxf(m, mo);
      float pv = s0 == -INFINITY ? 0.f : expf(s0 - m), ss = pv;
      if(live) sc[j * GP + g] = pv;
      #pragma unroll
      for(int o = 8; o > 0; o >>= 1) ss += __shfl_xor_sync(0xffffffffu, ss, o);
      if(live && j == 0){ float al = mo == -INFINITY ? 0.f : expf(mo - m); alpha[g] = al; sum[g] = fmaf(sum[g], al, ss); mx[g] = m; }
    }
    __syncthreads();
    // Every head's values weighed in, `VPT` pairs a thread, each value read once from the step's rows.
    #pragma unroll
    for(int k = 0; k < VPT; ++k){
      int e = tid + k * 256;
      if(2 * e < vd){
        #pragma unroll
        for(int g = 0; g < G; ++g){ acc[g][k].x *= alpha[g]; acc[g][k].y *= alpha[g]; }
        for(int j = 0; j < nt; ++j){
          float2 v = VK ? *(const float2*)(kst + j * HD + voff + 2 * e) : lk_from2(vb + (long)(j0 + s * LK_ST + j) * vw + 2 * e);
          #pragma unroll
          for(int c4 = 0; c4 < HQ; ++c4){
            float4 w = ((const float4*)(sc + j * GP))[c4];
            float wv[4] = {w.x, w.y, w.z, w.w};
            #pragma unroll
            for(int i = 0; i < 4; ++i)
              if(4 * c4 + i < G){
                acc[4 * c4 + i][k].x = fmaf(wv[i], v.x, acc[4 * c4 + i][k].x);
                acc[4 * c4 + i][k].y = fmaf(wv[i], v.y, acc[4 * c4 + i][k].y);
              }
          }
        }
      }
    }
    __syncthreads();
    issue(s + 2);
  }
  if(pe == 1){
    #pragma unroll
    for(int g = 0; g < G; ++g){
      float sk = sinks ? sinks[hb * G + g] : -INFINITY, m = fmaxf(sk, mx[g]), f = expf(mx[g] - m);
      float l = fmaf(sum[g], f, sinks ? expf(sk - m) : 0.f);
      #pragma unroll
      for(int k = 0; k < VPT; ++k){
        int e = tid + k * 256;
        lk_attn_store<QO>(out, qo, (head0 + g) * vd + 2 * e, 2 * e < vd, acc[g][k].x * f / l, acc[g][k].y * f / l);
      }
    }
    return;
  }
  long stride = (long)vd + 2;
  #pragma unroll
  for(int g = 0; g < G; ++g){
    float* pp = part + ((head0 + g) * splits + t) * stride;
    if(tid == 0){ pp[0] = mx[g]; pp[1] = sum[g]; }
    #pragma unroll
    for(int k = 0; k < VPT; ++k){
      int e = tid + k * 256;
      if(2 * e < vd) ((float2*)(pp + 2))[e] = acc[g][k];
    }
  }
}
// The fold of `lk_attention_shared`'s chunks: a block a head of a row and 128 of its values, each thread's pair of
// values summed over the chunks in order, sixteen chunks' pairs read at once — the first sixteen while a warp takes
// the chunks' factors (each chunk's maximum against the head's, sink included) and the denominator.
template<int QO = 0>
__global__ void __launch_bounds__(64) lk_attention_fold(const float* __restrict__ part, const int* __restrict__ pos,
    const float* __restrict__ sinks, float* __restrict__ out, int len, int win, int vd, int heads, int minc, int splits,
    int seqs = 0, lk_qout qo = {}){
  extern __shared__ float lk_ff[];
  __shared__ float fl;
  int h = blockIdx.x, r = blockIdx.y, tid = threadIdx.x, lane = tid & 31;
  int end = pos ? (seqs ? pos[r] : *pos + r) : len;
  int lo = lk_window_lo(win, end), chunk;
  int pe = lk_shared_chunks(end - lo, splits, minc, chunk);
  if(pe == 1) return;
  long head = (long)r * heads + h, stride = (long)vd + 2;
  const float* hp = part + head * splits * stride;
  int e = blockIdx.z * 64 + tid;
  bool in = 2 * e < vd;
  float2 v[16];
  #pragma unroll
  for(int i = 0; i < 16; ++i) v[i] = in && i < pe ? __ldcg((const float2*)(hp + i * stride + 2) + e) : make_float2(0.f, 0.f);
  if(tid < 32){
    float m = sinks ? sinks[h] : -INFINITY;
    for(int u = lane; u < pe; u += 32) m = fmaxf(m, __ldcg(hp + u * stride));
    m = lk_warp_max(m);
    float l = 0.f;
    for(int u = lane; u < pe; u += 32){
      float mu = __ldcg(hp + u * stride), f = mu > -INFINITY ? expf(mu - m) : 0.f;
      lk_ff[u] = f;
      l = fmaf(__ldcg(hp + u * stride + 1), f, l);
    }
    l = lk_warp_sum(l);
    if(lane == 0) fl = sinks ? l + expf(sinks[h] - m) : l;
  }
  __syncthreads();
  float2 a = make_float2(0.f, 0.f);
  for(int u0 = 0; u0 < pe; u0 += 16){
    if(u0 > 0)
      #pragma unroll
      for(int i = 0; i < 16; ++i) v[i] = in && u0 + i < pe ? __ldcg((const float2*)(hp + (u0 + i) * stride + 2) + e) : make_float2(0.f, 0.f);
    #pragma unroll
    for(int i = 0; i < 16; ++i)
      if(u0 + i < pe){ a.x = fmaf(v[i].x, lk_ff[u0 + i], a.x); a.y = fmaf(v[i].y, lk_ff[u0 + i], a.y); }
  }
  lk_attn_store<QO>(out, qo, head * vd + 2 * e, in, a.x / fl, a.y / fl);
}
"#;

/// The most query heads `lk_attention_shared` takes in its one block: four warps of four heads.
pub const ATTN_SHARED_HEADS: usize = 16;

/// The cached rows `lk_attention_shared` stages a step (`LK_ST` in its source).
pub const ATTN_SHARED_STEP: usize = 16;

/// The value pairs a block of `lk_attention_fold` folds, a thread each.
pub const ATTN_FOLD_PAIRS: usize = 64;

/// The most chunks `lk_attention_shared` cuts a row's span of `span` positions into, the decode step's in every form:
/// one block a streaming multiprocessor (`sms`), its shared memory a block's, and no more than give each chunk
/// `ATTN_MIN_SPLIT` positions.
pub fn attn_shared_splits(span: usize, sms: usize) -> usize {
    sms.clamp(1, span.div_ceil(ATTN_MIN_SPLIT).max(1))
}

/// The heads `lk_attention_shared` pads its `g` heads to: whole warps of four.
pub fn attn_shared_padded_heads(g: usize) -> usize {
    4 * if g <= 4 { 1 } else if g <= 8 { 2 } else { 4 }
}

/// The dynamic shared memory of a block of `lk_attention_shared` over `g` heads of `hd`: two steps' rows, and a step's
/// scores of every padded head.
pub fn attn_shared_smem_bytes(g: usize, hd: usize) -> usize {
    (2 * ATTN_SHARED_STEP * hd + ATTN_SHARED_STEP * attn_shared_padded_heads(g)) * 4
}

/// Every group of `every` consecutive values of `x`, its `width` from `from`: `y[g·width + i] = x[g·every + from + i]`.
/// A wide chunk's attention in tiles: a block takes `ATTN_TILE_Q` query vectors — `HG` heads of one key/value group
/// over `ATTN_TILE_Q / HG` consecutive rows — and walks the keys its rows attend 64 at a time. Each tile's scores are
/// taken over the head width in chunks of 32 staged in shared memory, a thread 4 queries by 2 keys; then each query's
/// running maximum and sum (the online softmax); then the tile's weighed values, eight keys' values staged at a time,
/// a thread 4 queries by `VI` groups of 4 values 128 apart. Row `r` attends the keys `lk_attention` gives it — `[lo,
/// end)`, `end` its position, `lo` its window's start — and a sink joins each head's denominator. Every sum is `f32`.
/// Split at `kh` (when `kt` is given), each query and key is two parts: a query's first `kh` values its own, `qp` apart
/// a (row, head) in `q`, the rest `qtp` apart in `qt`; a key's first `kh` its group's own in `kc`, the rest the key
/// row's in `kt` from `kto` (rows `ktw` apart), every group's alike — an expanded latent attention's, each head's query
/// read in place in the query projection and its rotated part, each head's expanded key and the rotated key the heads
/// share, read in place in the cache.
const ATTENTION_TILED: &str = r#"
#define LK_FQ 32
#define LK_FK 64
#define LK_FD 32
#define LK_FV 8
template<int HG, int VI, typename KE = float, typename VE = float, typename TE = float>
__global__ void __launch_bounds__(256) lk_attention_tiled(const float* __restrict__ q, const KE* __restrict__ kc,
    const VE* __restrict__ vc, const int* __restrict__ pos, const float* __restrict__ sinks, float* __restrict__ out,
    int len, int win, int hd, int vd, int group, int heads, int kvw, int vw, int voff, float scale, int rows,
    const float* __restrict__ qt = nullptr, long qp = 0, long qtp = 0, const TE* __restrict__ kt = nullptr, long ktw = 0,
    long kto = 0, int kh = 0, float cap = 0.f){
  constexpr int BR = LK_FQ / HG;
  extern __shared__ __align__(16) float lk_fa[];
  float* qs = lk_fa;                       // [LK_FD][LK_FQ]: a chunk of the queries
  float* ks = qs + LK_FD * LK_FQ;          // [LK_FD][LK_FK]: the same chunk of the tile's keys
  float* ps = ks + LK_FD * LK_FK;          // [LK_FK][LK_FQ]: the tile's scores, then its weights
  float* vs = ps + LK_FK * LK_FQ;          // [LK_FV][vd]: eight keys' values
  __shared__ float mq[LK_FQ], lq[LK_FQ], aq[LK_FQ];
  __shared__ int loq[LK_FQ], endq[LK_FQ], span[2];
  // Scores: lane `q8` a group of 4 queries, lane `k4` and the warp a pair of keys. Values: lane `q8` the same 4
  // queries, lane `k4` and the warp a group of 4 values, `VI` groups 128 apart.
  int tid = threadIdx.x, lane = tid & 31, warp = tid >> 5, q8 = lane & 7, k4 = lane >> 3, kp = 4 * warp + k4;
  int r0 = blockIdx.x * BR, h0 = blockIdx.y * HG;
  long koff = (long)(h0 / group) * (kt ? kh : hd), vo = voff + (long)(h0 / group) * vd;
  if(tid < LK_FQ){
    int r = r0 + tid / HG;
    int end = r < rows ? (pos ? *pos + r : len) : 0;
    endq[tid] = end; loq[tid] = lk_window_lo(win, end);
    mq[tid] = -INFINITY; lq[tid] = 0.f;
  }
  __syncthreads();
  if(tid == 0){
    int lo = 0x7fffffff, hi = 0;
    for(int i = 0; i < LK_FQ; ++i) if(endq[i] > 0){ lo = min(lo, loq[i]); hi = max(hi, endq[i]); }
    span[0] = hi > 0 ? lo : 0; span[1] = hi;
  }
  __syncthreads();
  int jlo = span[0], jhi = span[1];
  float o[4][VI][4];
  #pragma unroll
  for(int a = 0; a < 4; ++a)
    #pragma unroll
    for(int i = 0; i < VI; ++i)
      #pragma unroll
      for(int c = 0; c < 4; ++c) o[a][i][c] = 0.f;
  for(int jt = jlo; jt < jhi; jt += LK_FK){
    // The tile's scores: queries 4·q8 … 4·q8 + 3 against keys 2·kp, 2·kp + 1.
    float s[4][2];
    #pragma unroll
    for(int a = 0; a < 4; ++a){ s[a][0] = 0.f; s[a][1] = 0.f; }
    // A chunk of the queries' and keys' values, four a thread (two fours of keys): fetched a chunk ahead, so their loads
    // are in flight while the chunk before multiplies.
    auto fetch_q = [&](int d0){
      int qi = tid >> 3, dd = (tid & 7) * 4, r = r0 + qi / HG, h = h0 + qi % HG, d = d0 + dd;
      long u = (long)r * heads + h;
      if(r >= rows || d >= hd) return make_float4(0.f, 0.f, 0.f, 0.f);
      return !kt ? *(const float4*)(q + u * hd + d) : d < kh ? *(const float4*)(q + u * qp + d) : *(const float4*)(qt + u * qtp + (d - kh));
    };
    auto fetch_k = [&](int d0, int part){
      int c = tid + part * 256, kj = c >> 3, d = d0 + (c & 7) * 4, j = jt + kj;
      if(j >= jhi || d >= hd) return make_float4(0.f, 0.f, 0.f, 0.f);
      return kt && d >= kh ? lk_from4(kt + (long)j * ktw + kto + (d - kh)) : lk_from4(kc + (long)j * kvw + koff + d);
    };
    float4 nq = fetch_q(0), nk0 = fetch_k(0, 0), nk1 = fetch_k(0, 1);
    for(int d0 = 0; d0 < hd; d0 += LK_FD){
      {
        int qi = tid >> 3, dd = (tid & 7) * 4;
        qs[(dd + 0) * LK_FQ + qi] = nq.x; qs[(dd + 1) * LK_FQ + qi] = nq.y;
        qs[(dd + 2) * LK_FQ + qi] = nq.z; qs[(dd + 3) * LK_FQ + qi] = nq.w;
      }
      #pragma unroll
      for(int u = 0; u < 2; ++u){
        int c = tid + u * 256, kj = c >> 3, dd = (c & 7) * 4;
        float4 v = u == 0 ? nk0 : nk1;
        ks[(dd + 0) * LK_FK + kj] = v.x; ks[(dd + 1) * LK_FK + kj] = v.y;
        ks[(dd + 2) * LK_FK + kj] = v.z; ks[(dd + 3) * LK_FK + kj] = v.w;
      }
      __syncthreads();
      if(d0 + LK_FD < hd){ nq = fetch_q(d0 + LK_FD); nk0 = fetch_k(d0 + LK_FD, 0); nk1 = fetch_k(d0 + LK_FD, 1); }
      #pragma unroll 8
      for(int dd = 0; dd < LK_FD; ++dd){
        float4 a4 = *(const float4*)(qs + dd * LK_FQ + 4 * q8);
        float2 b2 = *(const float2*)(ks + dd * LK_FK + 2 * kp);
        s[0][0] = fmaf(a4.x, b2.x, s[0][0]); s[0][1] = fmaf(a4.x, b2.y, s[0][1]);
        s[1][0] = fmaf(a4.y, b2.x, s[1][0]); s[1][1] = fmaf(a4.y, b2.y, s[1][1]);
        s[2][0] = fmaf(a4.z, b2.x, s[2][0]); s[2][1] = fmaf(a4.z, b2.y, s[2][1]);
        s[3][0] = fmaf(a4.w, b2.x, s[3][0]); s[3][1] = fmaf(a4.w, b2.y, s[3][1]);
      }
      __syncthreads();
    }
    #pragma unroll
    for(int a = 0; a < 4; ++a)
      #pragma unroll
      for(int b = 0; b < 2; ++b){
        int qi = 4 * q8 + a, j = jt + 2 * kp + b;
        ps[(2 * kp + b) * LK_FQ + qi] = j >= loq[qi] && j < endq[qi] ? lk_cap(s[a][b] * scale, cap) : -INFINITY;
      }
    __syncthreads();
    // Each query's running maximum and sum, eight threads a query.
    {
      int qi = tid >> 3, part = tid & 7;
      float m = -INFINITY;
      #pragma unroll
      for(int k = 0; k < 8; ++k) m = fmaxf(m, ps[(part * 8 + k) * LK_FQ + qi]);
      #pragma unroll
      for(int x = 1; x < 8; x <<= 1) m = fmaxf(m, __shfl_xor_sync(0xffffffffu, m, x));
      float mo = mq[qi], mn = fmaxf(mo, m), sum = 0.f;
      #pragma unroll
      for(int k = 0; k < 8; ++k){
        float* sp = ps + (part * 8 + k) * LK_FQ + qi;
        float p = *sp == -INFINITY ? 0.f : expf(*sp - mn);
        *sp = p; sum += p;
      }
      #pragma unroll
      for(int x = 1; x < 8; x <<= 1) sum += __shfl_xor_sync(0xffffffffu, sum, x);
      // Every lane's read of the query's maximum before the first lane writes it.
      __syncwarp();
      if(part == 0){
        float al = mo == -INFINITY ? 0.f : expf(mo - mn);
        aq[qi] = al; lq[qi] = fmaf(lq[qi], al, sum); mq[qi] = mn;
      }
    }
    __syncthreads();
    // The tile's weighed values: queries 4·q8 … 4·q8 + 3, values 4·kp + 128·i.
    #pragma unroll
    for(int a = 0; a < 4; ++a){
      float al = aq[4 * q8 + a];
      #pragma unroll
      for(int i = 0; i < VI; ++i)
        #pragma unroll
        for(int c = 0; c < 4; ++c) o[a][i][c] *= al;
    }
    int nk = min(LK_FK, jhi - jt), v4 = vd >> 2;
    // Eight keys' values, `VI` fours a thread, fetched a group ahead.
    float4 nv[VI];
    auto fetch_v = [&](int k0){
      #pragma unroll
      for(int i = 0; i < VI; ++i){
        int c = tid + i * 256, kk = c / v4, e = c - kk * v4, j = jt + k0 + kk;
        nv[i] = c < LK_FV * v4 && j < jhi ? lk_from4(vc + (long)j * vw + vo + 4 * e) : make_float4(0.f, 0.f, 0.f, 0.f);
      }
    };
    fetch_v(0);
    for(int k0 = 0; k0 < nk; k0 += LK_FV){
      #pragma unroll
      for(int i = 0; i < VI; ++i){
        int c = tid + i * 256, kk = c / v4, e = c - kk * v4;
        if(c < LK_FV * v4) *(float4*)(vs + kk * vd + 4 * e) = nv[i];
      }
      __syncthreads();
      if(k0 + LK_FV < nk) fetch_v(k0 + LK_FV);
      #pragma unroll
      for(int kk = 0; kk < LK_FV; ++kk){
        float4 p = *(const float4*)(ps + (k0 + kk) * LK_FQ + 4 * q8);
        #pragma unroll
        for(int i = 0; i < VI; ++i){
          int col = 4 * kp + 128 * i;
          if(col < vd){
            float4 v = *(const float4*)(vs + kk * vd + col);
            o[0][i][0] = fmaf(p.x, v.x, o[0][i][0]); o[0][i][1] = fmaf(p.x, v.y, o[0][i][1]); o[0][i][2] = fmaf(p.x, v.z, o[0][i][2]); o[0][i][3] = fmaf(p.x, v.w, o[0][i][3]);
            o[1][i][0] = fmaf(p.y, v.x, o[1][i][0]); o[1][i][1] = fmaf(p.y, v.y, o[1][i][1]); o[1][i][2] = fmaf(p.y, v.z, o[1][i][2]); o[1][i][3] = fmaf(p.y, v.w, o[1][i][3]);
            o[2][i][0] = fmaf(p.z, v.x, o[2][i][0]); o[2][i][1] = fmaf(p.z, v.y, o[2][i][1]); o[2][i][2] = fmaf(p.z, v.z, o[2][i][2]); o[2][i][3] = fmaf(p.z, v.w, o[2][i][3]);
            o[3][i][0] = fmaf(p.w, v.x, o[3][i][0]); o[3][i][1] = fmaf(p.w, v.y, o[3][i][1]); o[3][i][2] = fmaf(p.w, v.z, o[3][i][2]); o[3][i][3] = fmaf(p.w, v.w, o[3][i][3]);
          }
        }
      }
      __syncthreads();
    }
  }
  #pragma unroll
  for(int a = 0; a < 4; ++a){
    int qi = 4 * q8 + a, r = r0 + qi / HG, h = h0 + qi % HG;
    if(r >= rows) continue;
    float m = mq[qi], sk = sinks ? sinks[h] : -INFINITY, mf = fmaxf(sk, m);
    float f = m == -INFINITY ? 0.f : expf(m - mf), l = fmaf(lq[qi], f, sinks ? expf(sk - mf) : 0.f);
    float* op = out + ((long)r * heads + h) * vd;
    #pragma unroll
    for(int i = 0; i < VI; ++i){
      int col = 4 * kp + 128 * i;
      if(col < vd) *(float4*)(op + col) = make_float4(o[a][i][0] * f / l, o[a][i][1] * f / l, o[a][i][2] * f / l, o[a][i][3] * f / l);
    }
  }
}
"#;

/// A wide chunk's attention for heads to 256 wide, flash style: a block of 128 threads takes `BQ` query vectors — `HG`
/// heads of one key/value group over `BQ / HG` consecutive rows, their queries staged once — and walks the keys its rows
/// attend 64 at a time. A tile's keys arrive `LK_KD` values of the head at a time and its values `LK_VK` keys at a time,
/// each the next stage's by `cp.async` while this one multiplies. Of `NT` threads, a thread scores queries `tq + TQ·i`
/// against keys `tk + 16·j` (`TQ = NT / 16`; a `BQ / TQ` × 4 tile), each query's running maximum and sum taken across
/// its 16 lanes; its
/// weights land in shared memory a thread's queries together, and it weighs the same queries' values `4·(tv + 16·g)`
/// (`VD / 16` values a query). Row `r` attends `lk_attention`'s keys — `[lo, end)` — and a sink joins each head's
/// denominator. Split at `kh` (with `kt`), each query and key is two parts as `lk_attention_tiled` reads them. A head is
/// `HD` wide, its values `VD`. Every sum is `f32`.
const ATTENTION_FLASH: &str = r#"
#define LK_FB 64
#define LK_KD 32
#define LK_VK 16
template<int HG, int HD, int VD, int BQ, int NT = 128, typename KE = float, typename VE = float, typename TE = float>
__global__ void __launch_bounds__(NT) lk_attention_flash(const float* __restrict__ q, const KE* __restrict__ kc,
    const VE* __restrict__ vc, const int* __restrict__ pos, const float* __restrict__ sinks, float* __restrict__ out,
    int len, int win, int group, int heads, int kvw, int vw, int voff, float scale, int rows,
    const float* __restrict__ qt = nullptr, long qp = 0, long qtp = 0, const TE* __restrict__ kt = nullptr, long ktw = 0,
    long kto = 0, int kh = 0, float cap = 0.f){
  constexpr int TQ = NT / 16, QI = BQ / TQ, QP = HD + 4, KP = LK_KD + 4, VP = VD + 4, PP = BQ + 4, VG = VD / 64 > 0 ? VD / 64 : 1;
  constexpr int BR = BQ / HG;
  extern __shared__ __align__(16) float lk_fl[];
  float* qs = lk_fl;                       // [BQ][QP]: the block's queries
  float* ks = qs + BQ * QP;                // [2][LK_FB][KP]: two stages of a chunk of the tile's keys
  float* vs = ks + 2 * LK_FB * KP;         // [2][LK_VK][VP]: two stages of the tile's values
  float* ps = vs + 2 * LK_VK * VP;         // [LK_FB][PP]: the tile's weights, a thread's queries together
  __shared__ int loq[BQ], endq[BQ], span[2];
  int tid = threadIdx.x, tq = tid >> 4, tk = tid & 15;
  int r0 = blockIdx.x * BR, h0 = blockIdx.y * HG;
  long koff = (long)(h0 / group) * (kt ? kh : HD), vo = voff + (long)(h0 / group) * VD;
  // Query `qi` of the block: head `h0 + qi % HG` of row `r0 + qi / HG`.
  for(int qi = tid; qi < BQ; qi += NT){
    int r = r0 + qi / HG;
    int end = r < rows ? (pos ? *pos + r : len) : 0;
    endq[qi] = end; loq[qi] = lk_window_lo(win, end);
  }
  for(int c = tid; c < BQ * (HD / 4); c += NT){
    int qi = c / (HD / 4), d = 4 * (c % (HD / 4)), r = r0 + qi / HG, h = h0 + qi % HG;
    long u = (long)r * heads + h;
    float4 v = make_float4(0.f, 0.f, 0.f, 0.f);
    if(r < rows) v = !kt ? *(const float4*)(q + u * HD + d) : d < kh ? *(const float4*)(q + u * qp + d) : *(const float4*)(qt + u * qtp + (d - kh));
    *(float4*)(qs + qi * QP + d) = v;
  }
  __syncthreads();
  if(tid == 0){
    int lo = 0x7fffffff, hi = 0;
    for(int i = 0; i < BQ; ++i) if(endq[i] > 0){ lo = min(lo, loq[i]); hi = max(hi, endq[i]); }
    span[0] = hi > 0 ? lo : 0; span[1] = hi;
  }
  __syncthreads();
  int jlo = span[0], jhi = span[1];
  float m[QI], l[QI], o[QI][VG][4];
  #pragma unroll
  for(int i = 0; i < QI; ++i){
    m[i] = -INFINITY; l[i] = 0.f;
    #pragma unroll
    for(int g = 0; g < VG; ++g)
      #pragma unroll
      for(int c = 0; c < 4; ++c) o[i][g][c] = 0.f;
  }
  // A chunk `d0` of tile `jt`'s keys into stage `st`; a step `k0` of its values into stage `st`.
  auto stage_k = [&](int jt, int d0, int st){
    float* dst = ks + st * LK_FB * KP;
    for(int c = tid; c < LK_FB * (LK_KD / 4); c += NT){
      int kj = c / (LK_KD / 4), d = d0 + 4 * (c % (LK_KD / 4)), j = jt + kj;
      bool in = j < jhi && d < HD;
      if(kt && d >= kh) lk_stage4(dst + kj * KP + (d - d0), kt + (long)(in ? j : jlo) * ktw + kto + (d - kh), in);
      else lk_stage4(dst + kj * KP + (d - d0), kc + (long)(in ? j : jlo) * kvw + koff + (in ? d : 0), in);
    }
    asm volatile("cp.async.commit_group;\n");
  };
  auto stage_v = [&](int jt, int k0, int st){
    float* dst = vs + st * LK_VK * VP;
    for(int c = tid; c < LK_VK * (VD / 4); c += NT){
      int kk = c / (VD / 4), e = 4 * (c % (VD / 4)), j = jt + k0 + kk;
      bool in = j < jhi;
      lk_stage4(dst + kk * VP + e, vc + (long)(in ? j : jlo) * vw + vo + e, in);
    }
    asm volatile("cp.async.commit_group;\n");
  };
  constexpr int DS = (HD + LK_KD - 1) / LK_KD, VS = LK_FB / LK_VK;
  for(int jt = jlo; jt < jhi; jt += LK_FB){
    // The tile's scores: queries tq + 8·i against keys tk + 16·j, over the head a chunk at a time.
    float s[QI][4];
    #pragma unroll
    for(int i = 0; i < QI; ++i)
      #pragma unroll
      for(int j = 0; j < 4; ++j) s[i][j] = 0.f;
    stage_k(jt, 0, 0);
    for(int dc = 0; dc < DS; ++dc){
      if(dc + 1 < DS){ stage_k(jt, (dc + 1) * LK_KD, (dc + 1) & 1); asm volatile("cp.async.wait_group 1;\n"); }
      else asm volatile("cp.async.wait_group 0;\n");
      __syncthreads();
      const float* kst = ks + (dc & 1) * LK_FB * KP;
      #pragma unroll
      for(int d4 = 0; d4 < LK_KD; d4 += 4){
        if(dc * LK_KD + d4 >= HD) break;
        float4 kv[4];
        #pragma unroll
        for(int j = 0; j < 4; ++j) kv[j] = *(const float4*)(kst + (tk + 16 * j) * KP + d4);
        #pragma unroll
        for(int i = 0; i < QI; ++i){
          float4 qv = *(const float4*)(qs + (tq + TQ * i) * QP + dc * LK_KD + d4);
          #pragma unroll
          for(int j = 0; j < 4; ++j){
            s[i][j] = fmaf(qv.x, kv[j].x, s[i][j]); s[i][j] = fmaf(qv.y, kv[j].y, s[i][j]);
            s[i][j] = fmaf(qv.z, kv[j].z, s[i][j]); s[i][j] = fmaf(qv.w, kv[j].w, s[i][j]);
          }
        }
      }
      __syncthreads();
    }
    stage_v(jt, 0, 0);
    // Each query's running maximum and sum across its 16 lanes; its weights into shared memory, a thread's together.
    #pragma unroll
    for(int i = 0; i < QI; ++i){
      int qi = tq + TQ * i;
      float mt = -INFINITY;
      #pragma unroll
      for(int j = 0; j < 4; ++j){
        int kj = jt + tk + 16 * j;
        s[i][j] = kj >= loq[qi] && kj < endq[qi] ? lk_cap(s[i][j] * scale, cap) : -INFINITY;
        mt = fmaxf(mt, s[i][j]);
      }
      #pragma unroll
      for(int x = 1; x < 16; x <<= 1) mt = fmaxf(mt, __shfl_xor_sync(0xffffffffu, mt, x));
      float mn = fmaxf(m[i], mt), sum = 0.f;
      #pragma unroll
      for(int j = 0; j < 4; ++j){
        float pv = s[i][j] == -INFINITY ? 0.f : expf(s[i][j] - mn);
        ps[(tk + 16 * j) * PP + QI * tq + i] = pv;
        sum += pv;
      }
      #pragma unroll
      for(int x = 1; x < 16; x <<= 1) sum += __shfl_xor_sync(0xffffffffu, sum, x);
      float al = m[i] == -INFINITY ? 0.f : expf(m[i] - mn);
      l[i] = fmaf(l[i], al, sum); m[i] = mn;
      #pragma unroll
      for(int g = 0; g < VG; ++g)
        #pragma unroll
        for(int c = 0; c < 4; ++c) o[i][g][c] *= al;
    }
    // The tile's weighed values, `LK_VK` keys a step: queries tq + 8·i, values 4·(tk + 16·g).
    int nk = min(LK_FB, jhi - jt);
    for(int vstep = 0; vstep < VS; ++vstep){
      int k0 = vstep * LK_VK;
      if(k0 >= nk){ asm volatile("cp.async.wait_group 0;\n"); break; }
      if(vstep + 1 < VS && k0 + LK_VK < nk){ stage_v(jt, k0 + LK_VK, (vstep + 1) & 1); asm volatile("cp.async.wait_group 1;\n"); }
      else asm volatile("cp.async.wait_group 0;\n");
      __syncthreads();
      const float* vst = vs + (vstep & 1) * LK_VK * VP;
      #pragma unroll 4
      for(int kk = 0; kk < LK_VK; ++kk){
        const float4* pr = (const float4*)(ps + (k0 + kk) * PP + QI * tq);
        float pw[QI];
        if constexpr(QI % 4 == 0){
          #pragma unroll
          for(int i4 = 0; i4 < QI / 4; ++i4){ float4 t = pr[i4]; pw[4 * i4] = t.x; pw[4 * i4 + 1] = t.y; pw[4 * i4 + 2] = t.z; pw[4 * i4 + 3] = t.w; }
        } else {
          #pragma unroll
          for(int i2 = 0; i2 < QI / 2; ++i2){ float2 t = ((const float2*)pr)[i2]; pw[2 * i2] = t.x; pw[2 * i2 + 1] = t.y; }
        }
        #pragma unroll
        for(int g = 0; g < VG; ++g){
          int col = 4 * (tk + 16 * g);
          if(col < VD){
            float4 v = *(const float4*)(vst + kk * VP + col);
            #pragma unroll
            for(int i = 0; i < QI; ++i){
              o[i][g][0] = fmaf(pw[i], v.x, o[i][g][0]); o[i][g][1] = fmaf(pw[i], v.y, o[i][g][1]);
              o[i][g][2] = fmaf(pw[i], v.z, o[i][g][2]); o[i][g][3] = fmaf(pw[i], v.w, o[i][g][3]);
            }
          }
        }
      }
      __syncthreads();
    }
  }
  #pragma unroll
  for(int i = 0; i < QI; ++i){
    int qi = tq + TQ * i, r = r0 + qi / HG, h = h0 + qi % HG;
    if(r >= rows) continue;
    float sk = sinks ? sinks[h] : -INFINITY, mf = fmaxf(sk, m[i]);
    float f = m[i] == -INFINITY ? 0.f : expf(m[i] - mf), lf = fmaf(l[i], f, sinks ? expf(sk - mf) : 0.f);
    float* op = out + ((long)r * heads + h) * VD;
    #pragma unroll
    for(int g = 0; g < VG; ++g){
      int col = 4 * (tk + 16 * g);
      if(col < VD) *(float4*)(op + col) = make_float4(o[i][g][0] * f / lf, o[i][g][1] * f / lf, o[i][g][2] * f / lf, o[i][g][3] * f / lf);
    }
  }
}
"#;

/// A wide chunk's attention on the f16 tensor cores, the one-word numerics' (llama.cpp's precision: f16 queries, keys,
/// values and weights, `f32` sums). An item is 64 query vectors — `HG` heads of one key/value group over `64 / HG`
/// consecutive rows — a block of four warps takes one, a warp 16 of its queries held in registers as `mma` fragments,
/// and walks the tiles of 64 keys (32 where heads or values pass 192 wide, `attn_mma_keys`) its rows attend, staged in
/// shared memory as f16, keys and values rows padded eight values: a binary16 cache's rows by `cp.async` and any other's
/// read into registers where a tile is four 16-byte pieces a thread (heads and values to 64 wide), each tile's values
/// arriving while its scores are taken and weighed and the next tile's keys while its values are weighed; wider rows of
/// other elements are staged at each tile's top. A warp's scores are `m16n8k16` products against the keys (`ldmatrix`),
/// its rows' running maxima and sums taken across the four lanes of each row — unmasked on the tiles every row attends
/// whole — and its weights, the scores' fragments rounded to f16, weigh the values (`ldmatrix.trans`) straight from
/// registers. Row `r` attends `lk_attention`'s keys — `[lo, end)` — and a sink joins each head's denominator. Split at
/// `kh` (with `kt`), each query and key is two parts as `lk_attention_tiled` reads them. A head is `HD` wide, its values
/// `VD`, both whole multiples of 16 and at most 256.
/// Without `SK` a block of a grid of row blocks by groups takes its item. Under `SK` (`attn_mma_stream`; the entry that
/// takes an `lk_amma_split` past the rows, its other arguments as one block an item takes them) the grid is
/// `attn_mma_schedule`'s: `ndp` blocks an item each behind the blocks the chip holds at once (`MB` a multiprocessor),
/// which split the tiles of the items past the whole waves evenly (stream-K) where that ends well before their longest
/// item would alone — each block's range meets two items at most — and each part's partial (its rows' maxima, sums and
/// weighed values, `part`) is folded by the last of its item's parts to arrive (`cnt`), in key order; an item one block
/// covers whole is the item a block computes, bit for bit. On the 3060 at Llama-3.2-1B's shape (32 heads over 8 groups,
/// 64 wide, 512 rows) after 3,584 keys: 1,236 µs one block an item over the `f32` cache, 866 µs over it under `SK`, 774
/// µs over a binary16 cache (20.8 TFLOPS of the causal products); Qwen3-8B's (128 wide) 2,502, 2,219 and 1,443 µs.
const ATTENTION_MMA: &str = r#"
__device__ __forceinline__ void lk_mma_h16(float* c, const unsigned* a, const unsigned* b){
  asm volatile("mma.sync.aligned.m16n8k16.row.col.f32.f16.f16.f32 {%0,%1,%2,%3}, {%4,%5,%6,%7}, {%8,%9}, {%0,%1,%2,%3};\n"
               : "+f"(c[0]), "+f"(c[1]), "+f"(c[2]), "+f"(c[3]) : "r"(a[0]), "r"(a[1]), "r"(a[2]), "r"(a[3]), "r"(b[0]), "r"(b[1]));
}
__device__ __forceinline__ void lk_ldsm4(unsigned* r, const void* p){
  unsigned s = (unsigned)__cvta_generic_to_shared(p);
  asm volatile("ldmatrix.sync.aligned.m8n8.x4.shared.b16 {%0,%1,%2,%3}, [%4];\n" : "=r"(r[0]), "=r"(r[1]), "=r"(r[2]), "=r"(r[3]) : "r"(s));
}
__device__ __forceinline__ void lk_ldsm4_t(unsigned* r, const void* p){
  unsigned s = (unsigned)__cvta_generic_to_shared(p);
  asm volatile("ldmatrix.sync.aligned.m8n8.x4.trans.shared.b16 {%0,%1,%2,%3}, [%4];\n" : "=r"(r[0]), "=r"(r[1]), "=r"(r[2]), "=r"(r[3]) : "r"(s));
}
__device__ __forceinline__ unsigned lk_h2(float a, float b){
  __half2 h = __floats2half2_rn(a, b);
  return *(unsigned*)&h;
}
template<typename E> struct lk_is_h16 { static constexpr bool v = false; };
template<> struct lk_is_h16<__half> { static constexpr bool v = true; };
// An item of the f16 tensor-core attention: 64 query vectors — `HG` heads of one key/value group over `64 / HG`
// consecutive rows — and the tiles of `KT` keys their rows attend, from the lowest key of the first row with any to the
// end of the last: item `i` is row block `i % nbx` of group `i / nbx`.
struct lk_amma_item { int r0, h0, lo, hi, tiles; };
template<int HG, int KT>
__device__ __forceinline__ lk_amma_item lk_amma_at(int i, int nbx, const int* pos, int len, int win, int rows){
  constexpr int BR = 64 / HG;
  lk_amma_item it;
  it.r0 = (i % nbx) * BR; it.h0 = (i / nbx) * HG;
  int last = min(it.r0 + BR, rows) - 1, p = pos ? *pos : 0;
  // A row's keys are [lk_window_lo(win, end), end), both ends rising with the row; a row of none (end 0) is past them.
  int e0 = pos ? p + it.r0 : len, e1 = pos ? p + last : len;
  if(e0 == 0) e0 = pos ? p + it.r0 + 1 : len;
  it.hi = last >= it.r0 ? e1 : 0;
  it.lo = it.hi > 0 ? lk_window_lo(win, e0) : 0;
  it.tiles = it.hi > it.lo ? (it.hi - it.lo + KT - 1) / KT : 0;
  return it;
}
// The stream-K range's blocks: block `c` of `s` takes its tiles [c·ts/s, (c+1)·ts/s); the block that takes tile `t`.
__device__ __forceinline__ long lk_amma_lo(int c, int s, long ts){ return (long)c * ts / s; }
__device__ __forceinline__ int lk_amma_block(long t, int s, long ts){
  int c = (int)(t * s / ts);
  while(c + 1 < s && lk_amma_lo(c + 1, s, ts) <= t) ++c;
  while(c > 0 && lk_amma_lo(c, s, ts) > t) --c;
  return c;
}
// A block's unit: tiles [ta, tb) of item `i`'s `tiles`, the item's first tile `base` tiles into the range.
struct lk_amma_unit { int i, ta, tb, tiles; long base; };
template<int HG, int HD, int VD, typename KE, typename VE, typename TE, int MB, int SK>
__device__ __forceinline__ void lk_attention_mma_run(const float* __restrict__ q, const KE* __restrict__ kc,
    const VE* __restrict__ vc, const int* __restrict__ pos, const float* __restrict__ sinks, float* __restrict__ out,
    int len, int win, int group, int heads, int kvw, int vw, int voff, float scale, int rows,
    const float* __restrict__ qt, long qp, long qtp, const TE* __restrict__ kt, long ktw, long kto, int kh, float cap,
    float* __restrict__ part, int* __restrict__ cnt, int ndp){
  constexpr int BQ = 64, BR = BQ / HG, KS = HD / 16, NV = VD / 8, KP = HD + 8, VP = VD + 8;
  // The keys a tile: 64, or 32 where heads or values pass 192 wide, so the values' sums fit a warp's registers.
  constexpr int KT = HD > 192 || VD > 192 ? 32 : 64;
  // A tile's 16-byte f16 pieces (eight values of a row) a thread; binary16 rows arrive by `cp.async`, others' are read
  // into registers where four pieces a thread hold them, and wider ones are staged at each tile's top.
  constexpr int KN = (KT * HD / 8 + 127) / 128, VN = (KT * VD / 8 + 127) / 128;
  constexpr bool KA = lk_is_h16<KE>::v && lk_is_h16<TE>::v, VA = lk_is_h16<VE>::v;
  constexpr bool KR = !KA && KN <= 4, VR = !VA && VN <= 4, TOP = (!KA && !KR) || (!VA && !VR);
  constexpr int SN = (KR ? KN : 0) > (VR ? VN : 0) ? (KR ? KN : 0) : (VR ? VN : 1);
  // A partial: each of the 64 queries' maximum and sum, then their weighed values, each thread's fragments in place.
  constexpr int PW = 2 * BQ + BQ * VD;
  extern __shared__ __align__(16) __half lk_hm[];
  __half* ks = lk_hm;                      // [KT][KP]: the tile's keys
  __half* vs = ks + KT * KP;               // [KT][VP]: its values
  __shared__ int loq[BQ], endq[BQ], sk_most, sk_least, sk_split, sk_last;
  __shared__ long sk_ts;
  __shared__ lk_amma_unit un;
  const int tid = threadIdx.x, lane = tid & 31, warp = tid >> 5, g = lane >> 2, t4 = lane & 3;
  const int qa = 16 * warp + g, qb = qa + 8, mi = lane >> 3, ri = lane & 7;
  const int nbx = (rows + BR - 1) / BR, items = nbx * (heads / HG);
  // Under SK the stream-K blocks come first: blocks [0, s) split the tiles of items [ndp, items) evenly, the blocks past
  // them an item each; without it a block of a grid of row blocks by groups takes its item.
  const int s = SK ? (int)gridDim.x - ndp : 0, c = SK && (int)blockIdx.x < s ? (int)blockIdx.x : -1;
  if constexpr(SK){
    if(c >= 0){
      if(tid == 0){ sk_ts = 0; sk_most = 0; sk_least = 0x7fffffff; }
      __syncthreads();
      long mine = 0; int most = 0, least = 0x7fffffff;
      for(int i = ndp + tid; i < items; i += 128){
        int n = lk_amma_at<HG, KT>(i, nbx, pos, len, win, rows).tiles;
        mine += n; most = max(most, n); least = min(least, n);
      }
      atomicAdd((unsigned long long*)&sk_ts, (unsigned long long)mine);
      atomicMax(&sk_most, most); atomicMin(&sk_least, least);
      __syncthreads();
    }
  }
  if(tid == 0){
    if(c < 0){
      int i = SK ? (int)blockIdx.x - s : (int)(blockIdx.x + blockIdx.y * gridDim.x);
      int n = lk_amma_at<HG, KT>(i, nbx, pos, len, win, rows).tiles;
      un = {i, 0, n, n, 0};
    } else {
      // The range splits its items where its blocks' even shares, with what a part costs past its tiles (its queries
      // read again, its partial kept and read back), end well before its longest item would alone, and no share
      // outlasts its shortest, so a block's range meets two items at most; otherwise each block takes an item whole.
      constexpr int OVT = 2 * (64 * HD * 4 + 2 * 64 * VD * 4) / (KT * (HD + VD) * 2);
      const long ts = sk_ts, share = (ts + s - 1) / s;
      sk_split = 4 * (share + OVT) < 3L * sk_most && share <= sk_least;
      if(!sk_split){
        int i = ndp + c, n = i < items ? lk_amma_at<HG, KT>(i, nbx, pos, len, win, rows).tiles : 0;
        if(i < items) un = {i, 0, n, n, 0}; else un.i = -1;
      } else {
        const long t0 = lk_amma_lo(c, s, ts), t1 = lk_amma_lo(c + 1, s, ts);
        long base = 0; int i = ndp, n = 0;
        for(; i < items; ++i, base += n){ n = lk_amma_at<HG, KT>(i, nbx, pos, len, win, rows).tiles; if(base + n > t0) break; }
        if(i >= items || base >= t1 || t0 >= t1) un.i = -1;
        else un = {i, (int)max(t0 - base, 0L), (int)min(t1 - base, (long)n), n, base};
      }
    }
  }
  // A unit: its item's queries and span, its tiles, and its end — whole, or a part kept and, the last to arrive, folded.
  auto run_unit = [&](){
    const int ui = un.i;
    const lk_amma_item it = lk_amma_at<HG, KT>(ui, nbx, pos, len, win, rows);
    const int r0 = it.r0, h0 = it.h0, jlo = it.lo, jhi = it.hi;
    const long koff = (long)(h0 / group) * (kt ? kh : HD), vo = voff + (long)(h0 / group) * VD;
    if(tid < BQ){
      int r = r0 + tid / HG;
      int end = r < rows ? (pos ? *pos + r : len) : 0;
      endq[tid] = end; loq[tid] = lk_window_lo(win, end);
    }
    // The warp's queries: rows g and g + 8 of its 16, as A fragments of every 16 values of the head.
    unsigned qf[KS][4];
    {
      auto qrow = [&](int qi, int d) -> const float* {
        int r = r0 + qi / HG, h = h0 + qi % HG;
        if(r >= rows) return nullptr;
        long u = (long)r * heads + h;
        return !kt ? q + u * HD + d : d < kh ? q + u * qp + d : qt + u * qtp + (d - kh);
      };
      #pragma unroll
      for(int k = 0; k < KS; ++k){
        int d0 = 16 * k + 2 * t4, d1 = d0 + 8;
        const float *a0 = qrow(qa, d0), *a1 = qrow(qb, d0), *a2 = qrow(qa, d1), *a3 = qrow(qb, d1);
        float2 v0 = a0 ? *(const float2*)a0 : make_float2(0.f, 0.f), v1 = a1 ? *(const float2*)a1 : make_float2(0.f, 0.f);
        float2 v2 = a2 ? *(const float2*)a2 : make_float2(0.f, 0.f), v3 = a3 ? *(const float2*)a3 : make_float2(0.f, 0.f);
        qf[k][0] = lk_h2(v0.x, v0.y); qf[k][1] = lk_h2(v1.x, v1.y); qf[k][2] = lk_h2(v2.x, v2.y); qf[k][3] = lk_h2(v3.x, v3.y);
      }
    }
    // Interior tiles — at or past the last row's lowest key and through the first row's end — take their scores unmasked.
    const int endmin = pos ? *pos + r0 : len, lomax = lk_window_lo(win, pos ? *pos + min(r0 + BR, rows) - 1 : len);
    __syncthreads();
    const int loa = loq[qa], enda = endq[qa], lob = loq[qb], endb = endq[qb];
    float4 st[SN][2];
    auto k_piece = [&](int n, int jt, int& kj, int& d, int& j){ int cc = tid + 128 * n; kj = cc / (HD / 8); d = 8 * (cc % (HD / 8)); j = jt + kj; return cc < KT * HD / 8; };
    auto v_piece = [&](int n, int jt, int& kj, int& e, int& j){ int cc = tid + 128 * n; kj = cc / (VD / 8); e = 8 * (cc % (VD / 8)); j = jt + kj; return cc < KT * VD / 8; };
    auto put8 = [&](__half* dst, const float4* v){
      *(uint4*)dst = make_uint4(lk_h2(v[0].x, v[0].y), lk_h2(v[0].z, v[0].w), lk_h2(v[1].x, v[1].y), lk_h2(v[1].z, v[1].w));
    };
    // A tile's keys issued (`cp.async`, or read into registers) and put in place (stored from the registers); values alike.
    auto fetch_k = [&](int jt){
      #pragma unroll
      for(int n = 0; n < KN; ++n){
        int kj, d, j; bool on = k_piece(n, jt, kj, d, j), in = on && j < jhi;
        if constexpr(KA){
          if(on) lk_cp16(ks + kj * KP + d, in ? (kt && d >= kh ? (const void*)(kt + (long)j * ktw + kto + (d - kh)) : (const void*)(kc + (long)j * kvw + koff + d)) : (const void*)kc, in);
        } else if constexpr(KR){
          if(in){
            st[n][0] = kt && d >= kh ? lk_from4(kt + (long)j * ktw + kto + (d - kh)) : lk_from4(kc + (long)j * kvw + koff + d);
            st[n][1] = kt && d + 4 >= kh ? lk_from4(kt + (long)j * ktw + kto + (d + 4 - kh)) : lk_from4(kc + (long)j * kvw + koff + d + 4);
          } else { st[n][0] = make_float4(0.f, 0.f, 0.f, 0.f); st[n][1] = st[n][0]; }
        }
      }
    };
    auto put_k = [&](int jt){
      if constexpr(KR){
        #pragma unroll
        for(int n = 0; n < KN; ++n){ int kj, d, j; if(k_piece(n, jt, kj, d, j)) put8(ks + kj * KP + d, st[n]); }
      }
    };
    auto fetch_v = [&](int jt){
      #pragma unroll
      for(int n = 0; n < VN; ++n){
        int kj, e, j; bool on = v_piece(n, jt, kj, e, j), in = on && j < jhi;
        if constexpr(VA){
          if(on) lk_cp16(vs + kj * VP + e, in ? (const void*)(vc + (long)j * vw + vo + e) : (const void*)vc, in);
        } else if constexpr(VR){
          if(in){ st[n][0] = lk_from4(vc + (long)j * vw + vo + e); st[n][1] = lk_from4(vc + (long)j * vw + vo + e + 4); }
          else { st[n][0] = make_float4(0.f, 0.f, 0.f, 0.f); st[n][1] = st[n][0]; }
        }
      }
    };
    auto put_v = [&](int jt){
      if constexpr(VR){
        #pragma unroll
        for(int n = 0; n < VN; ++n){ int kj, e, j; if(v_piece(n, jt, kj, e, j)) put8(vs + kj * VP + e, st[n]); }
      }
    };
    auto landed = [&](){ if constexpr(KA || VA) asm volatile("cp.async.wait_all;\n" ::: "memory"); };
    // Scores: the warp's 16 queries against the tile's keys, tiles of eight; each row's running maximum and sum across
    // its four lanes, the weights rounded to f16 as A fragments; the weighed values, 16 keys a step.
    float o[NV][4], ma = -INFINITY, mb = -INFINITY, la = 0.f, lb = 0.f;
    #pragma unroll
    for(int n = 0; n < NV; ++n)
      #pragma unroll
      for(int x = 0; x < 4; ++x) o[n][x] = 0.f;
    auto scores = [&](float (&sc)[KT / 8][4]){
      #pragma unroll
      for(int n = 0; n < KT / 8; ++n)
        #pragma unroll
        for(int x = 0; x < 4; ++x) sc[n][x] = 0.f;
      #pragma unroll
      for(int k = 0; k < KS; ++k)
        #pragma unroll
        for(int np = 0; np < KT / 16; ++np){
          unsigned b[4];
          lk_ldsm4(b, ks + (16 * np + (mi >= 2 ? 8 : 0) + ri) * KP + 16 * k + ((mi & 1) ? 8 : 0));
          lk_mma_h16(sc[2 * np], qf[k], b);
          lk_mma_h16(sc[2 * np + 1], qf[k], b + 2);
        }
    };
    auto weigh = [&](float (&sc)[KT / 8][4], int jt, bool full){
      float xa = -INFINITY, xb = -INFINITY, sa = 0.f, sb = 0.f, na, nb;
      if(full){
        #pragma unroll
        for(int n = 0; n < KT / 8; ++n)
          #pragma unroll
          for(int x = 0; x < 2; ++x){
            sc[n][x] = lk_cap(sc[n][x] * scale, cap); sc[n][x + 2] = lk_cap(sc[n][x + 2] * scale, cap);
            xa = fmaxf(xa, sc[n][x]); xb = fmaxf(xb, sc[n][x + 2]);
          }
        #pragma unroll
        for(int x = 1; x < 4; x <<= 1){ xa = fmaxf(xa, __shfl_xor_sync(0xffffffffu, xa, x)); xb = fmaxf(xb, __shfl_xor_sync(0xffffffffu, xb, x)); }
        na = fmaxf(ma, xa); nb = fmaxf(mb, xb);
        #pragma unroll
        for(int n = 0; n < KT / 8; ++n)
          #pragma unroll
          for(int x = 0; x < 2; ++x){
            sc[n][x] = expf(sc[n][x] - na); sc[n][x + 2] = expf(sc[n][x + 2] - nb);
            sa += sc[n][x]; sb += sc[n][x + 2];
          }
      } else {
        #pragma unroll
        for(int n = 0; n < KT / 8; ++n)
          #pragma unroll
          for(int x = 0; x < 2; ++x){
            int j = jt + 8 * n + 2 * t4 + x;
            sc[n][x] = j >= loa && j < enda ? lk_cap(sc[n][x] * scale, cap) : -INFINITY;
            sc[n][x + 2] = j >= lob && j < endb ? lk_cap(sc[n][x + 2] * scale, cap) : -INFINITY;
            xa = fmaxf(xa, sc[n][x]); xb = fmaxf(xb, sc[n][x + 2]);
          }
        #pragma unroll
        for(int x = 1; x < 4; x <<= 1){ xa = fmaxf(xa, __shfl_xor_sync(0xffffffffu, xa, x)); xb = fmaxf(xb, __shfl_xor_sync(0xffffffffu, xb, x)); }
        na = fmaxf(ma, xa); nb = fmaxf(mb, xb);
        #pragma unroll
        for(int n = 0; n < KT / 8; ++n)
          #pragma unroll
          for(int x = 0; x < 2; ++x){
            sc[n][x] = sc[n][x] == -INFINITY ? 0.f : expf(sc[n][x] - na);
            sc[n][x + 2] = sc[n][x + 2] == -INFINITY ? 0.f : expf(sc[n][x + 2] - nb);
            sa += sc[n][x]; sb += sc[n][x + 2];
          }
      }
      #pragma unroll
      for(int x = 1; x < 4; x <<= 1){ sa += __shfl_xor_sync(0xffffffffu, sa, x); sb += __shfl_xor_sync(0xffffffffu, sb, x); }
      float aa = ma == -INFINITY ? 0.f : expf(ma - na), ab = mb == -INFINITY ? 0.f : expf(mb - nb);
      la = fmaf(la, aa, sa); lb = fmaf(lb, ab, sb); ma = na; mb = nb;
      #pragma unroll
      for(int n = 0; n < NV; ++n){ o[n][0] *= aa; o[n][1] *= aa; o[n][2] *= ab; o[n][3] *= ab; }
    };
    auto values = [&](float (&sc)[KT / 8][4]){
      #pragma unroll
      for(int k = 0; k < KT / 16; ++k){
        unsigned pa[4] = {lk_h2(sc[2 * k][0], sc[2 * k][1]), lk_h2(sc[2 * k][2], sc[2 * k][3]),
                          lk_h2(sc[2 * k + 1][0], sc[2 * k + 1][1]), lk_h2(sc[2 * k + 1][2], sc[2 * k + 1][3])};
        #pragma unroll
        for(int np = 0; np < NV / 2; ++np){
          unsigned b[4];
          lk_ldsm4_t(b, vs + (16 * k + ((mi & 1) ? 8 : 0) + ri) * VP + 16 * np + (mi >= 2 ? 8 : 0));
          lk_mma_h16(o[2 * np], pa, b);
          lk_mma_h16(o[2 * np + 1], pa, b + 2);
        }
      }
    };
    const int ja = jlo + un.ta * KT, jb = min(jlo + un.tb * KT, jhi);
    if constexpr(TOP){
      // Each tile's keys and values staged as f16 at its top, four values a thread at a time, rows past the span zero.
      for(int jt = ja; jt < jb; jt += KT){
        for(int cc = tid; cc < KT * (HD / 4); cc += 128){
          int kj = cc / (HD / 4), d = 4 * (cc % (HD / 4)), j = jt + kj;
          float4 v = make_float4(0.f, 0.f, 0.f, 0.f);
          if(j < jhi) v = kt && d >= kh ? lk_from4(kt + (long)j * ktw + kto + (d - kh)) : lk_from4(kc + (long)j * kvw + koff + d);
          unsigned* dst = (unsigned*)(ks + kj * KP + d);
          dst[0] = lk_h2(v.x, v.y); dst[1] = lk_h2(v.z, v.w);
        }
        for(int cc = tid; cc < KT * (VD / 4); cc += 128){
          int kj = cc / (VD / 4), e = 4 * (cc % (VD / 4)), j = jt + kj;
          float4 v = j < jhi ? lk_from4(vc + (long)j * vw + vo + e) : make_float4(0.f, 0.f, 0.f, 0.f);
          unsigned* dst = (unsigned*)(vs + kj * VP + e);
          dst[0] = lk_h2(v.x, v.y); dst[1] = lk_h2(v.z, v.w);
        }
        __syncthreads();
        float sc[KT / 8][4];
        scores(sc);
        weigh(sc, jt, false);
        values(sc);
        __syncthreads();
      }
    } else {
      // A tile's values arrive while its scores are taken and weighed, the next tile's keys while its values are weighed.
      if(ja < jb){ fetch_k(ja); put_k(ja); }
      landed();
      __syncthreads();
      for(int jt = ja; jt < jb; jt += KT){
        fetch_v(jt);
        float sc[KT / 8][4];
        scores(sc);
        weigh(sc, jt, jt >= lomax && jt + KT <= endmin);
        put_v(jt);
        landed();
        __syncthreads();
        const bool more = jt + KT < jb;
        if(more) fetch_k(jt + KT);
        values(sc);
        if(more) put_k(jt + KT);
        landed();
        __syncthreads();
      }
    }
    // A whole item finishes; a part is kept, and the last of its item's parts to arrive folds them in key order and
    // finishes it.
    bool whole = !SK || (un.ta == 0 && un.tb == un.tiles);
    if(!whole){
      const long ts = sk_ts, base = un.base;
      float* p = part + (long)(2 * c + (lk_amma_lo(c, s, ts) >= base ? 0 : 1)) * PW;
      if(t4 == 0){ p[2 * qa] = ma; p[2 * qa + 1] = la; p[2 * qb] = mb; p[2 * qb + 1] = lb; }
      #pragma unroll
      for(int n = 0; n < NV; ++n)
        *(float4*)(p + 2 * BQ + ((warp * NV + n) * 32 + lane) * 4) = make_float4(o[n][0], o[n][1], o[n][2], o[n][3]);
      __threadfence();
      __syncthreads();
      const int cf = lk_amma_block(base, s, ts), cl = lk_amma_block(base + un.tiles - 1, s, ts);
      if(tid == 0){
        int parts = 0;
        for(int k = cf; k <= cl; ++k) parts += lk_amma_lo(k + 1, s, ts) > lk_amma_lo(k, s, ts);
        sk_last = atomicAdd(cnt + ui, 1) == parts - 1;
      }
      __syncthreads();
      if(sk_last){
        __threadfence();
        ma = -INFINITY; mb = -INFINITY; la = 0.f; lb = 0.f;
        #pragma unroll
        for(int n = 0; n < NV; ++n)
          #pragma unroll
          for(int x = 0; x < 4; ++x) o[n][x] = 0.f;
        for(int k = cf; k <= cl; ++k){
          const long lo = lk_amma_lo(k, s, ts);
          if(lk_amma_lo(k + 1, s, ts) <= lo) continue;
          // Block k's part: in its first slot where its range begins in the item, else in its second.
          const float* pk = part + (long)(2 * k + (lo >= base ? 0 : 1)) * PW;
          float pma = __ldcg(pk + 2 * qa), pla = __ldcg(pk + 2 * qa + 1), pmb = __ldcg(pk + 2 * qb), plb = __ldcg(pk + 2 * qb + 1);
          float na = fmaxf(ma, pma), nb = fmaxf(mb, pmb);
          float fa = ma == -INFINITY ? 0.f : expf(ma - na), ga = pma == -INFINITY ? 0.f : expf(pma - na);
          float fb = mb == -INFINITY ? 0.f : expf(mb - nb), gb = pmb == -INFINITY ? 0.f : expf(pmb - nb);
          la = fmaf(pla, ga, la * fa); lb = fmaf(plb, gb, lb * fb); ma = na; mb = nb;
          #pragma unroll
          for(int n = 0; n < NV; ++n){
            float4 v = __ldcg((const float4*)(pk + 2 * BQ + ((warp * NV + n) * 32 + lane) * 4));
            o[n][0] = fmaf(v.x, ga, o[n][0] * fa); o[n][1] = fmaf(v.y, ga, o[n][1] * fa);
            o[n][2] = fmaf(v.z, gb, o[n][2] * fb); o[n][3] = fmaf(v.w, gb, o[n][3] * fb);
          }
        }
        if(tid == 0) cnt[ui] = 0;
        whole = true;
      }
    }
    // Each row's outputs, its sink in the denominator.
    if(whole){
      #pragma unroll
      for(int half = 0; half < 2; ++half){
        int qi = half ? qb : qa, r = r0 + qi / HG, h = h0 + qi % HG;
        if(r >= rows) continue;
        float m = half ? mb : ma, l = half ? lb : la;
        float sk = sinks ? sinks[h] : -INFINITY, mf = fmaxf(sk, m);
        float f = m == -INFINITY ? 0.f : expf(m - mf), lf = fmaf(l, f, sinks ? expf(sk - mf) : 0.f);
        float* op = out + ((long)r * heads + h) * VD;
        #pragma unroll
        for(int n = 0; n < NV; ++n)
          *(float2*)(op + 8 * n + 2 * t4) = make_float2(o[n][2 * half] * f / lf, o[n][2 * half + 1] * f / lf);
      }
    }
  };
  // One item a block runs its one unit, its tile loop compiled outside any loop over units; under SK a block walks its
  // units (two at most).
  __syncthreads();
  if constexpr(!SK){
    if(un.i >= 0) run_unit();
  } else {
    for(;;){
      if(un.i < 0) break;
      run_unit();
      __syncthreads();
      if(tid == 0){
        // The next unit: a stream-K block's next item, while its range reaches it.
        if(c < 0 || !sk_split) un.i = -1;
        else {
          const long t1 = lk_amma_lo(c + 1, s, sk_ts), base = un.base + un.tiles;
          const int i = un.i + 1;
          if(i >= items || base >= t1) un.i = -1;
          else { int n = lk_amma_at<HG, KT>(i, nbx, pos, len, win, rows).tiles; un = {i, 0, (int)min(t1 - base, (long)n), n, base}; }
        }
      }
      __syncthreads();
    }
  }
}
// One block an item, on a grid of row blocks by groups.
template<int HG, int HD, int VD, typename KE = float, typename VE = float, typename TE = KE>
__global__ void __launch_bounds__(128) lk_attention_mma(const float* __restrict__ q, const KE* __restrict__ kc,
    const VE* __restrict__ vc, const int* __restrict__ pos, const float* __restrict__ sinks, float* __restrict__ out,
    int len, int win, int group, int heads, int kvw, int vw, int voff, float scale, int rows,
    const float* __restrict__ qt = nullptr, long qp = 0, long qtp = 0, const TE* __restrict__ kt = nullptr, long ktw = 0,
    long kto = 0, int kh = 0, float cap = 0.f){
  lk_attention_mma_run<HG, HD, VD, KE, VE, TE, 1, 0>(q, kc, vc, pos, sinks, out, len, win, group, heads, kvw, vw, voff, scale, rows,
                                                    qt, qp, qtp, kt, ktw, kto, kh, cap, nullptr, nullptr, 0);
}
// Under the stream-K schedule (`SK`, `MB` blocks a multiprocessor): past the rows, the partials, their counts and the
// items a block each.
struct lk_amma_split { float* part; int* cnt; int ndp; };
template<int HG, int HD, int VD, typename KE, typename VE, typename TE, int MB, int SK>
__global__ void __launch_bounds__(128, MB) lk_attention_mma(const float* __restrict__ q, const KE* __restrict__ kc,
    const VE* __restrict__ vc, const int* __restrict__ pos, const float* __restrict__ sinks, float* __restrict__ out,
    int len, int win, int group, int heads, int kvw, int vw, int voff, float scale, int rows, lk_amma_split sp,
    const float* __restrict__ qt = nullptr, long qp = 0, long qtp = 0, const TE* __restrict__ kt = nullptr, long ktw = 0,
    long kto = 0, int kh = 0, float cap = 0.f){
  lk_attention_mma_run<HG, HD, VD, KE, VE, TE, MB, SK>(q, kc, vc, pos, sinks, out, len, win, group, heads, kvw, vw, voff, scale, rows,
                                                      qt, qp, qtp, kt, ktw, kto, kh, cap, sp.part, sp.cnt, sp.ndp);
}
"#;

/// The query vectors a block of the f16 tensor-core attention (`lk_attention_mma`) takes, a warp 16 of them.
pub const ATTN_MMA_QUERIES: usize = 64;

/// The keys a tile of the f16 tensor-core attention takes: 64, or 32 where heads or values pass 192 wide.
pub fn attn_mma_keys(hd: usize, vd: usize) -> usize {
    if hd > 192 || vd > 192 {
        32
    } else {
        64
    }
}

/// The f16 tensor-core attention block's dynamic shared memory: a tile's keys (`attn_mma_keys`) and their values as
/// f16, rows padded eight values.
pub fn attn_mma_smem_bytes(hd: usize, vd: usize) -> usize {
    attn_mma_keys(hd, vd) * ((hd + 8) * 2 + (vd + 8) * 2)
}

/// The blocks of the f16 tensor-core attention a multiprocessor holds under the stream-K schedule (its `MB`, the
/// `__launch_bounds__` minimum): three where heads and values are at most 64 wide — 168 registers a thread, tiles of
/// 18 KiB — two past them.
pub fn attn_mma_blocks(hd: usize, vd: usize) -> usize {
    if hd <= 64 && vd <= 64 {
        3
    } else {
        2
    }
}

/// The items of a chunk's f16 tensor-core attention over `rows` rows of `heads` heads, `hg` of a group a block: 64
/// query vectors each, `64 / hg` rows of one group.
pub fn attn_mma_items(rows: usize, heads: usize, hg: usize) -> usize {
    rows.div_ceil(ATTN_MMA_QUERIES / hg) * (heads / hg)
}

/// Whether a chunk's f16 tensor-core attention takes the stream-K schedule (its `SK`): over a binary16 cache
/// (`sixteen`) at every width, over another where heads and values are at most 128 wide. Past that a tile's rows of
/// other elements are staged at its top and one block an item runs faster (the 3060, Gemma-3-1B's 256-wide heads over
/// the `f32` cache at the first chunk: 133 µs one block an item, 167 µs under the schedule).
pub fn attn_mma_stream(hd: usize, vd: usize, sixteen: bool) -> bool {
    sixteen || (hd <= 128 && vd <= 128)
}

/// The stream-K blocks of a chunk's f16 tensor-core attention on a chip of `sms` multiprocessors: as many as it holds
/// at once.
pub fn attn_mma_stream_blocks(hd: usize, vd: usize, sms: usize) -> usize {
    sms * attn_mma_blocks(hd, vd)
}

/// A stream-K launch of `items` items beside `s` stream-K blocks, `(ndp, grid)`: the items past the whole waves split
/// over the `s` blocks, the rest a block each behind them — every item a block when the waves are whole, every item in
/// the split when there is less than one.
pub fn attn_mma_schedule(items: usize, s: usize) -> (usize, usize) {
    if items % s == 0 {
        (items, items)
    } else if items < s {
        (0, s)
    } else {
        (items / s * s, items / s * s + s)
    }
}

/// The floats of a stream-K block's partial: each of the 64 queries' maximum and sum, then their `vd` weighed values.
pub fn attn_mma_part_floats(vd: usize) -> usize {
    2 * ATTN_MMA_QUERIES + ATTN_MMA_QUERIES * vd
}

/// A wide chunk's attention on the int8 tensor cores in two words, the two-word numerics' (`lk_attention_i8`, each
/// query, key, value and weight `s·(h + l/128)` under its row's largest magnitude, `≈ 2⁻¹⁵` of it). `lk_kv_two_words`
/// takes every key and value row the chunk's rows attend into two words once, a block a run of 32 keys of one key/value
/// group: keys `[kv][lpad][HD]`, values a column a row a run at a time `[kv][lpad / 32][VD][32]` with each run's keys in
/// the order the weights' A fragments take them — positions `4b … 4b + 3` the keys `2b, 2b + 1, 2b + 8, 2b + 9` (`b < 4`),
/// 16 on for `b ≥ 4`, so a score tile's accumulators are the weights' fragments as they lie — and each row's scale
/// `[kv][lpad]`; a split key's parts are read as `lk_attention_tiled` reads them. A block of four warps then takes 64 query
/// vectors — `HG` heads of one group over `64 / HG` consecutive rows — a warp 16 of them, its queries' two words in
/// registers, and walks tiles of 64 keys (32 where heads or values pass 128 wide) from a whole tile below its rows' span,
/// each tile's keys, value runs and scales arriving by `cp.async` — the next tile's keys while this one's weights weigh
/// its values, the next tile's values while the next scores are taken. A score is `m16n8k32` dots folded into one exact
/// integer, `128·qₕ·kₕ + qₕ·kₗ + qₗ·kₕ`, and its low dot `qₗ·kₗ` into another, times both scales — every product of the
/// stored words — so the scores are the two-word operands' to `f32` rounding; the running maximum and sum are
/// `lk_attention_mma`'s; each
/// weight times its value row's scale is taken into three words under its row's largest in the tile, and the weighed
/// values fold alike, their next-finest products in a second integer: the attention computes, to `f32` rounding, the
/// attention of its queries, keys and values taken into two words. Row `r` attends `lk_attention`'s keys — `[lo, end)` — and a sink joins each head's denominator.
/// Split at `kh` (with `qt`), each query is two parts. A head is `HD` wide, its values `VD`, both whole multiples of 32 and
/// at most 256.
const ATTENTION_I8: &str = r#"
template<int HD, int VD, typename KE = float, typename VE = float, typename TE = float>
__global__ void __launch_bounds__(256) lk_kv_two_words(const KE* __restrict__ kc, const VE* __restrict__ vc,
    const int* __restrict__ pos, int rows, int len, int kvw, int vw, int voff, int lpad,
    signed char* __restrict__ k8h, signed char* __restrict__ k8l, float* __restrict__ ksc,
    signed char* __restrict__ v8h, signed char* __restrict__ v8l, float* __restrict__ vsc,
    const TE* __restrict__ kt = nullptr, long ktw = 0, long kto = 0, int kh = 0){
  __shared__ __align__(16) int th[32][VD / 4], tl[32][VD / 4];  // the run's value rows' words, a key a row
  int hi = pos ? min(len, *pos + rows - 1) : len;
  int j0 = 32 * blockIdx.x, kv = blockIdx.y;
  if(j0 >= hi) return;
  int lane = threadIdx.x & 31, warp = threadIdx.x >> 5;
  long koff = (long)kv * (kt ? kh : HD), vo = voff + (long)kv * VD, row = (long)kv * lpad;
  auto amax4 = [](float m, float4 v){ return fmaxf(fmaxf(m, fmaxf(fabsf(v.x), fabsf(v.y))), fmaxf(fabsf(v.z), fabsf(v.w))); };
  constexpr int KI = (HD / 4 + 31) / 32, VI = (VD / 4 + 31) / 32;
  for(int kk = warp; kk < 32; kk += 8){
    int j = j0 + kk;
    bool in = j < hi;
    float4 v[KI > VI ? KI : VI];
    float a = 0.f;
    #pragma unroll
    for(int i = 0; i < KI; ++i){
      int c = lane + 32 * i, d = 4 * c;
      v[i] = !(in && c < HD / 4) ? make_float4(0.f, 0.f, 0.f, 0.f)
             : kt && d >= kh ? lk_from4(kt + (long)j * ktw + kto + (d - kh)) : lk_from4(kc + (long)j * kvw + koff + d);
      a = amax4(a, v[i]);
    }
    #pragma unroll
    for(int x = 16; x > 0; x >>= 1) a = fmaxf(a, __shfl_xor_sync(0xffffffffu, a, x));
    float s = lk_two_scale(a), rs = 1.f / s;
    #pragma unroll
    for(int i = 0; i < KI; ++i){
      int c = lane + 32 * i;
      if(c >= HD / 4) break;
      int h[4], l[4];
      lk_two_words_r(v[i].x, s, rs, h[0], l[0]); lk_two_words_r(v[i].y, s, rs, h[1], l[1]); lk_two_words_r(v[i].z, s, rs, h[2], l[2]); lk_two_words_r(v[i].w, s, rs, h[3], l[3]);
      *(int*)(k8h + (row + j) * HD + 4 * c) = lk_int8x4(h[0], h[1], h[2], h[3]);
      *(int*)(k8l + (row + j) * HD + 4 * c) = lk_int8x4(l[0], l[1], l[2], l[3]);
    }
    if(lane == 0) ksc[row + j] = in ? s : 0.f;
    a = 0.f;
    #pragma unroll
    for(int i = 0; i < VI; ++i){
      int c = lane + 32 * i;
      v[i] = in && c < VD / 4 ? lk_from4(vc + (long)j * vw + vo + 4 * c) : make_float4(0.f, 0.f, 0.f, 0.f);
      a = amax4(a, v[i]);
    }
    #pragma unroll
    for(int x = 16; x > 0; x >>= 1) a = fmaxf(a, __shfl_xor_sync(0xffffffffu, a, x));
    s = lk_two_scale(a); rs = 1.f / s;
    #pragma unroll
    for(int i = 0; i < VI; ++i){
      int c = lane + 32 * i;
      if(c >= VD / 4) break;
      int h[4], l[4];
      lk_two_words_r(v[i].x, s, rs, h[0], l[0]); lk_two_words_r(v[i].y, s, rs, h[1], l[1]); lk_two_words_r(v[i].z, s, rs, h[2], l[2]); lk_two_words_r(v[i].w, s, rs, h[3], l[3]);
      th[kk][c] = lk_int8x4(h[0], h[1], h[2], h[3]); tl[kk][c] = lk_int8x4(l[0], l[1], l[2], l[3]);
    }
    if(lane == 0) vsc[row + j] = in ? s : 0.f;
  }
  __syncthreads();
  long run = ((long)kv * (lpad / 32) + blockIdx.x) * VD * 32;
  // Each value column's 32 keys in the weights' order, 16 bytes a thread.
  const unsigned char* bh = (const unsigned char*)th;
  const unsigned char* bl = (const unsigned char*)tl;
  for(int c = threadIdx.x; c < VD * 2; c += 256){
    int d = c >> 1, half = c & 1;
    unsigned wh[4], wl[4];
    #pragma unroll
    for(int w = 0; w < 4; ++w){
      wh[w] = 0u; wl[w] = 0u;
      #pragma unroll
      for(int x = 0; x < 4; ++x){
        int kk = 16 * half + 2 * w + (x & 1) + 8 * (x >> 1);
        wh[w] |= (unsigned)bh[kk * VD + d] << (8 * x); wl[w] |= (unsigned)bl[kk * VD + d] << (8 * x);
      }
    }
    *(int4*)(v8h + run + d * 32 + 16 * half) = make_int4((int)wh[0], (int)wh[1], (int)wh[2], (int)wh[3]);
    *(int4*)(v8l + run + d * 32 + 16 * half) = make_int4((int)wl[0], (int)wl[1], (int)wl[2], (int)wl[3]);
  }
}
template<int HG, int HD, int VD>
__global__ void __launch_bounds__(128) lk_attention_i8(const float* __restrict__ q, const signed char* __restrict__ k8h,
    const signed char* __restrict__ k8l, const float* __restrict__ ksg, const signed char* __restrict__ v8h,
    const signed char* __restrict__ v8l, const float* __restrict__ vsg, int lpad, const int* __restrict__ pos,
    const float* __restrict__ sinks, float* __restrict__ out, int len, int win, int group, int heads, float scale, int rows,
    const float* __restrict__ qt = nullptr, long qp = 0, long qtp = 0, int kh = 0, float cap = 0.f){
  constexpr int BQ = 64, BR = BQ / HG, KS = HD / 32;
  constexpr int KT = HD > 128 || VD > 128 ? 32 : 64;
  constexpr int KP = HD + 16, VP = KT + 16;
  extern __shared__ __align__(16) signed char lk_im[];
  signed char* kh8 = lk_im;                 // [KT][KP]: the tile's keys, high words
  signed char* kl8 = kh8 + KT * KP;         // [KT][KP]: their low words
  signed char* vh8 = kl8 + KT * KP;         // [VD][VP]: the tile's values a column a row, the keys in the weights' order
  signed char* vl8 = vh8 + VD * VP;         // [VD][VP]: their low words
  float* ksc2 = (float*)(vl8 + VD * VP);    // [2][KT]: each key's scale, a buffer a tile's parity
  float* vsc2 = ksc2 + 2 * KT;              // [2][KT]: each value row's scale
  __shared__ int loq[BQ], endq[BQ], span[2];
  int tid = threadIdx.x, lane = tid & 31, warp = tid >> 5, g = lane >> 2, t4 = lane & 3;
  int r0 = blockIdx.x * BR, h0 = blockIdx.y * HG;
  long row = (long)(h0 / group) * lpad;
  if(tid < BQ){
    int r = r0 + tid / HG;
    int end = r < rows ? (pos ? *pos + r : len) : 0;
    endq[tid] = end; loq[tid] = lk_window_lo(win, end);
  }
  __syncthreads();
  if(tid == 0){
    int lo = 0x7fffffff, hi = 0;
    for(int i = 0; i < BQ; ++i) if(endq[i] > 0){ lo = min(lo, loq[i]); hi = max(hi, endq[i]); }
    span[0] = hi > 0 ? lo : 0; span[1] = hi;
  }
  // The warp's queries: rows g and g + 8 of its 16, each its two words under its largest magnitude's scale, as A
  // fragments of every 32 values of the head.
  int qa = 16 * warp + g, qb = qa + 8;
  auto qload = [&](int qi, int d) -> float4 {
    int r = r0 + qi / HG, h = h0 + qi % HG;
    if(r >= rows) return make_float4(0.f, 0.f, 0.f, 0.f);
    long u = (long)r * heads + h;
    return *(const float4*)(!qt ? q + u * HD + d : d < kh ? q + u * qp + d : qt + u * qtp + (d - kh));
  };
  auto amax4 = [](float m, float4 v){ return fmaxf(fmaxf(m, fmaxf(fabsf(v.x), fabsf(v.y))), fmaxf(fabsf(v.z), fabsf(v.w))); };
  float xa = 0.f, xb = 0.f;
  #pragma unroll
  for(int k = 0; k < KS; ++k)
    #pragma unroll
    for(int half = 0; half < 2; ++half){
      int d = 32 * k + 16 * half + 4 * t4;
      xa = amax4(xa, qload(qa, d)); xb = amax4(xb, qload(qb, d));
    }
  #pragma unroll
  for(int x = 1; x < 4; x <<= 1){ xa = fmaxf(xa, __shfl_xor_sync(0xffffffffu, xa, x)); xb = fmaxf(xb, __shfl_xor_sync(0xffffffffu, xb, x)); }
  const float sqa = lk_two_scale(xa), sqb = lk_two_scale(xb);
  int qh[KS][4], ql[KS][4];
  #pragma unroll
  for(int k = 0; k < KS; ++k)
    #pragma unroll
    for(int f = 0; f < 4; ++f){
      float4 v = qload(f & 1 ? qb : qa, 32 * k + 16 * (f >> 1) + 4 * t4);
      float s = f & 1 ? sqb : sqa;
      int h0_, l0_, h1_, l1_, h2_, l2_, h3_, l3_;
      lk_two_words(v.x, s, h0_, l0_); lk_two_words(v.y, s, h1_, l1_); lk_two_words(v.z, s, h2_, l2_); lk_two_words(v.w, s, h3_, l3_);
      qh[k][f] = lk_int8x4(h0_, h1_, h2_, h3_); ql[k][f] = lk_int8x4(l0_, l1_, l2_, l3_);
    }
  __syncthreads();
  // Tiles from a whole tile below the span's start, so each is whole runs of the values' order and whole scale chunks.
  int jlo = span[0] / KT * KT, jhi = span[1];
  int loa = loq[qa], enda = endq[qa], lob = loq[qb], endb = endq[qb];
  const float fqa = sqa * scale * 0.0078125f, fqb = sqb * scale * 0.0078125f;
  float o[VD / 8][4], ma = -INFINITY, mb = -INFINITY, la = 0.f, lb = 0.f;
  #pragma unroll
  for(int n = 0; n < VD / 8; ++n)
    #pragma unroll
    for(int c = 0; c < 4; ++c) o[n][c] = 0.f;
  // A tile's keys' two words and both its scales, into parity `pb`'s scale buffers; its value runs' two words. Past the
  // span zero. Each a `cp.async` group.
  auto load_k = [&](int jt){
    int pb = ((jt - jlo) / KT) & 1;
    for(int c = tid; c < KT * HD / 16; c += 128){
      int kj = c / (HD / 16), o16 = 16 * (c % (HD / 16));
      bool in = jt + kj < jhi;
      long at = (row + (in ? jt + kj : 0)) * HD + o16;
      lk_cp16(kh8 + kj * KP + o16, k8h + at, in); lk_cp16(kl8 + kj * KP + o16, k8l + at, in);
    }
    for(int c = tid; c < KT / 4; c += 128){
      bool in = jt + 4 * c < jhi;
      lk_cp16(ksc2 + KT * pb + 4 * c, ksg + row + (in ? jt + 4 * c : 0), in); lk_cp16(vsc2 + KT * pb + 4 * c, vsg + row + (in ? jt + 4 * c : 0), in);
    }
    asm volatile("cp.async.commit_group;\n");
  };
  auto load_v = [&](int jt){
    for(int c = tid; c < VD * KT / 16; c += 128){
      int d = c / (KT / 16), o16 = 16 * (c % (KT / 16)), r = (jt + o16) >> 5;
      bool in = 32 * r < jhi;
      long at = ((row / 32 + (in ? r : 0)) * VD + d) * 32 + (o16 & 31);
      lk_cp16(vh8 + d * VP + o16, v8h + at, in); lk_cp16(vl8 + d * VP + o16, v8l + at, in);
    }
    asm volatile("cp.async.commit_group;\n");
  };
  if(jlo < jhi){ load_k(jlo); load_v(jlo); }
  for(int jt = jlo; jt < jhi; jt += KT){
    asm volatile("cp.async.wait_group 1;\n");
    __syncthreads();
    const float* ksc = ksc2 + KT * (((jt - jlo) / KT) & 1);
    const float* vsc = vsc2 + KT * (((jt - jlo) / KT) & 1);
    // Scores: the warp's 16 queries against the tile's keys, every tile of eight keys at once; a score's high dot and its
    // two cross dots fold into one exact integer, 128·qₕ·kₕ + qₕ·kₗ + qₗ·kₕ, its low dot qₗ·kₗ, 128 times finer, into
    // another.
    float s[KT / 8][4];
    int mi = lane >> 3, ri = lane & 7;
    {
      int c[KT / 8][4];
      #pragma unroll
      for(int n = 0; n < KT / 8; ++n)
        #pragma unroll
        for(int i = 0; i < 4; ++i) c[n][i] = 0;
      #pragma unroll
      for(int k = 0; k < KS; ++k)
        #pragma unroll
        for(int np = 0; np < KT / 16; ++np){
          int bh[4];
          lk_ldsm_s8(bh, kh8 + (16 * np + (mi >= 2 ? 8 : 0) + ri) * KP + 32 * k + ((mi & 1) ? 16 : 0));
          lk_mma_s8(c[2 * np], qh[k], bh); lk_mma_s8(c[2 * np + 1], qh[k], bh + 2);
        }
      #pragma unroll
      for(int n = 0; n < KT / 8; ++n)
        #pragma unroll
        for(int i = 0; i < 4; ++i) c[n][i] *= 128;
      #pragma unroll
      for(int k = 0; k < KS; ++k){
        #pragma unroll
        for(int np = 0; np < KT / 16; ++np){
          int bl[4];
          lk_ldsm_s8(bl, kl8 + (16 * np + (mi >= 2 ? 8 : 0) + ri) * KP + 32 * k + ((mi & 1) ? 16 : 0));
          lk_mma_s8(c[2 * np], qh[k], bl); lk_mma_s8(c[2 * np + 1], qh[k], bl + 2);
        }
        #pragma unroll
        for(int np = 0; np < KT / 16; ++np){
          int bh[4];
          lk_ldsm_s8(bh, kh8 + (16 * np + (mi >= 2 ? 8 : 0) + ri) * KP + 32 * k + ((mi & 1) ? 16 : 0));
          lk_mma_s8(c[2 * np], ql[k], bh); lk_mma_s8(c[2 * np + 1], ql[k], bh + 2);
        }
      }
      int cl[KT / 8][4];
      #pragma unroll
      for(int n = 0; n < KT / 8; ++n)
        #pragma unroll
        for(int i = 0; i < 4; ++i) cl[n][i] = 0;
      #pragma unroll
      for(int k = 0; k < KS; ++k)
        #pragma unroll
        for(int np = 0; np < KT / 16; ++np){
          int bl[4];
          lk_ldsm_s8(bl, kl8 + (16 * np + (mi >= 2 ? 8 : 0) + ri) * KP + 32 * k + ((mi & 1) ? 16 : 0));
          lk_mma_s8(cl[2 * np], ql[k], bl); lk_mma_s8(cl[2 * np + 1], ql[k], bl + 2);
        }
      #pragma unroll
      for(int n = 0; n < KT / 8; ++n)
        #pragma unroll
        for(int i = 0; i < 4; ++i)
          s[n][i] = ((float)c[n][i] + (float)cl[n][i] * 0.0078125f) * (i < 2 ? fqa : fqb) * ksc[8 * n + 2 * t4 + (i & 1)];
    }
    __syncthreads();
    if(jt + KT < jhi) load_k(jt + KT);
    else asm volatile("cp.async.commit_group;\n");
    // Each row's running maximum and sum across its four lanes.
    float ya = -INFINITY, yb = -INFINITY;
    #pragma unroll
    for(int n = 0; n < KT / 8; ++n)
      #pragma unroll
      for(int c = 0; c < 2; ++c){
        int j = jt + 8 * n + 2 * t4 + c;
        s[n][c] = j >= loa && j < enda ? lk_cap(s[n][c], cap) : -INFINITY;
        s[n][c + 2] = j >= lob && j < endb ? lk_cap(s[n][c + 2], cap) : -INFINITY;
        ya = fmaxf(ya, s[n][c]); yb = fmaxf(yb, s[n][c + 2]);
      }
    #pragma unroll
    for(int x = 1; x < 4; x <<= 1){ ya = fmaxf(ya, __shfl_xor_sync(0xffffffffu, ya, x)); yb = fmaxf(yb, __shfl_xor_sync(0xffffffffu, yb, x)); }
    float na = fmaxf(ma, ya), nb = fmaxf(mb, yb), sa = 0.f, sb = 0.f;
    #pragma unroll
    for(int n = 0; n < KT / 8; ++n)
      #pragma unroll
      for(int c = 0; c < 2; ++c){
        s[n][c] = s[n][c] == -INFINITY ? 0.f : expf(s[n][c] - na);
        s[n][c + 2] = s[n][c + 2] == -INFINITY ? 0.f : expf(s[n][c + 2] - nb);
        sa += s[n][c]; sb += s[n][c + 2];
      }
    #pragma unroll
    for(int x = 1; x < 4; x <<= 1){ sa += __shfl_xor_sync(0xffffffffu, sa, x); sb += __shfl_xor_sync(0xffffffffu, sb, x); }
    float aa = ma == -INFINITY ? 0.f : expf(ma - na), ab = mb == -INFINITY ? 0.f : expf(mb - nb);
    la = fmaf(la, aa, sa); lb = fmaf(lb, ab, sb); ma = na; mb = nb;
    #pragma unroll
    for(int n = 0; n < VD / 8; ++n){ o[n][0] *= aa; o[n][1] *= aa; o[n][2] *= ab; o[n][3] *= ab; }
    // The weights times each value row's scale, each row's three words under its largest's scale — so the weights move
    // the outputs a hundredth of what the operands' two words do — as A fragments of every 32 keys: positions 4·t4 … of
    // a run its tiles 4k and 4k + 1, 16 + 4·t4 … its tiles 4k + 2 and 4k + 3.
    float wa = 0.f, wb = 0.f;
    #pragma unroll
    for(int n = 0; n < KT / 8; ++n)
      #pragma unroll
      for(int c = 0; c < 2; ++c){
        float sv = vsc[8 * n + 2 * t4 + c];
        s[n][c] *= sv; s[n][c + 2] *= sv;
        wa = fmaxf(wa, s[n][c]); wb = fmaxf(wb, s[n][c + 2]);
      }
    #pragma unroll
    for(int x = 1; x < 4; x <<= 1){ wa = fmaxf(wa, __shfl_xor_sync(0xffffffffu, wa, x)); wb = fmaxf(wb, __shfl_xor_sync(0xffffffffu, wb, x)); }
    const float swa = lk_two_scale(wa), swb = lk_two_scale(wb);
    const float ra = 1.f / swa, rb = 1.f / swb;
    int ah[KT / 32][4], al[KT / 32][4], aq[KT / 32][4];
    #pragma unroll
    for(int k = 0; k < KT / 32; ++k)
      #pragma unroll
      for(int f = 0; f < 4; ++f){
        int n = 4 * k + 2 * (f >> 1), c = 2 * (f & 1);
        float sw = f & 1 ? swb : swa, rw = f & 1 ? rb : ra;
        float e[4] = {s[n][c], s[n][c + 1], s[n + 1][c], s[n + 1][c + 1]};
        int h[4], l[4], q[4];
        #pragma unroll
        for(int i = 0; i < 4; ++i){
          h[i] = __float2int_rn(e[i] * rw);
          float r1 = __fmaf_rn(-(float)h[i], sw, e[i]);
          l[i] = __float2int_rn(r1 * rw * 128.f);
          q[i] = __float2int_rn(__fmaf_rn(-(float)l[i], sw * 0.0078125f, r1) * rw * 16384.f);
        }
        ah[k][f] = lk_int8x4(h[0], h[1], h[2], h[3]); al[k][f] = lk_int8x4(l[0], l[1], l[2], l[3]); aq[k][f] = lk_int8x4(q[0], q[1], q[2], q[3]);
      }
    asm volatile("cp.async.wait_group 1;\n");
    __syncthreads();
    // The weighed values: four tiles of eight value columns at a time, each sum folded as the scores' are — the weights'
    // and values' leading products, 128·wₕ·vₕ + wₕ·vₗ + wₗ·vₕ, in one exact integer, the next, wₗ·vₗ + wₗₗ·vₕ, 128 times
    // finer, in another.
    const float fwa = swa * 0.0078125f, fwb = swb * 0.0078125f;
    constexpr int NG = VD / 32, NPG = VD / 16 / NG;
    #pragma unroll
    for(int gq = 0; gq < NG; ++gq){
      int c[2 * NPG][4];
      #pragma unroll
      for(int n = 0; n < 2 * NPG; ++n)
        #pragma unroll
        for(int i = 0; i < 4; ++i) c[n][i] = 0;
      #pragma unroll
      for(int k = 0; k < KT / 32; ++k)
        #pragma unroll
        for(int np = 0; np < NPG; ++np){
          int bh[4];
          lk_ldsm_s8(bh, vh8 + (16 * (gq * NPG + np) + (mi >= 2 ? 8 : 0) + ri) * VP + 32 * k + ((mi & 1) ? 16 : 0));
          lk_mma_s8(c[2 * np], ah[k], bh); lk_mma_s8(c[2 * np + 1], ah[k], bh + 2);
        }
      #pragma unroll
      for(int n = 0; n < 2 * NPG; ++n)
        #pragma unroll
        for(int i = 0; i < 4; ++i) c[n][i] *= 128;
      #pragma unroll
      for(int k = 0; k < KT / 32; ++k){
        #pragma unroll
        for(int np = 0; np < NPG; ++np){
          int bl[4];
          lk_ldsm_s8(bl, vl8 + (16 * (gq * NPG + np) + (mi >= 2 ? 8 : 0) + ri) * VP + 32 * k + ((mi & 1) ? 16 : 0));
          lk_mma_s8(c[2 * np], ah[k], bl); lk_mma_s8(c[2 * np + 1], ah[k], bl + 2);
        }
        #pragma unroll
        for(int np = 0; np < NPG; ++np){
          int bh[4];
          lk_ldsm_s8(bh, vh8 + (16 * (gq * NPG + np) + (mi >= 2 ? 8 : 0) + ri) * VP + 32 * k + ((mi & 1) ? 16 : 0));
          lk_mma_s8(c[2 * np], al[k], bh); lk_mma_s8(c[2 * np + 1], al[k], bh + 2);
        }
      }
      int cl[2 * NPG][4];
      #pragma unroll
      for(int n = 0; n < 2 * NPG; ++n)
        #pragma unroll
        for(int i = 0; i < 4; ++i) cl[n][i] = 0;
      #pragma unroll
      for(int k = 0; k < KT / 32; ++k){
        #pragma unroll
        for(int np = 0; np < NPG; ++np){
          int bl[4];
          lk_ldsm_s8(bl, vl8 + (16 * (gq * NPG + np) + (mi >= 2 ? 8 : 0) + ri) * VP + 32 * k + ((mi & 1) ? 16 : 0));
          lk_mma_s8(cl[2 * np], al[k], bl); lk_mma_s8(cl[2 * np + 1], al[k], bl + 2);
        }
        #pragma unroll
        for(int np = 0; np < NPG; ++np){
          int bh[4];
          lk_ldsm_s8(bh, vh8 + (16 * (gq * NPG + np) + (mi >= 2 ? 8 : 0) + ri) * VP + 32 * k + ((mi & 1) ? 16 : 0));
          lk_mma_s8(cl[2 * np], aq[k], bh); lk_mma_s8(cl[2 * np + 1], aq[k], bh + 2);
        }
      }
      #pragma unroll
      for(int n = 0; n < 2 * NPG; ++n)
        #pragma unroll
        for(int i = 0; i < 4; ++i)
          o[2 * gq * NPG + n][i] = fmaf((float)c[n][i] + (float)cl[n][i] * 0.0078125f, i < 2 ? fwa : fwb, o[2 * gq * NPG + n][i]);
    }
    __syncthreads();
    if(jt + KT < jhi) load_v(jt + KT);
    else asm volatile("cp.async.commit_group;\n");
  }
  asm volatile("cp.async.wait_group 0;\n");
  // Each row's outputs, its sink in the denominator.
  #pragma unroll
  for(int half = 0; half < 2; ++half){
    int qi = half ? qb : qa, r = r0 + qi / HG, h = h0 + qi % HG;
    if(r >= rows) continue;
    float m = half ? mb : ma, l = half ? lb : la;
    float sk = sinks ? sinks[h] : -INFINITY, mf = fmaxf(sk, m);
    float f = m == -INFINITY ? 0.f : expf(m - mf), lf = fmaf(l, f, sinks ? expf(sk - mf) : 0.f);
    float* op = out + ((long)r * heads + h) * VD;
    #pragma unroll
    for(int n = 0; n < VD / 8; ++n)
      *(float2*)(op + 8 * n + 2 * t4) = make_float2(o[n][2 * half] * f / lf, o[n][2 * half + 1] * f / lf);
  }
}
"#;

/// The query vectors a block of the two-word attention (`lk_attention_i8`) takes, a warp 16 of them.
pub const ATTN_I8_QUERIES: usize = 64;

/// The keys a tile of the two-word attention takes: 64, or 32 where heads or values pass 128 wide.
pub const fn attn_i8_keys(hd: usize, vd: usize) -> usize {
    if hd > 128 || vd > 128 {
        32
    } else {
        64
    }
}

/// The two-word attention block's dynamic shared memory: a tile's keys' and values' two words, rows padded 16 bytes, and
/// both its scales in two buffers.
pub const fn attn_i8_smem_bytes(hd: usize, vd: usize) -> usize {
    let kt = attn_i8_keys(hd, vd);
    2 * kt * (hd + 16) + 2 * vd * (kt + 16) + 16 * kt
}

const _: () = assert!(
    attn_i8_smem_bytes(128, 128) <= 48 * 1024 && attn_i8_smem_bytes(256, 256) <= 48 * 1024,
    "the two-word attention keeps the default shared memory at every width it takes"
);

/// The key rows a key/value group's two words take for a cache of `len` positions: whole tiles of 64.
pub fn attn_i8_rows(len: usize) -> usize {
    len.next_multiple_of(64)
}

/// The two-word attention's scratch for a cache of `len` positions over `groups` key/value groups: the keys' high and
/// low words, the values', and each row's two scales — in bytes.
pub fn attn_i8_scratch(len: usize, groups: usize, hd: usize, vd: usize) -> [usize; 3] {
    let rows = groups * attn_i8_rows(len);
    [2 * rows * hd, 2 * rows * vd, 2 * rows * 4]
}

const SLICE: &str = r#"
template<typename XE>
__global__ void lk_slice(const XE* __restrict__ x, float* __restrict__ y, long n, int every, int from, int width){
  long o = (long)blockIdx.x * blockDim.x + threadIdx.x; if(o >= n) return;
  long g = o / width;
  y[o] = lk_from(x[g * every + from + (o - g * width)]);
}
"#;

/// Each unit's `ha` values of `a`, then its `hb` values of `b` — into the rows of a table from the row `yrow` names. A
/// batch's (`yz` > 0): row `r` of `heads` units into its own table, `yz` values on from the last's, at its own row.
const JOIN: &str = r#"
__global__ void lk_join(const float* __restrict__ a, const float* __restrict__ b, lk_out y,
                        const int* __restrict__ yrow, long ypitch, long n, int ha, int hb, int heads = 1, long yz = 0){
  long o = (long)blockIdx.x * blockDim.x + threadIdx.x; if(o >= n) return;
  long u = o / (ha + hb); int i = (int)(o - u * (ha + hb));
  float v = i < ha ? a[u * ha + i] : b[u * hb + (i - ha)];
  if(yz){ long r = u / heads; y[r * yz + (long)(yrow[r] - 1) * ypitch + (o - r * heads * (ha + hb))] = v; return; }
  LK_ROW(y, yrow, ypitch)[o] = v;
}
"#;

/// The per-head matmul `Y[t, h·b + r] = Σ_e X[t, h·a + e]·W[h·a + e, r]` over the `M` rows of `X` (`M ≤ 16`) and `H`
/// heads, the matrix's rows read in place in format `F`, the activation under numerics `Q` — `f32` with `f64` sums under
/// `Q` 0, `f32` with `f32` sums under `Q` 3 (the lane's own, [`head_mode`]), quantized under 1 and 2. `lk_head_gemv`: the rows are
/// the heads' outputs (`b` rows of `a` a head), a warp per output row dotting it with its head's part of each activation
/// row. `lk_head_axpy`: the rows are the heads' inputs (`a` rows of `b` a head), a block per (32 outputs, head), each
/// warp weighing the 32-weight sub-block of its rows by their inputs, a lane decoding its own weight (`lk_level`) — the
/// activation's quantized value under `Q` 1 and 2 — and the warps' sums added. Head `h` of row `t` reads its inputs
/// `(t·H + h)·xp + xo` into `X` (`xp`, `xo` whole blocks) and writes its outputs `(t·H + h)·yp + yo` into `Y`'s rows,
/// from the row `yrow` names. `lk_head_axpy_staged` (under the quantized numerics where [`axpy_staged`] says): a block of
/// `R` row groups of `W` warps takes `W` sub-blocks of a head's outputs, the head's rows' units holding them staged in
/// shared memory by 16-byte loads from each row's start aligned down (`delta` bytes before it), a tile of rows at a time;
/// a lane decodes its rows' sub-block whole (`lk_sub`, or a Q8_0 block's values and scale) and weighs it by the row's
/// input into its 32 sums, a transposing butterfly leaves lane `l` its warp's sum for output `l`, and the row groups'
/// sums add in group order.
const HEADS: &str = r#"
template<int F, int Q, int M>
__global__ void lk_head_gemv(lk_w w, const float* __restrict__ x, const signed char* __restrict__ xq,
                             const signed char* __restrict__ xl, const float* __restrict__ xs, lk_out y,
                             int H, int a, int b, long xp, long xo, const int* __restrict__ yrow, long ypitch, long yp, long yo){
  int o = (blockIdx.x * blockDim.x + threadIdx.x) >> 5; if(o >= H * b) return;
  long off = (long)(o / b) * xp + xo, ld = (long)H * xp;
  float v[M];
  constexpr bool X = lk_f32_acts<Q>();
  lk_dot<F, Q, M>(w, o, a, X ? x + off : nullptr, X ? nullptr : xq + off, Q == 2 ? xl + off : nullptr,
                  X ? nullptr : xs + (off >> 5), ld, v);
  lk_out yr = LK_ROW(y, yrow, ypitch) + (long)(o / b) * yp + yo + o % b;
  for(int t = threadIdx.x & 31; t < M; t += 32) yr[(long)t * H * yp] = lk_pick(v, t);
}
// The bytes of a format's smallest unit decoded alone and the sub-blocks it holds: a K-quant super-block (eight), an
// MXFP4 group (sixteen, its last as many as remain), a Q6_K group (eight blocks, sixty-four, its last as many as
// remain), a legacy block (one); a Q8_0 block is staged as its 32 values and, after a row's values, its scale.
template<int F> __host__ __device__ constexpr int lk_unit_bytes(){
  return F == 2 ? 18 : F == 3 ? 20 : F == 6 ? 22 : F == 7 ? 24 : F == 39 ? 272 : F == 20 ? 18 : F == 10 ? 84 : F == 11 ? 110 :
         F == 12 ? 144 : F == 13 ? 176 : F == 14 ? 1680 : F == 23 ? 136 : 34;
}
template<int F> __host__ __device__ constexpr int lk_unit_subs(){ return F == 39 ? 16 : F == 14 ? 64 : F == 10 || F == 11 || F == 12 || F == 13 || F == 23 ? 8 : 1; }
// A staged row: the units of `W` sub-blocks (and a unit more, should they straddle two), 15 bytes for the start's
// alignment, rounded to an odd number of words so the lanes' rows fall on distinct banks.
template<int F, int W> __host__ __device__ constexpr int lk_axpy_pitch(){
  return F == 8 ? ((W * 34 + 3) / 4 % 2 == 0 ? (W * 34 + 3) / 4 * 4 + 4 : (W * 34 + 3) / 4 * 4)
                : ((((W + lk_unit_subs<F>() - 1) / lk_unit_subs<F>() + 1) * lk_unit_bytes<F>() + 15 + 3) / 4 % 2 == 0
                       ? (((W + lk_unit_subs<F>() - 1) / lk_unit_subs<F>() + 1) * lk_unit_bytes<F>() + 15 + 3) / 4 * 4 + 4
                       : (((W + lk_unit_subs<F>() - 1) / lk_unit_subs<F>() + 1) * lk_unit_bytes<F>() + 15 + 3) / 4 * 4);
}
template<int O, typename T>
__device__ __forceinline__ void lk_fly_step(T* acc, int lane){
  bool up = lane & O;
  #pragma unroll
  for(int i = 0; i < O; ++i){
    T send = up ? acc[i] : acc[i + O];
    T keep = up ? acc[i + O] : acc[i];
    acc[i] = keep + __shfl_xor_sync(0xffffffffu, send, O);
  }
}
// The sums over a warp's lanes of each lane's 32 values, lane `l` ending with sum `l` in `acc[0]`.
template<typename T>
__device__ __forceinline__ void lk_fly(T* acc, int lane){
  lk_fly_step<16>(acc, lane); lk_fly_step<8>(acc, lane); lk_fly_step<4>(acc, lane); lk_fly_step<2>(acc, lane); lk_fly_step<1>(acc, lane);
}
template<int F, int Q, int M, int W, int R>
__global__ void __launch_bounds__(32 * W * R) lk_head_axpy_staged(lk_w w, const float* __restrict__ x,
    const signed char* __restrict__ xq, const signed char* __restrict__ xl, const float* __restrict__ xs,
    lk_out y, int H, int a, int b, long xp, long xo, const int* __restrict__ yrow, long ypitch, long yp,
    long yo, int tile){
  typedef typename lk_acc<Q>::t acc_t;
  constexpr int US = lk_unit_subs<F>(), UB = lk_unit_bytes<F>(), PITCH = lk_axpy_pitch<F, W>();
  extern __shared__ __align__(16) unsigned char rows[];
  __shared__ acc_t red[R][W][32];
  int nsb = b >> 5, h = blockIdx.y, s0 = blockIdx.x * W, u0 = s0 / US;
  int units = min((s0 + W - 1) / US - u0 + 1, (nsb + US - 1) / US - u0);
  int lane = threadIdx.x & 31, warp = threadIdx.x >> 5, wsb = warp % W, grp = warp / W, sb = s0 + wsb;
  long ld = (long)H * xp;
  for(int t = 0; t < M; ++t){
    acc_t acc[32];
    #pragma unroll
    for(int i = 0; i < 32; ++i) acc[i] = 0;
    for(int e0 = 0; e0 < a; e0 += tile){
      int here = min(tile, a - e0);
      if(t == 0 || tile < a){
        __syncthreads();
        if(F == 8){
          int nb = b >> 5;
          for(int c = threadIdx.x; c < here * units * 2; c += blockDim.x){
            int e = c / (units * 2), u = (c % (units * 2)) >> 1, half = c & 1;
            long row = (long)h * a + e0 + e;
            uint4 v = __ldg((const uint4*)(w.q + row * b + (long)(u0 + u) * 32) + half);
            unsigned int* d = (unsigned int*)(rows + e * PITCH + u * 32 + half * 16);
            d[0] = v.x; d[1] = v.y; d[2] = v.z; d[3] = v.w;
            if(half == 0) *(__half*)(rows + e * PITCH + W * 32 + u * 2) = w.s[row * nb + u0 + u];
          }
        } else {
          int chunks = (units * UB + 30) / 16;
          for(int c = threadIdx.x; c < here * chunks; c += blockDim.x){
            int e = c / chunks, k = c % chunks;
            const unsigned char* start = w.b + ((long)h * a + e0 + e) * w.rb + (long)u0 * UB;
            uint4 v = __ldg((const uint4*)((unsigned long)start & ~15ul) + k);
            unsigned int* d = (unsigned int*)(rows + e * PITCH + k * 16);
            d[0] = v.x; d[1] = v.y; d[2] = v.z; d[3] = v.w;
          }
        }
        __syncthreads();
      }
      if(sb < nsb){
        for(int e = grp * 32 + lane; e < here; e += 32 * R){
          long k = (long)h * xp + xo + e0 + e;
          float xv;
          if(lk_f32_acts<Q>()) xv = x[t * ld + k];
          else {
            float s = xs[t * (ld >> 5) + (k >> 5)];
            xv = Q == 1 ? s * (float)xq[t * ld + k] : s * ((float)xq[t * ld + k] + 0.0078125f * (float)xl[t * ld + k]);
          }
          if(F == 8){
            const signed char* q = (const signed char*)(rows + e * PITCH) + (sb - u0) * 32;
            float d = __half2float(*(const __half*)(rows + e * PITCH + W * 32 + (sb - u0) * 2));
            #pragma unroll
            for(int i = 0; i < 32; ++i) acc[i] += (acc_t)(d * (float)q[i]) * (acc_t)xv;
          } else {
            int delta = (int)(((unsigned long)(w.b + ((long)h * a + e0 + e) * w.rb + (long)u0 * UB)) & 15ul);
            signed char v[32]; float d[2], c[2];
            lk_sub<F>(rows + e * PITCH + delta, sb - u0 * US, v, d, c);
            #pragma unroll
            for(int i = 0; i < 32; ++i) acc[i] += (acc_t)__fadd_rn(__fmul_rn(d[i >> 4], (float)v[i]), c[i >> 4]) * (acc_t)xv;
          }
        }
      }
    }
    lk_fly(acc, lane);
    red[grp][wsb][lane] = acc[0];
    __syncthreads();
    if(grp == 0 && sb < nsb){
      acc_t sum = red[0][wsb][lane];
      #pragma unroll
      for(int r = 1; r < R; ++r) sum += red[r][wsb][lane];
      LK_ROW(y, yrow, ypitch)[((long)t * H + h) * yp + yo + sb * 32 + lane] = (float)sum;
    }
    __syncthreads();
  }
}
template<int F, int Q, int M>
__global__ void lk_head_axpy(lk_w w, const float* __restrict__ x, const signed char* __restrict__ xq,
                             const signed char* __restrict__ xl, const float* __restrict__ xs, lk_out y,
                             int H, int a, int b, long xp, long xo, const int* __restrict__ yrow, long ypitch, long yp, long yo){
  typedef typename lk_acc<Q>::t acc_t;
  __shared__ acc_t red[8][M][32];
  int j = blockIdx.x, h = blockIdx.y, lane = threadIdx.x & 31, warp = threadIdx.x >> 5;
  long ld = (long)H * xp;
  acc_t acc[M];
  #pragma unroll
  for(int t = 0; t < M; ++t) acc[t] = 0;
  for(int e = warp; e < a; e += 8){
    long row = (long)h * a + e, k = (long)h * xp + xo + e;
    float wv;
    if(F == 8) wv = __half2float(w.s[row * (b >> 5) + j]) * (float)w.q[row * b + j * 32 + lane];
    else {
      float d, c;
      int lv = lk_level<F>(w.b + row * w.rb, j, lane, d, c);
      wv = __fadd_rn(__fmul_rn(d, (float)lv), c);
    }
    #pragma unroll
    for(int t = 0; t < M; ++t){
      float xv;
      if(lk_f32_acts<Q>()) xv = x[t * ld + k];
      else {
        float s = xs[t * (ld >> 5) + (k >> 5)];
        xv = Q == 1 ? s * (float)xq[t * ld + k] : s * ((float)xq[t * ld + k] + 0.0078125f * (float)xl[t * ld + k]);
      }
      acc[t] += (acc_t)wv * (acc_t)xv;
    }
  }
  #pragma unroll
  for(int t = 0; t < M; ++t) red[warp][t][lane] = acc[t];
  __syncthreads();
  if(warp == 0){
    #pragma unroll
    for(int t = 0; t < M; ++t){
      acc_t sum = 0;
      #pragma unroll
      for(int u = 0; u < 8; ++u) sum += red[u][t][lane];
      LK_ROW(y, yrow, ypitch)[((long)t * H + h) * yp + yo + j * 32 + lane] = (float)sum;
    }
  }
}
"#;

/// The 1-based index of the first maximal value of `x`, across the grid: each block takes a run of `x` and leaves its
/// first maximum in `pv`/`pi`, and the last block to arrive (`cnt`, reset by it) takes the first maximum over the
/// blocks in order. `ADV` closes a step of `step` tokens: the token becomes the next step's input, the position moves
/// `step` on, and the token is appended to the generated run.
/// A wide chunk's per-head matmul in tiles: head `h` (grid layer `z`) multiplies its `M` rows' inputs — the `a` values
/// of each row's unit `h`, units `xp` apart from `xo` — by its matrix, whose rows are its `b` outputs (`OUT`) or its
/// `a` inputs, `HEAD_TILE_M` rows by 64 outputs a block over chunks of 32 inputs staged in shared memory, a thread 8 rows
/// by 4 outputs. Each weight is the dequantizer's `d·level + c` (a Q8_0 one `d·level`) and each input as computed under `Q` 3
/// (`x`), or its quantized words times their scale, as `lk_head_axpy` reads them; the sums are `f32`. Rows `xr` apart
/// with `xp` 0 give every head the row's one unit (a latent's expansion over the cache's rows); with `upto`, the rows
/// multiplied are those before `*upto + after`, at most `M`.
const HEAD_TILED: &str = r#"
template<int F, int Q, bool OUT, typename XE = float>
__global__ void __launch_bounds__(256) lk_head_tiled(lk_w w, const signed char* __restrict__ xq,
    const signed char* __restrict__ xl, const float* __restrict__ xs, lk_out y, int H, int a, int b, long xp,
    long xo, const int* __restrict__ yrow, long ypitch, long yp, long yo, int M, const XE* __restrict__ x = nullptr,
    long xr = 0, const int* __restrict__ upto = nullptr, int after = 0){
  __shared__ __align__(16) float Xs[32][128];  // [input][row]
  __shared__ __align__(16) float Ws[32][64];   // [input][output]
  int h = blockIdx.z, m0 = blockIdx.y * 128, o0 = blockIdx.x * 64, tid = threadIdx.x, tr = tid >> 4, to = tid & 15;
  if(upto) M = min(M, *upto + after);
  if(m0 >= M) return;
  long ld = xr ? xr : (long)H * xp;
  // A stage's 32 inputs by the block's 64 outputs are 64 sub-blocks: each output row's sub-block of the stage when the
  // rows are the outputs, each input row's two sub-blocks of the outputs otherwise. A thread takes eight values `8·sq`
  // … `+ 7` of one, its raw words fetched a stage ahead: a warp's 32 output rows at one `sq` (each store its own bank),
  // or eight threads to an input row (two 16-byte stores each, the row's outputs in order).
  int sq = OUT ? tid >> 6 : tid & 3;
  int srow = OUT ? tid & 63 : tid >> 3, shalf = OUT ? 0 : (tid >> 2) & 1;
  int K = OUT ? a : b, nb = K >> 5;
  long wrow = OUT ? (long)h * b + o0 + srow : (long)h * a + srow;
  bool wout = OUT ? o0 + srow < b : o0 + 32 * shalf < b;
  lk_raw<F> raw;
  auto fetch = [&](int i0){
    int ks = OUT ? i0 >> 5 : (o0 >> 5) + shalf;
    if(wout && (OUT || i0 + srow < a)) lk_fetch<F>(raw, w, wrow + (OUT ? 0 : i0), K, nb, ks);
  };
  float acc[8][4];
  #pragma unroll
  for(int r = 0; r < 8; ++r)
    #pragma unroll
    for(int o = 0; o < 4; ++o) acc[r][o] = 0.f;
  fetch(0);
  for(int i0 = 0; i0 < a; i0 += 32){
    // The rows' inputs, four a thread at a time, a warp's 32 rows at one input so each store lands on its own bank: as
    // computed under Q 3, as the inputs' quantized blocks hold them otherwise.
    #pragma unroll
    for(int u2 = 0; u2 < 4; ++u2){
      int c = tid + u2 * 256, r = c & 127, i = (c >> 7) * 4, m = m0 + r;
      float v[4] = {0.f, 0.f, 0.f, 0.f};
      if(m < M && i0 + i < a){
        long k = (long)h * xp + xo + i0 + i;
        if(Q == 3){
          float4 f = lk_from4(x + m * ld + k);
          v[0] = f.x; v[1] = f.y; v[2] = f.z; v[3] = f.w;
        } else {
          float sc = xs[m * (ld >> 5) + (k >> 5)];
          int hi = *(const int*)(xq + m * ld + k), lo = Q == 2 ? *(const int*)(xl + m * ld + k) : 0;
          #pragma unroll
          for(int u = 0; u < 4; ++u){
            float qv = (float)(signed char)(hi >> (8 * u));
            v[u] = Q == 1 ? sc * qv : sc * (qv + 0.0078125f * (float)(signed char)(lo >> (8 * u)));
          }
        }
      }
      #pragma unroll
      for(int u = 0; u < 4; ++u) Xs[i + u][r] = v[u];
    }
    // The stage's weights from their raw words: values 8·sq … 8·sq + 7 of this thread's sub-block, each the
    // dequantizer's `d·level + c`.
    {
      int ks = OUT ? i0 >> 5 : (o0 >> 5) + shalf;
      int lv[8]; float d[2], c[2];
      bool live = wout && (OUT || i0 + srow < a);
      if(live) lk_unpack<F>(raw, ks, lv, d, c);
      float wv[8];
      #pragma unroll
      for(int e = 0; e < 8; ++e){
        int l = 8 * sq + e;
        wv[e] = live ? __fadd_rn(__fmul_rn(d[l >> 4], (float)(signed char)(lv[l >> 2] >> (8 * (l & 3)))), c[l >> 4]) : 0.f;
      }
      if(OUT){
        #pragma unroll
        for(int e = 0; e < 8; ++e) Ws[8 * sq + e][srow] = wv[e];
      } else {
        float4* dst = (float4*)&Ws[srow][32 * shalf + 8 * sq];
        dst[0] = make_float4(wv[0], wv[1], wv[2], wv[3]); dst[1] = make_float4(wv[4], wv[5], wv[6], wv[7]);
      }
    }
    // The next stage's weights are fetched while this one multiplies.
    if(i0 + 32 < a) fetch(i0 + 32);
    __syncthreads();
    #pragma unroll 8
    for(int i = 0; i < 32; ++i){
      float4 x0 = *(const float4*)&Xs[i][8 * tr], x1 = *(const float4*)&Xs[i][8 * tr + 4];
      float4 wv = *(const float4*)&Ws[i][4 * to];
      float xa[8] = {x0.x, x0.y, x0.z, x0.w, x1.x, x1.y, x1.z, x1.w}, wa[4] = {wv.x, wv.y, wv.z, wv.w};
      #pragma unroll
      for(int r = 0; r < 8; ++r)
        #pragma unroll
        for(int o = 0; o < 4; ++o) acc[r][o] = fmaf(xa[r], wa[o], acc[r][o]);
    }
    __syncthreads();
  }
  #pragma unroll
  for(int r = 0; r < 8; ++r){
    int m = m0 + 8 * tr + r;
    if(m >= M) continue;
    lk_out yr = LK_ROW(y, yrow, ypitch) + ((long)m * H + h) * yp + yo + o0 + 4 * to;
    #pragma unroll
    for(int o = 0; o < 4; ++o) if(o0 + 4 * to + o < b) yr[o] = acc[r][o];
  }
}
"#;

const ARGMAX: &str = r#"
// The first maximum of a block's values (`best`, at `at`; `at` −1 for none), every thread's own first, reduced so the
// earlier index wins a tie.
__device__ __forceinline__ void lk_first_max(float& best, int& at, float* bv, int* bi){
  for(int o = 16; o > 0; o >>= 1){
    float ov = __shfl_down_sync(0xffffffffu, best, o); int oi = __shfl_down_sync(0xffffffffu, at, o);
    if(oi >= 0 && (at < 0 || ov > best || (ov == best && oi < at))){ best = ov; at = oi; }
  }
  int lane = threadIdx.x & 31, warp = threadIdx.x >> 5, nw = (blockDim.x + 31) >> 5;
  __syncthreads();
  if(lane == 0){ bv[warp] = best; bi[warp] = at; }
  __syncthreads();
  if(threadIdx.x == 0){
    for(int w = 1; w < nw; ++w)
      if(bi[w] >= 0 && (at < 0 || bv[w] > best || (bv[w] == best && bi[w] < at))){ best = bv[w]; at = bi[w]; }
  }
}
template<bool ADV>
__global__ void __launch_bounds__(256) lk_argmax(const float* __restrict__ x, int n, int* __restrict__ out, int* __restrict__ tok,
                          int* __restrict__ pos, int step, int* __restrict__ gen, int* __restrict__ count,
                          float* __restrict__ pv, int* __restrict__ pi, int* __restrict__ cnt, int seqs = 0, int genz = 0,
                          float cap = 0.f, float* __restrict__ y = nullptr){
  __shared__ float bv[32]; __shared__ int bi[32];
  __shared__ int last;
  // With `seqs`, grid row `y` is a batch's sequence: its logits, result, token, position, count and run its own, the
  // run `genz` tokens on from the last sequence's.
  if(seqs){
    long z = blockIdx.y;
    x += z * n; out += z; pv += z * gridDim.x; pi += z * gridDim.x; cnt += z;
    if(y) y += z * n;
    if(tok){ tok += z; pos += z; gen += z * genz; count += z; }
  }
  int run = (n + gridDim.x - 1) / gridDim.x, from = blockIdx.x * run, to = min(n, from + run);
  float best = -INFINITY; int at = -1;
  // A soft cap folded into the read (`cap_argmax`) takes each value as `lk_softcap` writes it, and writes the capped row
  // `y` where something reads it.
  for(int i = from + threadIdx.x; i < to; i += blockDim.x){
    float v = lk_cap(x[i], cap);
    if(y) y[i] = v;
    if(at < 0 || v > best){ best = v; at = i; }
  }
  lk_first_max(best, at, bv, bi);
  if(threadIdx.x == 0){ pv[blockIdx.x] = best; pi[blockIdx.x] = at; }
  __threadfence();
  __syncthreads();
  if(threadIdx.x == 0) last = atomicAdd(cnt, 1) == gridDim.x - 1;
  __syncthreads();
  if(!last) return;
  __threadfence();
  best = -INFINITY; at = -1;
  // The blocks' runs are in order: a thread's blocks, then the threads, keep the first maximum.
  for(int k = threadIdx.x; k < gridDim.x; k += blockDim.x){
    float v = __ldcg(pv + k); int i = __ldcg(pi + k);
    if(i >= 0 && (at < 0 || v > best)){ best = v; at = i; }
  }
  lk_first_max(best, at, bv, bi);
  if(threadIdx.x == 0){
    *out = at + 1;
    if(ADV){ *tok = at + 1; *pos = *pos + step; gen[*count] = at + 1; *count = *count + 1; }
    *cnt = 0;
  }
}
// A prefill chunk before the prompt's last, which computes no output: the arg-max's advance without its pick — the
// position past the chunk's rows, and the chunk's place in the run (the last chunk's pick is the one the host reads).
__global__ void lk_advance(int* __restrict__ pos, int step, int* __restrict__ gen, int* __restrict__ count){
  *pos = *pos + step; gen[*count] = 0; *count = *count + 1;
}
"#;


/// The depthwise causal conv over `C` channels with a `T`-row ring, fed the `R` rows of `x` one after another from the
/// 1-based position `pos` (`*pos + r` row `r`'s): the ring keeps position `p` (from 0) in slot `p mod T` — each row written
/// once, where the row `T` positions before it was — and each channel reduces its window, slots oldest to newest, against
/// its filter. `SILU` applies silu after; `L2` (one block per head of `blockDim` channels) then normalizes each of the
/// first `nl2` heads of every row to unit L2 norm. `lk_conv_rows` takes a block of rows a row at a time — row `r`'s
/// window the ring's slots of the positions before the first, then `x`'s rows, in the window's order, so every row
/// computes what the rows one after another compute — and `lk_conv_carry` then writes the block's last rows (up to `T`)
/// into their slots, leaving the ring as those rows leave it.
const CONV: &str = r#"
// Position `p`'s slot (from 0, positions before the first too) of a ring of `T` rows.
__device__ __forceinline__ int lk_slot(int p, int T){ return ((p % T) + T) % T; }
template<bool SILU, bool L2, typename A>
__global__ void lk_conv(const float* __restrict__ x, const float* __restrict__ wc, float* __restrict__ cs,
                        float* __restrict__ y, int C, int T, int R, int nl2, float eps, const int* __restrict__ pos){
  __shared__ A red[32];
  int c = blockIdx.x * blockDim.x + threadIdx.x;
  const int q0 = *pos - 1;
  for(int r = 0; r < R; ++r){
    float v = 0.f;
    if(c < C){
      const int q = q0 + r;
      const float cur = x[(long)r * C + c];
      cs[(long)lk_slot(q, T) * C + c] = cur;
      A sum = 0;
      for(int t = 0; t < T - 1; ++t) sum = lk_fma((A)cs[(long)lk_slot(q - (T - 1) + t, T) * C + c], (A)wc[(long)t * C + c], sum);
      sum = lk_fma((A)cur, (A)wc[(long)(T - 1) * C + c], sum);
      v = (float)sum;
      if(SILU) v = v / (1.0f + expf(-v));
    }
    if(L2 && blockIdx.x < nl2){
      A den = sqrt(lk_block_sum(c < C ? lk_mul((A)v, (A)v) : (A)0, red) + (A)eps);
      v = (float)((A)v / den);
    }
    if(c < C) y[(long)r * C + c] = v;
  }
}
template<bool SILU, bool L2, typename A>
__global__ void lk_conv_rows(const float* __restrict__ x, const float* __restrict__ wc, const float* __restrict__ cs,
                             float* __restrict__ y, int C, int T, int R, int nl2, float eps, const int* __restrict__ pos){
  __shared__ A red[32];
  int c = blockIdx.x * blockDim.x + threadIdx.x, r = blockIdx.y;
  const int q0 = *pos - 1;
  float v = 0.f;
  if(c < C){
    A sum = 0;
    for(int t = 0; t < T; ++t){
      int j = r - (T - 1) + t;
      sum = lk_fma((A)(j >= 0 ? x[(long)j * C + c] : cs[(long)lk_slot(q0 + j, T) * C + c]), (A)wc[(long)t * C + c], sum);
    }
    v = (float)sum;
    if(SILU) v = v / (1.0f + expf(-v));
  }
  if(L2 && blockIdx.x < nl2){
    A den = sqrt(lk_block_sum(c < C ? lk_mul((A)v, (A)v) : (A)0, red) + (A)eps);
    v = (float)((A)v / den);
  }
  if(c < C) y[(long)r * C + c] = v;
}
// One causal convolution of `lk_conv_parts`: its input row, weights, ring and output, its channels `C`, its blocks
// `nb`, and whether each head of its output is normed.
struct lk_conv_part { const float* x; const float* wc; float* cs; float* y; int C; int nb; int l2; };
// With `seqs`, grid row `y` is a batch's sequence: each part's input row, ring and output its part of one array of them,
// its position `pos[y]`.
struct lk_convs { lk_conv_part p[4]; };
// Channel `c` of `C` at position `q` (from 0) through the SiLU: `cur` set into its slot of the ring `cs` of `T` rows,
// then the taps of the `T − 1` positions before it, oldest first, and of `cur`, summed in `A`.
template<typename A>
__device__ __forceinline__ float lk_conv_tap(const float* __restrict__ wc, float* __restrict__ cs, int C, int T, int q, int c, float cur){
  cs[(long)lk_slot(q, T) * C + c] = cur;
  A sum = 0;
  for(int t = 0; t < T - 1; ++t) sum = lk_fma((A)cs[(long)lk_slot(q - (T - 1) + t, T) * C + c], (A)wc[(long)t * C + c], sum);
  sum = lk_fma((A)cur, (A)wc[(long)(T - 1) * C + c], sum);
  return lk_act<0>((float)sum);
}
// Value `v` of a head normed to unit length with `eps`, the head the block's values where `live`: thread `t` holds the
// head's value `t`, so its sum of squares is the tree of a block exactly the head wide whatever the block's width.
template<typename A>
__device__ __forceinline__ float lk_l2_unit(float v, bool live, float eps, A* red){
  A den = sqrt(lk_block_sum(live ? lk_mul((A)v, (A)v) : (A)0, red) + (A)eps);
  return (float)((A)v / den);
}
// One row of `np` causal convolutions through the SiLU in one launch, their blocks one after another: block `b` the
// part whose blocks hold it, each channel's sum — and a normed part's head of `blockDim.x` channels — as `lk_conv<true,
// L2>` computes them over one row.
template<typename A>
__global__ void lk_conv_parts(lk_convs ps, int np, int T, float eps, const int* __restrict__ pos, int seqs = 0){
  __shared__ A red[32];
  int b = blockIdx.x, p = 0;
  while(p + 1 < np && b >= ps.p[p].nb){ b -= ps.p[p].nb; ++p; }
  int C = ps.p[p].C, c = b * blockDim.x + threadIdx.x;
  long z = seqs ? blockIdx.y : 0;
  const int q = pos[z] - 1;
  const float* __restrict__ x = ps.p[p].x + z * C;
  float* __restrict__ cs = ps.p[p].cs + z * T * C;
  float* __restrict__ y = ps.p[p].y + z * C;
  float v = c < C ? lk_conv_tap<A>(ps.p[p].wc, cs, C, T, q, c, x[c]) : 0.f;
  if(ps.p[p].l2) v = lk_l2_unit<A>(v, c < C, eps, red);
  if(c < C) y[c] = v;
}
// A block of `R` rows from position `pos`: its last rows, up to `T`, each into its slot.
__global__ void lk_conv_carry(const float* __restrict__ x, float* __restrict__ cs, int C, int T, int R, const int* __restrict__ pos){
  int c = blockIdx.x * blockDim.x + threadIdx.x; if(c >= C) return;
  const int q0 = *pos - 1;
  for(int j = max(0, R - T); j < R; ++j) cs[(long)lk_slot(q0 + j, T) * C + c] = x[(long)j * C + c];
}
"#;

/// Per-row L2 normalization `y = x / sqrt(Σx² + eps)`, one block per row.
const L2NORM: &str = r#"
template<typename A>
__global__ void lk_l2norm(const float* __restrict__ x, float* __restrict__ y, int cols, float eps){
  __shared__ A red[32];
  long base = (long)blockIdx.x * cols;
  A acc = 0;
  for(int i = threadIdx.x; i < cols; i += blockDim.x) acc += (A)x[base + i] * (A)x[base + i];
  A den = sqrt(lk_block_sum(acc, red) + (A)eps);
  for(int i = threadIdx.x; i < cols; i += blockDim.x) y[base + i] = (float)((A)x[base + i] / den);
}
"#;

/// Gated DeltaNet's forget gate `(1 + e^(ar + dt))^rate` per head, every row of `ar` by the same `heads` biases and rates;
/// `lk_forget` is one head's, the one definition every kernel that makes the gate shares.
const DECAY: &str = r#"
__device__ __forceinline__ float lk_forget(float ar, float dt, float al){ return powf(1.0f + expf(ar + dt), al); }
__global__ void lk_decay(const float* __restrict__ ar, const float* __restrict__ dt, const float* __restrict__ al,
                         float* __restrict__ y, int n, int heads){
  int i = blockIdx.x * blockDim.x + threadIdx.x; if(i >= n) return;
  int h = i % heads;
  y[i] = lk_forget(ar[i], dt[h], al[h]);
}
"#;

/// Both Gated DeltaNet gates in one launch: the write strength `sigmoid(br)` and the forget gate. `lk_gate_proj` takes a
/// step's one row from its activation: head `o`'s two `f32` dots `x·wb[o]` and `x·wa[o]` each summed as `lk_gemv_f32`
/// sums it over `parts` warps (a lane's run of `k` in `f64`, the warp's sum, the runs in order), then both gates as
/// `lk_gates` computes them.
const GATES: &str = r#"
__global__ void lk_gates(const float* __restrict__ br, const float* __restrict__ ar, const float* __restrict__ dt,
                         const float* __restrict__ al, float* __restrict__ beta, float* __restrict__ decay, int n, int heads){
  int i = blockIdx.x * blockDim.x + threadIdx.x; if(i >= n) return;
  int h = i % heads;
  beta[i] = lk_act<1>(br[i]);
  decay[i] = lk_forget(ar[i], dt[h], al[h]);
}
// Block `blk` of `lk_gate_proj`: heads `blk·(8 / parts)` on, `parts` warps a head, by the block's first eight warps —
// any warps past them idle through its barrier.
__device__ __forceinline__ void lk_gate_heads(int blk, const float* __restrict__ wb, const float* __restrict__ wa,
    const float* __restrict__ x, const float* __restrict__ dt, const float* __restrict__ al, float* __restrict__ beta,
    float* __restrict__ decay, int K, int H, int parts){
  __shared__ double run[2][8];
  int warp = threadIdx.x >> 5, lane = threadIdx.x & 31;
  int o = blk * (8 / parts) + warp / parts, p = warp % parts;
  bool mine = warp < 8, live = mine && o < H;
  int span = (K + parts - 1) / parts, k0 = p * span, k1 = min(K, k0 + span);
  const float* rb = wb + (long)(live ? o : 0) * K;
  const float* ra = wa + (long)(live ? o : 0) * K;
  double sb = 0.0, sa = 0.0;
  if(live) for(int k = k0 + lane; k < k1; k += 32) sb += (double)rb[k] * (double)x[k];
  if(live) for(int k = k0 + lane; k < k1; k += 32) sa += (double)ra[k] * (double)x[k];
  sb = lk_warp_sum_d(sb);
  sa = lk_warp_sum_d(sa);
  if(parts > 1){
    if(mine && lane == 0){ run[0][warp] = sb; run[1][warp] = sa; }
    __syncthreads();
    if(mine && p == 0){
      sb = 0.0; sa = 0.0;
      for(int i = 0; i < parts; ++i){ sb += run[0][warp + i]; sa += run[1][warp + i]; }
    }
  }
  if(live && p == 0 && lane == 0){
    beta[o] = lk_act<1>((float)sb);
    decay[o] = lk_forget((float)sa, dt[o], al[o]);
  }
}
__global__ void __launch_bounds__(256) lk_gate_proj(const float* __restrict__ wb, const float* __restrict__ wa,
    const float* __restrict__ x, const float* __restrict__ dt, const float* __restrict__ al, float* __restrict__ beta,
    float* __restrict__ decay, int K, int H, int parts){
  lk_gate_heads((int)blockIdx.x, wb, wa, x, dt, al, beta, decay, K, H, parts);
}
"#;

/// The delta-rule step over the `R` rows of a block in turn, one warp per row of a value head's `D×D` memory
/// (`grid = (HV, D / warps)`) kept in registers across the block — a lane's `DW` values of the row, `DW` at least
/// `D / 32`: value head `h` reads key head `h % NK`; one pass over the row accumulates `S·k`, `S·q` and `k·q`, rewrites
/// the row `α·S + β·(v − α·S·k)·kᵀ`, and reads the updated memory as `α·(S·q) + coef·(k·q)`, scaled by `1/√D`. Given a
/// `qk_eps` (zero or more) the step norms each row's query and key heads itself (`delta_qk_norm`), each value its head's
/// over the head's L2 norm with `qk_eps` summed as the convolution's norm (`lk_l2_unit`) sums it; without one (negative,
/// its default) it takes the heads as given.
const DELTA_STEP: &str = r#"
// The L2 norm's denominator of a head of `D` values (whole warps of 32) a lane holds `v[i]`, value `lane + 32i`, with
// `eps`: the squares summed as `lk_block_sum` sums a block of the head's width — each run of 32 over its warp, then the
// runs in order — so a head normed here is the head `lk_l2_unit` norms. The runs go down `lk_warp_sum`'s tree together,
// a level at a time, and lane 0's runs then take the tree a warp of them would take, in its registers — the lanes past
// the head's runs add zeros, which change no sum — so the norm waits on six shuffles, not one run's after another's.
template<typename A, int DW>
__device__ __forceinline__ A lk_head_den(const float (&v)[DW], int D, float eps){
  A s[DW];
  #pragma unroll
  for(int i = 0; i < DW; ++i) s[i] = 32 * i < D ? lk_mul((A)v[i], (A)v[i]) : (A)0;
  #pragma unroll
  for(int o = 16; o > 0; o >>= 1)
    #pragma unroll
    for(int i = 0; i < DW; ++i) s[i] += __shfl_down_sync(0xffffffffu, s[i], o);
  #pragma unroll
  for(int o = 16; o > 0; o >>= 1)
    #pragma unroll
    for(int l = 0; l < DW; ++l) if(l < o && l + o < DW && l + o < (D >> 5)) s[l] += s[l + o];
  return __shfl_sync(0xffffffffu, sqrt(s[0] + (A)eps), 0);
}
template<typename A, int DW, typename SS>
__device__ __forceinline__ void lk_delta_rows(const float* __restrict__ q, const float* __restrict__ k, const float* __restrict__ v,
                              const float* __restrict__ bet, const float* __restrict__ alp, SS* __restrict__ S,
                              float* __restrict__ o, int D, int NK, int HV, int R, int seqs, float qk_eps = -1.f){
  // With `seqs`, grid layer `z` is a batch's sequence: its one row, its gates and its own memory, each the sequence's
  // part of one array of them all.
  if(seqs){
    long z = blockIdx.z;
    q += z * NK * D; k += z * NK * D; v += z * HV * D; bet += z * HV; alp += z * HV; S += z * HV * D * D; o += z * HV * D;
  }
  int h = blockIdx.x, lane = threadIdx.x & 31, warp = threadIdx.x >> 5;
  int a = blockIdx.y * (blockDim.x >> 5) + warp;
  if(a >= D) return;
  long base = (long)h * D * D + (long)a * D;
  float sreg[DW];
  #pragma unroll
  for(int i = 0; i < DW; ++i) if(lane + 32 * i < D) sreg[i] = lk_from(S[base + lane + 32 * i]);
  for(int r = 0; r < R; ++r){
    const float* kk = k + ((long)r * NK + h % NK) * D;
    const float* qq = q + ((long)r * NK + h % NK) * D;
    // The row's key and query values of this lane; given a `qk_eps` each head normed to unit length as `lk_l2_unit` norms
    // it.
    float kv[DW], qv[DW];
    #pragma unroll
    for(int i = 0; i < DW; ++i){
      const int b = lane + 32 * i;
      kv[i] = b < D ? kk[b] : 0.f;
      qv[i] = b < D ? qq[b] : 0.f;
    }
    if(qk_eps >= 0.f){
      const A dk = lk_head_den<A, DW>(kv, D, qk_eps), dq = lk_head_den<A, DW>(qv, D, qk_eps);
      #pragma unroll
      for(int i = 0; i < DW; ++i){ kv[i] = (float)((A)kv[i] / dk); qv[i] = (float)((A)qv[i] / dq); }
    }
    A su = 0, sw = 0, skq = 0;
    #pragma unroll
    for(int i = 0; i < DW; ++i){
      const int b = lane + 32 * i;
      if(b < D){
        A sv = sreg[i], kb = kv[i], qb = qv[i];
        su += sv * kb; sw += sv * qb; skq += kb * qb;
      }
    }
    float u = (float)lk_warp_sum(su), w = (float)lk_warp_sum(sw), kq = (float)lk_warp_sum(skq);
    long hr = (long)r * HV + h;
    float alpha = alp[hr], coef = bet[hr] * (v[hr * D + a] - alpha * u);
    if(lane == 0) o[hr * D + a] = (alpha * w + coef * kq) * (1.0f / sqrtf((float)D));
    #pragma unroll
    for(int i = 0; i < DW; ++i) if(lane + 32 * i < D) sreg[i] = lk_kept<SS>(alpha * sreg[i] + coef * kv[i]);
  }
  #pragma unroll
  for(int i = 0; i < DW; ++i) if(lane + 32 * i < D) S[base + lane + 32 * i] = lk_to<SS>(sreg[i]);
}
template<typename A, int DW = 32, typename SS = float>
__global__ void lk_delta_step(const float* __restrict__ q, const float* __restrict__ k, const float* __restrict__ v,
                              const float* __restrict__ bet, const float* __restrict__ alp, SS* __restrict__ S,
                              float* __restrict__ o, int D, int NK, int HV, int R, int seqs = 0){
  lk_delta_rows<A, DW, SS>(q, k, v, bet, alp, S, o, D, NK, HV, R, seqs);
}
"#;

/// A wide chunk's delta rule a chunk of `LK_DC` rows at a time, the chunkwise form of the recurrence `lk_delta_step`
/// walks a row at a time. Within a chunk of value head `h`, with `γ_t` the product of its decays to row `t` (kept as
/// cumulative logs, each at least −80) and `G_ts = γ_t / γ_s`, the step's corrections `δ_t = β_t·(v_t − α_t·S_{t−1}k_t)`
/// solve `δ_t + Σ_{s<t} β_t·G_ts·(k_t·k_s)·δ_s = β_t·(v_t − γ_t·S₀k_t)` — so, with `T = (I + A)⁻¹` for that strictly lower
/// `A`, `Δ = T·diag(β)·(V − diag(γ)·K·S₀ᵀ)` — the outputs are `o_t = (γ_t·S₀q_t + Σ_{s≤t} G_ts·(k_s·q_t)·δ_s) / √D`, and
/// the memory leaves the chunk as `γ_n·S₀ + Σ_s G_ns·δ_s·k_sᵀ`. `lk_delta_prep` takes, for every chunk and head at once,
/// what the memory does not enter — the decayed keys and queries `diag(γ)·K` and `diag(γ)·Q` (a value of the width at a
/// time), `T·diag(β)` (`T` by 16 × 16 blocks, each diagonal block's by forward substitution and the blocks below from
/// those), the masked scores `(Q·Kᵀ) ⊙ G` (both a column of the chunk's rows at a time), each row's decay to the chunk's
/// end and the chunk's own — its keys and queries a run of 32 values at a time. `lk_delta_state` then walks a head's
/// chunks in turn, a block of two warps `RV` (16) of its memory's rows (the rows of a value head's memory are
/// independent), held in shared memory a value of the width at a time and each warp's 8 in registers, a lane `D / 32`
/// values of their width: a lane one of the chunk's rows across the block's — `diag(γ)·K·S₀ᵀ` and `diag(γ)·Q·S₀ᵀ` over
/// the width, the corrections, the outputs — then each warp its rows of the memory the chunk leaves; each warp reads
/// half the chunk's decayed keys and queries. Heads `D` wide (at most 128, whole runs of 32), every sum `f32`.
const DELTA_CHUNKS: &str = r#"
#define LK_DC 64
template<int D>
__global__ void __launch_bounds__(256) lk_delta_prep(const float* __restrict__ q, const float* __restrict__ k,
    const float* __restrict__ bet, const float* __restrict__ alp, float* __restrict__ Kg, float* __restrict__ Qg,
    float* __restrict__ Tb, float* __restrict__ P, float* __restrict__ gl, int NK, int HV, int R){
  constexpr int CP = LK_DC + 1, KT = 32, KP = KT + 1;
  static_assert(D % KT == 0, "the width in whole runs of 32");
  extern __shared__ __align__(16) float lk_dp[];
  float* kt = lk_dp;                       // [LK_DC][KP]: a run of 32 of the chunk's keys' values
  float* qt = kt + LK_DC * KP;             // [LK_DC][KP]: and of its queries'
  float* ts = lk_dp;                       // [LK_DC][CP]: T = (I + A)⁻¹, once every run is summed
  float* as = lk_dp + 2 * LK_DC * KP;      // [LK_DC][CP]: A below the diagonal
  float* xs = as + LK_DC * CP;             // [3][16][16]: the sums T's lower blocks take
  __shared__ float lg[LK_DC], bs[LK_DC], eg[LK_DC];
  int h = blockIdx.x, c = blockIdx.y, tid = threadIdx.x, kh = h % NK, tq = tid >> 4, tr = tid & 15;
  int r0 = c * LK_DC, n = min(LK_DC, R - r0);
  long base = (long)h * gridDim.y + c;
  // Every row's log decay loaded and taken at once, then summed in order by one thread — its loads and logarithms
  // apart from the sum's chain (Qwen3.5-0.8B's 512-row chunk on the 3060: 210 µs a layer → 150).
  if(tid < LK_DC) eg[tid] = tid < n ? fmaxf(logf(alp[(long)(r0 + tid) * HV + h]), -80.f) : 0.f;
  __syncthreads();
  if(tid == 0){
    float acc = 0.f;
    for(int t = 0; t < LK_DC; ++t){
      if(t < n) acc += eg[t];
      lg[t] = acc;
    }
  }
  if(tid < LK_DC) bs[tid] = tid < n ? bet[(long)(r0 + tid) * HV + h] : 0.f;
  __syncthreads();
  if(tid < LK_DC) eg[tid] = tid < n ? expf(lg[tid]) : 0.f;
  // k·k and q·k over rows t = tq + 16·i and s = tr + 16·j, a 4 × 4 tile a thread, the width a run of 32 at a time; and
  // the decayed keys and queries diag(γ)·K and diag(γ)·Q, a value of the width at a time.
  float kk[4][4], qk[4][4];
  #pragma unroll
  for(int i = 0; i < 4; ++i)
    #pragma unroll
    for(int j = 0; j < 4; ++j){ kk[i][j] = 0.f; qk[i][j] = 0.f; }
  for(int d0 = 0; d0 < D; d0 += KT){
    __syncthreads();
    for(int i = tid; i < LK_DC * KT; i += 256){
      int t = i / KT, dd = i % KT;
      long at = ((long)(r0 + t) * NK + kh) * D + d0 + dd;
      kt[t * KP + dd] = t < n ? k[at] : 0.f;
      qt[t * KP + dd] = t < n ? q[at] : 0.f;
    }
    __syncthreads();
    for(int i = tid; i < KT * LK_DC; i += 256){
      int dd = i / LK_DC, t = i % LK_DC;
      long at = (base * D + d0 + dd) * LK_DC + t;
      Kg[at] = eg[t] * kt[t * KP + dd];
      Qg[at] = eg[t] * qt[t * KP + dd];
    }
    #pragma unroll 8
    for(int dd = 0; dd < KT; ++dd){
      float kr[4], qr[4], kc[4];
      #pragma unroll
      for(int i = 0; i < 4; ++i){ kr[i] = kt[(tq + 16 * i) * KP + dd]; qr[i] = qt[(tq + 16 * i) * KP + dd]; kc[i] = kt[(tr + 16 * i) * KP + dd]; }
      #pragma unroll
      for(int i = 0; i < 4; ++i)
        #pragma unroll
        for(int j = 0; j < 4; ++j){ kk[i][j] = fmaf(kr[i], kc[j], kk[i][j]); qk[i][j] = fmaf(qr[i], kc[j], qk[i][j]); }
    }
  }
  // A below the diagonal; the masked scores on and below it, a column of the chunk's rows at a time.
  #pragma unroll
  for(int i = 0; i < 4; ++i)
    #pragma unroll
    for(int j = 0; j < 4; ++j){
      int t = tq + 16 * i, s = tr + 16 * j;
      float g = expf(lg[t] - lg[s]);
      as[t * CP + s] = s < t && t < n ? bs[t] * g * kk[i][j] : 0.f;
      P[base * LK_DC * LK_DC + s * LK_DC + t] = s <= t && t < n ? g * qk[i][j] : 0.f;
    }
  __syncthreads();
  // T = (I + A)⁻¹ by 16 × 16 blocks: each diagonal block's inverse by forward substitution, a column a thread held in
  // registers, the blocks above the diagonal zero; then the blocks below it a diagonal at a time,
  // T_ij = −T_ii·Σ_{j≤k<i} A_ik·T_kj — its sums first, then T_ii times them.
  if(tid < LK_DC){
    int o = 16 * (tid >> 4), jj = tid & 15;
    float x[16];
    #pragma unroll
    for(int tt = 0; tt < 16; ++tt){
      float acc = tt == jj ? 1.f : 0.f;
      #pragma unroll
      for(int s = 0; s < tt; ++s) acc = fmaf(-as[(o + tt) * CP + o + s], x[s], acc);
      x[tt] = tt >= jj ? acc : 0.f;
    }
    #pragma unroll
    for(int tt = 0; tt < 16; ++tt) ts[(o + tt) * CP + o + jj] = x[tt];
  } else {
    for(int i = tid - LK_DC; i < LK_DC * LK_DC; i += 256 - LK_DC)
      if((i % LK_DC) >> 4 > (i / LK_DC) >> 4) ts[(i / LK_DC) * CP + i % LK_DC] = 0.f;
  }
  __syncthreads();
  for(int dg = 1; dg < 4; ++dg){
    for(int e = tid; e < (4 - dg) * 256; e += 256){
      int j = e >> 8, i = j + dg, r = (e >> 4) & 15, cc = e & 15;
      float acc = 0.f;
      for(int m = 16 * j; m < 16 * i; ++m) acc = fmaf(as[(16 * i + r) * CP + m], ts[m * CP + 16 * j + cc], acc);
      xs[e] = acc;
    }
    __syncthreads();
    for(int e = tid; e < (4 - dg) * 256; e += 256){
      int j = e >> 8, i = j + dg, r = (e >> 4) & 15, cc = e & 15;
      float acc = 0.f;
      #pragma unroll
      for(int m = 0; m < 16; ++m) acc = fmaf(ts[(16 * i + r) * CP + 16 * i + m], xs[(j << 8) + m * 16 + cc], acc);
      ts[(16 * i + r) * CP + 16 * j + cc] = -acc;
    }
    __syncthreads();
  }
  // T·diag(β), a column of the chunk's rows at a time; each row's decay to the chunk's end, and the chunk's own.
  for(int i = tid; i < LK_DC * LK_DC; i += 256){
    int s = i / LK_DC, t = i % LK_DC;
    Tb[base * LK_DC * LK_DC + i] = ts[t * CP + s] * bs[s];
  }
  if(tid <= LK_DC) gl[base * (LK_DC + 1) + tid] = tid < LK_DC ? (tid < n ? expf(lg[n - 1] - lg[tid]) : 0.f) : expf(lg[n - 1]);
}
// `B` values from `p`, sixteen bytes at a time where `B` is a multiple of four.
template<int B>
__device__ __forceinline__ void lk_load(const float* __restrict__ p, float (&r)[B]){
  if constexpr(B % 4 == 0){
    #pragma unroll
    for(int i = 0; i < B / 4; ++i){
      float4 v = __ldg((const float4*)p + i);
      r[4 * i] = v.x; r[4 * i + 1] = v.y; r[4 * i + 2] = v.z; r[4 * i + 3] = v.w;
    }
  } else {
    #pragma unroll
    for(int i = 0; i < B; ++i) r[i] = __ldg(p + i);
  }
}
template<int D, int RV, typename SS = float>
__global__ void __launch_bounds__(64) lk_delta_state(const float* __restrict__ k, const float* __restrict__ v,
    const float* __restrict__ Kg, const float* __restrict__ Qg, const float* __restrict__ Tb, const float* __restrict__ P,
    const float* __restrict__ gl, SS* __restrict__ S, float* __restrict__ o, int NK, int HV, int R){
  constexpr int RB = RV, B = D / 32;
  static_assert(RV == 16 && D % 32 == 0, "two warps, each 32 of the chunk's rows and 8 of the memory's; a lane D / 32 of the width");
  __shared__ __align__(16) float sm[D * RB];     // the block's rows of the memory, a value of the width at a time
  __shared__ __align__(16) float rd[LK_DC * RB];  // V − diag(γ)·K·S₀ᵀ over them, then the corrections Δ
  int h = blockIdx.x, lane = threadIdx.x & 31, warp = threadIdx.x >> 5, kh = h % NK, a0 = blockIdx.y * RB, b0 = lane * B;
  int t = 32 * warp + lane, aw = 8 * warp;
  int chunks = (R + LK_DC - 1) / LK_DC;
  const float sc = rsqrtf((float)D);
  float s[8][B];
  #pragma unroll
  for(int a = 0; a < 8; ++a){
    if constexpr(sizeof(SS) == 4) lk_load<B>((const float*)S + ((long)h * D + a0 + aw + a) * D + b0, s[a]);
    else
      #pragma unroll
      for(int i = 0; i < B; ++i) s[a][i] = lk_from(S[((long)h * D + a0 + aw + a) * D + b0 + i]);
  }
  for(int c = 0; c < chunks; ++c){
    int r0 = c * LK_DC, n = min(LK_DC, R - r0);
    long base = (long)h * chunks + c;
    const float *Kc = Kg + base * LK_DC * D, *Qc = Qg + base * LK_DC * D;
    const float *Tc = Tb + base * LK_DC * LK_DC, *Pc = P + base * LK_DC * LK_DC, *g = gl + base * (LK_DC + 1);
    #pragma unroll
    for(int i = 0; i < B; ++i)
      #pragma unroll
      for(int a = 0; a < 8; ++a) sm[(b0 + i) * RB + aw + a] = s[a][i];
    __syncthreads();
    float xr[RB], xo[RB];
    #pragma unroll
    for(int a = 0; a < RB; ++a){ xr[a] = 0.f; xo[a] = 0.f; }
    #pragma unroll 4
    for(int b = 0; b < D; ++b){
      float kv = __ldg(Kc + b * LK_DC + t), qv = __ldg(Qc + b * LK_DC + t);
      const float4* sp = (const float4*)(sm + b * RB);
      #pragma unroll
      for(int q4 = 0; q4 < RB / 4; ++q4){
        float4 sv = sp[q4];
        xr[4 * q4] = fmaf(kv, sv.x, xr[4 * q4]); xr[4 * q4 + 1] = fmaf(kv, sv.y, xr[4 * q4 + 1]);
        xr[4 * q4 + 2] = fmaf(kv, sv.z, xr[4 * q4 + 2]); xr[4 * q4 + 3] = fmaf(kv, sv.w, xr[4 * q4 + 3]);
        xo[4 * q4] = fmaf(qv, sv.x, xo[4 * q4]); xo[4 * q4 + 1] = fmaf(qv, sv.y, xo[4 * q4 + 1]);
        xo[4 * q4 + 2] = fmaf(qv, sv.z, xo[4 * q4 + 2]); xo[4 * q4 + 3] = fmaf(qv, sv.w, xo[4 * q4 + 3]);
      }
    }
    {
      float vv[RB];
      if(t < n) lk_load<RB>(v + ((long)(r0 + t) * HV + h) * D + a0, vv);
      else for(int a = 0; a < RB; ++a) vv[a] = 0.f;
      #pragma unroll
      for(int a = 0; a < RB; ++a) rd[t * RB + a] = vv[a] - xr[a];
    }
    __syncthreads();
    #pragma unroll
    for(int a = 0; a < RB; ++a) xr[a] = 0.f;
    #pragma unroll 8
    for(int sr = 0; sr < LK_DC; ++sr){
      float tv = __ldg(Tc + sr * LK_DC + t);
      const float4* rp = (const float4*)(rd + sr * RB);
      #pragma unroll
      for(int q4 = 0; q4 < RB / 4; ++q4){
        float4 rv = rp[q4];
        xr[4 * q4] = fmaf(tv, rv.x, xr[4 * q4]); xr[4 * q4 + 1] = fmaf(tv, rv.y, xr[4 * q4 + 1]);
        xr[4 * q4 + 2] = fmaf(tv, rv.z, xr[4 * q4 + 2]); xr[4 * q4 + 3] = fmaf(tv, rv.w, xr[4 * q4 + 3]);
      }
    }
    __syncthreads();
    #pragma unroll
    for(int a = 0; a < RB; ++a) rd[t * RB + a] = xr[a];
    __syncthreads();
    #pragma unroll 8
    for(int sr = 0; sr < LK_DC; ++sr){
      float pv = __ldg(Pc + sr * LK_DC + t);
      const float4* dp = (const float4*)(rd + sr * RB);
      #pragma unroll
      for(int q4 = 0; q4 < RB / 4; ++q4){
        float4 dv = dp[q4];
        xo[4 * q4] = fmaf(pv, dv.x, xo[4 * q4]); xo[4 * q4 + 1] = fmaf(pv, dv.y, xo[4 * q4 + 1]);
        xo[4 * q4 + 2] = fmaf(pv, dv.z, xo[4 * q4 + 2]); xo[4 * q4 + 3] = fmaf(pv, dv.w, xo[4 * q4 + 3]);
      }
    }
    if(t < n){
      float4* op = (float4*)(o + ((long)(r0 + t) * HV + h) * D + a0);
      #pragma unroll
      for(int q4 = 0; q4 < RB / 4; ++q4) op[q4] = make_float4(xo[4 * q4] * sc, xo[4 * q4 + 1] * sc, xo[4 * q4 + 2] * sc, xo[4 * q4 + 3] * sc);
    }
    float gn = g[LK_DC];
    #pragma unroll
    for(int a = 0; a < 8; ++a)
      #pragma unroll
      for(int i = 0; i < B; ++i) s[a][i] *= gn;
    #pragma unroll 8
    for(int tt = 0; tt < n; ++tt){
      float kv[B];
      lk_load<B>(k + ((long)(r0 + tt) * NK + kh) * D + b0, kv);
      float gt = g[tt];
      float4 d0 = *(const float4*)(rd + tt * RB + aw), d1 = *(const float4*)(rd + tt * RB + aw + 4);
      float dv[8] = {d0.x, d0.y, d0.z, d0.w, d1.x, d1.y, d1.z, d1.w};
      #pragma unroll
      for(int a = 0; a < 8; ++a){
        float da = gt * dv[a];
        #pragma unroll
        for(int i = 0; i < B; ++i) s[a][i] = fmaf(da, kv[i], s[a][i]);
      }
    }
    // The memory as its storage keeps it, at the chunk's end.
    #pragma unroll
    for(int a = 0; a < 8; ++a)
      #pragma unroll
      for(int i = 0; i < B; ++i) s[a][i] = lk_kept<SS>(s[a][i]);
    __syncthreads();
  }
  #pragma unroll
  for(int a = 0; a < 8; ++a)
    #pragma unroll
    for(int i = 0; i < B; ++i) S[((long)h * D + a0 + aw + a) * D + b0 + i] = lk_to<SS>(s[a][i]);
}
"#;

/// The rows of a chunk of the chunked delta rule (`LK_DC` in its source), the rows of a value head's memory a block of
/// its walk takes, and the threads of that block: two warps.
pub const DELTA_CHUNK: usize = 64;
pub const DELTA_STATE_ROWS: usize = 16;
pub const DELTA_STATE_THREADS: usize = 64;

/// The dynamic shared memory of `lk_delta_prep`: a run of 32 of a chunk's keys' and queries' values (then `T`), `A`,
/// and the sums `T`'s lower blocks take — rows padded a value.
pub const DELTA_PREP_SMEM: usize = (2 * DELTA_CHUNK * 33 + DELTA_CHUNK * (DELTA_CHUNK + 1) + 3 * 256) * 4;

const _: () = assert!(DELTA_PREP_SMEM <= 48 * 1024, "the chunked delta rule's preparation keeps the default shared memory");
const _: () = assert!(DELTA_CHUNK * 33 * 2 >= DELTA_CHUNK * (DELTA_CHUNK + 1), "T fits where the runs of keys and queries were");

/// Whether the chunked delta rule takes heads `d` wide: at most 128, in whole runs of 32 (a lane of the walk's warp
/// `d / 32` values of the width, the preparation's runs of 32).
pub fn delta_chunks_take(d: usize) -> bool {
    d <= 128 && d % 32 == 0
}

/// The floats of the chunked delta rule's scratch over `chunks` chunks of `hv` value heads `d` wide: each chunk's
/// decayed keys and queries (a value of the width at a time), its `T·diag(β)` and masked scores, and its rows' decays
/// to its end with its own.
pub fn delta_chunk_scratch(chunks: usize, hv: usize, d: usize) -> [usize; 5] {
    let per = chunks * hv;
    [per * DELTA_CHUNK * d, per * DELTA_CHUNK * d, per * DELTA_CHUNK * DELTA_CHUNK, per * DELTA_CHUNK * DELTA_CHUNK, per * (DELTA_CHUNK + 1)]
}

/// The delta-rule output's epilogue, one block of `D` threads per value head: RMSNorm the head, scale by `gn`, gate
/// by `silu(z)`; `Q` quantizes the result, `OUTF` writes it. The head's work is `lk_delta_norm_row`, which the step's
/// last block a head also runs (`DELTA_STEP_NORM`), its warps playing the norm's `D` threads.
const DELTA_NORM: &str = r#"
// The head of `D` values at `i0`, a value a thread of the norm's own block: warp `w` of the block's plays the norm's
// warps `w`, `w + nw`, … (`VW` of them), each value read past the L1 under `L2`.
template<bool OUTF, int Q, typename A, int VW, bool L2>
__device__ __forceinline__ void lk_delta_norm_row(const float* __restrict__ z, const float* __restrict__ gn, const float* o,
                              float* __restrict__ out, signed char* __restrict__ oq, signed char* __restrict__ ol,
                              float* __restrict__ os, long i0, int D, float eps, A* red){
  const int nw = blockDim.x >> 5, vw = (D + 31) >> 5, warp = threadIdx.x >> 5, lane = threadIdx.x & 31;
  float val[VW];
  A acc[VW];
  #pragma unroll
  for(int j = 0; j < VW; ++j){
    const int u = (warp + j * nw) * 32 + lane;
    val[j] = warp + j * nw < vw && u < D ? (L2 ? __ldcg(o + i0 + u) : o[i0 + u]) : 0.f;
    acc[j] = (A)val[j] * (A)val[j];
  }
  A den = sqrt(lk_warps_sum<VW>(acc, vw, red) / (A)D + (A)eps);
  #pragma unroll
  for(int j = 0; j < VW; ++j){
    const int u = (warp + j * nw) * 32 + lane;
    if(warp + j * nw >= vw || u >= D) continue;
    const long i = i0 + u;
    float zz = z[i];
    float r = (float)((A)gn[u] * (A)val[j] / den) * (zz / (1.0f + expf(-zz)));
    if(OUTF) out[i] = r;
    if(Q > 0) lk_quant_lane<Q>(r, oq, ol, os, i, i >> 5);
  }
}
template<bool OUTF, int Q, typename A>
__global__ void lk_delta_norm(const float* __restrict__ z, const float* __restrict__ gn, const float* __restrict__ o,
                              float* __restrict__ out, signed char* __restrict__ oq, signed char* __restrict__ ol,
                              float* __restrict__ os, int D, float eps){
  __shared__ A red[32];
  lk_delta_norm_row<OUTF, Q, A, 1, false>(z, gn, o, out, oq, ol, os, (long)blockIdx.x * D, D, eps, red);
}
"#;

/// The delta-rule step a row at a time with its epilogue in its last block a head: `lk_delta_step` given the norm's
/// operands (`OUTF`, `Q`, its gate `z` and weight `gn`) and a count a head (a sequence's own, in a batch) — its blocks
/// write their rows' outputs and count in; the last reads the head's outputs back through L2 and runs
/// `lk_delta_norm_row` over each row's head, its warps playing the norm's `D` threads (`VW` each), then resets the
/// count. Every output, memory, norm and quantized word is the one `lk_delta_step` then `lk_delta_norm` compute. Given a
/// `qk_eps` its rows' query and key heads are normed first, as `lk_delta_rows` norms them.
const DELTA_STEP_NORM: &str = r#"
template<typename A, int DW, bool OUTF, int Q, int VW, typename SS>
__global__ void lk_delta_step(const float* __restrict__ q, const float* __restrict__ k, const float* __restrict__ v,
                              const float* __restrict__ bet, const float* __restrict__ alp, SS* __restrict__ S,
                              float* o, int D, int NK, int HV, int R, int seqs, const float* __restrict__ z,
                              const float* __restrict__ gn, float* __restrict__ out, signed char* __restrict__ oq,
                              signed char* __restrict__ ol, float* __restrict__ os, float eps, int* __restrict__ cnt,
                              float qk_eps = -1.f){
  __shared__ A red[32];
  __shared__ int last;
  lk_delta_rows<A, DW, SS>(q, k, v, bet, alp, S, o, D, NK, HV, R, seqs, qk_eps);
  __threadfence();
  __syncthreads();
  const long zs = seqs ? blockIdx.z : 0, h = blockIdx.x;
  if(threadIdx.x == 0) last = atomicAdd(cnt + zs * HV + h, 1) == gridDim.y - 1;
  __syncthreads();
  if(!last) return;
  __threadfence();
  for(int r = 0; r < R; ++r) lk_delta_norm_row<OUTF, Q, A, VW, true>(z, gn, o, out, oq, ol, os, ((zs * R + r) * HV + h) * D, D, eps, red);
  if(threadIdx.x == 0) cnt[zs * HV + h] = 0;
}
"#;

/// A mixture of experts for one token: the router's dot products over the `E` experts, a softmax, the top `K` kept
/// (and, with `norm`, renormalized to sum to one), then each kept expert's gated FFN and its down projection summed by
/// its weight — the experts read by index from the stacked matrices, so only the chosen rows move — and the shared
/// experts' SwiGLU FFN (`sdff` wide, 0 without) every token takes, its rows beside the kept experts' and its down row
/// added after their sum. `Q` is the numerics: every dot reads the activation quantized (`Q` 1, 2; the hidden rows
/// quantized between the two expert launches) or in `f32` with `f64` sums (`Q` 0), each the dot the plain GEMV of its
/// format takes (`lk_gemv_dot`). `lk_moe_route` (a warp an expert across the grid; the last block the softmax and the
/// top `K`), `lk_moe_up` (a warp per row of every kept expert's gate and up, then the shared expert's), `lk_moe_down`
/// (a block per output, a warp per kept expert and one for the shared expert).
const MOE: &str = r#"
// The router's logits (an expert's row of a router in format `FR`, or with `RF` of an f32 one read against the f32
// input, plus the router's bias `rb` when there is one) across the grid's blocks into `logits`: a warp an expert, or an
// f32 router's row split over `WPE` warps reading four values a lane at a time, their sums added in order; the last
// block to arrive (`cnt`, reset by it) takes their softmax and the top `K` — each pick a warp-wide arg-max, the first
// expert of the largest weight winning as a scan in expert order would — renormalized with `norm`, each its softmax
// over every expert without. Grid row `y` routes row `y` of the input into its own logits, choices and count.
// A row's softmax over its `E` experts' logits and its top `K` — each pick a warp-wide arg-max, the first expert of the
// largest weight winning as a scan in expert order would — renormalized with `norm`, each its softmax over every expert
// without: a block's threads, `p` `E` floats of shared memory, `red` 32. Lane `r` of the first warp keeps pick `r`'s
// weight for the renormalization (`K` at most 32), every lane summing the picks in order.
template<int Q>
__device__ __forceinline__ void lk_moe_pick_row(const float* logits, float* p, float* red, int E, int K, int norm,
                                                int* __restrict__ idx, float* __restrict__ wt){
  int lane = threadIdx.x & 31, warp = threadIdx.x >> 5;
  for(int i = threadIdx.x; i < E; i += blockDim.x) p[i] = __ldcg(logits + i);
  __syncthreads();
  float m = -INFINITY;
  for(int i = threadIdx.x; i < E; i += blockDim.x) m = fmaxf(m, p[i]);
  m = lk_block_max(m, red);
  typedef typename lk_acc<Q>::t A;
  __shared__ A sums[32];
  A s = 0;
  for(int i = threadIdx.x; i < E; i += blockDim.x){ float v = expf(p[i] - m); p[i] = v; s += (A)v; }
  s = lk_block_sum(s, sums);
  __syncthreads();
  if(warp == 0){
    A total = 0, mine = 0;
    for(int r = 0; r < K; ++r){
      float bv = -1.0f; int bi = -1;
      for(int i = lane; i < E; i += 32) if(p[i] > bv){ bv = p[i]; bi = i; }
      for(int o = 16; o > 0; o >>= 1){
        float ov = __shfl_down_sync(0xffffffffu, bv, o);
        int oi = __shfl_down_sync(0xffffffffu, bi, o);
        if(ov > bv || (ov == bv && oi >= 0 && (bi < 0 || oi < bi))){ bv = ov; bi = oi; }
      }
      bv = __shfl_sync(0xffffffffu, bv, 0);
      bi = __shfl_sync(0xffffffffu, bi, 0);
      const A pick = (A)bv / s;
      if(lane == r) mine = pick;
      if(lane == 0){ idx[r] = bi; p[bi] = -2.0f; if(!norm) wt[r] = (float)pick; }
      total += pick;
      __syncwarp();
    }
    if(norm && lane < K) wt[lane] = (float)(mine / total);
  }
}
template<int Q, int RF, int FR, int WPE = 1>
__global__ void __launch_bounds__(256) lk_moe_route(const float* __restrict__ x, const signed char* __restrict__ xq,
                             const signed char* __restrict__ xl, const float* __restrict__ xs, lk_w wr,
                             const float* __restrict__ wf, const float* __restrict__ rb, int d, int E, int K, int norm,
                             int* __restrict__ idx, float* __restrict__ wt, float* __restrict__ logits, int* __restrict__ cnt){
  extern __shared__ float p[];
  __shared__ float red[32];
  __shared__ double split[8];
  __shared__ int last;
  int lane = threadIdx.x & 31, warp = threadIdx.x >> 5, nw = blockDim.x >> 5, e = (blockIdx.x * nw + warp) / WPE;
  {
    long row = blockIdx.y;
    if(x) x += row * d;
    if(xq) xq += row * d;
    if(xl) xl += row * d;
    if(xs) xs += row * (d >> 5);
    idx += row * K; wt += row * K; logits += row * E; cnt += row;
  }
  if(RF && WPE > 1){
    // Warp `q` of an expert's `WPE` sums its `q`-th part of the row, four values a lane at a time.
    int q = warp % WPE, part = d / WPE;
    double sum = 0.0;
    if(e < E){
      const float4* we = (const float4*)(wf + (long)e * d + (long)q * part);
      const float4* xv = (const float4*)(x + (long)q * part);
      if(Q == 0){
        double a = 0.0;
        for(int t = lane; t < (part >> 2); t += 32){
          float4 w = we[t], v = xv[t];
          a += (double)w.x * (double)v.x + (double)w.y * (double)v.y + (double)w.z * (double)v.z + (double)w.w * (double)v.w;
        }
        sum = lk_warp_sum_d(a);
      } else {
        float a = 0.f;
        for(int t = lane; t < (part >> 2); t += 32){
          float4 w = we[t], v = xv[t];
          a += w.x * v.x + w.y * v.y + w.z * v.z + w.w * v.w;
        }
        sum = lk_warp_sum(a);
      }
    }
    if(lane == 0) split[warp] = sum;
    __syncthreads();
    if(lane == 0 && q == 0 && e < E){
      float v;
      if(Q == 0){ double t = split[warp]; for(int i = 1; i < WPE; ++i) t += split[warp + i]; v = (float)t; }
      else { float t = (float)split[warp]; for(int i = 1; i < WPE; ++i) t += (float)split[warp + i]; v = t; }
      logits[e] = rb ? v + rb[e] : v;
    }
  } else if(e < E){
    float v;
    if(RF){
      const float* we = wf + (long)e * d;
      if(Q == 0){ double a = 0.0; for(int t = lane; t < d; t += 32) a += (double)we[t] * (double)x[t]; v = (float)lk_warp_sum_d(a); }
      else { float a = 0.f; for(int t = lane; t < d; t += 32) a += we[t] * x[t]; v = lk_warp_sum(a); }
    } else {
      lk_gemv_dot<FR, Q, 1>(wr, e, d, x, xq, xl, xs, &v);
    }
    if(lane == 0) logits[e] = rb ? v + rb[e] : v;
  }
  __threadfence();
  __syncthreads();
  if(threadIdx.x == 0) last = atomicAdd(cnt, 1) == gridDim.x - 1;
  __syncthreads();
  if(!last) return;
  __threadfence();
  lk_moe_pick_row<Q>(logits, p, red, E, K, norm, idx, wt);
  if(threadIdx.x == 0) *cnt = 0;
}
// Row `blockIdx.x`'s choices and weights from its logits (`lk_moe_pick_row`), as `lk_moe_route` takes them: a wide
// chunk's router, its logits a GEMM's.
template<int Q>
__global__ void __launch_bounds__(256) lk_moe_pick(const float* __restrict__ logits, int E, int K, int norm,
                                                   int* __restrict__ idx, float* __restrict__ wt){
  extern __shared__ float p[];
  __shared__ float red[32];
  long row = blockIdx.x;
  lk_moe_pick_row<Q>(logits + row * E, p, red, E, K, norm, idx + row * K, wt + row * K);
}
// The rows' choices grouped by expert, in row order then choice order: `ecount[e]` choices of expert `e` from `eoff[e]`
// in `list` (each a choice `r·K + k`), and each choice's place in `list` (`pos`). One block: the counts by shared
// atomics and their offsets by one thread, then the choices a block-width at a time, each placed after its expert's
// choices before it — in earlier block-widths (`run`), in earlier warps of its own (`wc`), in earlier lanes of its
// warp (`__match_any_sync`). Shared memory: `run`, then `wc`, (1 + warps) · `E` ints.
__global__ void lk_moe_group(const int* __restrict__ idx, int n, int E, int* __restrict__ ecount, int* __restrict__ eoff,
                             int* __restrict__ list, int* __restrict__ pos){
  extern __shared__ int cur[];
  int *run = cur, *wc = cur + E;
  int lane = threadIdx.x & 31, warp = threadIdx.x >> 5, nw = blockDim.x >> 5;
  for(int e = threadIdx.x; e < E; e += blockDim.x) run[e] = 0;
  for(int j = threadIdx.x; j < nw * E; j += blockDim.x) wc[j] = 0;
  __syncthreads();
  for(int i = threadIdx.x; i < n; i += blockDim.x) atomicAdd(&run[idx[i]], 1);
  __syncthreads();
  if(threadIdx.x == 0){
    int at = 0;
    for(int e = 0; e < E; ++e){ int c = run[e]; ecount[e] = c; eoff[e] = at; run[e] = at; at += c; }
    eoff[E] = at;
  }
  __syncthreads();
  for(int base = 0; base < n; base += blockDim.x){
    int i = base + threadIdx.x, e = i < n ? idx[i] : -1;
    unsigned peers = __match_any_sync(0xffffffffu, e);
    int rank = __popc(peers & ((1u << lane) - 1u));
    if(e >= 0 && rank == 0) wc[warp * E + e] = __popc(peers);
    __syncthreads();
    if(e >= 0){
      int p = run[e] + rank;
      for(int u = 0; u < warp; ++u) p += wc[u * E + e];
      list[p] = i; pos[i] = p;
    }
    __syncthreads();
    for(int x = threadIdx.x; x < E; x += blockDim.x){
      int sum = 0;
      for(int u = 0; u < nw; ++u){ sum += wc[u * E + x]; wc[u * E + x] = 0; }
      run[x] += sum;
    }
    __syncthreads();
  }
}
// Each choice's activation row, in `list` order: the row of `x` (f32 under Q 0, its int8 words and scales otherwise)
// of the choice's token.
template<int Q>
__global__ void lk_moe_gather(const float* __restrict__ x, const signed char* __restrict__ xq, const signed char* __restrict__ xl,
                              const float* __restrict__ xs, const int* __restrict__ list, int K, int d, float* __restrict__ gx,
                              signed char* __restrict__ gq, signed char* __restrict__ gl, float* __restrict__ gs){
  long p = blockIdx.x, r = list[p] / K;
  for(int i = threadIdx.x; i < d; i += blockDim.x){
    if(Q == 0) gx[p * d + i] = x[r * d + i];
    else { gq[p * d + i] = xq[r * d + i]; if(Q == 2) gl[p * d + i] = xl[r * d + i]; }
  }
  if(Q > 0) for(int b = threadIdx.x; b < (d >> 5); b += blockDim.x) gs[p * (d >> 5) + b] = xs[r * (d >> 5) + b];
}
// A choice count in slabs of 16, then of 8, 4, 2 and 1 rows — every row in exactly one slab, each row's dots those of
// one row (`lk_gemv_dot` sums a row alike whatever its slab), `slab<M>(first)` called for each.
template<typename S>
__device__ __forceinline__ void lk_slabs(int n, S slab){
  int s0 = 0;
  for(; n - s0 >= 16; s0 += 16) slab.template run<16>(s0);
  if(n - s0 >= 8){ slab.template run<8>(s0); s0 += 8; }
  if(n - s0 >= 4){ slab.template run<4>(s0); s0 += 4; }
  if(n - s0 >= 2){ slab.template run<2>(s0); s0 += 2; }
  if(n - s0 >= 1) slab.template run<1>(s0);
}
template<int Q, int ACT, int FG, int FU>
struct lk_up_slab {
  const float* x; const signed char* xq; const signed char* xl; const float* xs; lk_w wg, wu;
  const float* bg; const float* bu; float* h; long wrow, off, j; int d, dff; float limit, slope;
  template<int M> __device__ __forceinline__ void run(int s0) const {
    long at = (off + s0) * d;
    float g[M], u[M];
    lk_gemv_dot<FG, Q, M>(wg, wrow, d, Q ? nullptr : x + at, Q ? xq + at : nullptr, Q == 2 ? xl + at : nullptr,
                          Q ? xs + (at >> 5) : nullptr, g);
    lk_gemv_dot<FU, Q, M>(wu, wrow, d, Q ? nullptr : x + at, Q ? xq + at : nullptr, Q == 2 ? xl + at : nullptr,
                          Q ? xs + (at >> 5) : nullptr, u);
    for(int t = threadIdx.x & 31; t < M; t += 32){
      const float gt = lk_pick(g, t), ut = lk_pick(u, t);
      float gv = bg ? gt + bg[wrow] : gt, uv = bu ? ut + bu[wrow] : ut;
      float o;
      if(ACT == 0) o = gv / (1.0f + expf(-gv)) * uv;
      else { float gc = fminf(gv, limit); o = gc / (1.0f + expf(-slope * gc)) * (fminf(fmaxf(uv, -limit), limit) + 1.0f); }
      h[(off + s0 + t) * dff + j] = o;
    }
  }
};
// Each expert's gate and up rows over the activation rows of the choices of it (`n` from `off` of the gathered rows),
// in slabs (`lk_slabs`), each row's dots as one row's (`lk_gemv_dot`), through the gated activation of `lk_moe_up`: a
// warp a row of the expert's, a grid row an expert.
template<int Q, int ACT, int FG, int FU>
__global__ void lk_moe_up_rows(const float* __restrict__ x, const signed char* __restrict__ xq,
                               const signed char* __restrict__ xl, const float* __restrict__ xs, lk_w wg, lk_w wu,
                               const float* __restrict__ bg, const float* __restrict__ bu, const int* __restrict__ ecount,
                               const int* __restrict__ eoff, float* __restrict__ h, int d, int dff, float limit, float slope){
  int e = blockIdx.y, n = ecount[e];
  long j = ((long)blockIdx.x * blockDim.x + threadIdx.x) >> 5;
  if(n == 0 || j >= dff) return;
  lk_slabs(n, lk_up_slab<Q, ACT, FG, FU>{x, xq, xl, xs, wg, wu, bg, bu, h, (long)e * dff + j, eoff[e], j, d, dff, limit, slope});
}
// Each expert's down rows over the hidden rows of the choices of it, in slabs (`lk_slabs`), each choice's result
// weighed by its weight into `part` (its place in `list`), as `lk_moe_down` weighs it: a warp an output, a grid row an
// expert.
template<int Q, int FD>
struct lk_down_slab {
  const float* h; const signed char* hq; const signed char* hl; const float* hs; lk_w wd; const float* bd;
  const int* list; const float* wt; typename lk_acc<Q>::t* part; long wrow, off, o; int d, dff;
  template<int M> __device__ __forceinline__ void run(int s0) const {
    typedef typename lk_acc<Q>::t A;
    long at = (off + s0) * dff;
    float v[M];
    lk_gemv_dot<FD, Q, M>(wd, wrow, dff, Q ? nullptr : h + at, Q ? hq + at : nullptr, Q == 2 ? hl + at : nullptr,
                          Q ? hs + (at >> 5) : nullptr, v);
    for(int t = threadIdx.x & 31; t < M; t += 32){
      const float vt = lk_pick(v, t);
      part[(off + s0 + t) * d + o] = (A)(bd ? vt + bd[wrow] : vt) * (A)wt[list[off + s0 + t]];
    }
  }
};
template<int Q, int FD>
__global__ void lk_moe_down_rows(const float* __restrict__ h, const signed char* __restrict__ hq,
                                 const signed char* __restrict__ hl, const float* __restrict__ hs, lk_w wd,
                                 const float* __restrict__ bd, const int* __restrict__ ecount, const int* __restrict__ eoff,
                                 const int* __restrict__ list, const float* __restrict__ wt,
                                 typename lk_acc<Q>::t* __restrict__ part, int d, int dff){
  int e = blockIdx.y, n = ecount[e];
  long o = ((long)blockIdx.x * blockDim.x + threadIdx.x) >> 5;
  if(n == 0 || o >= d) return;
  lk_slabs(n, lk_down_slab<Q, FD>{h, hq, hl, hs, wd, bd, list, wt, part, (long)e * d + o, eoff[e], o, d, dff});
}
// Each choice's hidden row from its expert's gate and up rows over it (`g`, `u`, in `list` order), plus the expert's
// biases, through the gated activation of `lk_moe_up`.
template<int ACT>
__global__ void lk_moe_glu(const float* __restrict__ g, const float* __restrict__ u, const float* __restrict__ bg,
                           const float* __restrict__ bu, const int* __restrict__ idx, const int* __restrict__ list,
                           float* __restrict__ h, long n, int dff, float limit, float slope){
  long i = (long)blockIdx.x * blockDim.x + threadIdx.x;
  if(i >= n) return;
  long wrow = (long)idx[list[i / dff]] * dff + i % dff;
  float gv = bg ? g[i] + bg[wrow] : g[i], uv = bu ? u[i] + bu[wrow] : u[i];
  if(ACT == 0) h[i] = gv / (1.0f + expf(-gv)) * uv;
  else { float gc = fminf(gv, limit); h[i] = gc / (1.0f + expf(-slope * gc)) * (fminf(fmaxf(uv, -limit), limit) + 1.0f); }
}
// Output `o` of row `r`: its choices' weighed results summed in choice order, as `lk_moe_down` sums them.
template<int Q>
__device__ __forceinline__ float lk_moe_combined(const typename lk_acc<Q>::t* __restrict__ part, const int* __restrict__ pos,
                                                 int K, int d, long r, long o){
  typedef typename lk_acc<Q>::t A;
  A s = 0;
  for(int k = 0; k < K; ++k) s += part[(long)pos[r * K + k] * d + o];
  return (float)s;
}
// Each row's output (`lk_moe_combined`).
template<int Q>
__global__ void lk_moe_combine(const typename lk_acc<Q>::t* __restrict__ part, const int* __restrict__ pos, int K, int d,
                               float* __restrict__ y){
  long r = blockIdx.y, o = (long)blockIdx.x * blockDim.x + threadIdx.x;
  if(o >= d) return;
  y[r * d + o] = lk_moe_combined<Q>(part, pos, K, d, r, o);
}
// Each chosen expert's gate and up rows (formats `FG`, `FU`, plus their biases `bg`, `bu`) through the gated activation:
// SwiGLU, or with `ACT` 1 the gate clamped to at most `limit`, the up to `[−limit, limit]`, the sigmoid sharpened by
// `slope`; then the shared expert's (formats `FSG`, `FSU`, `sdff` rows) through SwiGLU, after the kept experts' rows.
// A warp takes `W` consecutive rows, all of one expert (`W` divides `dff` and `sdff`).
template<int Q, int ACT, int FG, int FU, int FSG, int FSU, int W = 1>
__global__ void lk_moe_up(const float* __restrict__ x, const signed char* __restrict__ xq,
                          const signed char* __restrict__ xl, const float* __restrict__ xs, lk_w wg, lk_w wu,
                          lk_w sg, lk_w su, const float* __restrict__ bg, const float* __restrict__ bu,
                          const int* __restrict__ idx, float* __restrict__ h, int d, int dff, int K, int sdff,
                          float limit, float slope){
  long row = (((long)blockIdx.x * blockDim.x + threadIdx.x) >> 5) * W, routed = (long)K * dff;
  if(row >= routed + sdff) return;
  float gv[W], uv[W];
  if(row < routed){
    int r = row / dff, j = row % dff;
    long wrow = (long)idx[r] * dff + j;
    lk_gemv_dot<FG, Q, 1, W>(wg, wrow, d, x, xq, xl, xs, gv);
    lk_gemv_dot<FU, Q, 1, W>(wu, wrow, d, x, xq, xl, xs, uv);
    #pragma unroll
    for(int i = 0; i < W; ++i){
      if(bg) gv[i] += bg[wrow + i];
      if(bu) uv[i] += bu[wrow + i];
    }
  } else {
    lk_gemv_dot<FSG, Q, 1, W>(sg, row - routed, d, x, xq, xl, xs, gv);
    lk_gemv_dot<FSU, Q, 1, W>(su, row - routed, d, x, xq, xl, xs, uv);
  }
  for(int i = threadIdx.x & 31; i < W; i += 32){
    const float gi = lk_pick(gv, i), ui = lk_pick(uv, i);
    if(ACT == 0 || row >= routed) h[row + i] = gi / (1.0f + expf(-gi)) * ui;
    else {
      float gc = fminf(gi, limit);
      h[row + i] = gc / (1.0f + expf(-slope * gc)) * (fminf(fmaxf(ui, -limit), limit) + 1.0f);
    }
  }
}
// Each chosen expert's down row (format `FD`, plus its bias `bd`), summed by the expert's weight; then the shared
// expert's down row (format `FSD`) over the hidden rows past the kept experts', added to the sum. A block takes `W`
// consecutive outputs: a warp each kept expert's `W` rows, and `SW` warps (`SW` dividing `W`) the shared expert's, `W /
// SW` rows each — every output's dots and sums as a block of one output computes them. `plus`, when given, is added
// to each output, as an add of the two arrays adds it.
template<int Q, int FD, int FSD, int W, int SW>
__device__ __forceinline__ void lk_moe_down_block(const float* __restrict__ h, const signed char* __restrict__ hq,
                            const signed char* __restrict__ hl, const float* __restrict__ hs, lk_w wd, lk_w sd,
                            const float* __restrict__ bd, const int* __restrict__ idx, const float* __restrict__ wt,
                            float* __restrict__ y, int d, int dff, int K, int sdff, const float* __restrict__ plus){
  typedef typename lk_acc<Q>::t A;
  constexpr int SR = W / SW;
  __shared__ A part[32][W];
  int o0 = blockIdx.x * W, lane = threadIdx.x & 31, r = threadIdx.x >> 5;
  if(r < K){
    long wrow = (long)idx[r] * d + o0;
    long hoff = (long)r * dff;
    float v[W];
    lk_gemv_dot<FD, Q, 1, W>(wd, wrow, dff, h + hoff, Q ? hq + hoff : nullptr, Q == 2 ? hl + hoff : nullptr,
                             Q ? hs + (hoff >> 5) : nullptr, v);
    if(lane == 0)
      #pragma unroll
      for(int i = 0; i < W; ++i) part[r][i] = (A)(bd ? v[i] + bd[wrow + i] : v[i]) * (A)wt[r];
  } else {
    int i0 = (r - K) * SR;
    long hoff = (long)K * dff;
    float v[SR];
    lk_gemv_dot<FSD, Q, 1, SR>(sd, o0 + i0, sdff, h + hoff, Q ? hq + hoff : nullptr, Q == 2 ? hl + hoff : nullptr,
                               Q ? hs + (hoff >> 5) : nullptr, v);
    if(lane == 0)
      #pragma unroll
      for(int i = 0; i < SR; ++i) part[K][i0 + i] = (A)v[i];
  }
  __syncthreads();
  if(threadIdx.x < W){
    int i = threadIdx.x;
    A s = 0;
    for(int e = 0; e < K; ++e) s += part[e][i];
    // The shared expert's row adds to the experts' sum as an `f32` sum of the two results.
    float v = sdff ? (float)s + (float)part[K][i] : (float)s;
    y[o0 + i] = plus ? v + plus[o0 + i] : v;
  }
}
template<int Q, int FD, int FSD, int W = 1, int SW = 1>
__global__ void lk_moe_down(const float* __restrict__ h, const signed char* __restrict__ hq,
                            const signed char* __restrict__ hl, const float* __restrict__ hs, lk_w wd, lk_w sd,
                            const float* __restrict__ bd, const int* __restrict__ idx, const float* __restrict__ wt,
                            float* __restrict__ y, int d, int dff, int K, int sdff){
  lk_moe_down_block<Q, FD, FSD, W, SW>(h, hq, hl, hs, wd, sd, bd, idx, wt, y, d, dff, K, sdff, nullptr);
}
"#;

/// A mixture with the residual add and the next norm in its last step (`moe_deferred_finalize`): `lk_moe_down` or a
/// chunk's `lk_moe_combine`, each output plus `plus` when the mixture's sum with another array is what the norm adds,
/// then `lk_rmsnorm` over the rows it wrote. The decode step's down projection counts its blocks in at `cnt` as they
/// write their outputs; the last reads the row back through L2 and norms it, its warps playing the norm's own block of
/// `vt` threads (`lk_rmsnorm_row`'s `VW`), then resets the count. A chunk's combine takes a block of the norm's threads a
/// row, combining its row before norming it. Either computes each output, sum and norm as the launches apart do.
const MOE_NORM: &str = r#"
template<int Q, int FD, int FSD, int W, int SW, bool ADD, bool SUM, bool OUTF, int QO, int PER, typename A, bool PRE, int VW>
__global__ void lk_moe_down_norm(const float* __restrict__ h, const signed char* __restrict__ hq,
                                 const signed char* __restrict__ hl, const float* __restrict__ hs, lk_w wd, lk_w sd,
                                 const float* __restrict__ bd, const int* __restrict__ idx, const float* __restrict__ wt,
                                 float* y, int d, int dff, int K, int sdff, const float* __restrict__ plus, lk_norm_of nm,
                                 int vt, int* __restrict__ cnt){
  __shared__ A red[32];
  __shared__ int last;
  lk_moe_down_block<Q, FD, FSD, W, SW>(h, hq, hl, hs, wd, sd, bd, idx, wt, y, d, dff, K, sdff, plus);
  __threadfence();
  __syncthreads();
  if(threadIdx.x == 0) last = atomicAdd(cnt, 1) == gridDim.x - 1;
  __syncthreads();
  if(!last) return;
  __threadfence();
  lk_out yr = nm.y ? LK_ROW(nm.y, nm.yrow, nm.ypitch) : lk_out();
  lk_rmsnorm_row<ADD, SUM, OUTF, QO, PER, A, PRE, true, VW>(nm.x, nm.r, nm.s, nm.w, yr, nm.yq, nm.yl, nm.ys, d, nm.eps, 0, 0,
                                                            nm.pw, nm.peps, red, vt);
  if(threadIdx.x == 0) *cnt = 0;
}
template<int Q, bool ADD, bool SUM, bool OUTF, int QO, int PER, typename A, bool PRE>
__global__ void __launch_bounds__(1024) lk_moe_combine_norm(const typename lk_acc<Q>::t* __restrict__ part,
                                 const int* __restrict__ pos, int K, int d, const float* __restrict__ plus, float* y,
                                 lk_norm_of nm){
  __shared__ A red[32];
  long r = blockIdx.x, base = r * d;
  for(int o = threadIdx.x; o < d; o += blockDim.x){
    float v = lk_moe_combined<Q>(part, pos, K, d, r, o);
    y[base + o] = plus ? v + plus[base + o] : v;
  }
  __syncthreads();
  lk_out yr = nm.y ? LK_ROW(nm.y, nm.yrow, nm.ypitch) + base : lk_out();
  lk_rmsnorm_row<ADD, SUM, OUTF, QO, PER, A, PRE, false>(nm.x, nm.r, nm.s, nm.w, yr, nm.yq, nm.yl, nm.ys, d, nm.eps, base,
                                                         base, nm.pw, nm.peps, red, blockDim.x);
}
"#;
