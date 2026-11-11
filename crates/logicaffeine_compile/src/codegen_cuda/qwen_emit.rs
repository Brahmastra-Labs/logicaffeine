//! The **whole-forward orchestrator** — composing a multi-stage LOGOS tensor-op program into ONE CUDA
//! program that chains genuine PARALLEL kernels through shared device buffers.
//!
//! The per-op emitters ([`super::matrix`], [`super::fused_row`]) each emit a *self-contained* `.cu` (its own
//! `main` that fills one input and prints a checksum). A transformer forward is a SEQUENCE of ops
//! (`rmsnorm → matmul → softmax → …`), so this module does what a single-op emitter cannot: it walks the
//! program's ordered tensor-op stages (each desugars to one `Stmt::Splice`, recognized independently via the
//! backend-agnostic [`crate::tensor_recognize`] recognizers), then emits — for each stage — a
//! uniquely-named `__global__` kernel plus a single host driver that allocates one device buffer per tensor
//! variable, fills the inputs deterministically, launches each stage's kernel in dataflow order (chaining by
//! variable name), and prints the final tensor. The kernels ARE composable — same bodies the standalone
//! emitters use, only re-parameterized (unique name, dims as args) so many coexist in one translation unit.
//!
//! This is the seam the "beat llama.cpp" path needs: the forward is authored in LOGOS tensor ops, and the
//! compiler emits one runnable device program of chained parallel kernels — not the single-thread whole-
//! program lowering.

use std::collections::HashMap;

use logicaffeine_language::ast::stmt::{Expr, Literal, Stmt};

use crate::intern::{Interner, Symbol};
use crate::model::quant::GgmlType;
use crate::tensor_graph::front::TensorOpSpec;
use crate::tensor_graph::fuse::{leaves, optimize};
use crate::tensor_graph::graph::{Fused, Op, Storage, Sym, TensorGraph};
use crate::tensor_graph::symbols::ClassId;
use crate::tensor_recognize::qwen;
use crate::tensor_recognize::{ElementwiseOp, ElementwiseSpec, FusedRowSpec, RowReduceKind};

use super::forward_shim::{kernel_refusal, ForwardShim};

/// A concrete element initializer value — integer or float.
#[derive(Debug, Clone, Copy)]
pub enum InitVal {
    I(i64),
    F(f64),
}

/// A **Q8_0 weight** loaded resident as int8 quants + f32 block scales (the real Qwen weights: 8 GB int8 fits
/// a 12 GB card where 32 GB of f32 would not). Dequant `W[o][t] = quants[o*in+t] * scales[o*nb + t/32]` — the
/// matmul against it becomes a dequant-in-kernel GEMV, matching `codegen_cuda::qwen`'s certified matvec.
#[derive(Debug, Clone)]
pub struct Q8Ref {
    pub quants: String,
    pub scales: String,
    pub in_dim: usize,
    pub out_dim: usize,
}

/// A **Mixture-of-Experts FFN block**'s weights, all int8-resident (Q8_0): the router `[E, d]`, and the E
/// experts' gate/up `[E·dff, d]` and down `[E·d, dff]` STACKED so expert `e`'s rows begin at `e·dff` (gate/up)
/// or `e·d` (down). This is the Qwen3-30B-A3B block; the SAME struct describes any `(d, dff, E, K)` — the
/// emitter indexes the experts by a base-pointer offset, so 8 experts and 128 experts share one code path.
#[derive(Debug, Clone)]
pub struct MoeWeights {
    pub router: Q8Ref,
    pub gate: Q8Ref,
    pub up: Q8Ref,
    pub down: Q8Ref,
    pub d: usize,
    pub dff: usize,
    pub ne: usize,
    pub k: usize,
}

/// Emit a self-contained CUDA program for ONE Mixture-of-Experts FFN block over real int8 weights: router GEMV
/// → top-K route (softmax, keep K, renormalize) → each expert's SwiGLU with an EXPERT-INDEXED weight base →
/// weighted combine. The weights stream from files (int8 quants + f32 block scales), never a 4×-larger f32
/// buffer, so the 30B block (~30 GB int8) fits a 48 GB card. Dequant-in-kernel GEMV (f32 activation × int8
/// weight, f64 accumulate) matches the LOGOS `moe` op on the VM exactly; the whole block is captured as one
/// CUDA graph. `x` is the block input (`d` floats, inlined). The experts are UNROLLED in the host launch list
/// (not the kernels), so E scales without changing a kernel — the same emitter serves 8B-scale and 30B-scale.
pub fn emit_moe_forward(x: &[f64], w: &MoeWeights) -> String {
    let (d, dff, ne, k) = (w.d, w.dff, w.ne, w.k);
    let (dnb, fnb) = (d / 32, dff / 32); // per-32-block counts for the two contraction dims
    assert!(
        ne <= 256,
        "moe emitter caps E at 256 (top-K scratch is a 256-float stack array)"
    );

    // ── kernels ──────────────────────────────────────────────────────────────────────────────────────
    // Dequant-in-kernel Q8 GEMV: y[o] = Σ_bl (Σ_j wq[o·K+bl·32+j]·a[bl·32+j]) · ws[o·nb+bl]. One output/thread,
    // f64 accumulate — the certified matvec, expert-selected by the base pointers the host passes.
    let kernels = format!(
        "__global__ void moe_gemv(const float* a, const signed char* wq, const float* ws, float* y, int N, int K){{\n\
         \x20 int o=blockIdx.x*blockDim.x+threadIdx.x; if(o>=N) return; int nb=K/32;\n\
         \x20 const signed char* wr=wq+(long)o*K; const float* wsr=ws+(long)o*nb; double acc=0.0;\n\
         \x20 for(int bl=0;bl<nb;++bl){{ double dot=0.0; for(int j=0;j<32;++j) dot+=(double)wr[bl*32+j]*(double)a[bl*32+j]; acc+=dot*(double)wsr[bl]; }}\n\
         \x20 y[o]=(float)acc;\n}}\n\
         __global__ void moe_silu_had(const float* g, const float* u, float* he, int n){{\n\
         \x20 int i=blockIdx.x*blockDim.x+threadIdx.x; if(i>=n) return; float gv=g[i]; he[i]=(gv/(1.0f+expf(-gv)))*u[i];\n}}\n\
         __global__ void moe_zero(float* y, int n){{ int i=blockIdx.x*blockDim.x+threadIdx.x; if(i<n) y[i]=0.0f; }}\n\
         __global__ void moe_combine(float* H, const float* oute, const float* wv, int e, int d){{\n\
         \x20 int o=blockIdx.x*blockDim.x+threadIdx.x; if(o<d) H[o]+=wv[e]*oute[o];\n}}\n\
         // top-K router: softmax over the E logits, keep the K largest (masked argmax), renormalize. w[e]=0 for\n\
         // the non-selected — the data-dependent routing decision, computed on-device.\n\
         __global__ void moe_topk(const float* logits, float* w, int E, int K){{\n\
         \x20 if(threadIdx.x!=0) return; float p[256]; float pm[256];\n\
         \x20 float m=logits[0]; for(int e=1;e<E;++e) if(logits[e]>m) m=logits[e];\n\
         \x20 float s=0.0f; for(int e=0;e<E;++e) s+=expf(logits[e]-m);\n\
         \x20 for(int e=0;e<E;++e){{ p[e]=expf(logits[e]-m)/s; pm[e]=p[e]; w[e]=0.0f; }}\n\
         \x20 float wsum=0.0f;\n\
         \x20 for(int r=0;r<K;++r){{ float bv=-1e30f; int bi=0; for(int e=0;e<E;++e) if(pm[e]>bv){{ bv=pm[e]; bi=e; }} wsum+=p[bi]; w[bi]=p[bi]; pm[bi]=-1e30f; }}\n\
         \x20 for(int e=0;e<E;++e) w[e]/=wsum;\n}}\n"
    );

    // ── host: load int8 weights, run the block on a stream, capture as one CUDA graph, print H ──────────
    let load_q8 = |var: &str, q: &Q8Ref| -> String {
        let nb = q.in_dim / 32;
        let (qn, sn) = (q.in_dim * q.out_dim, q.out_dim * nb);
        format!(
            "  signed char* {var}_q; if(cudaMallocManaged(&{var}_q,(size_t){qn})){{ fprintf(stderr,\"malloc\\n\"); return 1; }}\n\
             \x20 float* {var}_s; if(cudaMallocManaged(&{var}_s,(size_t){sn}*sizeof(float))){{ fprintf(stderr,\"malloc\\n\"); return 1; }}\n\
             \x20 {{ FILE* f=fopen(\"{qp}\",\"rb\"); if(!f||fread({var}_q,1,(size_t){qn},f)!=(size_t){qn}){{ fprintf(stderr,\"q {var}\\n\"); return 1; }} fclose(f); }}\n\
             \x20 {{ FILE* f=fopen(\"{sp}\",\"rb\"); if(!f||fread({var}_s,sizeof(float),(size_t){sn},f)!=(size_t){sn}){{ fprintf(stderr,\"s {var}\\n\"); return 1; }} fclose(f); }}\n",
            qp = q.quants, sp = q.scales
        )
    };
    let mut allocs = String::new();
    allocs.push_str(&load_q8("Wr", &w.router));
    allocs.push_str(&load_q8("Wg", &w.gate));
    allocs.push_str(&load_q8("Wu", &w.up));
    allocs.push_str(&load_q8("Wd", &w.down));
    // activation + scratch buffers
    for (nm, n) in [
        ("x", d),
        ("logits", ne),
        ("w", ne),
        ("gate_e", dff),
        ("up_e", dff),
        ("he", dff),
        ("oute", d),
        ("H", d),
    ] {
        allocs.push_str(&format!("  float* {nm}; if(cudaMallocManaged(&{nm},(size_t){n}*sizeof(float))){{ fprintf(stderr,\"malloc\\n\"); return 1; }}\n"));
    }
    let mut fills = String::new();
    for (i, v) in x.iter().enumerate() {
        fills.push_str(&format!("  x[{i}]={:?}f;\n", v));
    }

    // The block: router → topk → per-expert SwiGLU (expert-indexed weight bases) → weighted combine.
    let etopk = ne.min(1024);
    let mut body = String::new();
    body.push_str(&format!(
        "  moe_gemv<<<({ne}+255)/256,256,0,_st>>>(x,Wr_q,Wr_s,logits,{ne},{d});\n"
    ));
    body.push_str(&format!(
        "  moe_topk<<<1,{etopk},0,_st>>>(logits,w,{ne},{k});\n"
    ));
    body.push_str(&format!(
        "  moe_zero<<<({d}+255)/256,256,0,_st>>>(H,{d});\n"
    ));
    for e in 0..ne {
        let (goff, gsoff) = (e * dff * d, e * dff * dnb);
        let (doff, dsoff) = (e * d * dff, e * d * fnb);
        body.push_str(&format!(
            "  moe_gemv<<<({dff}+255)/256,256,0,_st>>>(x,Wg_q+{goff},Wg_s+{gsoff},gate_e,{dff},{d});\n\
             \x20 moe_gemv<<<({dff}+255)/256,256,0,_st>>>(x,Wu_q+{goff},Wu_s+{gsoff},up_e,{dff},{d});\n\
             \x20 moe_silu_had<<<({dff}+255)/256,256,0,_st>>>(gate_e,up_e,he,{dff});\n\
             \x20 moe_gemv<<<({d}+255)/256,256,0,_st>>>(he,Wd_q+{doff},Wd_s+{dsoff},oute,{d},{dff});\n\
             \x20 moe_combine<<<({d}+255)/256,256,0,_st>>>(H,oute,w,{e},{d});\n"
        ));
    }

    format!(
        "// Auto-generated by LOGOS codegen_cuda::qwen_emit — a Mixture-of-Experts FFN block (E={ne}, top-{k},\n\
         // d={d}, dff={dff}) over real int8 weights, captured as one CUDA graph. The SAME emitter serves any E.\n\
         #include <cstdio>\n#include <cstdlib>\n#include <cuda_runtime.h>\n#include <cmath>\n\n\
         {kernels}\nint main(void){{\n{allocs}{fills}\n\
         \x20 cudaStream_t _st; if(cudaStreamCreate(&_st)!=cudaSuccess){{ fprintf(stderr,\"stream\\n\"); return 1; }}\n\
         {body}  if(cudaStreamSynchronize(_st)!=cudaSuccess){{ fprintf(stderr,\"sync\\n\"); return 1; }}\n\
         \x20 cudaGraph_t _g; cudaGraphExec_t _ge;\n\
         \x20 if(cudaStreamBeginCapture(_st, cudaStreamCaptureModeGlobal)!=cudaSuccess){{ fprintf(stderr,\"capture\\n\"); return 1; }}\n\
         {body}  if(cudaStreamEndCapture(_st,&_g)!=cudaSuccess){{ fprintf(stderr,\"endcap\\n\"); return 1; }}\n\
         \x20 if(cudaGraphInstantiate(&_ge,_g,0)!=cudaSuccess){{ fprintf(stderr,\"inst\\n\"); return 1; }}\n\
         \x20 cudaGraphLaunch(_ge,_st); cudaStreamSynchronize(_st);\n\
         \x20 cudaEvent_t _e0,_e1; cudaEventCreate(&_e0); cudaEventCreate(&_e1);\n\
         \x20 cudaEventRecord(_e0,_st); for(int _it=0;_it<20;++_it){{ cudaGraphLaunch(_ge,_st); }}\n\
         \x20 cudaEventRecord(_e1,_st); cudaStreamSynchronize(_st); float _ms=0.0f; cudaEventElapsedTime(&_ms,_e0,_e1);\n\
         \x20 fprintf(stderr,\"MOE_MS %f\\n\", _ms/20.0f);\n\
         \x20 for(int o=0;o<{d};++o) printf(\"%.6f\\n\",(double)H[o]);\n  return 0;\n}}\n"
    )
}

pub use crate::tensor_recognize::qwen::{recognize_moe, MoeSpec};

/// Collect the concrete initializers a program writes — `Set item N of V to <literal>` with literal `N` and an
/// integer/float value (1-based `N`, recorded 0-based) — so the orchestrator's INPUT buffers hold the
/// program's own values and its GPU output equals the VM's, not merely a deterministic fill. Recurses through
/// desugar `Splice`s.
pub fn collect_inits(
    stmts: &[Stmt],
    interner: &Interner,
) -> HashMap<String, Vec<(usize, InitVal)>> {
    fn lit_int(e: &Expr) -> Option<i64> {
        match e {
            Expr::Literal(Literal::Number(n)) => Some(*n),
            _ => None,
        }
    }
    fn lit_val(e: &Expr) -> Option<InitVal> {
        match e {
            Expr::Literal(Literal::Number(n)) => Some(InitVal::I(*n)),
            Expr::Literal(Literal::Float(f)) => Some(InitVal::F(*f)),
            _ => None,
        }
    }
    fn walk(stmts: &[Stmt], interner: &Interner, out: &mut HashMap<String, Vec<(usize, InitVal)>>) {
        for s in stmts {
            match s {
                Stmt::SetIndex {
                    collection,
                    index,
                    value,
                } => {
                    let (coll, idx, val): (&Expr, &Expr, &Expr) = (*collection, *index, *value);
                    if let (Expr::Identifier(c), Some(k), Some(v)) =
                        (coll, lit_int(idx), lit_val(val))
                    {
                        if k >= 1 {
                            out.entry(interner.resolve(*c).to_string())
                                .or_default()
                                .push((k as usize - 1, v));
                        }
                    }
                }
                Stmt::Splice { body } => walk(body, interner, out),
                Stmt::While { body, .. } | Stmt::Repeat { body, .. } => walk(body, interner, out),
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    walk(then_block, interner, out);
                    if let Some(eb) = else_block {
                        walk(eb, interner, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = HashMap::new();
    walk(stmts, interner, &mut out);
    out
}

/// Why a program is not a lowerable multi-stage tensor-op forward.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardError {
    /// No tensor-op stage was recognized in the program.
    NoStages,
    /// A `moe` stage produced this output var but the MoE weight manifest has no entry for it.
    MissingMoeWeights(String),
    /// Statements the forward cannot lower faithfully, in program order. A program is lowered whole or not at all.
    Refused(Vec<StageRefusal>),
}

impl std::fmt::Display for ForwardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ForwardError::NoStages => write!(f, "no tensor-op stage recognized in the program"),
            ForwardError::MissingMoeWeights(v) => {
                write!(f, "moe stage `{v}` has no entry in the MoE weight manifest")
            }
            ForwardError::Refused(rs) => {
                write!(f, "the CUDA forward refuses {} statement(s): ", rs.len())?;
                for (k, r) in rs.iter().enumerate() {
                    if k > 0 {
                        write!(f, "; ")?;
                    }
                    write!(f, "{r}")?;
                }
                Ok(())
            }
        }
    }
}

/// Why the forward refuses one statement — the statement by its top-level index, and what it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageRefusal {
    /// A compound statement no tensor recognizer lifts (a hand-written loop, a branch, an unknown block), named by
    /// its binder or loop variable.
    Unrecognized { stmt: usize, what: String },
    /// A write to a declared tensor that no stage computes and no starting fill can hold.
    UnliftedWrite {
        stmt: usize,
        tensor: String,
        why: &'static str,
    },
    /// An op the tensor front end lifts whose meaning this lane's kernels do not compute — no kernel for the
    /// op, or a form of it (a field of its spec) the kernel does not implement.
    NoKernel {
        stmt: usize,
        kind: &'static str,
        out: String,
        why: &'static str,
    },
}

impl std::fmt::Display for StageRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StageRefusal::Unrecognized { stmt, what } => {
                write!(f, "statement {stmt} ({what}) is no tensor op the front end lifts")
            }
            StageRefusal::UnliftedWrite { stmt, tensor, why } => {
                write!(f, "statement {stmt} writes `{tensor}`: {why}")
            }
            StageRefusal::NoKernel {
                stmt,
                kind,
                out,
                why,
            } => write!(
                f,
                "statement {stmt} binds `{out}` with {kind}, which the CUDA forward cannot lower: {why}"
            ),
        }
    }
}

/// The SPARSE INT8-ACTIVATION DP4A MoE kernels (the llama.cpp-beating path): quantize the activation to int8
/// once (`moe_qact`), route to the top-K experts by INDEX (`moe_topk_idx` — softmax → keep K → renormalize,
/// emitting the chosen expert indices, not a dense mask), then run ONLY the selected experts through the two
/// deeply-fused kernels (`moe_gate_up_silu`, `moe_down_combine` — one warp per output row, each lane streams a
/// 128-byte-coalesced int of 4 packed int8 of BOTH operands, `__dp4a`-accumulates, warp-reduces; the expert
/// index is read on-device so the weight base is data-dependent yet the launch is static ⇒ CUDA-graph-
/// capturable). This reads ~K/E of the weights per token (the "A3B = 3B active" sparsity) at full coalesced
/// int8 bandwidth — the two wins over the dense dequant path. The router + top-K pair is precision-independent;
/// [`moe_dp4a_kernels`] appends the single-pass (default, llama-matched) or two-pass (logit-exact) trio.
const MOE_ROUTER_TOPK_KERNELS: &str = "\
// FAST router GEMV (warp per output row, `char4`-coalesced int8 × f32 activation, f32 accumulate) — replaces the
// dense f64 thread-per-output `moe_gemv` that ran the WHOLE 128-expert routing on ONE block/SM (the batch-1\n\
// bottleneck: 2000× slower than bandwidth). f32 vs f64 does not change which top-K experts route ⇒ token-exact.\n\
__global__ void moe_router_fast(const float* a, const signed char* wq, const float* ws, float* y, int N, int K){\n\
\x20 int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31; int nb=K/32, nq=K/128;\n\
\x20 const signed char* wr=wq+(long)o*K; const float* wsr=ws+(long)o*nb; const char4* w4=(const char4*)wr;\n\
\x20 float acc=0.0f;\n\
\x20 for(int b=0;b<nq;++b){ char4 w=w4[b*32+lane]; int blk=b*4+lane/8; const float* at=a+b*128+lane*4;\n\
\x20   acc += wsr[blk]*((float)w.x*at[0]+(float)w.y*at[1]+(float)w.z*at[2]+(float)w.w*at[3]); }\n\
\x20 for(int t=nq*128+lane;t<K;t+=32){ int blk=t/32; acc += wsr[blk]*(float)wr[t]*a[t]; }\n\
\x20 for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s);\n\
\x20 if(lane==0) y[o]=acc;\n}\n\
// Top-K router select. The 128-expert softmax (max + exp + sum) is done BLOCK-PARALLEL into shared `sp` (each\n\
// global logit read + expf once, not serialized in one thread — the old single-thread form was 30us/call, the\n\
// #2 batch-1 hog); only the tiny top-K argmax scan stays on thread 0, now over fast shared memory. E<=1024.\n\
__global__ void moe_topk_idx(const float* logits, int* idx, float* w, int E, int K){\n\
\x20 __shared__ float sp[1024]; __shared__ float red[32];\n\
\x20 int t=threadIdx.x, nt=blockDim.x, wp=t>>5, ln=t&31, nw=(nt+31)>>5;\n\
\x20 float lm=-1e30f; for(int e=t;e<E;e+=nt) lm=fmaxf(lm,logits[e]);\n\
\x20 for(int s=16;s>0;s>>=1) lm=fmaxf(lm,__shfl_down_sync(0xffffffffu,lm,s));\n\
\x20 if(ln==0) red[wp]=lm; __syncthreads();\n\
\x20 if(t==0){ float m=red[0]; for(int i=1;i<nw;++i) m=fmaxf(m,red[i]); red[0]=m; } __syncthreads();\n\
\x20 float mx=red[0];\n\
\x20 float ls=0.0f; for(int e=t;e<E;e+=nt){ float p=expf(logits[e]-mx); sp[e]=p; ls+=p; }\n\
\x20 for(int s=16;s>0;s>>=1) ls+=__shfl_down_sync(0xffffffffu,ls,s);\n\
\x20 if(ln==0) red[wp]=ls; __syncthreads();\n\
\x20 if(t==0){ float s=0.0f; for(int i=0;i<nw;++i) s+=red[i]; red[0]=s; } __syncthreads();\n\
\x20 float sum=red[0];\n\
\x20 if(t==0){ float wsum=0.0f;\n\
\x20   for(int r=0;r<K;++r){ float bv=-1e30f; int bi=0; for(int e=0;e<E;++e) if(sp[e]>bv){ bv=sp[e]; bi=e; } idx[r]=bi; float pv=sp[bi]/sum; w[r]=pv; wsum+=pv; sp[bi]=-1e30f; }\n\
\x20   for(int r=0;r<K;++r) w[r]/=wsum; }\n}\n";

