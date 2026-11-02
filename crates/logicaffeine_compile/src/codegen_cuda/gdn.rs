//! Gated DeltaNet lowered to CUDA — the mechanism behind 48 of Qwen3.8-27B's 64 layers.
//!
//! Each value head carries a fixed `D×D` associative memory `S` and rewrites it per token:
//!
//! ```text
//!   u = S·k                      what the memory predicts for this key
//!   S = α·S + β·(v − α·u)·kᵀ     decay, then correct by the prediction error
//!   o = S·q                      read the UPDATED memory
//!   o = rmsnorm(o ⊙ silu(z))·GN  gate FIRST, then normalize
//! ```
//!
//! ## The fusion
//!
//! Written literally that is three passes over `S` — one for `S·k`, one for the rank-1 update, one for `S·q`.
//! A dataflow graph of separate operators (ggml's shape) cannot do better, because each operator's output is
//! a materialized tensor. But the update is affine in `S`, so the second matvec expands:
//!
//! ```text
//!   o[a] = Σ_b S_new[a][b]·q[b]
//!        = Σ_b (α·S_old[a][b] + coef[a]·k[b])·q[b]
//!        = α·(S_old·q)[a]  +  coef[a]·(k·q)
//! ```
//!
//! `(k·q)` is ONE SCALAR per head. So a single pass that loads `S_old[a][b]` into a register accumulates
//! `S·k`, `S·q` and `k·q` together, and the register is still live for the write-back — **one read and one
//! write of the memory per token**, with the post-update read costing nothing extra. That is the whole reason
//! this op is a first-class LOGOS block rather than a composition of smaller ones.

/// One block per `(value head, row)` computes that row's `u`, `w` and the shared `k·q` in a single pass over
/// the memory, then rewrites the row from the register it already holds. `o` lands in scratch; the gated
/// RMSNorm needs the whole head, so it is a second (tiny) kernel.
pub const GDN_KERNELS: &str = r#"
// Split one f32 activation into two int8 words: a ~= s*(qh + ql/127). Lives here because it is the first
// device code emitted, and both the GDN gate norm and the projection producers need it.
__device__ __forceinline__ void lgx_q8hl(float t, signed char* __restrict__ qh,
                                         signed char* __restrict__ ql, long i){
  int h = __float2int_rn(t);
  h = h > 127 ? 127 : (h < -127 ? -127 : h);
  int l = __float2int_rn((t - (float)h) * 127.0f);
  qh[i] = (signed char)h;
  ql[i] = (signed char)l;
}

__device__ __forceinline__ float gdn_warp_sum(float v){
  for(int off=16; off>0; off>>=1) v += __shfl_down_sync(0xffffffffu, v, off);
  return v;
}
// Three block-wide reductions sharing one traversal of the memory row.
__device__ __forceinline__ void gdn_block_sum3(float a, float b, float c, float* oa, float* ob, float* oc){
  __shared__ float sa[32], sb[32], sc[32];
  int lane = threadIdx.x & 31, warp = threadIdx.x >> 5, nw = (blockDim.x + 31) >> 5;
  a = gdn_warp_sum(a); b = gdn_warp_sum(b); c = gdn_warp_sum(c);
  if(lane == 0){ sa[warp] = a; sb[warp] = b; sc[warp] = c; }
  __syncthreads();
  if(threadIdx.x == 0){
    float ta = 0.f, tb = 0.f, tc = 0.f;
    for(int i = 0; i < nw; ++i){ ta += sa[i]; tb += sb[i]; tc += sc[i]; }
    *oa = ta; *ob = tb; *oc = tc;
  }
  __syncthreads();
}