/// The TWO-PASS (logit-exact) fused trio — see [`Q8MatKind::TwoPassInt8`].
const MOE_TWO_PASS_KERNELS: &str = "\
// DEEPEST FUSION + TWO-PASS INT8-ACTIVATION dp4a (batch-1 latency-optimal AND ~14-bit exact). Two hard barriers
// (router, h-complete); everything between fuses. gate+up+SwiGLU collapse into ONE kernel, down+combine into
// another; only h and H touch HBM. The shared activation is quantized ONCE per MoE stage into a hi byte + a lo
// residual byte (moe_qact); both fused kernels run TWO int8×int8 `__dp4a` per block (hi and lo) — the SHORT
// dependency chain that wins the latency-bound regime, but with ~14-bit activation so a near-tie token does NOT
// flip (single int8 flips it — like llama.cpp). Weights read once ⇒ memory cost unchanged. sl=sh/128. Scalar
// tails cover Kd%128. N=dff/d, Kd=d/dff.\n\
__global__ void moe_qact(const float* a, signed char* aqh, signed char* aql, float* as_, int K){\n\
\x20 int nb=K/32; int blk=blockIdx.x*(blockDim.x/32)+threadIdx.x/32; if(blk>=nb) return;\n\
\x20 int lane=threadIdx.x&31; float v=a[blk*32+lane], av=fabsf(v);\n\
\x20 for(int s=16;s>0;s>>=1) av=fmaxf(av,__shfl_down_sync(0xffffffffu,av,s));\n\
\x20 av=__shfl_sync(0xffffffffu,av,0); float amax=av>1e-30f?av:1e-30f; float sh=amax/127.0f;\n\
\x20 if(lane==0) as_[blk]=sh;\n\
\x20 int qh=(int)rintf(v/sh); if(qh>127)qh=127; if(qh<-128)qh=-128; aqh[blk*32+lane]=(signed char)qh;\n\
\x20 float r=v-qh*sh; float sl=sh*0.0078125f; int ql=(int)rintf(r/sl); if(ql>127)ql=127; if(ql<-128)ql=-128; aql[blk*32+lane]=(signed char)ql;\n}\n\
__global__ void moe_gate_up_silu(const signed char* xqh, const signed char* xql, const float* xs, const signed char* wg_q, const float* wg_s, const signed char* wu_q, const float* wu_s, const int* idx, float* h, int K, int N, int Kd){\n\
\x20 long row=(blockIdx.x*(long)blockDim.x+threadIdx.x)>>5; if(row>=(long)K*N) return; int lane=threadIdx.x&31;\n\
\x20 int kk=row/N, j=row%N; int e=idx[kk]; int nb=Kd/32, nq=Kd/128;\n\
\x20 const signed char* gwr=wg_q+((long)e*N+j)*Kd; const int* gw4=(const int*)gwr; const float* gsr=wg_s+((long)e*N+j)*nb;\n\
\x20 const signed char* uwr=wu_q+((long)e*N+j)*Kd; const int* uw4=(const int*)uwr; const float* usr=wu_s+((long)e*N+j)*nb;\n\
\x20 const signed char* xhb=xqh; const int* xh4=(const int*)xqh; const signed char* xlb=xql; const int* xl4=(const int*)xql;\n\
\x20 float ga=0.0f, ua=0.0f;\n\
\x20 for(int b=0;b<nq;++b){ int blk=b*4+lane/8; int aih=xh4[b*32+lane]; int ail=xl4[b*32+lane]; float sh=xs[blk]; float sl=sh*0.0078125f; int gwi=gw4[b*32+lane]; int uwi=uw4[b*32+lane];\n\
\x20   ga+=gsr[blk]*(sh*(float)__dp4a(gwi,aih,0)+sl*(float)__dp4a(gwi,ail,0));\n\
\x20   ua+=usr[blk]*(sh*(float)__dp4a(uwi,aih,0)+sl*(float)__dp4a(uwi,ail,0)); }\n\
\x20 for(int t=nq*128+lane;t<Kd;t+=32){ int blk=t/32; float sh=xs[blk]; float sl=sh*0.0078125f; int gwj=(int)gwr[t]; int uwj=(int)uwr[t]; int xh=(int)xhb[t]; int xl=(int)xlb[t]; ga+=gsr[blk]*(sh*(float)(gwj*xh)+sl*(float)(gwj*xl)); ua+=usr[blk]*(sh*(float)(uwj*xh)+sl*(float)(uwj*xl)); }\n\
\x20 for(int s=16;s>0;s>>=1){ ga+=__shfl_down_sync(0xffffffffu,ga,s); ua+=__shfl_down_sync(0xffffffffu,ua,s); }\n\
\x20 if(lane==0) h[row]=(ga/(1.0f+expf(-ga)))*ua;\n}\n\
__global__ void moe_down_combine(const signed char* hqh, const signed char* hql, const float* hs, const signed char* wd_q, const float* wd_s, const int* idx, const float* wc, float* H, int K, int N, int Kd){\n\
\x20 int o=blockIdx.x; int warp=threadIdx.x>>5, lane=threadIdx.x&31;\n\
\x20 int e=idx[warp]; int nb=Kd/32, nq=Kd/128;\n\
\x20 const signed char* hhb=hqh+(long)warp*Kd; const int* hh4=(const int*)hhb; const signed char* hlb=hql+(long)warp*Kd; const int* hl4=(const int*)hlb; const float* hsr=hs+(long)warp*nb;\n\
\x20 const signed char* wr=wd_q+((long)e*N+o)*Kd; const int* w4=(const int*)wr; const float* wsr=wd_s+((long)e*N+o)*nb;\n\
\x20 float acc=0.0f;\n\
\x20 for(int b=0;b<nq;++b){ int blk=b*4+lane/8; int wi=w4[b*32+lane]; float sh=hsr[blk]; float sl=sh*0.0078125f; acc+=wsr[blk]*(sh*(float)__dp4a(wi,hh4[b*32+lane],0)+sl*(float)__dp4a(wi,hl4[b*32+lane],0)); }\n\
\x20 for(int t=nq*128+lane;t<Kd;t+=32){ int blk=t/32; float sh=hsr[blk]; float sl=sh*0.0078125f; int wj=(int)wr[t]; acc+=wsr[blk]*(sh*(float)(wj*(int)hhb[t])+sl*(float)(wj*(int)hlb[t])); }\n\
\x20 for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s);\n\
\x20 __shared__ float part[64];\n\
\x20 if(lane==0) part[warp]=wc[warp]*acc;\n\
\x20 __syncthreads();\n\
\x20 if(threadIdx.x==0){ float sum=0.0f; for(int kk=0;kk<K;++kk) sum+=part[kk]; H[o]=sum; }\n}\n";

/// The SINGLE-PASS (default, llama-matched precision) fused trio — see [`Q8MatKind::Int8Act`]. Identical
/// fusion shape to the two-pass trio with the lo-residual pass deleted: ONE int8 quantization per activation
/// (`moe_qact`) and ONE `__dp4a` per operand per 32-block — the shortest dependency chain, the fastest path
/// (the A100 head-to-head beat over llama.cpp at matched precision).
const MOE_SINGLE_PASS_KERNELS: &str = "\
// DEEPEST FUSION + SINGLE-PASS INT8-ACTIVATION dp4a (batch-1 latency-optimal, llama.cpp-matched precision).
// Two hard barriers (router, h-complete); everything between fuses. gate+up+SwiGLU collapse into ONE kernel,\n\
// down+combine into another; only h and H touch HBM. The shared activation is quantized ONCE per MoE stage\n\
// (moe_qact); both fused kernels run ONE int8×int8 `__dp4a` per block — the SHORT dependency chain that wins\n\
// the latency-bound regime, at llama.cpp's activation precision (a near-tie logit can flip exactly as its\n\
// does; the TwoPassInt8 mode holds it). Weights read once ⇒ memory cost unchanged. Scalar tails cover Kd%128.\n\
__global__ void moe_qact(const float* a, signed char* aq, float* as_, int K){\n\
\x20 int nb=K/32; int blk=blockIdx.x*(blockDim.x/32)+threadIdx.x/32; if(blk>=nb) return;\n\
\x20 int lane=threadIdx.x&31; float v=a[blk*32+lane], av=fabsf(v);\n\
\x20 for(int s=16;s>0;s>>=1) av=fmaxf(av,__shfl_down_sync(0xffffffffu,av,s));\n\
\x20 av=__shfl_sync(0xffffffffu,av,0); float amax=av>1e-30f?av:1e-30f; float sc=amax/127.0f;\n\
\x20 if(lane==0) as_[blk]=sc;\n\
\x20 int q=(int)rintf(v/sc); if(q>127)q=127; if(q<-128)q=-128; aq[blk*32+lane]=(signed char)q;\n}\n\
__global__ void moe_gate_up_silu(const signed char* xq, const float* xs, const signed char* wg_q, const float* wg_s, const signed char* wu_q, const float* wu_s, const int* idx, float* h, int K, int N, int Kd){\n\
\x20 long row=(blockIdx.x*(long)blockDim.x+threadIdx.x)>>5; if(row>=(long)K*N) return; int lane=threadIdx.x&31;\n\
\x20 int kk=row/N, j=row%N; int e=idx[kk]; int nb=Kd/32, nq=Kd/128;\n\
\x20 const signed char* gwr=wg_q+((long)e*N+j)*Kd; const int* gw4=(const int*)gwr; const float* gsr=wg_s+((long)e*N+j)*nb;\n\
\x20 const signed char* uwr=wu_q+((long)e*N+j)*Kd; const int* uw4=(const int*)uwr; const float* usr=wu_s+((long)e*N+j)*nb;\n\
\x20 const signed char* xb=xq; const int* x4=(const int*)xq;\n\
\x20 float ga=0.0f, ua=0.0f; int nq16=Kd/512;\n\
\x20 const int4* gw16=(const int4*)gwr; const int4* uw16=(const int4*)uwr; const int4* x16=(const int4*)xq;\n\
\x20 for(int b=0;b<nq16;++b){ int blk=b*16+lane/2; float sc=xs[blk]; int4 av=x16[b*32+lane]; int4 gv=gw16[b*32+lane]; int4 uv=uw16[b*32+lane];\n\
\x20   int gd=__dp4a(gv.x,av.x,0); gd=__dp4a(gv.y,av.y,gd); gd=__dp4a(gv.z,av.z,gd); gd=__dp4a(gv.w,av.w,gd);\n\
\x20   int ud=__dp4a(uv.x,av.x,0); ud=__dp4a(uv.y,av.y,ud); ud=__dp4a(uv.z,av.z,ud); ud=__dp4a(uv.w,av.w,ud);\n\
\x20   ga+=gsr[blk]*sc*(float)gd; ua+=usr[blk]*sc*(float)ud; }\n\
\x20 for(int b4=nq16*4;b4<nq;++b4){ int blk=b4*4+lane/8; int ai=x4[b4*32+lane]; float sc=xs[blk]; int gwi=gw4[b4*32+lane]; int uwi=uw4[b4*32+lane]; ga+=gsr[blk]*sc*(float)__dp4a(gwi,ai,0); ua+=usr[blk]*sc*(float)__dp4a(uwi,ai,0); }\n\
\x20 for(int t=nq*128+lane;t<Kd;t+=32){ int blk=t/32; float sc=xs[blk]; int gwj=(int)gwr[t]; int uwj=(int)uwr[t]; int xv=(int)xb[t]; ga+=gsr[blk]*sc*(float)(gwj*xv); ua+=usr[blk]*sc*(float)(uwj*xv); }\n\
\x20 for(int s=16;s>0;s>>=1){ ga+=__shfl_down_sync(0xffffffffu,ga,s); ua+=__shfl_down_sync(0xffffffffu,ua,s); }\n\
\x20 if(lane==0) h[row]=(ga/(1.0f+expf(-ga)))*ua;\n}\n\
__global__ void moe_down_combine(const signed char* hq, const float* hs, const signed char* wd_q, const float* wd_s, const int* idx, const float* wc, float* H, int K, int N, int Kd){\n\
\x20 int o=blockIdx.x; int warp=threadIdx.x>>5, lane=threadIdx.x&31;\n\
\x20 int e=idx[warp]; int nb=Kd/32, nq=Kd/128;\n\
\x20 const signed char* hb=hq+(long)warp*Kd; const int* h4=(const int*)hb; const float* hsr=hs+(long)warp*nb;\n\
\x20 const signed char* wr=wd_q+((long)e*N+o)*Kd; const int* w4=(const int*)wr; const float* wsr=wd_s+((long)e*N+o)*nb;\n\
\x20 float acc=0.0f; int nq16=Kd/512;\n\
\x20 const int4* w16=(const int4*)wr; const int4* h16=(const int4*)hb;\n\
\x20 for(int b=0;b<nq16;++b){ int blk=b*16+lane/2; int4 wv=w16[b*32+lane]; int4 hv=h16[b*32+lane];\n\
\x20   int d=__dp4a(wv.x,hv.x,0); d=__dp4a(wv.y,hv.y,d); d=__dp4a(wv.z,hv.z,d); d=__dp4a(wv.w,hv.w,d);\n\
\x20   acc+=wsr[blk]*hsr[blk]*(float)d; }\n\
\x20 for(int b4=nq16*4;b4<nq;++b4){ int blk=b4*4+lane/8; int wi=w4[b4*32+lane]; acc+=wsr[blk]*hsr[blk]*(float)__dp4a(wi,h4[b4*32+lane],0); }\n\
\x20 for(int t=nq*128+lane;t<Kd;t+=32){ int blk=t/32; int wj=(int)wr[t]; acc+=wsr[blk]*hsr[blk]*(float)(wj*(int)hb[t]); }\n\
\x20 for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s);\n\
\x20 __shared__ float part[64];\n\
\x20 if(lane==0) part[warp]=wc[warp]*acc;\n\
\x20 __syncthreads();\n\
\x20 if(threadIdx.x==0){ float sum=0.0f; for(int kk=0;kk<K;++kk) sum+=part[kk]; H[o]=sum; }\n}\n";

/// The sparse fused MoE kernel set for a coalesced int8 path: the precision-independent router + top-K pair,
/// then the single-pass (default) or two-pass (logit-exact) quantize/gate-up-SwiGLU/down-combine trio.
/// Public so a checked-in copy of the kernel set can be pinned byte-equal to what the compiler emits today.
pub fn moe_dp4a_kernels(two_pass: bool) -> String {
    let mut s = String::from(MOE_ROUTER_TOPK_KERNELS);
    s.push_str(if two_pass {
        MOE_TWO_PASS_KERNELS
    } else {
        MOE_SINGLE_PASS_KERNELS
    });
    s
}

/// The shared kernels a MoE stage launches (emitted once when any `Stage::Moe` is present): a dequant-in-kernel
/// Q8 GEMV (f64 accumulate — the certified matvec, expert-selected by the base pointers the host passes), the
/// SwiGLU element op, a zero, a top-K router (softmax → keep K → renormalize, on-device), and a weighted
/// combine. Identical to `emit_moe_forward`'s block, re-parameterized for the chained-buffer host.
const MOE_KERNELS: &str = "\
__global__ void moe_gemv(const float* a, const signed char* wq, const float* ws, float* y, int N, int K){\n\
\x20 int o=blockIdx.x*blockDim.x+threadIdx.x; if(o>=N) return; int nb=K/32;\n\
\x20 const signed char* wr=wq+(long)o*K; const float* wsr=ws+(long)o*nb; double acc=0.0;\n\
\x20 for(int bl=0;bl<nb;++bl){ double dot=0.0; for(int j=0;j<32;++j) dot+=(double)wr[bl*32+j]*(double)a[bl*32+j]; acc+=dot*(double)wsr[bl]; }\n\
\x20 y[o]=(float)acc;\n}\n\
__global__ void moe_silu_had(const float* g, const float* u, float* he, int n){\n\
\x20 int i=blockIdx.x*blockDim.x+threadIdx.x; if(i>=n) return; float gv=g[i]; he[i]=(gv/(1.0f+expf(-gv)))*u[i];\n}\n\
__global__ void moe_zero(float* y, int n){ int i=blockIdx.x*blockDim.x+threadIdx.x; if(i<n) y[i]=0.0f; }\n\
__global__ void moe_combine(float* H, const float* oute, const float* wv, int e, int d){\n\
\x20 int o=blockIdx.x*blockDim.x+threadIdx.x; if(o<d) H[o]+=wv[e]*oute[o];\n}\n\
__global__ void moe_topk(const float* logits, float* w, int E, int K){\n\
\x20 if(threadIdx.x!=0) return; float p[256]; float pm[256];\n\
\x20 float m=logits[0]; for(int e=1;e<E;++e) if(logits[e]>m) m=logits[e];\n\
\x20 float s=0.0f; for(int e=0;e<E;++e) s+=expf(logits[e]-m);\n\
\x20 for(int e=0;e<E;++e){ p[e]=expf(logits[e]-m)/s; pm[e]=p[e]; w[e]=0.0f; }\n\
\x20 float wsum=0.0f;\n\
\x20 for(int r=0;r<K;++r){ float bv=-1e30f; int bi=0; for(int e=0;e<E;++e) if(pm[e]>bv){ bv=pm[e]; bi=e; } wsum+=p[bi]; w[bi]=p[bi]; pm[bi]=-1e30f; }\n\
\x20 for(int e=0;e<E;++e) w[e]/=wsum;\n}\n";

/// The single-query GQA `attention` block as this lane's tuple: `(q, kc, vc, out, h, kv, hd, l)` — the shared
/// [`qwen::AttentionSpec`] without its position bound (this lane attends to every cached row).
#[allow(clippy::type_complexity)]
pub fn recognize_attention(
    stmts: &[Stmt],
) -> Option<(Symbol, Symbol, Symbol, Symbol, usize, usize, usize, usize)> {
    qwen::recognize_attention(stmts).map(|a| (a.q, a.kc, a.vc, a.out, a.h, a.kv, a.hd, a.l))
}

/// The forward of a program as the shared table fuses it under the forward's shim ([`ForwardShim`]): every statement
/// the front end lifts ([`crate::spatial::passes::recognize_stages::walk`]), or the program refused whole, each refusal
/// naming its statement. The front end decides what a statement IS (an op, a declaration, a fill, a write no op
/// computes); [`kernel_refusal`] decides whether this lane's kernels compute it. `q8` and `moe` name the weights it
/// reads in Q8_0.
pub fn forward_graph(
    stmts: &[Stmt],
    interner: &Interner,
    q8: &HashMap<String, Q8Ref>,
    moe: &HashMap<String, MoeWeights>,
    kind: Q8MatKind,
) -> Result<TensorGraph, ForwardError> {
    forward_graph_holding(stmts, interner, q8, moe, kind, &[])
}

/// [`forward_graph`] with the optimizer's steps `held` held back: the program run apart where they would fuse.
pub fn forward_graph_holding(
    stmts: &[Stmt],
    interner: &Interner,
    q8: &HashMap<String, Q8Ref>,
    moe: &HashMap<String, MoeWeights>,
    kind: Q8MatKind,
    held: &[&str],
) -> Result<TensorGraph, ForwardError> {
    let front = crate::spatial::passes::recognize_stages::walk(stmts, interner);
    let mut refusals: Vec<StageRefusal> = front
        .unrecognized
        .iter()
        .map(|(stmt, what)| StageRefusal::Unrecognized {
            stmt: *stmt,
            what: what.clone(),
        })
        .chain(
            front
                .unlifted_writes
                .iter()
                .map(|(stmt, tensor, why)| StageRefusal::UnliftedWrite {
                    stmt: *stmt,
                    tensor: tensor.clone(),
                    why,
                }),
        )
        .collect();
    for st in &front.stages {
        if let Some(why) = kernel_refusal(&st.op) {
            refusals.push(StageRefusal::NoKernel {
                stmt: st.stmt_index,
                kind: st.op.kind(),
                out: st.out.clone(),
                why,
            });
        }
    }
    if !refusals.is_empty() {
        refusals.sort_by_key(StageRefusal::stmt);
        return Err(ForwardError::Refused(refusals));
    }
    if front.stages.is_empty() {
        return Err(ForwardError::NoStages);
    }
    let mut stored: std::collections::BTreeSet<&str> = q8.keys().map(String::as_str).collect();
    for st in &front.stages {
        if matches!(st.op, TensorOpSpec::Moe(_)) && moe.contains_key(&st.out) {
            stored.extend(st.ins[1..5].iter().map(String::as_str));
        }
    }
    let storage = |t: &str| {
        if stored.contains(t) {
            Storage::Rows { format: GgmlType::Q8_0, transposed: true }
        } else {
            Storage::Values
        }
    };
    let lifted = TensorGraph::lift_stages(&front, None, None, &storage);
    Ok(optimize(&lifted, &ForwardShim { kind }, held).graph)
}

impl StageRefusal {
    /// The top-level statement the refusal names.
    pub fn stmt(&self) -> usize {
        match self {
            StageRefusal::Unrecognized { stmt, .. }
            | StageRefusal::UnliftedWrite { stmt, .. }
            | StageRefusal::NoKernel { stmt, .. } => *stmt,
        }
    }
}

/// One step of a fused elementwise chain applied to the register-carried value `t`: a binary op against an
/// external operand (`t = t ⊕ ext[idx]`), or a unary SiLU (`t = silu(t)`, no operand).
#[derive(Debug, Clone)]
pub enum ChainStep {
    Binary(ElementwiseOp, Symbol),
    Silu,
}

/// The op a quantize folded into it computes, and whether it quantizes its result in its epilogue.
fn folded(op: &Op) -> (&Op, bool) {
    match &op.sym {
        Sym::Fused(f) if matches!(f.class, ClassId::NormQuant | ClassId::AddNormQuant | ClassId::AttnOutputQuant) => (&f.parts[0], true),
        _ => (op, false),
    }
}

/// The kernels the forward writes for its graph, in order, each with the stage number its kernels are named by, and the
/// count of stages: every op but a quantize, which the product reading it launches. A sum and the norm it adds into keep
/// the two numbers their two statements have.
fn kernels_of(graph: &TensorGraph) -> (Vec<(usize, &Op)>, usize) {
    let mut at = 0;
    let mut out = Vec::new();
    for op in &graph.ops {
        if matches!(op.sym, Sym::Quantize(_)) {
            continue;
        }
        out.push((at, op));
        at += match &folded(op).0.sym {
            Sym::Fused(Fused { class: ClassId::AddNorm, .. }) => 2,
            _ => 1,
        };
    }
    (out, at)
}

/// An elementwise op's spec, and the RMS norm it adds into when the op is a sum and its norm.
fn sum_and_norm(op: &Op) -> (&ElementwiseSpec, Option<&FusedRowSpec>) {
    match (&op.sym, op.prim()) {
        (Sym::Fused(f), _) => match (f.parts[0].prim(), f.parts[1].prim()) {
            (Some(TensorOpSpec::Elementwise(e)), Some(TensorOpSpec::FusedRow(r))) => (e, Some(r)),
            _ => unreachable!("a sum and the norm it adds into"),
        },
        (_, Some(TensorOpSpec::Elementwise(e))) => (e, None),
        _ => unreachable!("an elementwise op"),
    }
}

/// A fused run of elementwise ops as the forward's one kernel takes it: the first op's operands and op, each later
/// member as a step carrying the running value through it, the run's result and its length.
fn chain(members: &[Op]) -> (Symbol, Symbol, ElementwiseOp, Vec<ChainStep>, Symbol, usize) {
    let Some(TensorOpSpec::Elementwise(e0)) = members[0].prim() else { unreachable!("a run starts with an elementwise op") };
    let mut carry = e0.c;
    let mut tail = Vec::new();
    for m in &members[1..] {
        match m.prim() {
            Some(TensorOpSpec::Elementwise(e)) => {
                tail.push(ChainStep::Binary(e.op, if e.a == carry { e.b } else { e.a }));
                carry = e.c;
            }
            Some(TensorOpSpec::Silu(u)) => {
                tail.push(ChainStep::Silu);
                carry = u.out;
            }
            _ => unreachable!("a run carries elementwise ops and SiLUs"),
        }
    }
    (e0.a, e0.b, e0.op, tail, carry, e0.n)
}

/// Under the single-pass int8 numerics, each activation an int8 product of a Q8_0 weight reads, with the product's
/// `(M, K)`.
fn q8_fed(ops: &[Op], q8: &HashMap<String, Q8Ref>, kind: Q8MatKind, interner: &Interner) -> HashMap<String, (usize, usize)> {
    let mut fed = HashMap::new();
    if kind == Q8MatKind::Int8Act {
        for op in ops {
            if let Some(TensorOpSpec::Matmul(m)) = op.prim() {
                if q8.contains_key(interner.resolve(m.b)) {
                    fed.insert(interner.resolve(m.a).to_string(), (m.m, m.k));
                }
            }
        }
    }
    fed
}

/// The variables a kernel touches, each with its element count, in the order the forward sizes their buffers: a sum and
/// its norm as their two statements, a run by its operands and result (its intermediates live in registers).
fn kernel_vars(op: &Op, interner: &Interner) -> Vec<(String, usize)> {
    let name = |s: Symbol| interner.resolve(s).to_string();
    let host = folded(op).0;
    match &host.sym {
        Sym::Fused(f) if f.class == ClassId::ElementwiseChains => {
            let (a, b, _, tail, out, n) = chain(&leaves(host.clone()));
            let mut v = vec![(name(a), n), (name(b), n), (name(out), n)];
            v.extend(tail.iter().filter_map(|step| match step {
                ChainStep::Binary(_, ext) => Some((name(*ext), n)),
                ChainStep::Silu => None,
            }));
            v
        }
        Sym::Fused(f) if f.class == ClassId::ElementwiseRegions => {
            let n = region_body(host, "float").1;
            host.ins.iter().cloned().chain(region_outputs(host)).map(|v| (v, n)).collect()
        }
        Sym::Fused(f) => f.parts.iter().flat_map(|p| kernel_vars(p, interner)).collect(),
        _ => match host.prim().expect("a lifted op") {
            TensorOpSpec::Matmul(m) => vec![(name(m.a), m.m * m.k), (name(m.b), m.k * m.n), (name(m.c), m.m * m.n)],
            TensorOpSpec::Elementwise(e) => vec![(name(e.a), e.n), (name(e.b), e.n), (name(e.c), e.n)],
            TensorOpSpec::FusedRow(r) => {
                let mut v = vec![(name(r.x), r.rows * r.cols), (name(r.y), r.rows * r.cols)];
                if let Some(w) = r.weight {
                    v.push((name(w), r.cols));
                }
                // Residual (a same-shape fused-add operand) and bias (per-column) are genuine INPUTS, sized as buffers.
                if let Some(res) = r.residual {
                    v.push((name(res), r.rows * r.cols));
                }
                if let Some(bias) = r.bias {
                    v.push((name(bias), r.cols));
                }
                v
            }
            TensorOpSpec::Transpose(t) => vec![(name(t.x), t.rows * t.cols), (name(t.out), t.rows * t.cols)],
            TensorOpSpec::Silu(u) => vec![(name(u.x), u.n), (name(u.out), u.n)],
            TensorOpSpec::Rope(r) => {
                let d = r.h * 2 * r.half;
                vec![(name(r.q), d), (name(r.rc), r.half), (name(r.rs), r.half), (name(r.out), d)]
            }
            // The grouped weights load from the MoE manifest (not chained buffers); only x (input) and out size here.
            TensorOpSpec::Moe(m) => vec![(name(m.x), m.d), (name(m.out), m.d)],
            TensorOpSpec::Attention(a) => vec![
                (name(a.q), a.h * a.hd),
                (name(a.kc), a.l * a.kv * a.hd),
                (name(a.vc), a.l * a.kv * a.hd),
                (name(a.out), a.h * a.hd),
            ],
            other => unreachable!("the forward's shim admits no {}", other.kind()),
        },
    }
}

/// The variables a kernel produces: a sum and its norm both, every other kernel its result.
fn kernel_outputs(op: &Op) -> Vec<String> {
    match &folded(op).0.sym {
        Sym::Fused(f) if f.class == ClassId::AddNorm => f.parts.iter().map(|p| p.out.clone()).collect(),
        Sym::Fused(f) if f.class == ClassId::ElementwiseRegions => region_outputs(op),
        _ => vec![op.out.clone()],
    }
}

/// What a region writes: its result, then every member's a later op reads.
fn region_outputs(op: &Op) -> Vec<String> {
    std::iter::once(op.out.clone()).chain(op.keeps.iter().cloned()).collect()
}

/// The values a region computes element by element: each member's, as its own kernel computes it, over the region's
/// operands read at the element's place and the earlier members' values in registers `t{k}`; then the count of
/// elements. A real product is rounded as its own kernel rounds it (`__fmul_rn`), which the compiler never contracts
/// with a sum after it into one fused multiply-add.
fn region_body(op: &Op, ty: &str) -> (Vec<String>, usize) {
    let members = leaves(op.clone());
    let value = |t: &str, upto: usize| match members[..upto].iter().position(|m| m.out == t) {
        Some(k) => format!("t{k}"),
        None => format!("i{}[idx]", op.ins.iter().position(|x| x == t).expect("a region reads its operands")),
    };
    let mut n = 0;
    let lines = members
        .iter()
        .enumerate()
        .map(|(k, m)| match m.prim() {
            Some(TensorOpSpec::Elementwise(e)) => {
                n = e.n;
                let (x, y) = (value(&m.ins[0], k), value(&m.ins[1], k));
                match e.op {
                    ElementwiseOp::Mul if ty == "float" => format!("{ty} t{k} = __fmul_rn({x}, {y});"),
                    op => format!("{ty} t{k} = {x} {} {y};", op.c_op()),
                }
            }
            Some(TensorOpSpec::Silu(u)) => {
                n = u.n;
                format!("{ty} t{k}; {{ float v=(float){}; t{k} = v/(1.0f+expf(-v)); }}", value(&m.ins[0], k))
            }
            _ => unreachable!("a region the forward links holds elementwise ops and SiLUs"),
        })
        .collect();
    (lines, n)
}

/// How a Q8_0 matmul lowers to CUDA. At batch-1 decode a matmul is bound on the int8 WEIGHT bytes, not the
/// math — so these three modes trade accuracy/latency, NOT weight bandwidth.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Q8MatKind {
    /// f64 dequant, thread-per-output — the numeric reference (exact, but uncoalesced and slow).
    Dequant,
    /// SINGLE-PASS int8-activation MMQ (`__dp4a`, int32 accumulate), coalesced — THE DEFAULT fast path:
    /// llama.cpp-matched activation precision (identical greedy token) with the shortest per-iteration
    /// dependency chain, which wins the latency-bound batch-1 regime (the A100 head-to-head beat). A
    /// near-tie logit can flip exactly as llama.cpp's does.
    Int8Act,
    /// TWO-PASS int8 activation (hi byte + lo residual byte ⇒ ~14-bit) — the logit-exact mode: the same
    /// short `__dp4a` dependency chain, ~2× the activation math (two dp4a per block), logits within ~1e-3
    /// of the f64 reference so a near-tie token does NOT flip. The correctness mode llama.cpp lacks.
    TwoPassInt8,
}