// grid = (HV, D), block = D threads. Thread b owns column b of row a of head h's memory.
// ONE WARP PER STATE ROW. The state is D x D per value head; row `a` needs S[a]·k, S[a]·q and k·q, which is
// three reductions over D. Giving the row to a warp makes all three warp shuffles over registers the lanes
// already hold — no __syncthreads, and the row is re-used from those same registers for the write-back, so
// the D x D memory is read once and written once. The block-per-row form this replaces launched D blocks per
// head (295k blocks per token across 48 layers) and paid three block-wide barriers to reduce one float each.
// `sreg` is sized for D <= 256; the loops are lane-strided so any D, including D < 32, is handled.
__global__ void gdn_step(const float* __restrict__ q, const float* __restrict__ k, const float* __restrict__ v,
                         const float* __restrict__ bet, const float* __restrict__ alp,
                         float* __restrict__ S, float* __restrict__ o, int D, int NK){
  const unsigned m = 0xffffffffu;
  int h = blockIdx.x, lane = threadIdx.x & 31, warp = threadIdx.x >> 5;
  int a = blockIdx.y * (blockDim.x >> 5) + warp;
  if(a >= D) return;
  // The key head this value head reads. llama.cpp broadcasts k/q from NK heads to NV with
  // `ggml_repeat_4d`, and ggml_repeat TILES (index modulo) rather than repeat-interleaving — so the
  // pairing is h % NK ([0,1,..,15,0,1,..,15,..]), NOT h / (NV/NK) ([0,0,0,1,1,1,..]).
  int g = h % NK;
  const float* kk = k + (long)g * D;
  const float* qq = q + (long)g * D;
  long base = (long)h * D * D + (long)a * D;

  float sreg[8];
  float u_ = 0.f, w_ = 0.f, kq_ = 0.f;
  int n = 0;
  for(int b = lane; b < D; b += 32){
    float sv = S[base + b], kb = kk[b], qb = qq[b];
    sreg[n++] = sv; u_ += sv * kb; w_ += sv * qb; kq_ += kb * qb;
  }
  for(int sh = 16; sh > 0; sh >>= 1){
    u_ += __shfl_down_sync(m, u_, sh); w_ += __shfl_down_sync(m, w_, sh); kq_ += __shfl_down_sync(m, kq_, sh);
  }
  u_ = __shfl_sync(m, u_, 0); w_ = __shfl_sync(m, w_, 0); kq_ = __shfl_sync(m, kq_, 0);

  float alpha = alp[h], beta = bet[h];
  float coef  = beta * (v[(long)h * D + a] - alpha * u_);
  // The 1/sqrt(D) output scale llama.cpp applies (`attn_col * scale`). It does NOT cancel under the
  // following RMSNorm: the recurrence output is tiny (RMS ~1e-2), so `eps` is comparable to the scaled
  // variance and `s*x/sqrt(s^2*m + eps) != x/sqrt(m + eps)`. Dropping it scaled the whole layer by ~1.25x.
  if(lane == 0) o[(long)h * D + a] = (alpha * w_ + coef * kq_) * rsqrtf((float)D);
  n = 0;
  for(int b = lane; b < D; b += 32) S[base + b] = alpha * sreg[n++] + coef * kk[b];
}

// grid = (HV), block = D. RMSNorm the head and scale by the shared weight, THEN gate by silu(z) — the order
// llama.cpp's `build_norm_gated` uses for this architecture. (HF's Qwen3-Next gates first; Qwen3.5 does not,
// and the checkpoint follows Qwen3.5.)
__global__ void gdn_gate_norm(const float* __restrict__ z, const float* __restrict__ gn,
                              const float* __restrict__ o, float* __restrict__ out, int D, float eps){
  int h = blockIdx.x, a = threadIdx.x;
  long i = (long)h * D + a;
  float zz = z[i];
  float val = o[i];
  __shared__ float ss[32];
  int lane = threadIdx.x & 31, warp = threadIdx.x >> 5, nw = (blockDim.x + 31) >> 5;
  float p = gdn_warp_sum(val * val);
  if(lane == 0) ss[warp] = p;
  __syncthreads();
  __shared__ float den;
  if(threadIdx.x == 0){ float t = 0.f; for(int j = 0; j < nw; ++j) t += ss[j]; den = sqrtf(t / (float)D + eps); }
  __syncthreads();
  out[i] = (gn[a] * val / den) * (zz / (1.0f + __expf(-zz)));
}

// The same epilogue with the ACTIVATION QUANTIZATION fused in. The o-projection that follows consumes int8,
// and `blockDim == D` with `D % 32 == 0` means one warp owns exactly one 32-element quantization group — so
// the group amax is a warp shuffle over registers the block already holds, not a second pass over global
// memory. Saves a launch and a full round-trip of the value vector per GDN layer.
__global__ void gdn_gate_norm_q(const float* __restrict__ z, const float* __restrict__ gn,
                                const float* __restrict__ o, float* __restrict__ out,
                                signed char* __restrict__ xq, float* __restrict__ xs, int D, float eps){
  int h = blockIdx.x, a = threadIdx.x;
  long i = (long)h * D + a;
  float zz = z[i];
  float val = o[i];
  __shared__ float ss[32];
  int lane = threadIdx.x & 31, warp = threadIdx.x >> 5, nw = (blockDim.x + 31) >> 5;
  float p = gdn_warp_sum(val * val);
  if(lane == 0) ss[warp] = p;
  __syncthreads();
  __shared__ float den;
  if(threadIdx.x == 0){ float t = 0.f; for(int j = 0; j < nw; ++j) t += ss[j]; den = sqrtf(t / (float)D + eps); }
  __syncthreads();
  float y = (gn[a] * val / den) * (zz / (1.0f + __expf(-zz)));
  out[i] = y;
  float am = fabsf(y);
  for(int sh = 16; sh > 0; sh >>= 1) am = fmaxf(am, __shfl_down_sync(0xffffffffu, am, sh));
  am = __shfl_sync(0xffffffffu, am, 0);
  float sc = am > 0.f ? am / 127.0f : 1.0f;
  if(lane == 0) xs[i >> 5] = sc;
  xq[i] = (signed char)__float2int_rn(y / sc);
}