/// Emit the composed CUDA program for a forward's graph ([`forward_graph`]): one uniquely-named kernel per stage plus a
/// host driver that allocates a buffer per variable, fills the inputs `v[p] = (p % 7) - 3` (float chains scale by
/// 0.5), launches each stage in order, and prints every element of the final stage's output. Deterministic,
/// self-contained, `nvcc`-compilable; its output is checked against a clean-room reference of the same graph.
pub fn emit_forward(
    graph: &TensorGraph,
    inits: &HashMap<String, Vec<(usize, InitVal)>>,
    weights: &HashMap<String, (String, usize)>,
    q8: &HashMap<String, Q8Ref>,
    moe: &HashMap<String, MoeWeights>,
    kind: Q8MatKind,
    interner: &Interner,
) -> Result<String, ForwardError> {
    let (kernel_ops, stage_count) = kernels_of(graph);
    if kernel_ops.is_empty() {
        return Err(ForwardError::NoStages);
    }
    let lifted: Vec<Op> = graph.ops.iter().flat_map(|op| leaves(op.clone())).collect();
    // A chain is float-typed if any stage is a (float) reduction or any input carries a float initializer;
    // every kernel is then emitted over `float` (a matmul/elementwise inherits its operands' element type,
    // and LOGOS tensor DSL programs are consistently typed, so this per-chain decision is exact). This lets a
    // real transformer sub-block — `rmsnorm → matmul → residual`, all float — compose. A MoE stage is float too.
    let has_float_init = inits
        .values()
        .flatten()
        .any(|(_, v)| matches!(v, InitVal::F(_)));
    let any_moe = lifted.iter().any(|o| matches!(o.prim(), Some(TensorOpSpec::Moe(_))));
    let floaty = lifted.iter().any(|o| {
        matches!(o.prim(), Some(TensorOpSpec::FusedRow(_) | TensorOpSpec::Moe(_) | TensorOpSpec::Attention(_) | TensorOpSpec::Silu(_)))
    }) || has_float_init;
    let ty = if floaty { "float" } else { "long long" };

    // Collect every variable + its size, and which vars are produced (outputs) vs supplied (inputs).
    let mut sizes: Vec<(String, usize)> = Vec::new();
    let mut outputs: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (_, op) in &kernel_ops {
        for (v, sz) in kernel_vars(op, interner) {
            if seen.insert(v.clone()) {
                sizes.push((v, sz));
            }
        }
        outputs.extend(kernel_outputs(op));
    }
    let is_output = |v: &str| outputs.iter().any(|o| o == v);

    // ── kernels: one per stage, uniquely named k{i}_{kind} ──────────────────────────────────────────
    let mut kernels = String::new();
    // The MoE stages share one set of kernels (many layers reuse them); emit once. Both coalesced paths add
    // the sparse fused set (router+topk, gate+up+SwiGLU, down+combine) in their precision; the dense
    // router/silu/zero come from the base set.
    if any_moe {
        kernels.push_str(MOE_KERNELS);
        if kind != Q8MatKind::Dequant {
            kernels.push_str(&moe_dp4a_kernels(kind == Q8MatKind::TwoPassInt8));
        }
    }
    let mut launches = String::new();
    // Int8Act only: each distinct activation feeding an int8-activation DP4A matmul needs a scratch pair (int8
    // quants + f32 block-scales) that `k{i}_qact` writes and `k{i}_q8dp4a` reads; collected here, allocated below.
    let mut dp4a_scratch: Vec<(String, usize, usize)> = Vec::new();
    // PRODUCER-FUSED activation quantization (Int8Act): an activation feeding a q8 matmul is quantized to int8
    // INLINE by its producer kernel (rmsnorm/attention epilogue), so the consuming matmul skips its own `qact`
    // launch — llama.cpp's inline-quant edge. `q8_fed[act] = (M, K)`; `producer_quantized` records the ones a
    // producer already wrote (the matmul then just reads `buf_{act}_q8act`/`_q8scl`).
    let q8_fed = q8_fed(&lifted, q8, kind, interner);
    let mut producer_quantized: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for &(i, op) in &kernel_ops {
        let (host, quantized) = folded(op);
        match &host.sym {
            Sym::Prim(TensorOpSpec::Matmul(m)) => {
                let (a, b, c) = (
                    interner.resolve(m.a),
                    interner.resolve(m.b),
                    interner.resolve(m.c),
                );
                if q8.contains_key(b) {
                    match kind {
                        Q8MatKind::TwoPassInt8 => {
                            // INT8-ACTIVATION dp4a (llama.cpp's MMQ) for the attention/LM-head Q8 matmul: `k{i}_qact`
                            // quantizes the f32 activation to int8 per 32-block (careful per-block amax/127, matching
                            // llama's q8_1), then `k{i}_q8dp4a` streams int8 weight × int8 activation via `__dp4a` (int32
                            // accumulate) — a SHORT per-iteration dependency chain (one instruction vs the f32 chain of
                            // 4 int→float casts + half→float + FMA), which is the win in the batch-1 LATENCY-bound regime
                            // (both ILP and split-K hurt ⇒ we're latency-, not throughput-bound). Same math llama uses ⇒
                            // TWO-PASS int8 (hi byte + lo residual byte ⇒ ~14-bit activation) so `__dp4a` keeps its short
                            // dependency chain AND enough precision to hold the true token on a near-tie (single int8-act
                            // flips it — like llama.cpp). Weights read ONCE (memory-bound cost unchanged); the extra dp4a is
                            // near-free. `sl = sh/128` — the residual lives in ±sh/2, so /128 fills the int8 range.
                            let nb_ct = m.k / 32;
                            kernels.push_str(&format!(
                        "__global__ void k{i}_qact(const float* a, signed char* aqh, signed char* aql, float* as_, int M, int K){{\n\
                         \x20 int nb=K/32; int blk=blockIdx.x*(blockDim.x/32)+threadIdx.x/32; if(blk>=M*nb) return;\n\
                         \x20 int lane=threadIdx.x&31; float v=a[blk*32+lane], av=fabsf(v);\n\
                         \x20 for(int s=16;s>0;s>>=1) av=fmaxf(av,__shfl_down_sync(0xffffffffu,av,s));\n\
                         \x20 av=__shfl_sync(0xffffffffu,av,0); float amax=av>1e-30f?av:1e-30f; float sh=amax/127.0f;\n\
                         \x20 if(lane==0) as_[blk]=sh;\n\
                         \x20 int qh=(int)rintf(v/sh); if(qh>127)qh=127; if(qh<-128)qh=-128; aqh[blk*32+lane]=(signed char)qh;\n\
                         \x20 float r=v-qh*sh; float sl=sh*0.0078125f; int ql=(int)rintf(r/sl); if(ql>127)ql=127; if(ql<-128)ql=-128; aql[blk*32+lane]=(signed char)ql;\n}}\n\
                         __global__ void k{i}_q8dp4a(const signed char* aqh, const signed char* aql, const float* as_, const signed char* wq, const __half* ws, float* y, int M, int N, int K){{\n\
                         \x20 int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31; int nb=K/32;\n\
                         \x20 const signed char* wrow=wq+(long)o*K; const int* w4=(const int*)wrow; const __half* wsr=ws+(long)o*nb;\n\
                         \x20 for(int r=0;r<M;++r){{\n\
                         \x20   const signed char* arh=aqh+(long)r*K; const int* ah4=(const int*)arh; const signed char* arl=aql+(long)r*K; const int* al4=(const int*)arl; const float* asr=as_+(long)r*nb;\n\
                         \x20   float acc=0.0f; int nq=K/128;\n\
                         \x20   for(int b4=0;b4<nq;++b4){{ int wi=w4[b4*32+lane]; int blk=b4*4+lane/8; float ws_=__half2float(wsr[blk]); float sh=asr[blk];\n\
                         \x20     acc += ws_*(sh*(float)__dp4a(wi,ah4[b4*32+lane],0) + sh*0.0078125f*(float)__dp4a(wi,al4[b4*32+lane],0)); }}\n\
                         \x20   for(int j=nq*128+lane;j<K;j+=32){{ int blk=j/32; float ws_=__half2float(wsr[blk]); float sh=asr[blk]; int wj=(int)wrow[j];\n\
                         \x20     acc += ws_*(sh*(float)(wj*(int)arh[j]) + sh*0.0078125f*(float)(wj*(int)arl[j])); }}\n\
                         \x20   for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s);\n\
                         \x20   if(lane==0) y[(long)r*N+o]=acc;\n\
                         \x20 }}\n}}\n"
                    ));
                            if !dp4a_scratch.iter().any(|(v, _, _)| v == a) {
                                dp4a_scratch.push((a.to_string(), m.m, m.k));
                            }
                            launches.push_str(&format!(
                        "  {{ k{i}_qact<<<({M}*{NB}+7)/8,256>>>(buf_{a},buf_{a}_q8act,buf_{a}_q8actlo,buf_{a}_q8scl,{M},{K});\n\
                         \x20   k{i}_q8dp4a<<<({N}+7)/8,256>>>(buf_{a}_q8act,buf_{a}_q8actlo,buf_{a}_q8scl,buf_{b}_q,buf_{b}_s,buf_{c},{M},{N},{K}); }}\n",
                        M = m.m, N = m.n, K = m.k, NB = nb_ct
                    ));
                        }
                        Q8MatKind::Int8Act => {
                            // INT8-ACTIVATION MMQ, COALESCED — mirrors `cuda/qwen_forward.cu`'s `q8_gemv_dp4a`. Two kernels:
                            // (1) `k{i}_qact` quantizes each 32-block of the f32 activation to int8 + an f32 block-scale (one
                            // warp per block, `__shfl` max-reduce); (2) `k{i}_q8dp4a` assigns ONE WARP per output row — each
                            // lane streams a 128-byte-coalesced `int` (4 packed int8) of BOTH int8 operands, `__dp4a`-
                            // accumulates (4 MACs/instr, int32), recombining per 32-block by weight-scale × activation-scale,
                            // a scalar tail covering K%128, then a warp-shuffle reduction. THE DEFAULT batch-1 path: the
                            // single dp4a per block is the shortest dependency chain — the fastest in the latency-bound
                            // regime, at llama.cpp-matched activation precision; `TwoPassInt8` is the logit-exact opt-in.
                            let nb_ct = m.k / 32;
                            kernels.push_str(&format!(
                        "__global__ void k{i}_qact(const float* a, signed char* aq, float* as_, int M, int K){{\n\
                         \x20 int nb=K/32; int blk=blockIdx.x*(blockDim.x/32)+threadIdx.x/32; if(blk>=M*nb) return;\n\
                         \x20 int lane=threadIdx.x&31; float v=a[blk*32+lane], av=fabsf(v);\n\
                         \x20 for(int s=16;s>0;s>>=1) av=fmaxf(av,__shfl_down_sync(0xffffffffu,av,s));\n\
                         \x20 av=__shfl_sync(0xffffffffu,av,0); float amax=av>1e-30f?av:1e-30f; float scale=amax/127.0f;\n\
                         \x20 if(lane==0) as_[blk]=scale;\n\
                         \x20 int q=(int)rintf(v/scale); if(q>127)q=127; if(q<-128)q=-128; aq[blk*32+lane]=(signed char)q;\n}}\n\
                         __global__ void k{i}_q8dp4a(const signed char* aq, const float* as_, const signed char* wq, const __half* ws, float* y, int M, int N, int K){{\n\
                         \x20 int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31; int nb=K/32;\n\
                         \x20 const signed char* wrow=wq+(long)o*K; const int* w4=(const int*)wrow; const __half* wsr=ws+(long)o*nb;\n\
                         \x20 for(int r=0;r<M;++r){{\n\
                         \x20   const signed char* arow=aq+(long)r*K; const int* a4=(const int*)arow; const float* asr=as_+(long)r*nb;\n\
                         \x20   float acc=0.0f; int nq16=K/512; int nq=K/128;\n\
                         \x20   const int4* w16=(const int4*)wrow; const int4* a16=(const int4*)arow;\n\
                         \x20   for(int b=0;b<nq16;++b){{ int4 wv=w16[b*32+lane]; int4 av=a16[b*32+lane]; int blk=b*16+lane/2; float sw=__half2float(wsr[blk])*asr[blk];\n\
                         \x20     int d=__dp4a(wv.x,av.x,0); d=__dp4a(wv.y,av.y,d); d=__dp4a(wv.z,av.z,d); d=__dp4a(wv.w,av.w,d); acc += (float)d * sw; }}\n\
                         \x20   for(int b4=nq16*4;b4<nq;++b4){{ int wi=w4[b4*32+lane], ai=a4[b4*32+lane]; int blk=b4*4+lane/8; acc += (float)__dp4a(wi,ai,0) * (__half2float(wsr[blk])*asr[blk]); }}\n\
                         \x20   for(int j=nq*128+lane;j<K;j+=32){{ int blk=j/32; acc += (float)((int)arow[j]*(int)wrow[j]) * (__half2float(wsr[blk])*asr[blk]); }}\n\
                         \x20   for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s);\n\
                         \x20   if(lane==0) y[(long)r*N+o]=acc;\n\
                         \x20 }}\n}}\n"
                    ));
                            // DEDUP the activation quantization: an activation feeding several projections (Q/K/V all read
                            // the SAME `xn`) is quantized to int8 ONCE, not per-matmul — the `q8act`/`q8scl` buffers are
                            // shared (allocated once via `dp4a_scratch`). Emit `k{i}_qact` only on the first matmul that
                            // uses `a`; later matmuls read the already-quantized buffer. (Kills the redundant Q/K/V qacts.)
                            let first_qact = !dp4a_scratch.iter().any(|(v, _, _)| v == a);
                            if first_qact {
                                dp4a_scratch.push((a.to_string(), m.m, m.k));
                            }
                            // Skip the qact launch entirely if the PRODUCER already quantized this activation inline
                            // (fused into its rmsnorm/attention epilogue), or if a prior matmul on the same activation
                            // already quantized it (Q/K/V dedup). `buf_{a}_q8act`/`_q8scl` are filled either way.
                            let qact_launch = if first_qact && !producer_quantized.contains(a) {
                                format!("k{i}_qact<<<({M}*{NB}+7)/8,256>>>(buf_{a},buf_{a}_q8act,buf_{a}_q8scl,{M},{K}); ", M = m.m, K = m.k, NB = nb_ct)
                            } else {
                                String::new()
                            };
                            launches.push_str(&format!(
                        "  {{ {qact_launch}k{i}_q8dp4a<<<({N}+7)/8,256>>>(buf_{a}_q8act,buf_{a}_q8scl,buf_{b}_q,buf_{b}_s,buf_{c},{M},{N},{K}); }}\n",
                        M = m.m, N = m.n, K = m.k
                    ));
                        }
                        Q8MatKind::Dequant => {
                            // INT8-quantized projection `y = W·x`: W is the GGUF `[out, in]` Q8_0 weight (quants + f32
                            // block scales), the activation f32. Dequant-in-kernel GEMV — one output per thread,
                            // accumulate over 32-wide blocks scaled by the block scale (matches `qwen.rs` matvec).
                            kernels.push_str(&format!(
                        "__global__ void k{i}_q8gemv(const float* a, const signed char* wq, const float* ws, float* y, int M, int N, int K){{\n\
                         \x20 int idx = blockIdx.x*blockDim.x + threadIdx.x; if(idx >= M*N) return;\n\
                         \x20 int r = idx / N, n = idx % N; int nb = K/32; double acc = 0.0;\n\
                         \x20 for(int bl=0;bl<nb;++bl){{ double dot=0.0; for(int j=0;j<32;++j) dot += (double)wq[n*K + bl*32 + j] * (double)a[r*K + bl*32 + j]; acc += dot * (double)ws[n*nb + bl]; }}\n\
                         \x20 y[idx] = (float)acc;\n}}\n"
                    ));
                            launches.push_str(&format!(
                        "  {{ int total={M}*{N}; k{i}_q8gemv<<<(total+255)/256,256>>>(buf_{a},buf_{b}_q,buf_{b}_s,buf_{c},{M},{N},{K}); }}\n",
                        M = m.m, N = m.n, K = m.k
                    ));
                        }
                    }
                } else {
                    kernels.push_str(&format!(
                        "__global__ void k{i}_matmul(const {ty}* a, const {ty}* b, {ty}* c, int M, int N, int K){{\n\
                         \x20 int idx = blockIdx.x*blockDim.x + threadIdx.x; if(idx >= M*N) return;\n\
                         \x20 int r = idx / N, col = idx % N; {ty} s = 0;\n\
                         \x20 for(int k=0;k<K;++k) s += a[r*K+k]*b[k*N+col];\n\
                         \x20 c[idx] = s;\n}}\n"
                    ));
                    launches.push_str(&format!(
                        "  {{ int total={M}*{N}; k{i}_matmul<<<(total+255)/256,256>>>(buf_{a},buf_{b},buf_{c},{M},{N},{K}); }}\n",
                        M = m.m, N = m.n, K = m.k
                    ));
                }
            }
            Sym::Prim(TensorOpSpec::Elementwise(_)) | Sym::Fused(Fused { class: ClassId::AddNorm, .. }) => {
                let (e, normed) = sum_and_norm(host);
                let (a, b, c) = (
                    interner.resolve(e.a),
                    interner.resolve(e.b),
                    interner.resolve(e.c),
                );
                let op = e.op.c_op();
                // FUSE `c = a + b` directly into an immediately-following RMSNorm of `c` (the transformer's
                // residual-then-norm): emit ONE kernel that adds, writes `c` (still needed by the next residual),
                // and normalizes. The standalone add kernel + its residual read-back disappear. Only Add fuses
                // (norm needs the summed row), and only when the whole row lines up (e.n == rows*cols).
                if let Some(r) = normed {
                    assert_eq!(r.weight_rows, 1, "the forward's shim admits no norm with a weight row per lane");
                    let (cols, rows, y) = (r.cols, r.rows, interner.resolve(r.y));
                    let eps = format!("{:e}", r.eps.get());
                    let wparam = if r.weight.is_some() {
                        ", const float* w"
                    } else {
                        ""
                    };
                    let ymul = if r.weight.is_some() { "*w[i]" } else { "" };
                    // Fused activation quant (as in the plain norm): if this add+norm's output feeds a q8 matmul,
                    // quantize `yr` inline so the consuming matmul skips its own qact.
                    let qact_out = quantized;
                    let qparam = if qact_out {
                        ", signed char* yq, float* ys"
                    } else {
                        ""
                    };
                    let qepilogue = if qact_out {
                        format!(
                            "\x20 __syncthreads(); {{ int qlane=threadIdx.x&31, qwp=threadIdx.x>>5, qnw=blockDim.x>>5; int qnb={cols}/32;\n\
                             \x20   for(int blk=qwp;blk<qnb;blk+=qnw){{ float v=yr[blk*32+qlane], av=fabsf(v);\n\
                             \x20     for(int s=16;s>0;s>>=1) av=fmaxf(av,__shfl_down_sync(0xffffffffu,av,s)); av=__shfl_sync(0xffffffffu,av,0);\n\
                             \x20     float amax=av>1e-30f?av:1e-30f, scale=amax/127.0f; if(qlane==0) ys[blk]=scale;\n\
                             \x20     int qq=(int)rintf(v/scale); if(qq>127)qq=127; if(qq<-128)qq=-128; yq[blk*32+qlane]=(signed char)qq; }} }}\n",
                            cols = cols
                        )
                    } else {
                        String::new()
                    };
                    kernels.push_str(&format!(
                        "__global__ void k{i}_add_row(const float* a, const float* b{wparam}, float* c, float* y{qparam}){{\n\
                         \x20 int row=blockIdx.x; const float* ar=a+(long)row*{cols}; const float* br=b+(long)row*{cols};\n\
                         \x20 float* cr=c+(long)row*{cols}; float* yr=y+(long)row*{cols}; __shared__ double sh[256];\n\
                         \x20 double sc=0; for(int i=threadIdx.x;i<{cols};i+=blockDim.x){{ float v=ar[i]+br[i]; cr[i]=v; sc+=(double)v*v; }}\n\
                         \x20 sh[threadIdx.x]=sc; __syncthreads();\n\
                         \x20 for(int k=blockDim.x/2;k>0;k>>=1){{ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }}\n\
                         \x20 double scale=1.0/sqrt(sh[0]/(double){cols}+{eps});\n\
                         \x20 for(int i=threadIdx.x;i<{cols};i+=blockDim.x) yr[i]=(float)((double)cr[i]*scale){ymul};\n{qepilogue}}}\n"
                    ));
                    let warg = if let Some(w) = r.weight {
                        format!(",buf_{}", interner.resolve(w))
                    } else {
                        String::new()
                    };
                    let qarg = if qact_out {
                        format!(",buf_{y}_q8act,buf_{y}_q8scl")
                    } else {
                        String::new()
                    };
                    launches.push_str(&format!(
                        "  k{i}_add_row<<<{rows},256>>>(buf_{a},buf_{b}{warg},buf_{c},buf_{y}{qarg});\n"
                    ));
                    if qact_out {
                        if !dp4a_scratch.iter().any(|(v, _, _)| v == y) {
                            let (mm, kk) = q8_fed[y];
                            dp4a_scratch.push((y.to_string(), mm, kk));
                        }
                        producer_quantized.insert(y.to_string());
                    }
                } else {
                    kernels.push_str(&format!(
                        "__global__ void k{i}_elem(const {ty}* a, const {ty}* b, {ty}* c, int N){{\n\
                         \x20 int idx = blockIdx.x*blockDim.x + threadIdx.x; if(idx < N) c[idx] = a[idx] {op} b[idx];\n}}\n"
                    ));
                    launches.push_str(&format!(
                        "  k{i}_elem<<<({N}+255)/256,256>>>(buf_{a},buf_{b},buf_{c},{N});\n",
                        N = e.n
                    ));
                }
            }
            Sym::Fused(Fused { class: ClassId::ElementwiseChains, .. }) => {
                let (a, b, first_op, tail, out, n) = chain(&leaves(host.clone()));
                let (a, b, first_op, tail, out, n) = (&a, &b, &first_op, &tail, &out, &n);
                // The whole run in ONE kernel: seed `t = a ⊕₀ b`, carry `t = t ⊕ ext` per tail op (registers,
                // no global round-trip), store `o = t` once. Params i0,i1 are the seed operands; i{k+2} the
                // k-th tail external. This is where the (N−1) intermediate loads/stores + launches vanish.
                let (a, b, out) = (
                    interner.resolve(*a),
                    interner.resolve(*b),
                    interner.resolve(*out),
                );
                // A real product the next step adds to is rounded as its own kernel rounds it (`__fmul_rn`), which the
                // compiler never contracts with the sum into one fused multiply-add: the run computes what its ops
                // compute apart, bit for bit.
                let summed = |k: usize| ty == "float" && matches!(tail.get(k), Some(ChainStep::Binary(ElementwiseOp::Add, _)));
                let apply = |op: ElementwiseOp, x: &str, y: &str, k: usize| match op {
                    ElementwiseOp::Mul if summed(k) => format!("__fmul_rn({x}, {y})"),
                    op => format!("{x} {} {y}", op.c_op()),
                };
                let mut params = format!("const {ty}* i0, const {ty}* i1");
                let mut args = format!("buf_{a},buf_{b}");
                let mut body = format!("{ty} t = {};", apply(*first_op, "i0[idx]", "i1[idx]", 0));
                let mut pk = 2; // next input-param index; only BINARY steps consume an operand param
                for (k, step) in tail.iter().enumerate() {
                    match step {
                        ChainStep::Binary(op, ext) => {
                            params.push_str(&format!(", const {ty}* i{pk}"));
                            args.push_str(&format!(",buf_{}", interner.resolve(*ext)));
                            body.push_str(&format!(" t = {};", apply(*op, "t", &format!("i{pk}[idx]"), k + 1)));
                            pk += 1;
                        }
                        ChainStep::Silu => body.push_str(" t = t/(1.0f+expf(-t));"),
                    }
                }
                kernels.push_str(&format!(
                    "__global__ void k{i}_fused({params}, {ty}* o, int N){{\n\
                     \x20 int idx = blockIdx.x*blockDim.x + threadIdx.x; if(idx < N){{ {body} o[idx] = t; }}\n}}\n"
                ));
                launches.push_str(&format!(
                    "  k{i}_fused<<<({N}+255)/256,256>>>({args},buf_{out},{N});\n",
                    N = n
                ));
            }
            Sym::Fused(Fused { class: ClassId::ElementwiseRegions, .. }) => {
                // The whole region in ONE kernel: each member in registers, the region's operands read once at the
                // element's place, its result and every member a later op reads written out.
                let (lines, n) = region_body(host, ty);
                let members = leaves(host.clone());
                let outputs = region_outputs(host);
                let mut params: Vec<String> = (0..host.ins.len()).map(|p| format!("const {ty}* i{p}")).collect();
                params.extend((0..outputs.len()).map(|q| format!("{ty}* o{q}")));
                let stores: String = outputs
                    .iter()
                    .enumerate()
                    .map(|(q, o)| format!(" o{q}[idx] = t{};", members.iter().position(|m| m.out == *o).expect("a region writes its members")))
                    .collect();
                kernels.push_str(&format!(
                    "__global__ void k{i}_region({}, int N){{\n\
                     \x20 int idx = blockIdx.x*blockDim.x + threadIdx.x; if(idx < N){{ {}{stores} }}\n}}\n",
                    params.join(", "),
                    lines.join(" ")
                ));
                let args: Vec<String> = host.ins.iter().chain(&outputs).map(|v| format!("buf_{v}")).collect();
                launches.push_str(&format!("  k{i}_region<<<({n}+255)/256,256>>>({},{n});\n", args.join(",")));
            }
            Sym::Prim(TensorOpSpec::FusedRow(r)) => {
                assert_eq!(r.weight_rows, 1, "the forward's shim admits no norm with a weight row per lane");
                let (x, y) = (interner.resolve(r.x), interner.resolve(r.y));
                let cols = r.cols;
                let eps = format!("{:e}", r.eps.get());
                // The reduced value: `X[i]`, or `X[i] + R[i]` when the norm carries a fused residual (summed in
                // double, as the reference adds two reals).
                let xv = |i: &str| match r.residual {
                    Some(_) => format!("((double)xr[{i}]+(double)rr[{i}])"),
                    None => format!("(double)xr[{i}]"),
                };
                let body = match (r.reduce, r.weight.is_some()) {
                    (RowReduceKind::RmsNorm, false) => format!(
                        "  double sc=0; for(int i=threadIdx.x;i<{cols};i+=blockDim.x) sc+={v}*{v};\n\
                         \x20 sh[threadIdx.x]=sc; __syncthreads();\n\
                         \x20 for(int k=blockDim.x/2;k>0;k>>=1){{ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }}\n\
                         \x20 double scale=1.0/sqrt(sh[0]/(double){cols}+{eps});\n\
                         \x20 for(int i=threadIdx.x;i<{cols};i+=blockDim.x) yr[i]=(float)({v}*scale);\n",
                        v = xv("i")
                    ),
                    (RowReduceKind::RmsNorm, true) => format!(
                        "  double sc=0; for(int i=threadIdx.x;i<{cols};i+=blockDim.x) sc+={v}*{v};\n\
                         \x20 sh[threadIdx.x]=sc; __syncthreads();\n\
                         \x20 for(int k=blockDim.x/2;k>0;k>>=1){{ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }}\n\
                         \x20 double scale=1.0/sqrt(sh[0]/(double){cols}+{eps});\n\
                         \x20 for(int i=threadIdx.x;i<{cols};i+=blockDim.x) yr[i]=(float)({v}*scale)*w[i];\n",
                        v = xv("i")
                    ),
                    (RowReduceKind::Softmax, _) => format!(
                        "  double m=-1e30; for(int i=threadIdx.x;i<{cols};i+=blockDim.x) m=fmax(m,(double)xr[i]);\n\
                         \x20 sh[threadIdx.x]=m; __syncthreads();\n\
                         \x20 for(int k=blockDim.x/2;k>0;k>>=1){{ if(threadIdx.x<k) sh[threadIdx.x]=fmax(sh[threadIdx.x],sh[threadIdx.x+k]); __syncthreads(); }}\n\
                         \x20 double mx=sh[0]; __syncthreads();\n\
                         \x20 double s=0; for(int i=threadIdx.x;i<{cols};i+=blockDim.x) s+=exp((double)xr[i]-mx);\n\
                         \x20 sh[threadIdx.x]=s; __syncthreads();\n\
                         \x20 for(int k=blockDim.x/2;k>0;k>>=1){{ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }}\n\
                         \x20 double sm=sh[0]; for(int i=threadIdx.x;i<{cols};i+=blockDim.x) yr[i]=(float)(exp((double)xr[i]-mx)/sm);\n"
                    ),
                    // Two reductions over the row, each in double: the mean, then the population variance about it;
                    // then `(X - mean) / sqrt(var + eps) * W + B` in the reference's order.
                    (RowReduceKind::LayerNorm, _) => {
                        let wmul = if r.weight.is_some() { "*(double)w[i]" } else { "" };
                        let badd = if r.bias.is_some() { "+(double)bias[i]" } else { "" };
                        format!(
                            "  double sm=0; for(int i=threadIdx.x;i<{cols};i+=blockDim.x) sm+=(double)xr[i];\n\
                             \x20 sh[threadIdx.x]=sm; __syncthreads();\n\
                             \x20 for(int k=blockDim.x/2;k>0;k>>=1){{ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }}\n\
                             \x20 double mean=sh[0]/(double){cols}; __syncthreads();\n\
                             \x20 double vs=0; for(int i=threadIdx.x;i<{cols};i+=blockDim.x){{ double d=(double)xr[i]-mean; vs+=d*d; }}\n\
                             \x20 sh[threadIdx.x]=vs; __syncthreads();\n\
                             \x20 for(int k=blockDim.x/2;k>0;k>>=1){{ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }}\n\
                             \x20 double denom=sqrt(sh[0]/(double){cols}+{eps});\n\
                             \x20 for(int i=threadIdx.x;i<{cols};i+=blockDim.x) yr[i]=(float)(((double)xr[i]-mean)/denom{wmul}{badd});\n"
                        )
                    }
                };
                let wparam = format!(
                    "{}{}{}",
                    if r.weight.is_some() { ", const float* w" } else { "" },
                    if r.residual.is_some() { ", const float* res" } else { "" },
                    if r.bias.is_some() { ", const float* bias" } else { "" },
                );
                let rrow = if r.residual.is_some() {
                    format!(" const float* rr = res + (long)row*{cols};")
                } else {
                    String::new()
                };
                // FUSED activation quantization: if this norm's output feeds an int8-act q8 matmul, quantize `yr`
                // to int8 + per-32-block scales in the SAME kernel (warp per 32-block, `__shfl` max) — the
                // consuming matmul then skips its own `qact` launch (llama's inline-quant edge).
                let qact_out = quantized;
                let qparam = if qact_out {
                    ", signed char* yq, float* ys"
                } else {
                    ""
                };
                let qepilogue = if qact_out {
                    format!(
                        "\x20 __syncthreads(); {{ int qlane=threadIdx.x&31, qwp=threadIdx.x>>5, qnw=blockDim.x>>5; int qnb={cols}/32;\n\
                         \x20   for(int blk=qwp;blk<qnb;blk+=qnw){{ float v=yr[blk*32+qlane], av=fabsf(v);\n\
                         \x20     for(int s=16;s>0;s>>=1) av=fmaxf(av,__shfl_down_sync(0xffffffffu,av,s)); av=__shfl_sync(0xffffffffu,av,0);\n\
                         \x20     float amax=av>1e-30f?av:1e-30f, scale=amax/127.0f; if(qlane==0) ys[blk]=scale;\n\
                         \x20     int qq=(int)rintf(v/scale); if(qq>127)qq=127; if(qq<-128)qq=-128; yq[blk*32+qlane]=(signed char)qq; }} }}\n",
                        cols = cols
                    )
                } else {
                    String::new()
                };
                kernels.push_str(&format!(
                    "__global__ void k{i}_row(const float* x{wparam}, float* y{qparam}){{\n\
                     \x20 int row = blockIdx.x; const float* xr = x + (long)row*{cols}; float* yr = y + (long)row*{cols};{rrow}\n\
                     \x20 __shared__ double sh[256];\n{body}{qepilogue}}}\n"
                ));
                let warg: String = [r.weight, r.residual, r.bias]
                    .into_iter()
                    .flatten()
                    .map(|t| format!(",buf_{}", interner.resolve(t)))
                    .collect();
                let qarg = if qact_out {
                    format!(",buf_{y}_q8act,buf_{y}_q8scl")
                } else {
                    String::new()
                };
                launches.push_str(&format!(
                    "  k{i}_row<<<{rows},256>>>(buf_{x}{warg},buf_{y}{qarg});\n",
                    rows = r.rows
                ));
                if qact_out {
                    if !dp4a_scratch.iter().any(|(v, _, _)| v == y) {
                        let (mm, kk) = q8_fed[y];
                        dp4a_scratch.push((y.to_string(), mm, kk));
                    }
                    producer_quantized.insert(y.to_string());
                }
            }
            Sym::Prim(TensorOpSpec::Transpose(t)) => {
                let (x, out, rows, cols) = (&t.x, &t.out, &t.rows, &t.cols);
                let (xn, on) = (interner.resolve(*x), interner.resolve(*out));
                kernels.push_str(&format!(
                    "__global__ void k{i}_transpose(const {ty}* x, {ty}* y, int R, int C){{\n\
                     \x20 int idx = blockIdx.x*blockDim.x + threadIdx.x; if(idx >= R*C) return;\n\
                     \x20 int r = idx / C, c = idx % C; y[c*R + r] = x[r*C + c];\n}}\n"
                ));
                launches.push_str(&format!(
                    "  {{ int total={R}*{C}; k{i}_transpose<<<(total+255)/256,256>>>(buf_{xn},buf_{on},{R},{C}); }}\n",
                    R = rows, C = cols
                ));
            }
            Sym::Prim(TensorOpSpec::Silu(s)) => {
                let (x, out, n) = (&s.x, &s.out, &s.n);
                let (xn, on) = (interner.resolve(*x), interner.resolve(*out));
                kernels.push_str(&format!(
                    "__global__ void k{i}_silu(const {ty}* x, {ty}* y, int N){{\n\
                     \x20 int idx = blockIdx.x*blockDim.x + threadIdx.x; if(idx < N){{ float v=(float)x[idx]; y[idx] = v/(1.0f+expf(-v)); }}\n}}\n"
                ));
                launches.push_str(&format!(
                    "  k{i}_silu<<<({N}+255)/256,256>>>(buf_{xn},buf_{on},{N});\n",
                    N = n
                ));
            }
            Sym::Prim(TensorOpSpec::Rope(r)) => {
                let (q, rc, rs, out, h, half) = (&r.q, &r.rc, &r.rs, &r.out, &r.h, &r.half);
                let (qn, rcn, rsn, on) = (
                    interner.resolve(*q),
                    interner.resolve(*rc),
                    interner.resolve(*rs),
                    interner.resolve(*out),
                );
                kernels.push_str(&format!(
                    "__global__ void k{i}_rope(const float* q, const float* rc, const float* rs, float* y, int H, int HALF){{\n\
                     \x20 int hd = 2*HALF; int idx = blockIdx.x*blockDim.x + threadIdx.x; if(idx >= H*HALF) return;\n\
                     \x20 int hh = idx/HALF, i = idx%HALF; float a=q[hh*hd+i], b=q[hh*hd+i+HALF], c=rc[i], s=rs[i];\n\
                     \x20 y[hh*hd+i] = a*c - b*s; y[hh*hd+i+HALF] = a*s + b*c;\n}}\n"
                ));
                launches.push_str(&format!(
                    "  {{ int total={H}*{HALF}; k{i}_rope<<<(total+255)/256,256>>>(buf_{qn},buf_{rcn},buf_{rsn},buf_{on},{H},{HALF}); }}\n",
                    H = h, HALF = half
                ));
            }
            Sym::Prim(TensorOpSpec::Moe(m)) => {
                // MoE FFN: router GEMV → top-K route → each expert's SwiGLU (expert-indexed grouped weight bases
                // buf_{out}_W{r,g,u,d}) → weighted combine, through shared scratch (moe_logits/w/gate_e/up_e/he/
                // oute) reused across MoE stages (they run sequentially on the stream). Weights load from the MoE
                // manifest keyed by the output var (allocated below). Stream added by the graph rewrite.
                let (x, out) = (interner.resolve(m.x), interner.resolve(m.out));
                let (d, dff, ne, k) = (m.d, m.dff, m.ne, m.k);
                let (dnb, fnb) = (d / 32, dff / 32);
                let etopk = ne.min(1024);
                if kind != Q8MatKind::Dequant {
                    // SPARSE, DEEPLY FUSED, coalesced int8: route to the top-K expert INDICES, then just THREE
                    // kernels — quantize-activation, gate+up+SwiGLU fused (Wg/Wu rows read interleaved for 2×
                    // MLP, only h written), and down+combine fused (K warps/output dim, only H written). The
                    // two barriers (router, h-complete) are the only kernel boundaries; every intermediate
                    // (g, u, per-expert o) stays off HBM. Reads ~K/E of the weights per token (A3B sparsity)
                    // at full coalesced int8 bandwidth. Int8Act quantizes once per activation (the DEFAULT,
                    // llama-matched precision); TwoPassInt8 adds the lo-residual pass (~14-bit, logit-exact).
                    if kind == Q8MatKind::TwoPassInt8 {
                        launches.push_str(&format!(
                            "  moe_router_fast<<<({ne}+7)/8,256>>>(buf_{x},buf_{out}_Wr_q,buf_{out}_Wr_s,moe_logits,{ne},{d});\n\
                             \x20 moe_topk_idx<<<1,{etopk}>>>(moe_logits,moe_idx,moe_w,{ne},{k});\n\
                             \x20 moe_qact<<<(({d}/32)+7)/8,256>>>(buf_{x},moe_xqh,moe_xql,moe_xs,{d});\n\
                             \x20 moe_gate_up_silu<<<({k}*{dff}+7)/8,256>>>(moe_xqh,moe_xql,moe_xs,buf_{out}_Wg_q,buf_{out}_Wg_s,buf_{out}_Wu_q,buf_{out}_Wu_s,moe_idx,moe_he_all,{k},{dff},{d});\n\
                             \x20 moe_qact<<<((({k}*{dff})/32)+7)/8,256>>>(moe_he_all,moe_hqh,moe_hql,moe_hs,{k}*{dff});\n\
                             \x20 moe_down_combine<<<{d},{k}*32>>>(moe_hqh,moe_hql,moe_hs,buf_{out}_Wd_q,buf_{out}_Wd_s,moe_idx,moe_w,buf_{out},{k},{d},{dff});\n"
                        ));
                    } else {
                        launches.push_str(&format!(
                            "  moe_router_fast<<<({ne}+7)/8,256>>>(buf_{x},buf_{out}_Wr_q,buf_{out}_Wr_s,moe_logits,{ne},{d});\n\
                             \x20 moe_topk_idx<<<1,{etopk}>>>(moe_logits,moe_idx,moe_w,{ne},{k});\n\
                             \x20 moe_qact<<<(({d}/32)+7)/8,256>>>(buf_{x},moe_xqh,moe_xs,{d});\n\
                             \x20 moe_gate_up_silu<<<({k}*{dff}+7)/8,256>>>(moe_xqh,moe_xs,buf_{out}_Wg_q,buf_{out}_Wg_s,buf_{out}_Wu_q,buf_{out}_Wu_s,moe_idx,moe_he_all,{k},{dff},{d});\n\
                             \x20 moe_qact<<<((({k}*{dff})/32)+7)/8,256>>>(moe_he_all,moe_hqh,moe_hs,{k}*{dff});\n\
                             \x20 moe_down_combine<<<{d},{k}*32>>>(moe_hqh,moe_hs,buf_{out}_Wd_q,buf_{out}_Wd_s,moe_idx,moe_w,buf_{out},{k},{d},{dff});\n"
                        ));
                    }
                } else {
                    // A host `for` loop over experts (runtime expert index → runtime weight-base offset) keeps
                    // the emitted source tiny at 48 layers × 128 experts; the CUDA-graph capture still records
                    // every per-expert launch, so the replayed graph is identical to the unrolled form. This is
                    // the DENSE dequant path (computes all E experts, weighted by the top-K-masked router).
                    launches.push_str(&format!(
                        "  moe_gemv<<<({ne}+255)/256,256>>>(buf_{x},buf_{out}_Wr_q,buf_{out}_Wr_s,moe_logits,{ne},{d});\n\
                         \x20 moe_topk<<<1,{etopk}>>>(moe_logits,moe_w,{ne},{k});\n\
                         \x20 moe_zero<<<({d}+255)/256,256>>>(buf_{out},{d});\n\
                         \x20 for(int _e=0;_e<{ne};++_e){{\n\
                         \x20   moe_gemv<<<({dff}+255)/256,256>>>(buf_{x},buf_{out}_Wg_q+(long)_e*{gstride},buf_{out}_Wg_s+(long)_e*{gsstride},moe_gate_e,{dff},{d});\n\
                         \x20   moe_gemv<<<({dff}+255)/256,256>>>(buf_{x},buf_{out}_Wu_q+(long)_e*{gstride},buf_{out}_Wu_s+(long)_e*{gsstride},moe_up_e,{dff},{d});\n\
                         \x20   moe_silu_had<<<({dff}+255)/256,256>>>(moe_gate_e,moe_up_e,moe_he,{dff});\n\
                         \x20   moe_gemv<<<({d}+255)/256,256>>>(moe_he,buf_{out}_Wd_q+(long)_e*{dstride},buf_{out}_Wd_s+(long)_e*{dsstride},moe_oute,{d},{dff});\n\
                         \x20   moe_combine<<<({d}+255)/256,256>>>(buf_{out},moe_oute,moe_w,_e,{d});\n\
                         \x20 }}\n",
                        gstride = dff * d, gsstride = dff * dnb, dstride = d * dff, dsstride = d * fnb, etopk = etopk
                    ));
                }
            }
            Sym::Prim(TensorOpSpec::Attention(a)) => {
                assert!(a.softcap.is_none(), "the forward's shim admits no soft-capped attention");
                let (q, kc, vc, out, h, kv, hd, l) = (&a.q, &a.kc, &a.vc, &a.out, &a.h, &a.kv, &a.hd, &a.l);
                // Single-query multi-head GQA attention over the L-length KV cache — ONE parallel kernel, a BLOCK
                // per query head. Each block: dot the query head against every cached key of its GQA kv-head →
                // `sc[0..L]`, block-softmax (shuffle+shared reduce, exp in place), then the weighted sum of the
                // cached values → `ctx[head]`. `scale = 1/√hd`. This is the batch-1 decode attention llama does.
                let (qn, kcn, vcn, on) = (
                    interner.resolve(*q),
                    interner.resolve(*kc),
                    interner.resolve(*vc),
                    interner.resolve(*out),
                );
                let scale = 1.0f64 / (*hd as f64).sqrt();
                // Fused activation quant: ctx feeds Wo (a q8 matmul). Each block owns head*HD..+HD contiguous ctx
                // elements — head*HD is a multiple of 32 (HD % 32 == 0), so every 32-lane quant block lies wholly
                // inside this head's region. Quantize inline so the Wo matmul skips its own qact.
                let qact_out = quantized;
                let qparam = if qact_out {
                    ", signed char* yq, float* ys"
                } else {
                    ""
                };
                let qepilogue = if qact_out {
                    "\x20 __syncthreads(); {{ int qlane=threadIdx.x&31, qwp=threadIdx.x>>5, qnw=blockDim.x>>5; int qnb=HD/32; int base=head*HD;\n\
                     \x20   for(int lb=qwp;lb<qnb;lb+=qnw){{ float v=ctx[base+lb*32+qlane], av=fabsf(v);\n\
                     \x20     for(int s=16;s>0;s>>=1) av=fmaxf(av,__shfl_down_sync(0xffffffffu,av,s)); av=__shfl_sync(0xffffffffu,av,0);\n\
                     \x20     float amax=av>1e-30f?av:1e-30f, scl=amax/127.0f; int gb=base/32+lb; if(qlane==0) ys[gb]=scl;\n\
                     \x20     int qq=(int)rintf(v/scl); if(qq>127)qq=127; if(qq<-128)qq=-128; yq[gb*32+qlane]=(signed char)qq; }} }}\n"
                } else {
                    ""
                };
                kernels.push_str(&format!(
                    "__global__ void k{i}_attn(const float* q, const float* kc, const float* vc, float* ctx, int L, int H, int KV, int HD, float scale{qparam}){{\n\
                     \x20 int head=blockIdx.x; if(head>=H) return; int kvh=head/(H/KV); const float* qh=q+head*HD;\n\
                     \x20 __shared__ float sc[{L}]; __shared__ float red[32];\n\
                     \x20 int lane=threadIdx.x&31, wp=threadIdx.x>>5, nw=(blockDim.x+31)>>5;\n\
                     \x20 for(int j=wp;j<L;j+=nw){{ const float* kj=kc+(long)j*KV*HD+kvh*HD; float s=0.0f; for(int e=lane;e<HD;e+=32) s+=qh[e]*kj[e]; for(int r=16;r>0;r>>=1) s+=__shfl_down_sync(0xffffffffu,s,r); if(lane==0) sc[j]=s*scale; }}\n\
                     \x20 __syncthreads();\n\
                     \x20 float m=-1e30f; for(int j=threadIdx.x;j<L;j+=blockDim.x) m=fmaxf(m,sc[j]);\n\
                     \x20 for(int s=16;s>0;s>>=1) m=fmaxf(m,__shfl_down_sync(0xffffffffu,m,s)); if(lane==0) red[wp]=m; __syncthreads();\n\
                     \x20 if(threadIdx.x==0){{ float mm=red[0]; for(int i=1;i<nw;++i) mm=fmaxf(mm,red[i]); red[0]=mm; }} __syncthreads(); float mx=red[0];\n\
                     \x20 float ss=0.0f; for(int j=threadIdx.x;j<L;j+=blockDim.x){{ float e=expf(sc[j]-mx); sc[j]=e; ss+=e; }}\n\
                     \x20 for(int s=16;s>0;s>>=1) ss+=__shfl_down_sync(0xffffffffu,ss,s); if(lane==0) red[wp]=ss; __syncthreads();\n\
                     \x20 if(threadIdx.x==0){{ float su=0.0f; for(int i=0;i<nw;++i) su+=red[i]; red[0]=su; }} __syncthreads(); float den=red[0];\n\
                     \x20 for(int e=threadIdx.x;e<HD;e+=blockDim.x){{ float acc=0.0f; for(int j=0;j<L;++j) acc+=sc[j]*vc[(long)j*KV*HD+kvh*HD+e]; ctx[head*HD+e]=acc/den; }}\n{qepilogue}}}\n",
                    L = l
                ));
                // 2-arg launch (grid=one block per query head, block=256) so the graph rewrite `>>>`→`,0,_st>>>`
                // yields the valid `<<<H,256,0,_st>>>` stream form; the `sc[L]` scores live in STATIC shared memory.
                let qarg = if qact_out {
                    format!(",buf_{on}_q8act,buf_{on}_q8scl")
                } else {
                    String::new()
                };
                launches.push_str(&format!(
                    "  k{i}_attn<<<{H},256>>>(buf_{qn},buf_{kcn},buf_{vcn},buf_{on},{L},{H},{KV},{HD},{scale}f{qarg});\n",
                    H = h, KV = kv, HD = hd, L = l, scale = scale
                ));
                if qact_out {
                    if !dp4a_scratch.iter().any(|(v, _, _)| v == on) {
                        let (mm, kk) = q8_fed[on];
                        dp4a_scratch.push((on.to_string(), mm, kk));
                    }
                    producer_quantized.insert(on.to_string());
                }
            }
            _ => unreachable!("the forward's shim links no `{}` whole", host.kind()),
        }
    }

    // ── host: one managed buffer per variable; fill inputs; chain launches; print the final output ───
    let final_out = outputs.last().cloned().unwrap();
    let final_size = sizes
        .iter()
        .find(|(v, _)| v == &final_out)
        .map(|(_, s)| *s)
        .unwrap_or(0);
    let mut allocs = String::new();
    let mut fills = String::new();
    for (v, sz) in &sizes {
        // A Q8_0 weight stays INT8-resident: allocate int8 quants + f32 block scales and load them from files
        // (never a 4×-larger f32 buffer). The matmul against it dequantizes in-kernel.
        if let Some(q) = q8.get(v) {
            let nb = q.in_dim / 32;
            let (qn, sn) = (q.in_dim * q.out_dim, q.out_dim * nb);
            allocs.push_str(&format!(
                "  signed char* buf_{v}_q; if(cudaMallocManaged(&buf_{v}_q,(size_t){qn})){{ fprintf(stderr,\"malloc q\\n\"); return 1; }}\n"
            ));
            fills.push_str(&format!(
                "  {{ FILE* f=fopen(\"{qp}\",\"rb\"); if(!f||fread(buf_{v}_q,1,(size_t){qn},f)!=(size_t){qn}){{ fprintf(stderr,\"q8 quants {v}\\n\"); return 1; }} fclose(f); }}\n",
                qp = q.quants
            ));
            if kind != Q8MatKind::Dequant {
                // Both coalesced paths (Int8Act, TwoPassInt8) store block scales as __half — the checkpoint's
                // NATIVE Q8_0 scale precision — halving scale bandwidth vs f32 (the f32 file is host-converted).
                allocs.push_str(&format!(
                    "  __half* buf_{v}_s; if(cudaMallocManaged(&buf_{v}_s,(size_t){sn}*sizeof(__half))){{ fprintf(stderr,\"malloc s\\n\"); return 1; }}\n"
                ));
                fills.push_str(&format!(
                    "  {{ float* _t=(float*)malloc((size_t){sn}*sizeof(float)); FILE* f=fopen(\"{sp}\",\"rb\"); if(!f||fread(_t,sizeof(float),(size_t){sn},f)!=(size_t){sn}){{ fprintf(stderr,\"q8 scales {v}\\n\"); return 1; }} fclose(f); for(size_t _i=0;_i<(size_t){sn};++_i) buf_{v}_s[_i]=__float2half(_t[_i]); free(_t); }}\n",
                    sp = q.scales
                ));
            } else {
                allocs.push_str(&format!(
                    "  float* buf_{v}_s; if(cudaMallocManaged(&buf_{v}_s,(size_t){sn}*sizeof(float))){{ fprintf(stderr,\"malloc s\\n\"); return 1; }}\n"
                ));
                fills.push_str(&format!(
                    "  {{ FILE* f=fopen(\"{sp}\",\"rb\"); if(!f||fread(buf_{v}_s,sizeof(float),(size_t){sn},f)!=(size_t){sn}){{ fprintf(stderr,\"q8 scales {v}\\n\"); return 1; }} fclose(f); }}\n",
                    sp = q.scales
                ));
            }
            continue;
        }
        allocs.push_str(&format!(
            "  {ty}* buf_{v}; if(cudaMallocManaged(&buf_{v}, (size_t){sz}*sizeof({ty}))){{ fprintf(stderr,\"malloc\\n\"); return 1; }}\n"
        ));
        if is_output(v) {
            continue;
        }
        // A WEIGHT handle: the tensor is too big to inline — its `count` float values are loaded from an
        // external file at runtime (the real Qwen weights, extracted from the GGUF checkpoint). This is the
        // mechanism the real-8B forward needs; the file is populated by the host loader / test.
        if let Some((path, count)) = weights.get(v) {
            fills.push_str(&format!(
                "  {{ FILE* wf=fopen(\"{path}\",\"rb\"); if(!wf){{ fprintf(stderr,\"weight open {v}\\n\"); return 1; }}\n\
                 \x20   size_t rd=fread(buf_{v},sizeof(float),(size_t){count},wf); fclose(wf);\n\
                 \x20   if(rd!=(size_t){count}){{ fprintf(stderr,\"weight size {v}\\n\"); return 1; }} }}\n"
            ));
            continue;
        }
        // Inputs with the program's own `Set item` values → the orchestrator computes the SAME thing the VM
        // does (not merely a deterministic fill). Unset elements default to 0 (the tensor's init).
        if let Some(vals) = inits.get(v) {
            let zero = if floaty { "0.0f" } else { "0" };
            fills.push_str(&format!(
                "  for(size_t p=0;p<(size_t){sz};++p) buf_{v}[p]={zero};\n"
            ));
            for (idx, val) in vals {
                if *idx >= *sz {
                    continue;
                }
                let lit = match (floaty, val) {
                    (true, InitVal::F(f)) => format!("(float){:?}", f),
                    (true, InitVal::I(n)) => format!("(float){n}"),
                    (false, InitVal::I(n)) => format!("{n}"),
                    (false, InitVal::F(f)) => format!("(long long){}", *f as i64),
                };
                fills.push_str(&format!("  buf_{v}[{idx}]={lit};\n"));
            }
            continue;
        }
        // Fallback: deterministic fill (no literal initializers recognized).
        let fill = if floaty {
            "buf_{v}[p] = (float)((int)(p%7)-3) * 0.5f;"
        } else {
            "buf_{v}[p] = (long long)((int)(p%7)-3);"
        };
        fills.push_str(&format!(
            "  for(size_t p=0;p<(size_t){sz};++p) {}\n",
            fill.replace("{v}", v)
        ));
    }
    // Int8-activation DP4A scratch: per activation feeding a DP4A matmul, the quantized int8 view + its f32
    // per-32-block scales; the `k{i}_qact` kernel fills them each launch (M×K int8, M×(K/32) f32). The
    // two-pass path adds the lo-residual byte view (the ~14-bit exactness).
    for (a, m_rows, k_dim) in &dp4a_scratch {
        let (qn, sn) = (m_rows * k_dim, m_rows * (k_dim / 32));
        allocs.push_str(&format!(
            "  signed char* buf_{a}_q8act; if(cudaMallocManaged(&buf_{a}_q8act,(size_t){qn})){{ fprintf(stderr,\"malloc qact\\n\"); return 1; }}\n"
        ));
        if kind == Q8MatKind::TwoPassInt8 {
            allocs.push_str(&format!(
                "  signed char* buf_{a}_q8actlo; if(cudaMallocManaged(&buf_{a}_q8actlo,(size_t){qn})){{ fprintf(stderr,\"malloc qactlo\\n\"); return 1; }}\n"
            ));
        }
        allocs.push_str(&format!(
            "  float* buf_{a}_q8scl; if(cudaMallocManaged(&buf_{a}_q8scl,(size_t){sn}*sizeof(float))){{ fprintf(stderr,\"malloc qscl\\n\"); return 1; }}\n"
        ));
    }
    // MoE grouped weights (per stage, keyed by output var) + shared scratch (one set reused across stages).
    let mut moe_scratch: Option<(usize, usize, usize)> = None; // (max_d, max_dff, max_E)
    for (_, op) in &kernel_ops {
        if let Some(TensorOpSpec::Moe(m)) = folded(op).0.prim() {
            let out = interner.resolve(m.out);
            let w = moe
                .get(out)
                .ok_or_else(|| ForwardError::MissingMoeWeights(out.to_string()))?;
            for (tag, q) in [
                ("Wr", &w.router),
                ("Wg", &w.gate),
                ("Wu", &w.up),
                ("Wd", &w.down),
            ] {
                let nb = q.in_dim / 32;
                let (qn, sn) = (q.in_dim * q.out_dim, q.out_dim * nb);
                allocs.push_str(&format!(
                    "  signed char* buf_{out}_{tag}_q; if(cudaMallocManaged(&buf_{out}_{tag}_q,(size_t){qn})){{ fprintf(stderr,\"malloc moe q\\n\"); return 1; }}\n\
                     \x20 float* buf_{out}_{tag}_s; if(cudaMallocManaged(&buf_{out}_{tag}_s,(size_t){sn}*sizeof(float))){{ fprintf(stderr,\"malloc moe s\\n\"); return 1; }}\n"
                ));
                fills.push_str(&format!(
                    "  {{ FILE* f=fopen(\"{qp}\",\"rb\"); if(!f||fread(buf_{out}_{tag}_q,1,(size_t){qn},f)!=(size_t){qn}){{ fprintf(stderr,\"moe q {out}_{tag}\\n\"); return 1; }} fclose(f); }}\n\
                     \x20 {{ FILE* f=fopen(\"{sp}\",\"rb\"); if(!f||fread(buf_{out}_{tag}_s,sizeof(float),(size_t){sn},f)!=(size_t){sn}){{ fprintf(stderr,\"moe s {out}_{tag}\\n\"); return 1; }} fclose(f); }}\n",
                    qp = q.quants, sp = q.scales
                ));
            }
            let (md, mdff, me) = moe_scratch.unwrap_or((0, 0, 0));
            moe_scratch = Some((md.max(m.d), mdff.max(m.dff), me.max(m.ne)));
        }
    }
    if let Some((md, mdff, me)) = moe_scratch {
        for (nm, n) in [
            ("moe_logits", me),
            ("moe_w", me),
            ("moe_gate_e", mdff),
            ("moe_up_e", mdff),
            ("moe_he", mdff),
            ("moe_oute", md),
        ] {
            allocs.push_str(&format!(
                "  float* {nm}; if(cudaMallocManaged(&{nm},(size_t){n}*sizeof(float))){{ fprintf(stderr,\"malloc moe scratch\\n\"); return 1; }}\n"
            ));
        }
        if kind != Q8MatKind::Dequant {
            // Deeply-fused path scratch: just the top-K expert-index array + the K-expert-wide SwiGLU
            // intermediate `h` (`K·dff`, sized to the max over stages). The gate/up/per-expert-output buffers of
            // the old 6-kernel path are gone — those intermediates never leave registers now.
            allocs.push_str(&format!("  int* moe_idx; if(cudaMallocManaged(&moe_idx,(size_t){me}*sizeof(int))){{ fprintf(stderr,\"malloc idx\\n\"); return 1; }}\n"));
            allocs.push_str(&format!("  float* moe_he_all; if(cudaMallocManaged(&moe_he_all,(size_t){n}*sizeof(float))){{ fprintf(stderr,\"malloc moe fused\\n\"); return 1; }}\n", n = me * mdff));
            // int8 scratch: the shared block input `x` (Kd=d) and the SwiGLU intermediate `h` (K·dff), each
            // quantized once per stage into an int8 view + a per-32-block f32 scale; the two-pass path adds
            // the lo residual byte view.
            allocs.push_str(&format!("  signed char* moe_xqh; if(cudaMallocManaged(&moe_xqh,(size_t){md})){{ fprintf(stderr,\"malloc xqh\\n\"); return 1; }}\n", md = md));
            allocs.push_str(&format!("  float* moe_xs; if(cudaMallocManaged(&moe_xs,(size_t)(({md}/32)+1)*sizeof(float))){{ fprintf(stderr,\"malloc xs\\n\"); return 1; }}\n", md = md));
            allocs.push_str(&format!("  signed char* moe_hqh; if(cudaMallocManaged(&moe_hqh,(size_t){n})){{ fprintf(stderr,\"malloc hqh\\n\"); return 1; }}\n", n = me * mdff));
            allocs.push_str(&format!("  float* moe_hs; if(cudaMallocManaged(&moe_hs,(size_t)(({n}/32)+1)*sizeof(float))){{ fprintf(stderr,\"malloc hs\\n\"); return 1; }}\n", n = me * mdff));
            if kind == Q8MatKind::TwoPassInt8 {
                allocs.push_str(&format!("  signed char* moe_xql; if(cudaMallocManaged(&moe_xql,(size_t){md})){{ fprintf(stderr,\"malloc xql\\n\"); return 1; }}\n", md = md));
                allocs.push_str(&format!("  signed char* moe_hql; if(cudaMallocManaged(&moe_hql,(size_t){n})){{ fprintf(stderr,\"malloc hql\\n\"); return 1; }}\n", n = me * mdff));
            }
        }
    }
    let printf = if floaty {
        "printf(\"%.6f\\n\", (double)buf_{v}[p]);"
    } else {
        "printf(\"%lld\\n\", (long long)buf_{v}[p]);"
    };
    let print_final = format!(
        "  for(size_t p=0;p<(size_t){final_size};++p) {}\n",
        printf.replace("{v}", &final_out)
    );

    // Every launch runs on a dedicated stream so the whole forward can be captured as ONE CUDA graph
    // (all launches are 2-arg `<<<grid,block>>>` with static shared memory ⇒ the blanket rewrite yields the
    // 4-arg `<<<grid,block,0,_st>>>` stream form). Graph replay collapses the hundreds of per-token kernel
    // launches into a single dispatch — the last step past llama.cpp's tok/s (mirrors cuda/qwen_forward.cu).
    let launches_st = launches.replace(">>>", ",0,_st>>>");
    // ── Flamegraph pins ─────────────────────────────────────────────────────────────────────────────────
    // GPU hardware counters are perms-gated on cloud pods (`ncu` → ERR_NVGPUCTRPERM), so we time on the HOST:
    // an optional pass (runtime-gated on `LOGOS_CUDA_PINS`) re-runs the forward once with a CUDA-event bracket
    // around EVERY kernel launch, accumulating GPU time into per-phase buckets. Emits folded `PIN <phase> <ms>`
    // lines to stderr → `scripts/accel/cuda-pins-flamegraph.sh` renders a flamegraph SVG. Zero cost when unset.
    let bucket = |k: &str| -> usize {
        if k.contains("moe_router") || k.contains("moe_topk") {
            3
        } else if k.contains("moe_qact") {
            4
        } else if k.contains("moe_gate_up")
            || k.contains("moe_down")
            || k.contains("moe_silu")
            || k.contains("moe_combine")
            || k.contains("moe_zero")
            || k.contains("moe_gemv")
        {
            5
        } else if k.contains("_attn") {
            1
        } else if k.contains("_q8dp4a") {
            2
        } else if k.contains("_add_row") || k.contains("_row") {
            0
        } else if k.contains("_qact") {
            6
        } else {
            7
        }
    };
    let mut pinned = String::new();
    for line in launches.lines() {
        let t = line.trim_start();
        if let Some(pos) = t.find("<<<") {
            let kname = t[..pos]
                .rsplit(|c: char| c == ' ' || c == '{' || c == '}' || c == ';')
                .find(|s| !s.is_empty())
                .unwrap_or("");
            let b = bucket(kname);
            let ln = line.replace(">>>", ",0,_st>>>");
            pinned.push_str(&format!(
                "  cudaEventRecord(_pa,_st); {} cudaEventRecord(_pb,_st); cudaEventSynchronize(_pb); {{ float _dt=0; cudaEventElapsedTime(&_dt,_pa,_pb); _pin[{b}]+=_dt; _cnt[{b}]++; }}\n",
                ln.trim_start()
            ));
        } else if !t.is_empty() {
            pinned.push_str(&line.replace(">>>", ",0,_st>>>"));
            pinned.push('\n');
        }
    }
    let pins_pass = format!(
        "\x20 if(getenv(\"LOGOS_CUDA_PINS\")){{\n\
         \x20   const char* _bn[8]={{\"norm+quant\",\"attention\",\"attn_proj+lmhead\",\"moe_router+topk\",\"moe_qact\",\"moe_ffn\",\"qact\",\"other\"}};\n\
         \x20   double _pin[8]={{0}}; long _cnt[8]={{0}}; cudaEvent_t _pa,_pb; cudaEventCreate(&_pa); cudaEventCreate(&_pb);\n\
         {pinned}\
         \x20   double _tot=0; for(int _b=0;_b<8;++_b) _tot+=_pin[_b];\n\
         \x20   for(int _b=0;_b<8;++_b) fprintf(stderr,\"PIN %-18s %8.4f ms  x%-5ld %5.1f%%\\n\",_bn[_b],_pin[_b],_cnt[_b], _tot>0?100.0*_pin[_b]/_tot:0.0);\n\
         \x20   fprintf(stderr,\"PIN %-18s %8.4f ms  (%.1f tok/s single-pass, event-serialized)\\n\",\"TOTAL\",_tot, _tot>0?1000.0/_tot:0.0);\n\
         \x20 }}\n"
    );
    let prov = super::provenance::header(super::provenance::Provenance::Compiled);
    Ok(format!(
        "{prov}// A multi-stage tensor-op forward composed as chained PARALLEL kernels through shared device buffers\n\
         // ({n} stages), captured as one CUDA graph for replay.\n\
         #include <cstdio>\n#include <cstdlib>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n#include <cmath>\n\n\
         {kernels}\nint main(void){{\n{allocs}{fills}\n\
         \x20 cudaStream_t _st; if(cudaStreamCreate(&_st)!=cudaSuccess){{ fprintf(stderr,\"stream\\n\"); return 1; }}\n\
         \x20 // Warm-up launch: runs the forward once, migrating the managed weight pages resident on the GPU.\n\
         {launches_st}\
         \x20 if(cudaStreamSynchronize(_st)!=cudaSuccess){{ fprintf(stderr,\"sync\\n\"); return 1; }}\n\
         {pins_pass}\
         \x20 // Capture the entire forward into a single CUDA graph, then instantiate + replay it.\n\
         \x20 cudaGraph_t _g; cudaGraphExec_t _ge;\n\
         \x20 if(cudaStreamBeginCapture(_st, cudaStreamCaptureModeGlobal)!=cudaSuccess){{ fprintf(stderr,\"capture\\n\"); return 1; }}\n\
         {launches_st}\
         \x20 if(cudaStreamEndCapture(_st, &_g)!=cudaSuccess){{ fprintf(stderr,\"endcap\\n\"); return 1; }}\n\
         \x20 if(cudaGraphInstantiate(&_ge, _g, 0)!=cudaSuccess){{ fprintf(stderr,\"instantiate\\n\"); return 1; }}\n\
         \x20 cudaGraphLaunch(_ge, _st); cudaStreamSynchronize(_st);\n\
         \x20 cudaEvent_t _e0,_e1; cudaEventCreate(&_e0); cudaEventCreate(&_e1);\n\
         \x20 cudaEventRecord(_e0,_st); for(int _it=0;_it<20;++_it){{ cudaGraphLaunch(_ge,_st); }}\n\
         \x20 cudaEventRecord(_e1,_st); cudaStreamSynchronize(_st); float _ms=0.0f; cudaEventElapsedTime(&_ms,_e0,_e1);\n\
         \x20 fprintf(stderr,\"FORWARD_MS %f\\n\", _ms/20.0f);\n\
         {print_final}  return 0;\n}}\n",
        n = stage_count
    ))
}