// The gate norm with the two-word activation split, for the lane that keeps the activation nearly exact.
__global__ void gdn_gate_norm_hl(const float* __restrict__ z, const float* __restrict__ gn,
                                const float* __restrict__ o, float* __restrict__ out,
                                signed char* __restrict__ xq, signed char* __restrict__ xl,
                                 float* __restrict__ xs, int D, float eps){
  int h = blockIdx.x, a = threadIdx.x;
  long i = (long)h * D + a;
  float zz = z[i];
  float val = o[i];
  __shared__ float ss[32];
  int lane = threadIdx.x & 31, warp = threadIdx.x >> 5, nw = (blockDim.x + 31) >> 5;
  float p = gdn_warp_sum(val * val);
  if(lane == 0) ss[warp] = p;
  __syncthreads();
  __shared__ float den;
  if(threadIdx.x == 0){ float t = 0.f; for(int j = 0; j < nw; ++j) t += ss[j]; den = sqrtf(t / (float)D + eps); }
  __syncthreads();
  float y = (gn[a] * val / den) * (zz / (1.0f + __expf(-zz)));
  out[i] = y;
  float am = fabsf(y);
  for(int sh = 16; sh > 0; sh >>= 1) am = fmaxf(am, __shfl_down_sync(0xffffffffu, am, sh));
  am = __shfl_sync(0xffffffffu, am, 0);
  float sc = am > 0.f ? am / 127.0f : 1.0f;
  if(lane == 0) xs[i >> 5] = sc;
  lgx_q8hl(y / sc, xq, xl, i);
}
"#;

/// The rest of the Qwen3.8-specific device code: the pieces of a GDN layer that surround the recurrence, plus
/// the two things gated attention needs that a plain transformer does not (a partial rotary and an output
/// gate). Each is deliberately fused past what a per-operator graph could do — the conv carries silu, the
/// gate kernel computes both gates from one launch, and the norm+rope stay one pass over the head.
pub const QWEN38_KERNELS: &str = r#"
// Depthwise causal conv over C channels with a `T`-deep ring, silu FUSED into the epilogue. The ring is the
// layer's carried state: shift up one, append this token, reduce. Depthwise means no cross-channel mixing, so
// one thread owns a channel end to end and the whole thing is a single pass.
__global__ void gk_conv_silu(const float* __restrict__ x, const float* __restrict__ wc,
                             float* __restrict__ cs, float* __restrict__ y, int C, int T){
  int c = blockIdx.x * blockDim.x + threadIdx.x; if(c >= C) return;
  for(int t = 0; t < T - 1; ++t) cs[(long)t * C + c] = cs[(long)(t + 1) * C + c];
  cs[(long)(T - 1) * C + c] = x[c];
  float acc = 0.f;
  for(int t = 0; t < T; ++t) acc += cs[(long)t * C + c] * wc[(long)t * C + c];
  y[c] = acc / (1.0f + __expf(-acc));            // silu, fused
}

// The conv with BOTH epilogues fused: silu, then — for the leading `nl2` head-blocks, which hold q and k —
// the per-head L2 normalize GDN applies before the delta update. Launched one block per head-sized chunk
// (`blockDim == D`, `grid == C / D`), so the normalize is a reduction over registers the block already owns.
// The v region (blocks >= nl2) takes the silu and stops. One launch where there were two, and q/k never
// leave the SM between the two steps.
__global__ void gk_conv_silu_l2(const float* __restrict__ x, const float* __restrict__ wc,
                                float* __restrict__ cs, float* __restrict__ y, int C, int T,
                                int nl2, float eps){
  int c = blockIdx.x * blockDim.x + threadIdx.x; if(c >= C) return;
  for(int t = 0; t < T - 1; ++t) cs[(long)t * C + c] = cs[(long)(t + 1) * C + c];
  cs[(long)(T - 1) * C + c] = x[c];
  float acc = 0.f;
  for(int t = 0; t < T; ++t) acc += cs[(long)t * C + c] * wc[(long)t * C + c];
  float v = acc / (1.0f + __expf(-acc));          // silu, fused
  if(blockIdx.x >= nl2){ y[c] = v; return; }      // the v region needs no normalize
  __shared__ float ss[32];
  int lane = threadIdx.x & 31, warp = threadIdx.x >> 5, nw = (blockDim.x + 31) >> 5;
  float p = gdn_warp_sum(v * v);
  if(lane == 0) ss[warp] = p;
  __syncthreads();
  __shared__ float den;
  if(threadIdx.x == 0){ float t = 0.f; for(int j = 0; j < nw; ++j) t += ss[j]; den = sqrtf(t + eps); }
  __syncthreads();
  y[c] = v / den;
}

// Per-head L2 normalize in place — GDN normalizes q and k before the delta update. grid = heads, block = D.
__global__ void gk_l2norm_heads(float* __restrict__ x, int D, float eps){
  int h = blockIdx.x, i = threadIdx.x; long o = (long)h * D + i;
  float v = x[o];
  __shared__ float ss[32];
  int lane = threadIdx.x & 31, warp = threadIdx.x >> 5, nw = (blockDim.x + 31) >> 5;
  float p = gdn_warp_sum(v * v);
  if(lane == 0) ss[warp] = p;
  __syncthreads();
  __shared__ float den;
  if(threadIdx.x == 0){ float t = 0.f; for(int j = 0; j < nw; ++j) t += ss[j]; den = sqrtf(t + eps); }
  __syncthreads();
  x[o] = v / den;
}

// Both GDN gates in one launch: write strength and forget rate, one value per value head.
//   beta  = sigmoid(br)
//   decay = (1 + e^(ar + dt))^rate
// The paper writes decay as exp(−exp(A_log)·softplus(ar+dt)), but the GGUF conversion ALREADY stores
// −exp(A_log) in `ssm_a` (verified: all 48 values negative, |v| < 0.08), so `rate` is used directly — 
// exponentiating it again turns a ^(−0.04) long memory into a ^(−0.96) one that forgets almost every token.
// softplus is ln(1+e^u), so the logarithm cancels against the outer exponential: one fewer transcendental.
__global__ void gk_gdn_gates(const float* __restrict__ br, const float* __restrict__ ar,
                             const float* __restrict__ dt, const float* __restrict__ alog,
                             float* __restrict__ beta, float* __restrict__ decay, int NV){
  int i = blockIdx.x * blockDim.x + threadIdx.x; if(i >= NV) return;
  beta[i] = 1.0f / (1.0f + __expf(-br[i]));
  decay[i] = powf(1.0f + __expf(ar[i] + dt[i]), alog[i]);
}

// Per-head RMSNorm + PARTIAL RoPE. Qwen3.8 rotates `rot` of each head's `hd` dims (64 of 256) and passes the
// rest through, and the inverse frequencies are computed over `rot`, not `hd` — using `hd` there would detune
// every rotation while still producing plausible-looking output.
__global__ void gk_qk_norm_rope_part(float* __restrict__ base, const int* __restrict__ d_pos, int stride,
                                     const float* __restrict__ nw, int n_heads, int hd, int rot,
                                     float rbase, float eps){
  int head = blockIdx.x; if(head >= n_heads) return;
  float* h = base + (long)(*d_pos) * stride + (long)head * hd;
  __shared__ float sh[128];
  float s = 0.f; for(int i = threadIdx.x; i < hd; i += blockDim.x) s += h[i] * h[i];
  sh[threadIdx.x] = s; __syncthreads();
  for(int k = blockDim.x / 2; k > 0; k >>= 1){ if(threadIdx.x < k) sh[threadIdx.x] += sh[threadIdx.x + k]; __syncthreads(); }
  float sc = rsqrtf(sh[0] / hd + eps); __syncthreads();
  for(int i = threadIdx.x; i < hd; i += blockDim.x) h[i] = h[i] * sc * nw[i];
  __syncthreads();
  int half = rot / 2, pos = *d_pos;
  for(int i = threadIdx.x; i < half; i += blockDim.x){
    float f = powf(rbase, -2.0f * (float)i / (float)rot);       // over the ROTARY width
    float a = pos * f, c = cosf(a), si = sinf(a);
    float x0 = h[i], x1 = h[i + half];
    h[i] = x0 * c - x1 * si; h[i + half] = x0 * si + x1 * c;
  }
  // dims [rot, hd) are carried through untouched — that is what "partial" means.
}