/// The Qwen3-30B-A3B architecture (n_layer is overridable to keep local emit/compile fast; the per-layer shape
/// is identical to the real model).
pub struct Qwen30bCfg {
    pub d: usize,      // hidden (n_embd) = 2048
    pub dff: usize,    // moe expert intermediate = 768
    pub ne: usize,     // experts = 128
    pub k: usize,      // active experts = 8
    pub layers: usize, // blocks (48 real)
    pub h: usize,      // query heads = 32
    pub kv: usize,     // KV heads (GQA) = 4
    pub hd: usize,     // head dim = 128
    pub vocab: usize,  // 151936
    /// RoPE base frequency (`<arch>.rope.freq_base` in the GGUF). **Read it from the checkpoint** — it was a
    /// hardcoded `1e6` in the emitted CUDA until Qwen3.8-27B arrived with `1e7`, which would have silently
    /// mis-rotated every query and key while still producing plausible-looking tokens.
    pub rope_theta: f64,
    /// RMSNorm epsilon (`<arch>.attention.layer_norm_rms_epsilon`). Also formerly hardcoded.
    pub rms_eps: f64,
    /// Split-K factor for the flash-attention lane. Per-chip tuning knob, not a model property: measured best
    /// at 4 on the A100 and 8 as the portable default.
    pub attn_split: usize,
}

impl Qwen30bCfg {
    pub fn real() -> Self {
        Qwen30bCfg {
            d: 2048,
            dff: 768,
            ne: 128,
            k: 8,
            layers: 48,
            h: 32,
            kv: 4,
            hd: 128,
            vocab: 151936,
            rope_theta: 1.0e6,
            rms_eps: 1e-6,
            attn_split: 8,
        }
    }
}

/// A placeholder or real Q8 file pair under `dir`.
fn q8ref(dir: &str, tag: &str, inn: usize, out: usize) -> Q8Ref {
    Q8Ref {
        quants: format!("{dir}/{tag}.q"),
        scales: format!("{dir}/{tag}.s"),
        in_dim: inn,
        out_dim: out,
    }
}

/// Generate the full Qwen3-30B-A3B forward as a LOGOS `.lg` string, plus the manifests the emitter needs:
/// `weights` (f32 embedding + RMSNorm weights), `q8` (attention V/O + LM head, int8), `moe` (per-layer FFN,
/// keyed by the block's FFN output var). `dir` is where the weight files are (real `/workspace/...` on the pod,
/// a placeholder for compile-only use). This is THE canonical authored-in-LOGOS 30B program: the pod run, the
/// local emit lock, and every printed copy of the program come from it.
/// The device-state kernels for autoregressive generation — a mirror of the proven 8B `cuda/qwen_forward.cu`
/// decode loop, with f32 block scales (the extractor writes f32 `.s` files; scales are a negligible bandwidth
/// fraction). Position + token live in device memory (`d_pos`/`d_tok`) so ONE captured graph replays every token.
/// The GEMVs are the SAME uint4-coalesced DP4A kernels that beat llama on the single-token forward.
/// ★ M5: the compiler EMITS the certified Marlin-class Q8_0 GEMM (it is not a hand-written `.cu`). Given the
/// auto-tuned subtile count `nt` (the fastest golden config the auto-tuner picked for this shape/GPU — M2), this
/// lowers the recognized `matmul` of a Q8_0 weight into a register-dequant tensor-core kernel:
/// `mma.sync.m16n8k32.s8` (int8 tensor core), the mma-k==32 aligned to the Q8_0 scale block (SAT-certified, M3),
/// per-block dequant scale applied in REGISTERS in the same float grouping as our dp4a GEMV (golden-bounded to
/// 4.2e-7, M4). This is the fusion pass in action: GEMM + per-block-dequant + scale fused into ONE kernel, and
/// the compiler right-sizes `nt` per shape rather than a hand-tuned constant. Beats dp4a's 7.5x wall on batch.
pub fn emit_marlin_q8_gemm(nt: usize) -> String {
    format!(
        r#"// AUTO-TUNED (nt={nt}) + SAT-CERTIFIED (fragment bijection, k==Q8_0-block==32, no int32 overflow) + GOLDEN
// (<=4.2e-7 vs dp4a: int32 exact, only f32 reassociation). Register-dequant tensor-core Q8_0 GEMM, emitted by our
// compiler from the recognized `A matmul Wq8` op -- the world's-smartest-compiler Marlin lowering.
__global__ void gk_marlin_q8(const signed char* Xq,const float* Xs,const signed char* Wq,const float* Ws,float* Y,int B,int N,int K){{
  const int NT={nt}; int KB=K/32;
  int warp=(blockIdx.x*blockDim.x+threadIdx.x)>>5; int lane=threadIdx.x&31;
  int ntiles=N/(8*NT); int mtile=warp/ntiles, ntile=warp%ntiles; int m0=mtile*16, n0=ntile*8*NT;
  int gid=lane>>2, tid=lane&3;
  float d[NT][4]; for(int j=0;j<NT;j++){{d[j][0]=d[j][1]=d[j][2]=d[j][3]=0;}}
  for(int kb=0;kb<KB;kb++){{ int k0=kb*32;
    int a0=*(const int*)(Xq+(long)(m0+gid)*K+k0+tid*4);
    int a1=*(const int*)(Xq+(long)(m0+gid+8)*K+k0+tid*4);
    int a2=*(const int*)(Xq+(long)(m0+gid)*K+k0+16+tid*4);
    int a3=*(const int*)(Xq+(long)(m0+gid+8)*K+k0+16+tid*4);
    float xs0=Xs[(long)(m0+gid)*KB+kb], xs1=Xs[(long)(m0+gid+8)*KB+kb];
    #pragma unroll
    for(int j=0;j<NT;j++){{ int nn=n0+j*8;
      int b0=*(const int*)(Wq+(long)(nn+gid)*K+k0+tid*4);
      int b1=*(const int*)(Wq+(long)(nn+gid)*K+k0+16+tid*4);
      int c0=0,c1=0,c2=0,c3=0;
      asm volatile("mma.sync.aligned.m16n8k32.row.col.s32.s8.s8.s32 {{%0,%1,%2,%3}}, {{%4,%5,%6,%7}}, {{%8,%9}}, {{%0,%1,%2,%3}};"
        : "+r"(c0),"+r"(c1),"+r"(c2),"+r"(c3) : "r"(a0),"r"(a1),"r"(a2),"r"(a3), "r"(b0),"r"(b1));
      float ws0=Ws[(long)(nn+tid*2)*KB+kb], ws1=Ws[(long)(nn+tid*2+1)*KB+kb];
      d[j][0]+=(float)c0*(ws0*xs0); d[j][1]+=(float)c1*(ws1*xs0);
      d[j][2]+=(float)c2*(ws0*xs1); d[j][3]+=(float)c3*(ws1*xs1);
    }}
  }}
  for(int j=0;j<NT;j++){{ int nn=n0+j*8; int r0=m0+gid,r1=m0+gid+8,cc0=nn+tid*2,cc1=nn+tid*2+1;
    if(r0<B){{ if(cc0<N)Y[(long)r0*N+cc0]=d[j][0]; if(cc1<N)Y[(long)r0*N+cc1]=d[j][1]; }}
    if(r1<B){{ if(cc0<N)Y[(long)r1*N+cc0]=d[j][2]; if(cc1<N)Y[(long)r1*N+cc1]=d[j][3]; }} }}
}}
"#
    )
}

pub const GEN_DEVICE_KERNELS: &str = r#"
__global__ void gk_embed(float* x, const signed char* wq, const float* ws, const int* d_tok, int E){
  int t=blockIdx.x*blockDim.x+threadIdx.x; if(t>=E) return;
  long tok=*d_tok; x[t]=(float)wq[tok*E+t]*ws[tok*(E/32)+t/32];
}
__global__ void gk_rmsnorm(float* out, const float* x, const float* w, int n, float eps){
  __shared__ float sh[256]; float s=0; for(int i=threadIdx.x;i<n;i+=blockDim.x) s+=x[i]*x[i];
  sh[threadIdx.x]=s; __syncthreads();
  for(int k=blockDim.x/2;k>0;k>>=1){ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }
  float sc=rsqrtf(sh[0]/n+eps); for(int i=threadIdx.x;i<n;i+=blockDim.x) out[i]=x[i]*sc*w[i];
}
// FUSED residual-add + RMSNorm: `x += y` (the residual, kept for the next block) AND `out = rmsnorm(x)*w`, in
// ONE kernel launch and ONE pass over x. The reduction is the SAME 256-thread tree as gk_rmsnorm over the SAME
// summed values (v = x[i]+y[i]), so the float sum is bit-identical ⇒ token-exact. Removes a kernel node and the
// residual write/re-read per layer — the win in the latency-bound batch-1 H100 decode.
__global__ void gk_add_rmsnorm(float* x, const float* y, float* out, const float* w, int n, float eps){
  __shared__ float sh[256]; float s=0; for(int i=threadIdx.x;i<n;i+=blockDim.x){ float v=x[i]+y[i]; x[i]=v; s+=v*v; }
  sh[threadIdx.x]=s; __syncthreads();
  for(int k=blockDim.x/2;k>0;k>>=1){ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }
  float sc=rsqrtf(sh[0]/n+eps); for(int i=threadIdx.x;i<n;i+=blockDim.x) out[i]=x[i]*sc*w[i];
}
// FUSED residual-add + RMSNorm + INT8 QUANT: like gk_add_rmsnorm, but ALSO emits `out` quantized to int8 per
// 32-block (oq + per-block scale os_) in the SAME kernel — folding the standalone `moe_qact(x2n)` away. `out`
// (float) still feeds the router; `oq`/`os_` (int8) feed the experts. The quant replicates moe_qact's per-32
// amax/127 scheme EXACTLY (one warp per 32-block, shfl amax) ⇒ the experts see bit-identical int8 (token-exact).
__global__ void gk_add_rmsnorm_q8(float* x, const float* y, float* out, const float* w, signed char* oq, float* os_, int n, float eps){
  __shared__ float sh[256]; float s=0; for(int i=threadIdx.x;i<n;i+=blockDim.x){ float v=x[i]+y[i]; x[i]=v; s+=v*v; }
  sh[threadIdx.x]=s; __syncthreads();
  for(int k=blockDim.x/2;k>0;k>>=1){ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }
  float scale=rsqrtf(sh[0]/n+eps); for(int i=threadIdx.x;i<n;i+=blockDim.x) out[i]=x[i]*scale*w[i];
  __syncthreads();
  int warp=threadIdx.x>>5, lane=threadIdx.x&31, nb=n/32;
  for(int b=warp;b<nb;b+=(blockDim.x>>5)){ float v=out[b*32+lane], av=fabsf(v);
    for(int t=16;t>0;t>>=1) av=fmaxf(av,__shfl_down_sync(0xffffffffu,av,t));
    av=__shfl_sync(0xffffffffu,av,0); float amax=av>1e-30f?av:1e-30f; float qs=amax/127.0f;
    if(lane==0) os_[b]=qs; int q=(int)rintf(v/qs); if(q>127)q=127; if(q<-128)q=-128; oq[b*32+lane]=(signed char)q; }
}
__global__ void gk_rmsnorm_q8(const float* x, signed char* xq, float* xs, const float* w, int n, float eps){
  __shared__ float red[256]; float s=0; for(int i=threadIdx.x;i<n;i+=blockDim.x) s+=x[i]*x[i];
  red[threadIdx.x]=s; __syncthreads();
  for(int k=128;k>0;k>>=1){ if(threadIdx.x<k) red[threadIdx.x]+=red[threadIdx.x+k]; __syncthreads(); }
  float rms=rsqrtf(red[0]/n+eps); int warp=threadIdx.x>>5, lane=threadIdx.x&31, nb=n/32;
  for(int b=warp;b<nb;b+=8){ float y=x[b*32+lane]*rms*w[b*32+lane], a=fabsf(y);
    for(int sh=16;sh>0;sh>>=1) a=fmaxf(a,__shfl_down_sync(0xffffffffu,a,sh));
    a=__shfl_sync(0xffffffffu,a,0); float sc=a>0?a/127.0f:1.0f;
    if(lane==0) xs[b]=sc; xq[b*32+lane]=(signed char)__float2int_rn(y/sc); }
}
// FUSED residual-add + RMSNorm + int8 quant for the ATTENTION norm: `x += y` (residual kept) AND quantize
// rmsnorm(x)*w to int8 (xq/xs) — folds the post-MoE `gk_add` into the NEXT layer's attn norm+quant. Identical
// 128-thread reduction + per-32 quant as gk_rmsnorm_q8 over v=x[i]+y[i] ⇒ bit-identical ⇒ token-exact.
__global__ void gk_add_rmsnorm_q8a(float* x, const float* y, signed char* xq, float* xs, const float* w, int n, float eps){
  __shared__ float red[256]; float s=0; for(int i=threadIdx.x;i<n;i+=blockDim.x){ float v=x[i]+y[i]; x[i]=v; s+=v*v; }
  red[threadIdx.x]=s; __syncthreads();
  for(int k=128;k>0;k>>=1){ if(threadIdx.x<k) red[threadIdx.x]+=red[threadIdx.x+k]; __syncthreads(); }
  float rms=rsqrtf(red[0]/n+eps); int warp=threadIdx.x>>5, lane=threadIdx.x&31, nb=n/32;
  for(int b=warp;b<nb;b+=8){ float y2=x[b*32+lane]*rms*w[b*32+lane], a=fabsf(y2);
    for(int sh=16;sh>0;sh>>=1) a=fmaxf(a,__shfl_down_sync(0xffffffffu,a,sh));
    a=__shfl_sync(0xffffffffu,a,0); float sc=a>0?a/127.0f:1.0f;
    if(lane==0) xs[b]=sc; xq[b*32+lane]=(signed char)__float2int_rn(y2/sc); }
}
__global__ void gk_quant_act(const float* x, signed char* xq, float* xs, int K){
  int blk=blockIdx.x*(blockDim.x/32)+threadIdx.x/32; if((long)blk*32>=K) return; int lane=threadIdx.x&31;
  float v=x[blk*32+lane], a=fabsf(v);
  for(int s=16;s>0;s>>=1) a=fmaxf(a,__shfl_down_sync(0xffffffffu,a,s)); a=__shfl_sync(0xffffffffu,a,0);
  float sc=a>0?a/127.0f:1.0f; if(lane==0) xs[blk]=sc; xq[blk*32+lane]=(signed char)__float2int_rn(v/sc);
}
__global__ void gk_gemv(float* y, const signed char* wq, const float* ws, const signed char* xq, const float* xs, int K, int N){
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31;
  const int4* w16=(const int4*)(wq+(long)o*K); const int4* x16=(const int4*)xq; const float* wsr=ws+(long)o*(K/32);
  float acc=0.f; int nq=K/512;
  for(int b=0;b<nq;b++){ int4 w=w16[b*32+lane], xx=x16[b*32+lane]; int blk=b*16+lane/2;
    int d=__dp4a(w.x,xx.x,0)+__dp4a(w.y,xx.y,0)+__dp4a(w.z,xx.z,0)+__dp4a(w.w,xx.w,0);
    acc += (float)d * (wsr[blk]*xs[blk]); }
  int done=nq*512; for(int j=done+lane;j<K;j+=32){ int blk=j/32; acc += (float)((int)xq[j]*(int)wq[(long)o*K+j])*(wsr[blk]*xs[blk]); }
  for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s); if(lane==0) y[o]=acc;
}
__global__ void gk_gemv_kv(float* cache, const int* d_pos, int KD, const signed char* wq, const float* ws, const signed char* xq, const float* xs, int K, int N){
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31;
  const int4* w16=(const int4*)(wq+(long)o*K); const int4* x16=(const int4*)xq; const float* wsr=ws+(long)o*(K/32);
  float acc=0.f; int nq=K/512;
  for(int b=0;b<nq;b++){ int4 w=w16[b*32+lane], xx=x16[b*32+lane]; int blk=b*16+lane/2;
    int d=__dp4a(w.x,xx.x,0)+__dp4a(w.y,xx.y,0)+__dp4a(w.z,xx.z,0)+__dp4a(w.w,xx.w,0);
    acc += (float)d * (wsr[blk]*xs[blk]); }
  int done=nq*512; for(int j=done+lane;j<K;j+=32){ int blk=j/32; acc += (float)((int)xq[j]*(int)wq[(long)o*K+j])*(wsr[blk]*xs[blk]); }
  for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s); if(lane==0) cache[(long)(*d_pos)*KD+o]=acc;
}
// FUSED Q/K/V projection: one launch over QD+2·KD output warps, each picking its weight matrix by output range
// (Q→qbuf, K→cache[pos], V→cache[pos]) and reading the SHARED int8 activation once. Replaces three separate
// launch-bound gemvs with one — measured the single largest decode-throughput win (A100: +7 tok/s), because
// batch-1 decode is dominated by per-kernel launch/latency, not weight bandwidth.
__global__ void gk_gemv_qkv(float* qbuf,float* kc,float* vc,const int* d_pos,int KD,
    const signed char* wqq,const float* wqs,const signed char* wkq,const float* wks,const signed char* wvq,const float* wvs,
    const signed char* xq,const float* xs,int K,int QD){
  int og=(blockIdx.x*blockDim.x+threadIdx.x)>>5; int lane=threadIdx.x&31; int Ntot=QD+2*KD; if(og>=Ntot) return;
  const signed char* wq; const float* ws; int o; float* dst; long dsti;
  if(og<QD){ wq=wqq; ws=wqs; o=og; dst=qbuf; dsti=o; }
  else if(og<QD+KD){ wq=wkq; ws=wks; o=og-QD; dst=kc; dsti=(long)(*d_pos)*KD+o; }
  else { wq=wvq; ws=wvs; o=og-QD-KD; dst=vc; dsti=(long)(*d_pos)*KD+o; }
  const int4* w16=(const int4*)(wq+(long)o*K); const int4* x16=(const int4*)xq; const float* wsr=ws+(long)o*(K/32);
  float acc=0.f; int nq=K/512;
  for(int b=0;b<nq;b++){ int4 w=w16[b*32+lane], xx=x16[b*32+lane]; int blk=b*16+lane/2;
    int d=__dp4a(w.x,xx.x,0)+__dp4a(w.y,xx.y,0)+__dp4a(w.z,xx.z,0)+__dp4a(w.w,xx.w,0);
    acc += (float)d * (wsr[blk]*xs[blk]); }
  int done=nq*512; for(int j=done+lane;j<K;j+=32){ int blk=j/32; acc += (float)((int)xq[j]*(int)wq[(long)o*K+j])*(wsr[blk]*xs[blk]); }
  for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s); if(lane==0) dst[dsti]=acc;
}
__global__ void gk_qk_norm_rope(float* base, const int* d_pos, int stride, const float* nw, int n_heads, int hd, float rbase, float eps){
  int head=blockIdx.x; if(head>=n_heads) return; float* h=base+(long)(*d_pos)*stride+head*hd;
  __shared__ float sh[128]; float s=0; for(int i=threadIdx.x;i<hd;i+=blockDim.x) s+=h[i]*h[i]; sh[threadIdx.x]=s; __syncthreads();
  for(int k=blockDim.x/2;k>0;k>>=1){ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }
  float sc=rsqrtf(sh[0]/hd+eps); __syncthreads();
  for(int i=threadIdx.x;i<hd;i+=blockDim.x) h[i]=h[i]*sc*nw[i]; __syncthreads();
  int half=hd/2; int pos=*d_pos;
  for(int i=threadIdx.x;i<half;i+=blockDim.x){ float f=powf(rbase,-2.0f*i/hd); float a=pos*f,c=cosf(a),si=sinf(a);
    float x0=h[i],x1=h[i+half]; h[i]=x0*c-x1*si; h[i+half]=x0*si+x1*c; }
}
// FUSED Q+K per-head RMSNorm+RoPE: one launch over NH+NKV head-blocks, picking Q params (block<NH) or K params
// (block≥NH). Replaces two underfilled launches (NH and NKV blocks) with one — the Q and K norm-rope are the
// same body over different (base,stride,weight), so a branch on blockIdx fuses them (A100: +4 tok/s).
__global__ void gk_qk_norm_rope2(float* qbase, float* kbase, const int* d_pos, int kstride, const float* qnw, const float* knw, int nh, int nkv, int hd, float rbase, float eps){
  int b=blockIdx.x; float* base; const float* nw; int stride; int head;
  if(b<nh){ base=qbase; nw=qnw; stride=0; head=b; } else { base=kbase; nw=knw; stride=kstride; head=b-nh; }
  float* h=base+(long)(*d_pos)*stride+head*hd;
  __shared__ float sh[128]; float s=0; for(int i=threadIdx.x;i<hd;i+=blockDim.x) s+=h[i]*h[i]; sh[threadIdx.x]=s; __syncthreads();
  for(int k=blockDim.x/2;k>0;k>>=1){ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }
  float sc=rsqrtf(sh[0]/hd+eps); __syncthreads();
  for(int i=threadIdx.x;i<hd;i+=blockDim.x) h[i]=h[i]*sc*nw[i]; __syncthreads();
  int half=hd/2; int pos=*d_pos;
  for(int i=threadIdx.x;i<half;i+=blockDim.x){ float f=powf(rbase,-2.0f*i/hd); float a=pos*f,c=cosf(a),si=sinf(a);
    float x0=h[i],x1=h[i+half]; h[i]=x0*c-x1*si; h[i+half]=x0*si+x1*c; }
}
// Flash-attention-style single-query decode: one block per query head, a WARP per cached key (parallel qh·kj
// dot, warp-reduced), a parallel block-softmax (shuffle max/sum + shared cross-warp reduce), then a parallel
// weighted sum over V. Q is normed+roped by the preceding gk_qk_norm_rope; fusing it in here perturbs the float
// order enough to flip a precise near-tie AND doesn't cut graph time (the work just moves) — so kept separate.
__global__ void gk_attn(float* out, const float* q, const float* kc, const float* vc, const int* d_pos, int n_head, int n_kv, int hd, float scale){
  int head=blockIdx.x; if(head>=n_head) return; int kvh=head/(n_head/n_kv); int L=*d_pos+1; int KD=n_kv*hd;
  const float* qh=q+(long)head*hd; extern __shared__ float sc[]; __shared__ float red[32];
  int lane=threadIdx.x&31, wp=threadIdx.x>>5, nw=(blockDim.x+31)>>5;
  for(int j=wp;j<L;j+=nw){ const float* kj=kc+(long)j*KD+kvh*hd; float s=0.f; for(int e=lane;e<hd;e+=32) s+=qh[e]*kj[e];
    for(int r=16;r>0;r>>=1) s+=__shfl_down_sync(0xffffffffu,s,r); if(lane==0) sc[j]=s*scale; }
  __syncthreads();
  float m=-1e30f; for(int j=threadIdx.x;j<L;j+=blockDim.x) m=fmaxf(m,sc[j]);
  for(int r=16;r>0;r>>=1) m=fmaxf(m,__shfl_down_sync(0xffffffffu,m,r)); if(lane==0) red[wp]=m; __syncthreads();
  if(threadIdx.x==0){ float mm=red[0]; for(int i=1;i<nw;i++) mm=fmaxf(mm,red[i]); red[0]=mm; } __syncthreads(); float mx=red[0];
  float ss=0.f; for(int j=threadIdx.x;j<L;j+=blockDim.x){ float e=expf(sc[j]-mx); sc[j]=e; ss+=e; }
  for(int r=16;r>0;r>>=1) ss+=__shfl_down_sync(0xffffffffu,ss,r); if(lane==0) red[wp]=ss; __syncthreads();
  if(threadIdx.x==0){ float su=0.f; for(int i=0;i<nw;i++) su+=red[i]; red[0]=su; } __syncthreads(); float den=red[0];
  for(int e=threadIdx.x;e<hd;e+=blockDim.x){ float a=0.f; for(int j=0;j<L;j++) a+=sc[j]*vc[(long)j*KD+kvh*hd+e]; out[(long)head*hd+e]=a/den; }
}
// ONLINE-SOFTMAX flash-attention decode (llama's fattn-vec shape): one block per query head, warps stream
// DISJOINT slabs of the KV cache, each keeping a RUNNING (max, sum) + weighted-V accumulator in REGISTERS —
// a single pass, warp-shuffle score reduce (no per-key __syncthreads, no shared score array), then ONE
// cross-warp flash-combine. Q lives in registers. Replaces the multi-pass score→sync→max→sync→exp→sync→V form.
__global__ void gk_attn_online(float* out, const float* q, const float* kc, const float* vc, const int* d_pos, int n_head, int n_kv, int hd, float scale){
  int head=blockIdx.x; if(head>=n_head) return; int kvh=head/(n_head/n_kv); int L=*d_pos+1; int KD=n_kv*hd;
  const float* qh=q+(long)head*hd;
  int lane=threadIdx.x&31, wp=threadIdx.x>>5, nw=blockDim.x>>5; int dpl=hd>>5;
  float qreg[8], acc[8]; for(int i=0;i<dpl;i++){ qreg[i]=qh[lane+(i<<5)]; acc[i]=0.f; }
  float m=-1e30f, dsum=0.f;
  for(int j=wp;j<L;j+=nw){
    const float* kj=kc+(long)j*KD+kvh*hd; float part=0.f;
    for(int i=0;i<dpl;i++) part+=qreg[i]*kj[lane+(i<<5)];
    for(int r=16;r>0;r>>=1) part+=__shfl_xor_sync(0xffffffffu,part,r);
    float sj=part*scale; float mn=fmaxf(m,sj), c=__expf(m-mn), p=__expf(sj-mn); m=mn; dsum=dsum*c+p;
    const float* vj=vc+(long)j*KD+kvh*hd;
    for(int i=0;i<dpl;i++) acc[i]=acc[i]*c + p*vj[lane+(i<<5)];
  }
  __shared__ float sm[8], sd[8], sa[8][128];
  if(lane==0){ sm[wp]=m; sd[wp]=dsum; }
  for(int i=0;i<dpl;i++) sa[wp][lane+(i<<5)]=acc[i];
  __syncthreads();
  float M=-1e30f; for(int w=0;w<nw;w++) M=fmaxf(M,sm[w]);
  float D=0.f; for(int w=0;w<nw;w++) D+=sd[w]*__expf(sm[w]-M);
  for(int d=threadIdx.x; d<hd; d+=blockDim.x){ float a=0.f; for(int w=0;w<nw;w++) a+=sa[w][d]*__expf(sm[w]-M); out[(long)head*hd+d]=a/D; }
}
// SPLIT-K flash-attention (llama's fattn-vec `parallel_blocks`): P blocks PER query head each stream a disjoint
// slab of the KV cache with a register online-softmax → a partial (local max, denom, un-normalized weighted-V)
// in scratch; grid = n_head*P for FULL SM occupancy at decode (32 heads × 4 = 128 blocks vs 32). `gk_attn_comb`
// then flash-merges the P partials per head. This is the measured +14 tok/s edge llama gets from `-fa`.
__global__ void gk_attn_part(const float* q, const float* kc, const float* vc, const int* d_pos, int n_head, int n_kv, int hd, float scale, int P, float* acc_part, float* m_part, float* d_part){
  int b=blockIdx.x; int head=b/P, p=b%P; if(head>=n_head) return;
  int kvh=head/(n_head/n_kv); int L=*d_pos+1; int KD=n_kv*hd; const float* qh=q+(long)head*hd;
  int lane=threadIdx.x&31, wp=threadIdx.x>>5, nw=blockDim.x>>5; int dpl=hd>>5;
  float qreg[8], acc[8]; for(int i=0;i<dpl;i++){ qreg[i]=qh[lane+(i<<5)]; acc[i]=0.f; }
  float m=-1e30f, dsum=0.f;
  for(int idx=wp; ; idx+=nw){ int j=p+idx*P; if(j>=L) break;
    const float* kj=kc+(long)j*KD+kvh*hd; float pt=0.f;
    for(int i=0;i<dpl;i++) pt+=qreg[i]*kj[lane+(i<<5)];
    for(int r=16;r>0;r>>=1) pt+=__shfl_xor_sync(0xffffffffu,pt,r);
    float sj=pt*scale; float mn=fmaxf(m,sj), c=__expf(m-mn), pp=__expf(sj-mn); m=mn; dsum=dsum*c+pp;
    const float* vj=vc+(long)j*KD+kvh*hd; for(int i=0;i<dpl;i++) acc[i]=acc[i]*c + pp*vj[lane+(i<<5)];
  }
  __shared__ float sm[8], sd[8], sa[8][128];
  if(lane==0){ sm[wp]=m; sd[wp]=dsum; }
  for(int i=0;i<dpl;i++) sa[wp][lane+(i<<5)]=acc[i];
  __syncthreads();
  float M=-1e30f; for(int w=0;w<nw;w++) M=fmaxf(M,sm[w]);
  float D=0.f; for(int w=0;w<nw;w++) D+=sd[w]*__expf(sm[w]-M);
  if(threadIdx.x==0){ m_part[b]=M; d_part[b]=D; }
  for(int d=threadIdx.x; d<hd; d+=blockDim.x){ float a=0.f; for(int w=0;w<nw;w++) a+=sa[w][d]*__expf(sm[w]-M); acc_part[(long)b*hd+d]=a; }
}
__global__ void gk_attn_comb(float* out, int n_head, int hd, int P, const float* acc_part, const float* m_part, const float* d_part){
  int head=blockIdx.x; if(head>=n_head) return; __shared__ float GM, GD;
  if(threadIdx.x==0){ float M=-1e30f; for(int p=0;p<P;p++) M=fmaxf(M,m_part[head*P+p]); GM=M;
    float D=0.f; for(int p=0;p<P;p++) D+=d_part[head*P+p]*__expf(m_part[head*P+p]-M); GD=D; }
  __syncthreads(); float M=GM, D=GD;
  for(int d=threadIdx.x; d<hd; d+=blockDim.x){ float a=0.f;
    for(int p=0;p<P;p++) a += acc_part[(long)(head*P+p)*hd+d]*__expf(m_part[head*P+p]-M);
    out[(long)head*hd+d]=a/D; }
}
// FUSED split-K combine + activation-quant: the combine already lands each head's hd attention outputs in one
// block, so it quantizes them into int8 32-blocks in its epilogue (per-32-block warp max-abs) instead of a
// separate gk_quant_act launch feeding the O-projection. One fewer kernel on the decode critical path (+~1).
__global__ void gk_attn_comb_q(signed char* aq, float* as_, int n_head, int hd, int P, const float* acc_part, const float* m_part, const float* d_part){
  int head=blockIdx.x; if(head>=n_head) return; __shared__ float GM,GD; __shared__ float so[128];
  if(threadIdx.x==0){ float M=-1e30f; for(int p=0;p<P;p++) M=fmaxf(M,m_part[head*P+p]); GM=M;
    float D=0.f; for(int p=0;p<P;p++) D+=d_part[head*P+p]*__expf(m_part[head*P+p]-M); GD=D; }
  __syncthreads(); float M=GM,D=GD;
  for(int d=threadIdx.x; d<hd; d+=blockDim.x){ float a=0.f; for(int p=0;p<P;p++) a+=acc_part[(long)(head*P+p)*hd+d]*__expf(m_part[head*P+p]-M); so[d]=a/D; }
  __syncthreads();
  int nb=hd/32, warp=threadIdx.x>>5, lane=threadIdx.x&31, nw=blockDim.x>>5;
  for(int blk=warp; blk<nb; blk+=nw){ int gi=head*nb+blk; float v=so[blk*32+lane], av=fabsf(v);
    for(int s2=16;s2>0;s2>>=1) av=fmaxf(av,__shfl_down_sync(0xffffffffu,av,s2));
    av=__shfl_sync(0xffffffffu,av,0); float amax=av>1e-30f?av:1e-30f; float qsc=amax/127.0f;
    if(lane==0) as_[gi]=qsc; int q=(int)rintf(v/qsc); if(q>127)q=127; if(q<-128)q=-128; aq[gi*32+lane]=(signed char)q; }
}
__global__ void gk_add(float* x, const float* y, int n){ int i=blockIdx.x*blockDim.x+threadIdx.x; if(i<n) x[i]+=y[i]; }
__global__ void gk_argmax_adv(const float* logits, int vocab, int* d_tok, int* d_pos){
  __shared__ float bv[256]; __shared__ int bi[256]; float bestv=-1e30f; int besti=0;
  for(int i=threadIdx.x;i<vocab;i+=blockDim.x){ float v=logits[i]; if(v>bestv){bestv=v;besti=i;} }
  bv[threadIdx.x]=bestv; bi[threadIdx.x]=besti; __syncthreads();
  for(int k=blockDim.x/2;k>0;k>>=1){ if(threadIdx.x<k && bv[threadIdx.x+k]>bv[threadIdx.x]){ bv[threadIdx.x]=bv[threadIdx.x+k]; bi[threadIdx.x]=bi[threadIdx.x+k]; } __syncthreads(); }
  if(threadIdx.x==0){ *d_tok=bi[0]; *d_pos=*d_pos+1; }
}
// ── PRECISE (dequant) lane — llama's batch-1 recipe: f32 activation × int8 weight (no activation quantization),
// so the logits have clear margins (no int8-act near-ties) and the attention can be FUSED without flipping a
// token. Memory-bound on the same int8 weights ⇒ ~same bandwidth as the dp4a lane, but robustly llama-exact. ──
__global__ void gk_gemv_deq(float* y, const signed char* wq, const float* ws, const float* x, int K, int N){
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31;
  const int4* w16=(const int4*)(wq+(long)o*K); const float* wsr=ws+(long)o*(K/32); float acc=0.f; int nq=K/512;
  for(int b=0;b<nq;b++){ int4 wv=w16[b*32+lane]; int blk=b*16+lane/2; float sc=wsr[blk];
    const signed char* wb=(const signed char*)&wv; const float* xt=x+b*512+lane*16;
    float ba=0.f; for(int i=0;i<16;i++) ba += (float)wb[i]*xt[i]; acc += sc*ba; }
  for(int t=nq*512+lane;t<K;t+=32){ int blk=t/32; acc += wsr[blk]*(float)wq[(long)o*K+t]*x[t]; }
  for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s); if(lane==0) y[o]=acc;
}
__global__ void gk_gemv_kv_deq(float* cache, const int* d_pos, int KD, const signed char* wq, const float* ws, const float* x, int K, int N){
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31;
  const int4* w16=(const int4*)(wq+(long)o*K); const float* wsr=ws+(long)o*(K/32); float acc=0.f; int nq=K/512;
  for(int b=0;b<nq;b++){ int4 wv=w16[b*32+lane]; int blk=b*16+lane/2; float sc=wsr[blk];
    const signed char* wb=(const signed char*)&wv; const float* xt=x+b*512+lane*16;
    float ba=0.f; for(int i=0;i<16;i++) ba += (float)wb[i]*xt[i]; acc += sc*ba; }
  for(int t=nq*512+lane;t<K;t+=32){ int blk=t/32; acc += wsr[blk]*(float)wq[(long)o*K+t]*x[t]; }
  for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s); if(lane==0) cache[(long)(*d_pos)*KD+o]=acc;
}
// FUSED flash-attention: Q per-head QK-RMSNorm + RoPE + scores + block-softmax + weighted-V, all in one kernel.
__global__ void gk_fused_attn(float* out, const float* q, const float* kc, const float* vc, const int* d_pos, int n_head, int n_kv, int hd, float scale, const float* qnw, float rbase, float eps){
  int head=blockIdx.x; if(head>=n_head) return; int kvh=head/(n_head/n_kv); int pos=*d_pos; int L=pos+1; int KD=n_kv*hd;
  extern __shared__ float sc[]; __shared__ float red[32]; __shared__ float qh[256];
  int lane=threadIdx.x&31, wp=threadIdx.x>>5, nw=(blockDim.x+31)>>5;
  const float* qsrc=q+(long)head*hd;
  float qsum=0.f; for(int i=threadIdx.x;i<hd;i+=blockDim.x) qsum+=qsrc[i]*qsrc[i];
  for(int r=16;r>0;r>>=1) qsum+=__shfl_down_sync(0xffffffffu,qsum,r); if(lane==0) red[wp]=qsum; __syncthreads();
  if(threadIdx.x==0){ float s=0.f; for(int i=0;i<nw;i++) s+=red[i]; red[0]=s; } __syncthreads();
  float qrms=rsqrtf(red[0]/hd+eps);
  for(int i=threadIdx.x;i<hd;i+=blockDim.x) qh[i]=qsrc[i]*qrms*qnw[i]; __syncthreads();
  int half=hd/2;
  for(int i=threadIdx.x;i<half;i+=blockDim.x){ float f=powf(rbase,-2.0f*i/hd); float a=pos*f,c=cosf(a),si=sinf(a); float x0=qh[i],x1=qh[i+half]; qh[i]=x0*c-x1*si; qh[i+half]=x0*si+x1*c; }
  __syncthreads();
  for(int j=wp;j<L;j+=nw){ const float* kj=kc+(long)j*KD+kvh*hd; float s=0.f; for(int e=lane;e<hd;e+=32) s+=qh[e]*kj[e];
    for(int r=16;r>0;r>>=1) s+=__shfl_down_sync(0xffffffffu,s,r); if(lane==0) sc[j]=s*scale; }
  __syncthreads();
  float m=-1e30f; for(int j=threadIdx.x;j<L;j+=blockDim.x) m=fmaxf(m,sc[j]);
  for(int r=16;r>0;r>>=1) m=fmaxf(m,__shfl_down_sync(0xffffffffu,m,r)); if(lane==0) red[wp]=m; __syncthreads();
  if(threadIdx.x==0){ float mm=red[0]; for(int i=1;i<nw;i++) mm=fmaxf(mm,red[i]); red[0]=mm; } __syncthreads(); float mx=red[0];
  float ss=0.f; for(int j=threadIdx.x;j<L;j+=blockDim.x){ float e=expf(sc[j]-mx); sc[j]=e; ss+=e; }
  for(int r=16;r>0;r>>=1) ss+=__shfl_down_sync(0xffffffffu,ss,r); if(lane==0) red[wp]=ss; __syncthreads();
  if(threadIdx.x==0){ float su=0.f; for(int i=0;i<nw;i++) su+=red[i]; red[0]=su; } __syncthreads(); float den=red[0];
  for(int e=threadIdx.x;e<hd;e+=blockDim.x){ float a=0.f; for(int j=0;j<L;j++) a+=sc[j]*vc[(long)j*KD+kvh*hd+e]; out[(long)head*hd+e]=a/den; }
}
// Dequant MoE (f32 activation): gate/up SwiGLU + down-combine over the top-K experts, no activation quant.
__global__ void gk_moe_gate_up_deq(const float* x, const signed char* wg_q, const float* wg_s, const signed char* wu_q, const float* wu_s, const int* idx, float* h, int K, int N, int Kd){
  long row=(blockIdx.x*(long)blockDim.x+threadIdx.x)>>5; if(row>=(long)K*N) return; int lane=threadIdx.x&31;
  int kk=row/N, j=row%N; int e=idx[kk]; int nb=Kd/32, nq=Kd/512;
  const signed char* gwr=wg_q+((long)e*N+j)*Kd; const float* gsr=wg_s+((long)e*N+j)*nb;
  const signed char* uwr=wu_q+((long)e*N+j)*Kd; const float* usr=wu_s+((long)e*N+j)*nb;
  const int4* gw16=(const int4*)gwr; const int4* uw16=(const int4*)uwr; float ga=0.f, ua=0.f;
  for(int b=0;b<nq;b++){ int blk=b*16+lane/2; int4 gv=gw16[b*32+lane], uv=uw16[b*32+lane];
    const signed char* gb=(const signed char*)&gv; const signed char* ub=(const signed char*)&uv; const float* xt=x+b*512+lane*16;
    float gsc=gsr[blk], usc=usr[blk]; float gba=0.f, uba=0.f;
    for(int i=0;i<16;i++){ float xi=xt[i]; gba+=(float)gb[i]*xi; uba+=(float)ub[i]*xi; } ga+=gsc*gba; ua+=usc*uba; }
  for(int t=nq*512+lane;t<Kd;t+=32){ int blk=t/32; float xi=x[t]; ga+=gsr[blk]*(float)gwr[t]*xi; ua+=usr[blk]*(float)uwr[t]*xi; }
  for(int s=16;s>0;s>>=1){ ga+=__shfl_down_sync(0xffffffffu,ga,s); ua+=__shfl_down_sync(0xffffffffu,ua,s); }
  if(lane==0) h[row]=(ga/(1.0f+expf(-ga)))*ua;
}
__global__ void gk_moe_down_deq(const float* hbuf, const signed char* wd_q, const float* wd_s, const int* idx, const float* wc, float* H, int K, int N, int Kd){
  int o=blockIdx.x; int warp=threadIdx.x>>5, lane=threadIdx.x&31; int e=idx[warp]; int nb=Kd/32, nq=Kd/512;
  const float* hb=hbuf+(long)warp*Kd; const signed char* wr=wd_q+((long)e*N+o)*Kd; const float* wsr=wd_s+((long)e*N+o)*nb;
  const int4* w16=(const int4*)wr; float acc=0.f;
  for(int b=0;b<nq;b++){ int blk=b*16+lane/2; int4 wv=w16[b*32+lane]; const signed char* wb=(const signed char*)&wv; const float* ht=hb+b*512+lane*16;
    float sc=wsr[blk]; float ba=0.f; for(int i=0;i<16;i++) ba += (float)wb[i]*ht[i]; acc += sc*ba; }
  for(int t=nq*512+lane;t<Kd;t+=32){ int blk=t/32; acc += wsr[blk]*(float)wr[t]*hb[t]; }
  for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s);
  __shared__ float part[64]; if(lane==0) part[warp]=wc[warp]*acc; __syncthreads();
  if(threadIdx.x==0){ float sum=0.f; for(int kk=0;kk<K;kk++) sum+=part[kk]; H[o]=sum; }
}
"#;