// Gated attention's output gate: ctx *= sigmoid(g). The checkpoint packs g into the Q projection, so this is
// the only extra work a gated attention layer does over an ordinary one.
__global__ void gk_gate_mul(float* __restrict__ ctx, const float* __restrict__ g, int n){
  int i = blockIdx.x * blockDim.x + threadIdx.x;
  if(i < n) ctx[i] *= 1.0f / (1.0f + __expf(-g[i]));
}
"#;

/// Emit a self-contained CUDA program for ONE Gated DeltaNet step with every input inlined, so the fused
/// kernels above can be checked against the LOGOS `gated delta` op on the VM at small shapes on any GPU —
/// the same shape of proof `emit_moe_forward` gives the MoE block. Prints `O` then the mutated memory `S`,
/// because a desugar that ignored the memory would otherwise pass.
#[allow(clippy::too_many_arguments)]
pub fn emit_gated_delta_forward(
    q: &[f64],
    k: &[f64],
    v: &[f64],
    z: &[f64],
    beta: &[f64],
    alpha: &[f64],
    s0: &[f64],
    gn: &[f64],
    hv: usize,
    hk: usize,
    d: usize,
) -> String {
    let arr = |name: &str, xs: &[f64]| {
        let body = xs
            .iter()
            .map(|x| format!("{:.17e}f", x))
            .collect::<Vec<_>>()
            .join(",");
        format!("static const float {name}[] = {{{body}}};\n")
    };
    format!(
        "#include <cstdio>\n#include <cmath>\n#include <cuda_runtime.h>\n{kern}\n\
         {aq}{ak}{av}{az}{ab}{aa}{as_}{ag}\n\
         int main(){{\n\
         \x20 const int D={d}, HV={hv}, NK={hk};\n\
         \x20 float *dq,*dk,*dv,*dz,*db,*da,*dS,*dgn,*dO,*dOut;\n\
         \x20 cudaMalloc(&dq,sizeof(hq)); cudaMemcpy(dq,hq,sizeof(hq),cudaMemcpyHostToDevice);\n\
         \x20 cudaMalloc(&dk,sizeof(hk_)); cudaMemcpy(dk,hk_,sizeof(hk_),cudaMemcpyHostToDevice);\n\
         \x20 cudaMalloc(&dv,sizeof(hv_)); cudaMemcpy(dv,hv_,sizeof(hv_),cudaMemcpyHostToDevice);\n\
         \x20 cudaMalloc(&dz,sizeof(hz)); cudaMemcpy(dz,hz,sizeof(hz),cudaMemcpyHostToDevice);\n\
         \x20 cudaMalloc(&db,sizeof(hb)); cudaMemcpy(db,hb,sizeof(hb),cudaMemcpyHostToDevice);\n\
         \x20 cudaMalloc(&da,sizeof(ha)); cudaMemcpy(da,ha,sizeof(ha),cudaMemcpyHostToDevice);\n\
         \x20 cudaMalloc(&dS,sizeof(hs)); cudaMemcpy(dS,hs,sizeof(hs),cudaMemcpyHostToDevice);\n\
         \x20 cudaMalloc(&dgn,sizeof(hg)); cudaMemcpy(dgn,hg,sizeof(hg),cudaMemcpyHostToDevice);\n\
         \x20 cudaMalloc(&dO,(size_t)HV*D*4); cudaMalloc(&dOut,(size_t)HV*D*4);\n\
         \x20 gdn_step<<<dim3(HV,(D+3)/4),128>>>(dq,dk,dv,db,da,dS,dO,D,NK);\n\
         \x20 gdn_gate_norm<<<HV,D>>>(dz,dgn,dO,dOut,D,1e-6f);\n\
         \x20 cudaDeviceSynchronize();\n\
         \x20 float* out=(float*)malloc((size_t)HV*D*4); cudaMemcpy(out,dOut,(size_t)HV*D*4,cudaMemcpyDeviceToHost);\n\
         \x20 float* st=(float*)malloc(sizeof(hs)); cudaMemcpy(st,dS,sizeof(hs),cudaMemcpyDeviceToHost);\n\
         \x20 for(int i=0;i<HV*D;++i) printf(\"%.9g\\n\", out[i]);\n\
         \x20 for(size_t i=0;i<sizeof(hs)/4;++i) printf(\"%.9g\\n\", st[i]);\n\
         \x20 return 0;\n}}\n",
        kern = GDN_KERNELS,
        aq = arr("hq", q), ak = arr("hk_", k), av = arr("hv_", v), az = arr("hz", z),
        ab = arr("hb", beta), aa = arr("ha", alpha), as_ = arr("hs", s0), ag = arr("hg", gn),
        d = d, hv = hv, hk = hk,
    )
}