/// Emit a self-contained CUDA program that AUTOREGRESSIVELY GENERATES from the real Qwen3-30B-A3B weights: the
/// per-token math is the recognized LOGOS forward (RMSNorm → Q/K/V → per-head QK-norm + RoPE → causal attention
/// over a KV cache → O-proj → residual → RMSNorm → 8-of-128 MoE → residual → LM head), wrapped in the compiler's
/// generation runtime — a prompt prefill, then a T=1 decode step captured as ONE CUDA graph and replayed per
/// token (device-side position + token). This is the "llama.cpp, but authored in LOGOS" whole demo: it prints
/// the generated token stream and the fair tg-style tok/s (a growing KV cache, exactly like llama-bench tg128).
pub fn emit_qwen30b_generate(
    cfg: &Qwen30bCfg,
    wdir: &str,
    prompt: &[usize],
    n_gen: usize,
    precise: bool,
    flash: bool,
) -> String {
    let (e, qd, kd, ff) = (cfg.d, cfg.h * cfg.hd, cfg.kv * cfg.hd, cfg.dff);
    let (ne, ktop, vocab, layers) = (cfg.ne, cfg.k, cfg.vocab, cfg.layers);
    let (nh, nkv, hd) = (cfg.h, cfg.kv, cfg.hd);
    let (rope_theta, rms_eps, attn_split) = (cfg.rope_theta, cfg.rms_eps, cfg.attn_split);
    let maxseq = prompt.len() + n_gen + 110;
    let scale = 1.0f64 / (hd as f64).sqrt();
    let prompt_c = prompt
        .iter()
        .map(|t| t.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let moe_kernels = moe_dp4a_kernels(false);
    // FAST-lane attention: default `gk_attn` (byte-exact-vs-llama token stream, the lock) or `flash` = SPLIT-K
    // flash-attention (n_head*P blocks → full SM occupancy, llama's `-fa` trick; reorders floats ⇒ coherent but
    // not byte-identical, the throughput lane).
    let fast_attn: &str = if flash {
        "gk_attn_part<<<NH*ATTN_SPLIT,256,0,s>>>(qbuf,kc[l],vc[l],d_pos,NH,NKV,HD,SCALEV,ATTN_SPLIT,acc_part,m_part,d_part); gk_attn_comb_q<<<NH,128,0,s>>>(aqQ,asQ,NH,HD,ATTN_SPLIT,acc_part,m_part,d_part);"
    } else {
        "gk_attn<<<NH,256,(size_t)MAXSEQ*4,s>>>(attnb,qbuf,kc[l],vc[l],d_pos,NH,NKV,HD,SCALEV); gk_quant_act<<<(QD+255)/256,256,0,s>>>(attnb,aqQ,asQ,QD);"
    };
    // The decode step, selected by lane. FAST = int8-activation DP4A (fastest GEMVs, but the lossy activation
    // makes the final logits a near-tie fusion would flip). PRECISE = f32-activation dequant (llama's batch-1
    // recipe) → clear logit margins → the attention is FUSED without breaking the token.
    let decode_fn: String = if precise {
        r#"static void decode_step(cudaStream_t s){
  gk_embed<<<(E+255)/256,256,0,s>>>(x,emb_q,emb_s,d_tok,E);
  for(int l=0;l<LAYERS;l++){
    gk_rmsnorm<<<1,256,0,s>>>(xn,x,an[l],E,EPSV);
    gk_gemv_deq<<<Gg(QD),256,0,s>>>(qbuf,wq_q[l],wq_s[l],xn,E,QD);
    gk_gemv_kv_deq<<<Gg(KD),256,0,s>>>(kc[l],d_pos,KD,wk_q[l],wk_s[l],xn,E,KD);
    gk_gemv_kv_deq<<<Gg(KD),256,0,s>>>(vc[l],d_pos,KD,wv_q[l],wv_s[l],xn,E,KD);
    gk_qk_norm_rope<<<NKV,128,0,s>>>(kc[l],d_pos,KD,akn[l],NKV,HD,ROPEV,EPSV);
    gk_fused_attn<<<NH,256,(size_t)MAXSEQ*4,s>>>(attnb,qbuf,kc[l],vc[l],d_pos,NH,NKV,HD,SCALEV,aqn[l],ROPEV,EPSV);
    gk_gemv_deq<<<Gg(E),256,0,s>>>(tmp,wo_q[l],wo_s[l],attnb,QD,E);
    gk_add_rmsnorm<<<1,256,0,s>>>(x,tmp,x2n,fn_[l],E,EPSV);
    moe_router_fast<<<(NE+7)/8,256,0,s>>>(x2n,wr_q[l],wr_s[l],moe_logits,NE,E);
    moe_topk_idx<<<1,NE,0,s>>>(moe_logits,moe_idx,moe_w,NE,KTOP);
    gk_moe_gate_up_deq<<<(KTOP*FF+7)/8,256,0,s>>>(x2n,wg_q[l],wg_s[l],wu_q[l],wu_s[l],moe_idx,moe_he,KTOP,FF,E);
    gk_moe_down_deq<<<E,KTOP*32,0,s>>>(moe_he,wd_q[l],wd_s[l],moe_idx,moe_w,moe_out,KTOP,E,FF);
    gk_add<<<(E+255)/256,256,0,s>>>(x,moe_out,E);
  }
  gk_rmsnorm<<<1,256,0,s>>>(xn,x,on_w,E,EPSV);
  gk_gemv_deq<<<Gg(VOCAB),256,0,s>>>(logits,wout_q,wout_s,xn,E,VOCAB);
}
"#.to_string()
    } else {
        r#"static void decode_step(cudaStream_t s){
  gk_embed<<<(E+255)/256,256,0,s>>>(x,emb_q,emb_s,d_tok,E);
  gk_rmsnorm_q8<<<1,256,0,s>>>(x,aqE,asE,an[0],E,EPSV);
  for(int l=0;l<LAYERS;l++){
    gk_gemv_qkv<<<Gg(QD+2*KD),256,0,s>>>(qbuf,kc[l],vc[l],d_pos,KD,wq_q[l],wq_s[l],wk_q[l],wk_s[l],wv_q[l],wv_s[l],aqE,asE,E,QD);
    gk_qk_norm_rope2<<<NH+NKV,128,0,s>>>(qbuf,kc[l],d_pos,KD,aqn[l],akn[l],NH,NKV,HD,ROPEV,EPSV);
    {fast_attn}
    gk_gemv<<<Gg(E),256,0,s>>>(tmp,wo_q[l],wo_s[l],aqQ,asQ,QD,E);
    gk_add_rmsnorm_q8<<<1,256,0,s>>>(x,tmp,x2n,fn_[l],moe_xq,moe_xs,E,EPSV);
    moe_router_fast<<<(NE+7)/8,256,0,s>>>(x2n,wr_q[l],wr_s[l],moe_logits,NE,E);
    moe_topk_idx<<<1,NE,0,s>>>(moe_logits,moe_idx,moe_w,NE,KTOP);
    moe_gate_up_silu<<<(KTOP*FF+7)/8,256,0,s>>>(moe_xq,moe_xs,wg_q[l],wg_s[l],wu_q[l],wu_s[l],moe_idx,moe_he,KTOP,FF,E);
    moe_qact<<<(((KTOP*FF)/32)+7)/8,256,0,s>>>(moe_he,moe_hq,moe_hs,KTOP*FF);
    moe_down_combine<<<E,KTOP*32,0,s>>>(moe_hq,moe_hs,wd_q[l],wd_s[l],moe_idx,moe_w,moe_out,KTOP,E,FF);
    if(l+1<LAYERS) gk_add_rmsnorm_q8a<<<1,256,0,s>>>(x,moe_out,aqE,asE,an[l+1],E,EPSV);
    else gk_add_rmsnorm_q8a<<<1,256,0,s>>>(x,moe_out,aqE,asE,on_w,E,EPSV);
  }
  gk_gemv<<<Gg(VOCAB),256,0,s>>>(logits,wout_q,wout_s,aqE,asE,E,VOCAB);
}
"#.replace("{fast_attn}", fast_attn)
    };
    // Per-layer weight file paths (int8 `.q` + f32 `.s`), matching the extractor's naming.
    let prov = super::provenance::header(super::provenance::Provenance::HandWritten {
        model: "Qwen3-30B-A3B",
        lane: "generation",
    });
    format!(
        "{prov}// Qwen3-30B-A3B autoregressive generation: a prefill, then a CUDA-graph decode replay over a growing\n\
         // KV cache. Weights fopen from {wdir}.\n\
         #include <cstdio>\n#include <cstdlib>\n#include <cstring>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n#include <cmath>\n\n\
         {moe_kernels}\n{GEN_DEVICE_KERNELS}\n\
         #define E {e}\n#define QD {qd}\n#define KD {kd}\n#define FF {ff}\n#define NE {ne}\n#define KTOP {ktop}\n\
         #define VOCAB {vocab}\n#define LAYERS {layers}\n#define NH {nh}\n#define NKV {nkv}\n#define HD {hd}\n\
         #define MAXSEQ {maxseq}\n#define ATTN_SPLIT {attn_split}\n#define EPSV {rms_eps:e}f\n#define ROPEV {rope_theta:e}f\n\
         static const float SCALEV = {scale}f;\n\
         static signed char *emb_q,*wq_q[LAYERS],*wk_q[LAYERS],*wv_q[LAYERS],*wo_q[LAYERS],*wout_q,*wr_q[LAYERS],*wg_q[LAYERS],*wu_q[LAYERS],*wd_q[LAYERS];\n\
         static float *emb_s,*wq_s[LAYERS],*wk_s[LAYERS],*wv_s[LAYERS],*wo_s[LAYERS],*wout_s;\n\
         static float *wr_s[LAYERS],*wg_s[LAYERS],*wu_s[LAYERS],*wd_s[LAYERS];\n\
         static float *an[LAYERS],*fn_[LAYERS],*aqn[LAYERS],*akn[LAYERS],*on_w;\n\
         static float *x,*xn,*x2n,*qbuf,*attnb,*tmp,*logits,*kc[LAYERS],*vc[LAYERS],*acc_part,*m_part,*d_part;\n\
         static signed char *aqE,*aqQ,*moe_xq,*moe_hq; static float *asE,*asQ,*moe_logits,*moe_w,*moe_xs,*moe_he,*moe_hs,*moe_out; static int *moe_idx,*d_pos,*d_tok;\n\
         static signed char* Li(const char* p, long n){{ signed char* d; if(cudaMalloc(&d,n)){{fprintf(stderr,\"malloc %s\\n\",p);exit(1);}} signed char* h=(signed char*)malloc(n); FILE* f=fopen(p,\"rb\"); if(!f||fread(h,1,n,f)!=(size_t)n){{fprintf(stderr,\"read %s\\n\",p);exit(1);}} fclose(f); cudaMemcpy(d,h,n,cudaMemcpyHostToDevice); free(h); return d; }}\n\
         static float* Lf(const char* p, long n){{ float* d; if(cudaMalloc(&d,n*4)){{fprintf(stderr,\"malloc %s\\n\",p);exit(1);}} float* h=(float*)malloc(n*4); FILE* f=fopen(p,\"rb\"); if(!f||fread(h,4,n,f)!=(size_t)n){{fprintf(stderr,\"read %s\\n\",p);exit(1);}} fclose(f); cudaMemcpy(d,h,n*4,cudaMemcpyHostToDevice); free(h); return d; }}\n\
         static __half* Lh(const char* p, long n){{ __half* d; if(cudaMalloc(&d,n*2)){{fprintf(stderr,\"malloc %s\\n\",p);exit(1);}} float* h=(float*)malloc(n*4); FILE* f=fopen(p,\"rb\"); if(!f||fread(h,4,n,f)!=(size_t)n){{fprintf(stderr,\"read %s\\n\",p);exit(1);}} fclose(f); __half* hh=(__half*)malloc(n*2); for(long i=0;i<n;i++) hh[i]=__float2half(h[i]); cudaMemcpy(d,hh,n*2,cudaMemcpyHostToDevice); free(h); free(hh); return d; }}\n\
         static inline unsigned Gg(long N){{ return (unsigned)((N+7)/8); }}\n\
         {decode_fn}
\
         static void prof_step(){{ cudaEvent_t pa,pb; cudaEventCreate(&pa); cudaEventCreate(&pb); float ms; double te=0,tp=0,tr=0,ta=0,tm=0,tl=0,trt=0,tqa=0,tgu=0,tqd=0,tdc=0;\n\
         \x20 cudaEventRecord(pa,0); gk_embed<<<(E+255)/256,256>>>(x,emb_q,emb_s,d_tok,E); cudaEventRecord(pb,0); cudaEventSynchronize(pb); cudaEventElapsedTime(&ms,pa,pb); te+=ms;\n\
         \x20 for(int l=0;l<LAYERS;l++){{\n\
         \x20   cudaEventRecord(pa,0); gk_rmsnorm_q8<<<1,256>>>(x,aqE,asE,an[l],E,EPSV); gk_gemv<<<Gg(QD),256>>>(qbuf,wq_q[l],wq_s[l],aqE,asE,E,QD); gk_gemv_kv<<<Gg(KD),256>>>(kc[l],d_pos,KD,wk_q[l],wk_s[l],aqE,asE,E,KD); gk_gemv_kv<<<Gg(KD),256>>>(vc[l],d_pos,KD,wv_q[l],wv_s[l],aqE,asE,E,KD); cudaEventRecord(pb,0); cudaEventSynchronize(pb); cudaEventElapsedTime(&ms,pa,pb); tp+=ms;\n\
         \x20   cudaEventRecord(pa,0); gk_qk_norm_rope<<<NH,128>>>(qbuf,d_pos,0,aqn[l],NH,HD,ROPEV,EPSV); gk_qk_norm_rope<<<NKV,128>>>(kc[l],d_pos,KD,akn[l],NKV,HD,ROPEV,EPSV); cudaEventRecord(pb,0); cudaEventSynchronize(pb); cudaEventElapsedTime(&ms,pa,pb); tr+=ms;\n\
         \x20   cudaEventRecord(pa,0); gk_attn<<<NH,256,(size_t)MAXSEQ*4>>>(attnb,qbuf,kc[l],vc[l],d_pos,NH,NKV,HD,SCALEV); gk_quant_act<<<(QD+255)/256,256>>>(attnb,aqQ,asQ,QD); gk_gemv<<<Gg(E),256>>>(tmp,wo_q[l],wo_s[l],aqQ,asQ,QD,E); gk_add<<<(E+255)/256,256>>>(x,tmp,E); cudaEventRecord(pb,0); cudaEventSynchronize(pb); cudaEventElapsedTime(&ms,pa,pb); ta+=ms;\n\
         \x20   cudaEventRecord(pa,0); gk_rmsnorm<<<1,256>>>(x2n,x,fn_[l],E,EPSV); moe_router_fast<<<(NE+7)/8,256>>>(x2n,wr_q[l],wr_s[l],moe_logits,NE,E); moe_topk_idx<<<1,NE>>>(moe_logits,moe_idx,moe_w,NE,KTOP); cudaEventRecord(pb,0); cudaEventSynchronize(pb); cudaEventElapsedTime(&ms,pa,pb); trt+=ms;\n\
         \x20   cudaEventRecord(pa,0); moe_qact<<<((E/32)+7)/8,256>>>(x2n,moe_xq,moe_xs,E); cudaEventRecord(pb,0); cudaEventSynchronize(pb); cudaEventElapsedTime(&ms,pa,pb); tqa+=ms;\n\
         \x20   cudaEventRecord(pa,0); moe_gate_up_silu<<<(KTOP*FF+7)/8,256>>>(moe_xq,moe_xs,wg_q[l],wg_s[l],wu_q[l],wu_s[l],moe_idx,moe_he,KTOP,FF,E); cudaEventRecord(pb,0); cudaEventSynchronize(pb); cudaEventElapsedTime(&ms,pa,pb); tgu+=ms;\n\
         \x20   cudaEventRecord(pa,0); moe_qact<<<(((KTOP*FF)/32)+7)/8,256>>>(moe_he,moe_hq,moe_hs,KTOP*FF); cudaEventRecord(pb,0); cudaEventSynchronize(pb); cudaEventElapsedTime(&ms,pa,pb); tqd+=ms;\n\
         \x20   cudaEventRecord(pa,0); moe_down_combine<<<E,KTOP*32>>>(moe_hq,moe_hs,wd_q[l],wd_s[l],moe_idx,moe_w,moe_out,KTOP,E,FF); gk_add<<<(E+255)/256,256>>>(x,moe_out,E); cudaEventRecord(pb,0); cudaEventSynchronize(pb); cudaEventElapsedTime(&ms,pa,pb); tdc+=ms;\n\
         \x20 }}\n\
         \x20 cudaEventRecord(pa,0); gk_rmsnorm_q8<<<1,256>>>(x,aqE,asE,on_w,E,EPSV); gk_gemv<<<Gg(VOCAB),256>>>(logits,wout_q,wout_s,aqE,asE,E,VOCAB); cudaEventRecord(pb,0); cudaEventSynchronize(pb); cudaEventElapsedTime(&ms,pa,pb); tl+=ms;\n\
         \x20 tm=trt+tqa+tgu+tqd+tdc; fprintf(stderr,\"PROF embed=%.3f proj=%.3f qknorm_rope=%.3f attn=%.3f moe=%.3f [router+topk=%.3f qact_x=%.3f gate_up=%.3f qact_h=%.3f down=%.3f] lmhead=%.3f total=%.3f ms\\n\", te,tp,tr,ta,tm,trt,tqa,tgu,tqd,tdc,tl, te+tp+tr+ta+tm+tl);\n\
         }}\n\
         int main(void){{\n\
         \x20 char p[512]; const char* W=\"{wdir}\";\n\
         \x20 snprintf(p,512,\"%s/emb.q\",W); emb_q=Li(p,(long)VOCAB*E); snprintf(p,512,\"%s/emb.s\",W); emb_s=Lf(p,(long)VOCAB*(E/32));\n\
         \x20 for(int l=0;l<LAYERS;l++){{\n\
         \x20   snprintf(p,512,\"%s/an%d.f\",W,l); an[l]=Lf(p,E); snprintf(p,512,\"%s/fn%d.f\",W,l); fn_[l]=Lf(p,E);\n\
         \x20   snprintf(p,512,\"%s/aqn%d.f\",W,l); aqn[l]=Lf(p,HD); snprintf(p,512,\"%s/akn%d.f\",W,l); akn[l]=Lf(p,HD);\n\
         \x20   snprintf(p,512,\"%s/wq%d.q\",W,l); wq_q[l]=Li(p,(long)QD*E); snprintf(p,512,\"%s/wq%d.s\",W,l); wq_s[l]=Lf(p,(long)QD*(E/32));\n\
         \x20   snprintf(p,512,\"%s/wk%d.q\",W,l); wk_q[l]=Li(p,(long)KD*E); snprintf(p,512,\"%s/wk%d.s\",W,l); wk_s[l]=Lf(p,(long)KD*(E/32));\n\
         \x20   snprintf(p,512,\"%s/wv%d.q\",W,l); wv_q[l]=Li(p,(long)KD*E); snprintf(p,512,\"%s/wv%d.s\",W,l); wv_s[l]=Lf(p,(long)KD*(E/32));\n\
         \x20   snprintf(p,512,\"%s/wo%d.q\",W,l); wo_q[l]=Li(p,(long)E*QD); snprintf(p,512,\"%s/wo%d.s\",W,l); wo_s[l]=Lf(p,(long)E*(QD/32));\n\
         \x20   snprintf(p,512,\"%s/wr%d.q\",W,l); wr_q[l]=Li(p,(long)NE*E); snprintf(p,512,\"%s/wr%d.s\",W,l); wr_s[l]=Lf(p,(long)NE*(E/32));\n\
         \x20   snprintf(p,512,\"%s/wg%d.q\",W,l); wg_q[l]=Li(p,(long)NE*FF*E); snprintf(p,512,\"%s/wg%d.s\",W,l); wg_s[l]=Lf(p,(long)NE*FF*(E/32));\n\
         \x20   snprintf(p,512,\"%s/wu%d.q\",W,l); wu_q[l]=Li(p,(long)NE*FF*E); snprintf(p,512,\"%s/wu%d.s\",W,l); wu_s[l]=Lf(p,(long)NE*FF*(E/32));\n\
         \x20   snprintf(p,512,\"%s/wd%d.q\",W,l); wd_q[l]=Li(p,(long)NE*E*FF); snprintf(p,512,\"%s/wd%d.s\",W,l); wd_s[l]=Lf(p,(long)NE*E*(FF/32));\n\
         \x20 }}\n\
         \x20 snprintf(p,512,\"%s/on.f\",W); on_w=Lf(p,E); snprintf(p,512,\"%s/wout.q\",W); wout_q=Li(p,(long)VOCAB*E); snprintf(p,512,\"%s/wout.s\",W); wout_s=Lf(p,(long)VOCAB*(E/32));\n\
         \x20 fprintf(stderr,\"weights loaded\\n\");\n\
         \x20 cudaMalloc(&x,E*4); cudaMalloc(&xn,E*4); cudaMalloc(&x2n,E*4); cudaMalloc(&acc_part,(long)NH*ATTN_SPLIT*HD*4); cudaMalloc(&m_part,(long)NH*ATTN_SPLIT*4); cudaMalloc(&d_part,(long)NH*ATTN_SPLIT*4); cudaMalloc(&qbuf,QD*4); cudaMalloc(&attnb,QD*4); cudaMalloc(&tmp,E*4); cudaMalloc(&logits,(long)VOCAB*4);\n\
         \x20 cudaMalloc(&aqE,E); cudaMalloc(&asE,(E/32)*4); cudaMalloc(&aqQ,QD); cudaMalloc(&asQ,(QD/32)*4);\n\
         \x20 cudaMalloc(&moe_logits,NE*4); cudaMalloc(&moe_w,NE*4); cudaMalloc(&moe_idx,NE*4); cudaMalloc(&moe_xq,E); cudaMalloc(&moe_xs,(E/32)*4);\n\
         \x20 cudaMalloc(&moe_he,(long)KTOP*FF*4); cudaMalloc(&moe_hq,(long)KTOP*FF); cudaMalloc(&moe_hs,(long)(KTOP*FF/32)*4); cudaMalloc(&moe_out,E*4);\n\
         \x20 for(int l=0;l<LAYERS;l++){{ cudaMalloc(&kc[l],(long)MAXSEQ*KD*4); cudaMalloc(&vc[l],(long)MAXSEQ*KD*4); }}\n\
         \x20 cudaMalloc(&d_pos,4); cudaMalloc(&d_tok,4);\n\
         \x20 int prompt[]={{{prompt_c}}}; int PL=sizeof(prompt)/sizeof(int);\n\
         \x20 // ── PREFILL: run each prompt token through the decode step (fills the KV cache), no argmax ──\n\
         \x20 for(int i=0;i<PL;i++){{ cudaMemcpy(d_pos,&i,4,cudaMemcpyHostToDevice); cudaMemcpy(d_tok,&prompt[i],4,cudaMemcpyHostToDevice); decode_step(0); cudaDeviceSynchronize(); }}\n\
         \x20 std::vector<int> gen; {{ std::vector<float> hl(VOCAB); cudaMemcpy(hl.data(),logits,(long)VOCAB*4,cudaMemcpyDeviceToHost); int b=0; for(int i=1;i<VOCAB;i++) if(hl[i]>hl[b]) b=i; gen.push_back(b); int pl=PL; cudaMemcpy(d_pos,&pl,4,cudaMemcpyHostToDevice); cudaMemcpy(d_tok,&b,4,cudaMemcpyHostToDevice); }}\n\
         \x20 if(getenv(\"LOGOS_GEN_PROF\")){{ prof_step(); prof_step(); }}\n\
         \x20 // ── capture the T=1 decode step as ONE graph, replay per token (device-side position + token) ──\n\
         \x20 cudaStream_t st; cudaStreamCreate(&st); cudaGraph_t g; cudaGraphExec_t ge;\n\
         \x20 cudaStreamBeginCapture(st,cudaStreamCaptureModeGlobal); decode_step(st); gk_argmax_adv<<<1,256,0,st>>>(logits,VOCAB,d_tok,d_pos); cudaStreamEndCapture(st,&g); cudaGraphInstantiate(&ge,g,0);\n\
         \x20 cudaEvent_t t0,t1; cudaEventCreate(&t0); cudaEventCreate(&t1); float genms=0; int steps=0;\n\
         \x20 for(int step=1; step<{n_gen}; step++){{ cudaEventRecord(t0,st); cudaGraphLaunch(ge,st); cudaEventRecord(t1,st); cudaStreamSynchronize(st); float ms; cudaEventElapsedTime(&ms,t0,t1); genms+=ms; steps++; int tok; cudaMemcpy(&tok,d_tok,4,cudaMemcpyDeviceToHost); gen.push_back(tok); }}\n\
         \x20 printf(\"GENERATED_TOKENS:\"); for(size_t i=0;i<gen.size();++i) printf(\" %d\",gen[i]); printf(\"\\n\");\n\
         \x20 if(steps) fprintf(stderr,\"GEN_TOK_S %.2f (%.3f ms/token, %d steps, growing KV cache)\\n\", 1000.0*steps/genms, genms/steps, steps);\n\
         \x20 {{ cudaEventRecord(t0,st); for(int i=0;i<100;i++) cudaGraphLaunch(ge,st); cudaEventRecord(t1,st); cudaStreamSynchronize(st); float bms; cudaEventElapsedTime(&bms,t0,t1); fprintf(stderr,\"GEN_TOK_S_BATCH %.2f (%.3f ms/token, 100 back-to-back replays, one sync)\\n\", 100000.0/bms, bms/100.0); }}\n\
         \x20 {{ int z=0; cudaMemcpy(d_pos,&z,4,cudaMemcpyHostToDevice); cudaStreamSynchronize(st); cudaEventRecord(t0,st); for(int i=0;i<128;i++) cudaGraphLaunch(ge,st); cudaEventRecord(t1,st); cudaStreamSynchronize(st); float tms; cudaEventElapsedTime(&tms,t0,t1); fprintf(stderr,\"GEN_TOK_S_TG128 %.2f (128 replays from an empty KV cache = llama-bench tg128, the apples-to-apples decode metric)\\n\", 128000.0/tms); }}\n\
         \x20 return 0;\n}}\n"
    )
}

pub fn gen_qwen30b_lg(
    cfg: &Qwen30bCfg,
    dir: &str,
) -> (
    String,
    HashMap<String, (String, usize)>,
    HashMap<String, Q8Ref>,
    HashMap<String, MoeWeights>,
) {
    let (d, dff, ne, k, h, kv, hd) = (cfg.d, cfg.dff, cfg.ne, cfg.k, cfg.h, cfg.kv, cfg.hd);
    let vh = h * hd; // query projection width (H·HD)
    let kvd = kv * hd; // KV projection width (KV·HD) — real GQA, not folded

    let mut weights: HashMap<String, (String, usize)> = HashMap::new();
    let mut q8: HashMap<String, Q8Ref> = HashMap::new();
    let mut moe: HashMap<String, MoeWeights> = HashMap::new();
    let mut f32w =
        |var: &str, cnt: usize| weights.insert(var.into(), (format!("{dir}/{var}.f"), cnt));

    let mut src = String::from("## Main\n");
    let decl = |src: &mut String, nm: &str, r: usize, c: usize| {
        src.push_str(&format!("Let {nm} be a {r} by {c} tensor of Real.\n"))
    };

    // Embedding: x = token_embd[token], an f32 vector loaded from the manifest.
    f32w("x", d);
    decl(&mut src, "x", 1, d);
    let mut xin = "x".to_string();

    for l in 0..cfg.layers {
        // weight handles for this block
        f32w(&format!("an{l}"), d);
        f32w(&format!("fn{l}"), d);
        q8.insert(format!("Wq{l}"), q8ref(dir, &format!("wq{l}"), d, vh)); // Q-proj [vh, d]
        q8.insert(format!("Wk{l}"), q8ref(dir, &format!("wk{l}"), d, kvd)); // K-proj [kvd, d]
        q8.insert(format!("Wv{l}"), q8ref(dir, &format!("wv{l}"), d, kvd)); // V-proj [kvd, d] (real GQA)
        q8.insert(format!("Wo{l}"), q8ref(dir, &format!("wo{l}"), vh, d)); // O-proj [d, vh]
        moe.insert(
            format!("ff{l}"),
            MoeWeights {
                router: q8ref(dir, &format!("wr{l}"), d, ne),
                gate: q8ref(dir, &format!("wg{l}"), d, ne * dff),
                up: q8ref(dir, &format!("wu{l}"), d, ne * dff),
                down: q8ref(dir, &format!("wd{l}"), dff, ne * d),
                d,
                dff,
                ne,
                k,
            },
        );
        decl(&mut src, &format!("an{l}"), 1, d);
        decl(&mut src, &format!("fn{l}"), 1, d);
        decl(&mut src, &format!("Wq{l}"), d, vh);
        decl(&mut src, &format!("Wk{l}"), d, kvd);
        decl(&mut src, &format!("Wv{l}"), d, kvd);
        decl(&mut src, &format!("Wo{l}"), vh, d);
        decl(&mut src, &format!("Wr{l}"), d, ne);
        decl(&mut src, &format!("Wg{l}"), ne * dff, d);
        decl(&mut src, &format!("Wu{l}"), ne * dff, d);
        decl(&mut src, &format!("Wd{l}"), ne * d, dff);
        // FULL GQA attention (reads Wq/Wk/Wv/Wo — the same weights llama.cpp reads) → residual → MoE FFN →
        // residual. `q/k/v` are the real projections; `attention` (one parallel kernel) attends the single query
        // to the KV cache. At a single decode step the cache holds this token's K/V, so `k`/`v` are 1×(KV·HD)
        // (L=1) — position-0 exact (softmax over one key = 1 ⇒ ctx = the query heads' grouped values). The cache
        // grows for real generation; the weight stream (the tok/s cost) is identical.
        src.push_str(&format!(
            "Let xn{l} be rmsnorm of {xin} with weight an{l}.\n"
        ));
        src.push_str(&format!("Let q{l} be xn{l} matmul Wq{l}.\n"));
        src.push_str(&format!("Let kk{l} be xn{l} matmul Wk{l}.\n"));
        src.push_str(&format!("Let vv{l} be xn{l} matmul Wv{l}.\n"));
        src.push_str(&format!(
            "Let ctx{l} be attention of q{l} over kk{l} and vv{l} with {h} heads {kv} groups.\n"
        ));
        src.push_str(&format!("Let ao{l} be ctx{l} matmul Wo{l}.\n"));
        src.push_str(&format!("Let xr{l} be {xin} plus ao{l}.\n"));
        src.push_str(&format!(
            "Let x2n{l} be rmsnorm of xr{l} with weight fn{l}.\n"
        ));
        src.push_str(&format!("Let ff{l} be moe of x2n{l} with router Wr{l} gate Wg{l} up Wu{l} down Wd{l} choosing {k} of {ne}.\n"));
        src.push_str(&format!("Let xo{l} be xr{l} plus ff{l}.\n"));
        xin = format!("xo{l}");
    }

    // final norm + LM head → logits (the emitter prints the final stage's output)
    f32w("on", d);
    q8.insert("Wout".into(), q8ref(dir, "wout", d, cfg.vocab));
    decl(&mut src, "on", 1, d);
    decl(&mut src, "Wout", d, cfg.vocab);
    src.push_str(&format!("Let xf be rmsnorm of {xin} with weight on.\n"));
    src.push_str("Let logits be xf matmul Wout.\n");

    (src, weights, q8, moe)
}
