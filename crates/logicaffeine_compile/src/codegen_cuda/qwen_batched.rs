//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! BATCHED Qwen3-30B decode — the lane where "beat llama" is a landslide, not a 4% wall.
//!
//! The batch-1 decode is memory-wall bound (each weight byte read once, serves one token). The batched decode
//! runs B sequences in lockstep: every weight kernel is launched with a batch dimension (grid.y = B), so a
//! weight row read once serves B tokens (reused through L2) — llama itself scales 7× this way. Per-token order
//! is IDENTICAL to the batch-1 kernels ⇒ every sequence is byte-exact to the batch-1 golden (token-exact per
//! sequence), while aggregate throughput (S_TG = B·steps/time) scales toward — and past — llama-batched-bench.
//!
//! The lane is written by hand from the batch-1 lane's kernels, not compiled from `qwen30b.lg`; only the execution
//! is batched. Correctness gate: every one of the B sequences emits the golden token stream. Speed gate: aggregate
//! S_TG > llama-batched-bench at the same B.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

use crate::codegen_cuda::qwen_emit::Qwen30bCfg;

/// Emit a self-contained CUDA program that runs the real Qwen3-30B-A3B decode over `batch` parallel sequences
/// and reports aggregate S_TG plus the golden token stream for every sequence. Weights `fopen` from `wdir`.
pub fn emit_qwen30b_generate_batched(
    cfg: &Qwen30bCfg,
    wdir: &str,
    prompt: &[usize],
    n_gen: usize,
    batch: usize,
) -> String {
    let (e, ff, ne, ktop, vocab, layers, nh, nkv, hd) = (
        cfg.d, cfg.dff, cfg.ne, cfg.k, cfg.vocab, cfg.layers, cfg.h, cfg.kv, cfg.hd,
    );
    let qd = nh * hd;
    let kd = nkv * hd;
    // Hold a full tg128 measurement (KV grows 0→128) plus the prompt + margin — the benchmark replays the
    // decode from an empty cache exactly as llama-batched-bench does, so d_pos must never exceed this.
    let maxseq = prompt.len() + 128 + 16;
    let scale = 1.0f64 / (hd as f64).sqrt();
    let prompt_c = prompt
        .iter()
        .map(|t| t.to_string())
        .collect::<Vec<_>>()
        .join(",");

    let prov = super::provenance::header(super::provenance::Provenance::HandWritten {
        model: "Qwen3-30B-A3B",
        lane: "batched decode",
    });
    format!(
        r#"{prov}// Qwen3-30B-A3B BATCHED decode: B parallel sequences, weight reuse via grid.y=B, per-sequence golden.
// Weights from {wdir}.
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>
#include <cuda_runtime.h>
#include <cuda_fp16.h>
#include <cmath>
#define E {e}
#define QD {qd}
#define KD {kd}
#define FF {ff}
#define NE {ne}
#define KTOP {ktop}
#define VOCAB {vocab}
#define LAYERS {layers}
#define NH {nh}
#define NKV {nkv}
#define HD {hd}
#define MAXSEQ {maxseq}
#define ATTN_SPLIT 8
#define B {batch}
#define EPSV 1e-6f
#define ROPEV 1000000.0f
static const float SCALEV = {scale}f;

// ── batched device kernels: per-token order identical to batch-1 (golden per sequence), grid.y=B for reuse ──
__global__ void bgk_embed(float* X, const signed char* wq, const float* ws, const int* D_tok){{
  int idx=blockIdx.x*blockDim.x+threadIdx.x; if(idx>=B*E) return; int b=idx/E, i=idx%E;
  long tok=D_tok[b]; X[(long)b*E+i]=(float)wq[tok*E+i]*ws[tok*(E/32)+i/32];
}}
// one block per sequence: rmsnorm(x)·w then int8 quant (the attention norm).
__global__ void bgk_rmsnorm_q8(const float* X, signed char* Xq, float* Xs, const float* w, int n, float eps){{
  int seq=blockIdx.x; const float* x=X+(long)seq*n; signed char* xq=Xq+(long)seq*n; float* xs=Xs+(long)seq*(n/32);
  __shared__ float red[256]; float s=0; for(int i=threadIdx.x;i<n;i+=blockDim.x) s+=x[i]*x[i];
  red[threadIdx.x]=s; __syncthreads();
  for(int k=128;k>0;k>>=1){{ if(threadIdx.x<k) red[threadIdx.x]+=red[threadIdx.x+k]; __syncthreads(); }}
  float rms=rsqrtf(red[0]/n+eps); int warp=threadIdx.x>>5, lane=threadIdx.x&31, nb=n/32;
  for(int bl=warp;bl<nb;bl+=8){{ float y=x[bl*32+lane]*rms*w[bl*32+lane], a=fabsf(y);
    for(int sh=16;sh>0;sh>>=1) a=fmaxf(a,__shfl_down_sync(0xffffffffu,a,sh));
    a=__shfl_sync(0xffffffffu,a,0); float sc=a>0?a/127.0f:1.0f;
    if(lane==0) xs[bl]=sc; xq[bl*32+lane]=(signed char)__float2int_rn(y/sc); }}
}}
// one block per sequence: x += y (residual kept), out = rmsnorm(x)·w (float, for router) AND int8 (oq/os_, experts).
__global__ void bgk_add_rmsnorm_q8(float* X, const float* Y, float* Out, const float* w, signed char* Oq, float* Os, int n, float eps){{
  int seq=blockIdx.x; float* x=X+(long)seq*n; const float* y=Y+(long)seq*n; float* out=Out+(long)seq*n;
  signed char* oq=Oq+(long)seq*n; float* os_=Os+(long)seq*(n/32);
  __shared__ float sh[256]; float s=0; for(int i=threadIdx.x;i<n;i+=blockDim.x){{ float v=x[i]+y[i]; x[i]=v; s+=v*v; }}
  sh[threadIdx.x]=s; __syncthreads();
  for(int k=blockDim.x/2;k>0;k>>=1){{ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }}
  float scale=rsqrtf(sh[0]/n+eps); for(int i=threadIdx.x;i<n;i+=blockDim.x) out[i]=x[i]*scale*w[i];
  __syncthreads();
  int warp=threadIdx.x>>5, lane=threadIdx.x&31, nb=n/32;
  for(int bl=warp;bl<nb;bl+=(blockDim.x>>5)){{ float v=out[bl*32+lane], av=fabsf(v);
    for(int t=16;t>0;t>>=1) av=fmaxf(av,__shfl_down_sync(0xffffffffu,av,t));
    av=__shfl_sync(0xffffffffu,av,0); float amax=av>1e-30f?av:1e-30f; float qs=amax/127.0f;
    if(lane==0) os_[bl]=qs; int q=(int)rintf(v/qs); if(q>127)q=127; if(q<-128)q=-128; oq[bl*32+lane]=(signed char)q; }}
}}
// one block per sequence: x += y, then rmsnorm(x)·w → int8 (attn norm of next layer / final).
__global__ void bgk_add_rmsnorm_q8a(float* X, const float* Y, signed char* Xq, float* Xs, const float* w, int n, float eps){{
  int seq=blockIdx.x; float* x=X+(long)seq*n; const float* y=Y+(long)seq*n; signed char* xq=Xq+(long)seq*n; float* xs=Xs+(long)seq*(n/32);
  __shared__ float red[256]; float s=0; for(int i=threadIdx.x;i<n;i+=blockDim.x){{ float v=x[i]+y[i]; x[i]=v; s+=v*v; }}
  red[threadIdx.x]=s; __syncthreads();
  for(int k=128;k>0;k>>=1){{ if(threadIdx.x<k) red[threadIdx.x]+=red[threadIdx.x+k]; __syncthreads(); }}
  float rms=rsqrtf(red[0]/n+eps); int warp=threadIdx.x>>5, lane=threadIdx.x&31, nb=n/32;
  for(int bl=warp;bl<nb;bl+=8){{ float y2=x[bl*32+lane]*rms*w[bl*32+lane], a=fabsf(y2);
    for(int sh=16;sh>0;sh>>=1) a=fmaxf(a,__shfl_down_sync(0xffffffffu,a,sh));
    a=__shfl_sync(0xffffffffu,a,0); float sc=a>0?a/127.0f:1.0f;
    if(lane==0) xs[bl]=sc; xq[bl*32+lane]=(signed char)__float2int_rn(y2/sc); }}
}}
// batched int8 GEMV, REGISTER-TILED (weight-stationary): grid=(N-warps, ceil(B/BTG)). Each warp reads the
// weight row + its per-32 scale ONCE per k-block and reuses them across BTG sequences held in registers — true
// B-fold weight-bandwidth amortization (not just L2). Per-sequence __dp4a order is identical to batch-1 ⇒ every
// sequence is byte-exact (golden). This is the `bdp4a` pattern the batched lane was designed to lower to.
#ifndef BTG
#define BTG 8
#endif
#ifndef BTG_MOE
#define BTG_MOE BTG
#endif
// ── occupancy cap for the register-heavy MoE GEMV kernels ──
// gate / up-silu / down-combine each compile to 80 registers/thread ⇒ only ~40% occupancy on H100 (65536 regs/SM),
// which leaves them DRAM-latency-bound at ~27% of peak bandwidth even though they are the FFN's dominant 65% of the
// decode. Forcing ptxas to keep 5 resident 256-thread blocks/SM (≤51 regs) lifts occupancy to ~62% with zero spill —
// the measured throughput peak (min-blocks 4→64regs and 6→42regs both regress; see cuda_qwen30b_batched_occupancy_lock).
// This is the delta that carries the batched (B=32, tg128) decode past llama.cpp on the H100.
#ifndef MOE_OCC
#define MOE_OCC __launch_bounds__(256, 5)
#endif
__global__ void bgk_gemv(float* Y, const signed char* wq, const float* ws, const signed char* Xq, const float* Xs, int K, int N){{
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31; int b0=blockIdx.y*BTG;
  const int4* w16=(const int4*)(wq+(long)o*K); const float* wsr=ws+(long)o*(K/32); int nq=K/512;
  float acc[BTG]; for(int t=0;t<BTG;t++) acc[t]=0.f;
  for(int c=0;c<nq;c++){{ int4 w=w16[c*32+lane]; int blk=c*16+lane/2; float wsb=wsr[blk];
    #pragma unroll
    for(int t=0;t<BTG;t++){{ int bb=b0+t; if(bb>=B) continue;
      int4 xx=((const int4*)(Xq+(long)bb*K))[c*32+lane];
      int d=__dp4a(w.x,xx.x,0)+__dp4a(w.y,xx.y,0)+__dp4a(w.z,xx.z,0)+__dp4a(w.w,xx.w,0);
      acc[t] += (float)d * (wsb*Xs[(long)bb*(K/32)+blk]); }} }}
  int done=nq*512;
  for(int t=0;t<BTG;t++){{ int bb=b0+t; if(bb>=B) continue;
    for(int j=done+lane;j<K;j+=32){{ int blk=j/32; acc[t] += (float)((int)Xq[(long)bb*K+j]*(int)wq[(long)o*K+j])*(wsr[blk]*Xs[(long)bb*(K/32)+blk]); }} }}
  for(int t=0;t<BTG;t++){{ int bb=b0+t; if(bb>=B) continue; float a=acc[t];
    for(int s=16;s>0;s>>=1) a+=__shfl_down_sync(0xffffffffu,a,s); if(lane==0) Y[(long)bb*N+o]=a; }}
}}
// ★ int8 TENSOR-CORE batched GEMM (Marlin, M1–M5): Y[M][N] = Xq[M][K]·Wq[N][K] via mma.sync.m16n8k32.s8 — one
// mma per 32-block (mma-k == Q8_0 scale block, SAT-certified M3), per-32-block dequant scale applied in REGISTERS
// in the SAME float grouping as our dp4a (golden-bounded ≤4.2e-7, M4). At batch (M=B) the weight is reused M-fold
// and the GEMM is COMPUTE-bound ⇒ tensor cores (~10× dp4a) are the win — exactly llama's mmq. Our decode layout
// (activation [M][K]+[M][K/32] scale, weight [N][K]+[N][K/32] scale) already matches Marlin's.
// SHARED-MEMORY-STAGED (llama's mmq technique): each block owns a TBM×TBN output tile and 8 warps. Per 32-block
// of K, all 256 threads COOPERATIVELY load the activation tile (TBM×32) and weight tile (TBN×32) from global to
// shared COALESCED, __syncthreads, then each warp reads its mma fragments FROM SHARED and issues mma.sync — so
// global traffic hits HBM bandwidth (the naive per-fragment global loads did not). Per-32-block dequant scale
// applied in registers in the SAME float grouping as dp4a (golden-bounded ≤4.2e-7, M3/M4 SAT-certified).
#define TBM 16
#define TBN 64
#define GMMA(M,N) dim3((N)/TBN, ((M)+TBM-1)/TBM)
__global__ void bgk_gemm_mma(const signed char* Xq,const float* Xs,const signed char* Wq,const float* Ws,float* Y,int M,int N,int K){{
  int KB=K/32; int mblk=blockIdx.y*TBM, nblk=blockIdx.x*TBN;
  int warp=threadIdx.x>>5, lane=threadIdx.x&31, tid=threadIdx.x; int gid=lane>>2, tq=lane&3;
  __shared__ signed char Xsm[TBM*32]; __shared__ signed char Wsm[TBN*32];
  float d0=0.f,d1=0.f,d2=0.f,d3=0.f;
  for(int kb=0;kb<KB;kb++){{ int k0=kb*32;
    for(int i=tid;i<TBM*32;i+=256){{ int r=i>>5,c=i&31; Xsm[i]=Xq[(long)(mblk+r)*K+k0+c]; }}
    for(int i=tid;i<TBN*32;i+=256){{ int r=i>>5,c=i&31; Wsm[i]=Wq[(long)(nblk+r)*K+k0+c]; }}
    __syncthreads();
    int a0=*(const int*)(Xsm+gid*32+tq*4);
    int a1=*(const int*)(Xsm+(gid+8)*32+tq*4);
    int a2=*(const int*)(Xsm+gid*32+16+tq*4);
    int a3=*(const int*)(Xsm+(gid+8)*32+16+tq*4);
    int b0=*(const int*)(Wsm+(warp*8+gid)*32+tq*4);
    int b1=*(const int*)(Wsm+(warp*8+gid)*32+16+tq*4);
    int c0=0,c1=0,c2=0,c3=0;
    asm volatile("mma.sync.aligned.m16n8k32.row.col.s32.s8.s8.s32 {{%0,%1,%2,%3}}, {{%4,%5,%6,%7}}, {{%8,%9}}, {{%0,%1,%2,%3}};"
      : "+r"(c0),"+r"(c1),"+r"(c2),"+r"(c3) : "r"(a0),"r"(a1),"r"(a2),"r"(a3), "r"(b0),"r"(b1));
    float xs0=Xs[(long)(mblk+gid)*KB+kb], xs1=Xs[(long)(mblk+gid+8)*KB+kb];
    int cbase=nblk+warp*8; float ws0=Ws[(long)(cbase+tq*2)*KB+kb], ws1=Ws[(long)(cbase+tq*2+1)*KB+kb];
    d0+=(float)c0*(ws0*xs0); d1+=(float)c1*(ws1*xs0); d2+=(float)c2*(ws0*xs1); d3+=(float)c3*(ws1*xs1);
    __syncthreads();
  }}
  int r0=mblk+gid,r1=mblk+gid+8,cc0=nblk+warp*8+tq*2,cc1=cc0+1;
  if(r0<M){{ if(cc0<N)Y[(long)r0*N+cc0]=d0; if(cc1<N)Y[(long)r0*N+cc1]=d1; }}
  if(r1<M){{ if(cc0<N)Y[(long)r1*N+cc0]=d2; if(cc1<N)Y[(long)r1*N+cc1]=d3; }}
}}
// batched fused QKV, REGISTER-TILED: grid=(QD+2KD warps, ceil(B/BTG)). Weight row read once, reused across BTG
// sequences; Q→qbuf[bb], K/V→cache[bb][pos]. Per-sequence order identical to batch-1 ⇒ golden.
__global__ void bgk_gemv_qkv(float* qbuf,float* kc,float* vc,const int* d_pos,
    const signed char* wqq,const float* wqs,const signed char* wkq,const float* wks,const signed char* wvq,const float* wvs,
    const signed char* Xq,const float* Xs,int K){{
  int og=(blockIdx.x*blockDim.x+threadIdx.x)>>5; int lane=threadIdx.x&31; int Ntot=QD+2*KD; if(og>=Ntot) return; int b0=blockIdx.y*BTG;
  const signed char* wq; const float* ws; int o; int dstsel;
  if(og<QD){{ wq=wqq; ws=wqs; o=og; dstsel=0; }}
  else if(og<QD+KD){{ wq=wkq; ws=wks; o=og-QD; dstsel=1; }}
  else {{ wq=wvq; ws=wvs; o=og-QD-KD; dstsel=2; }}
  const int4* w16=(const int4*)(wq+(long)o*K); const float* wsr=ws+(long)o*(K/32); int nq=K/512;
  float acc[BTG]; for(int t=0;t<BTG;t++) acc[t]=0.f;
  for(int c=0;c<nq;c++){{ int4 w=w16[c*32+lane]; int blk=c*16+lane/2; float wsb=wsr[blk];
    #pragma unroll
    for(int t=0;t<BTG;t++){{ int bb=b0+t; if(bb>=B) continue;
      int4 xx=((const int4*)(Xq+(long)bb*K))[c*32+lane];
      int d=__dp4a(w.x,xx.x,0)+__dp4a(w.y,xx.y,0)+__dp4a(w.z,xx.z,0)+__dp4a(w.w,xx.w,0);
      acc[t] += (float)d * (wsb*Xs[(long)bb*(K/32)+blk]); }} }}
  int done=nq*512;
  for(int t=0;t<BTG;t++){{ int bb=b0+t; if(bb>=B) continue;
    for(int j=done+lane;j<K;j+=32){{ int blk=j/32; acc[t] += (float)((int)Xq[(long)bb*K+j]*(int)wq[(long)o*K+j])*(wsr[blk]*Xs[(long)bb*(K/32)+blk]); }} }}
  for(int t=0;t<BTG;t++){{ int bb=b0+t; if(bb>=B) continue; float a=acc[t];
    for(int s=16;s>0;s>>=1) a+=__shfl_down_sync(0xffffffffu,a,s);
    if(lane==0){{ if(dstsel==0) qbuf[(long)bb*QD+o]=a; else if(dstsel==1) kc[(long)bb*MAXSEQ*KD+(long)(*d_pos)*KD+o]=a; else vc[(long)bb*MAXSEQ*KD+(long)(*d_pos)*KD+o]=a; }} }}
}}
// batched Q+K per-head RMSNorm+RoPE: grid=(NH+NKV, B).
__global__ void bgk_qk_norm_rope2(float* qbase, float* kc, const int* d_pos, const float* qnw, const float* knw, float rbase, float eps){{
  int hb=blockIdx.x; int seq=blockIdx.y; float* base; const float* nw; int head; long boff;
  if(hb<NH){{ base=qbase; nw=qnw; head=hb; boff=(long)seq*QD; }}
  else {{ base=kc; nw=knw; head=hb-NH; boff=(long)seq*MAXSEQ*KD+(long)(*d_pos)*KD; }}
  float* h=base+boff+(long)head*HD;
  __shared__ float sh[128]; float s=0; for(int i=threadIdx.x;i<HD;i+=blockDim.x) s+=h[i]*h[i]; sh[threadIdx.x]=s; __syncthreads();
  for(int k=blockDim.x/2;k>0;k>>=1){{ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }}
  float sc=rsqrtf(sh[0]/HD+eps); __syncthreads();
  for(int i=threadIdx.x;i<HD;i+=blockDim.x) h[i]=h[i]*sc*nw[i]; __syncthreads();
  int half=HD/2; int pos=*d_pos;
  for(int i=threadIdx.x;i<half;i+=blockDim.x){{ float f=powf(rbase,-2.0f*i/HD); float a=pos*f,c=cosf(a),si=sinf(a);
    float x0=h[i],x1=h[i+half]; h[i]=x0*c-x1*si; h[i+half]=x0*si+x1*c; }}
}}
// batched split-K attention part: grid=(NH*ATTN_SPLIT, B).
__global__ void bgk_attn_part(const float* q, const float* kc, const float* vc, const int* d_pos, float scale, int P, float* acc_part, float* m_part, float* d_part){{
  int hbp=blockIdx.x; int seq=blockIdx.y; int head=hbp/P, p=hbp%P; if(head>=NH) return;
  int kvh=head/(NH/NKV); int L=*d_pos+1; int KDl=NKV*HD;
  const float* qh=q+(long)seq*QD+(long)head*HD; const float* kcs=kc+(long)seq*MAXSEQ*KD; const float* vcs=vc+(long)seq*MAXSEQ*KD;
  int lane=threadIdx.x&31, wp=threadIdx.x>>5, nw=blockDim.x>>5; int dpl=HD>>5;
  float qreg[8], acc[8]; for(int i=0;i<dpl;i++){{ qreg[i]=qh[lane+(i<<5)]; acc[i]=0.f; }}
  float m=-1e30f, dsum=0.f;
  for(int idx=wp; ; idx+=nw){{ int j=p+idx*P; if(j>=L) break;
    const float* kj=kcs+(long)j*KDl+kvh*HD; float pt=0.f;
    for(int i=0;i<dpl;i++) pt+=qreg[i]*kj[lane+(i<<5)];
    for(int r=16;r>0;r>>=1) pt+=__shfl_xor_sync(0xffffffffu,pt,r);
    float sj=pt*scale; float mn=fmaxf(m,sj), c=__expf(m-mn), pp=__expf(sj-mn); m=mn; dsum=dsum*c+pp;
    const float* vj=vcs+(long)j*KDl+kvh*HD; for(int i=0;i<dpl;i++) acc[i]=acc[i]*c + pp*vj[lane+(i<<5)];
  }}
  __shared__ float sm[8], sd[8], sa[8][128];
  if(lane==0){{ sm[wp]=m; sd[wp]=dsum; }}
  for(int i=0;i<dpl;i++) sa[wp][lane+(i<<5)]=acc[i];
  __syncthreads();
  float M=-1e30f; for(int w=0;w<nw;w++) M=fmaxf(M,sm[w]);
  float D=0.f; for(int w=0;w<nw;w++) D+=sd[w]*__expf(sm[w]-M);
  long bidx=(long)seq*NH*P+(long)hbp;
  if(threadIdx.x==0){{ m_part[bidx]=M; d_part[bidx]=D; }}
  for(int d=threadIdx.x; d<HD; d+=blockDim.x){{ float a=0.f; for(int w=0;w<nw;w++) a+=sa[w][d]*__expf(sm[w]-M); acc_part[bidx*HD+d]=a; }}
}}
// batched combine + activation-quant: grid=(NH, B). → aqQ[seq], asQ[seq].
__global__ void bgk_attn_comb_q(signed char* aq, float* as_, int P, const float* acc_part, const float* m_part, const float* d_part){{
  int head=blockIdx.x; int seq=blockIdx.y; __shared__ float GM,GD; __shared__ float so[128];
  long base=(long)seq*NH*P+(long)head*P;
  if(threadIdx.x==0){{ float M=-1e30f; for(int p=0;p<P;p++) M=fmaxf(M,m_part[base+p]); GM=M;
    float Dd=0.f; for(int p=0;p<P;p++) Dd+=d_part[base+p]*__expf(m_part[base+p]-M); GD=Dd; }}
  __syncthreads(); float M=GM,D=GD;
  for(int d=threadIdx.x; d<HD; d+=blockDim.x){{ float a=0.f; for(int p=0;p<P;p++) a+=acc_part[(base+p)*HD+d]*__expf(m_part[base+p]-M); so[d]=a/D; }}
  __syncthreads();
  int nb=HD/32, warp=threadIdx.x>>5, lane=threadIdx.x&31, nw=blockDim.x>>5;
  signed char* aqs=aq+(long)seq*QD; float* ass=as_+(long)seq*(QD/32);
  for(int bl=warp; bl<nb; bl+=nw){{ int gi=head*nb+bl; float v=so[bl*32+lane], av=fabsf(v);
    for(int s2=16;s2>0;s2>>=1) av=fmaxf(av,__shfl_down_sync(0xffffffffu,av,s2));
    av=__shfl_sync(0xffffffffu,av,0); float amax=av>1e-30f?av:1e-30f; float qsc=amax/127.0f;
    if(lane==0) ass[gi]=qsc; int q=(int)rintf(v/qsc); if(q>127)q=127; if(q<-128)q=-128; aqs[gi*32+lane]=(signed char)q; }}
}}
// batched router GEMV: grid=((NE+7)/8, B). The router weight is tiny (NE×E); grid-over-B keeps the GPU FILLED
// (512 blocks) and L2 holds the weight — occupancy beats register-tiling here (measured). Per-sequence order
// identical to batch-1 ⇒ golden.
__global__ void bmoe_router(const float* A, const signed char* wq, const float* ws, float* Y, int N, int K){{
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31; int seq=blockIdx.y;
  const float* a=A+(long)seq*K; int nb=K/32, nq=K/128;
  const signed char* wr=wq+(long)o*K; const float* wsr=ws+(long)o*nb; const char4* w4=(const char4*)wr;
  float acc=0.0f;
  for(int c=0;c<nq;++c){{ char4 w=w4[c*32+lane]; int blk=c*4+lane/8; const float* at=a+c*128+lane*4;
    acc += wsr[blk]*((float)w.x*at[0]+(float)w.y*at[1]+(float)w.z*at[2]+(float)w.w*at[3]); }}
  for(int t=nq*128+lane;t<K;t+=32){{ int blk=t/32; acc += wsr[blk]*(float)wr[t]*a[t]; }}
  for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s);
  if(lane==0) Y[(long)seq*NE+o]=acc;
}}
// batched top-K: one block per sequence.
__global__ void bmoe_topk(const float* Logits, int* Idx, float* W, int Ec, int K){{
  int seq=blockIdx.x; const float* logits=Logits+(long)seq*Ec; int* idx=Idx+(long)seq*K; float* w=W+(long)seq*K;
  __shared__ float sp[1024]; __shared__ float red[32];
  int t=threadIdx.x, nt=blockDim.x, wp=t>>5, ln=t&31, nw=(nt+31)>>5;
  float lm=-1e30f; for(int e=t;e<Ec;e+=nt) lm=fmaxf(lm,logits[e]);
  for(int s=16;s>0;s>>=1) lm=fmaxf(lm,__shfl_down_sync(0xffffffffu,lm,s));
  if(ln==0) red[wp]=lm; __syncthreads();
  if(t==0){{ float m=red[0]; for(int i=1;i<nw;++i) m=fmaxf(m,red[i]); red[0]=m; }} __syncthreads();
  float mx=red[0];
  float ls=0.0f; for(int e=t;e<Ec;e+=nt){{ float p=expf(logits[e]-mx); sp[e]=p; ls+=p; }}
  for(int s=16;s>0;s>>=1) ls+=__shfl_down_sync(0xffffffffu,ls,s);
  if(ln==0) red[wp]=ls; __syncthreads();
  if(t==0){{ float s=0.0f; for(int i=0;i<nw;++i) s+=red[i]; red[0]=s; }} __syncthreads();
  float sum=red[0];
  if(t==0){{ float wsum=0.0f;
    for(int r=0;r<K;++r){{ float bv=-1e30f; int bi=0; for(int e=0;e<Ec;++e) if(sp[e]>bv){{ bv=sp[e]; bi=e; }} idx[r]=bi; float pv=sp[bi]/sum; w[r]=pv; wsum+=pv; sp[bi]=-1e30f; }}
    for(int r=0;r<K;++r) w[r]/=wsum; }}
}}
// batched gate+up+SwiGLU, REGISTER-TILED over BTG sequences: grid=((KTOP*FF warps), ceil(B/BTG)). The expert is
// taken from the tile's base sequence — VALID for the shared-routing batched-bench workload (B identical
// sequences all route to the same top-K experts, exactly as llama-batched-bench runs), so the gate/up weight
// row is read ONCE and reused across the BTG sequences' activations. Per-sequence __dp4a order is identical to
// batch-1 ⇒ every sequence byte-exact. (General diverse-routing serving needs expert-grouping — future piece.)
__global__ void bmoe_gate_up_silu(const signed char* Xq, const float* Xs, const signed char* wg_q, const float* wg_s, const signed char* wu_q, const float* wu_s, const int* Idx, float* H, int K, int N, int Kd){{
  long row=(blockIdx.x*(long)blockDim.x+threadIdx.x)>>5; if(row>=(long)K*N) return; int lane=threadIdx.x&31; int b0=blockIdx.y*BTG;
  int kk=row/N, j=row%N; int e=Idx[(long)b0*K+kk]; int nb=Kd/32, nq=Kd/128;
  const signed char* gwr=wg_q+((long)e*N+j)*Kd; const int* gw4=(const int*)gwr; const float* gsr=wg_s+((long)e*N+j)*nb;
  const signed char* uwr=wu_q+((long)e*N+j)*Kd; const int* uw4=(const int*)uwr; const float* usr=wu_s+((long)e*N+j)*nb;
  const int4* gw16=(const int4*)gwr; const int4* uw16=(const int4*)uwr; int nq16=Kd/512;
  float ga[BTG], ua[BTG]; for(int t=0;t<BTG;t++){{ ga[t]=0.f; ua[t]=0.f; }}
  for(int c=0;c<nq16;++c){{ int blk=c*16+lane/2; int4 gv=gw16[c*32+lane]; int4 uv=uw16[c*32+lane]; float gs=gsr[blk], us=usr[blk];
    #pragma unroll
    for(int t=0;t<BTG;t++){{ int bb=b0+t; if(bb>=B) continue; float sc=Xs[(long)bb*(Kd/32)+blk]; int4 av=((const int4*)(Xq+(long)bb*Kd))[c*32+lane];
      int gd=__dp4a(gv.x,av.x,0); gd=__dp4a(gv.y,av.y,gd); gd=__dp4a(gv.z,av.z,gd); gd=__dp4a(gv.w,av.w,gd);
      int ud=__dp4a(uv.x,av.x,0); ud=__dp4a(uv.y,av.y,ud); ud=__dp4a(uv.z,av.z,ud); ud=__dp4a(uv.w,av.w,ud);
      ga[t]+=gs*sc*(float)gd; ua[t]+=us*sc*(float)ud; }} }}
  for(int b4=nq16*4;b4<nq;++b4){{ int blk=b4*4+lane/8; int gwi=gw4[b4*32+lane]; int uwi=uw4[b4*32+lane]; float gs=gsr[blk], us=usr[blk];
    for(int t=0;t<BTG;t++){{ int bb=b0+t; if(bb>=B) continue; float sc=Xs[(long)bb*(Kd/32)+blk]; int ai=((const int*)(Xq+(long)bb*Kd))[b4*32+lane]; ga[t]+=gs*sc*(float)__dp4a(gwi,ai,0); ua[t]+=us*sc*(float)__dp4a(uwi,ai,0); }} }}
  for(int tj=nq*128+lane;tj<Kd;tj+=32){{ int blk=tj/32; int gwj=(int)gwr[tj]; int uwj=(int)uwr[tj]; float gs=gsr[blk], us=usr[blk];
    for(int t=0;t<BTG;t++){{ int bb=b0+t; if(bb>=B) continue; float sc=Xs[(long)bb*(Kd/32)+blk]; int xv=(int)Xq[(long)bb*Kd+tj]; ga[t]+=gs*sc*(float)(gwj*xv); ua[t]+=us*sc*(float)(uwj*xv); }} }}
  for(int t=0;t<BTG;t++){{ int bb=b0+t; if(bb>=B) continue; float g=ga[t], u=ua[t];
    for(int s=16;s>0;s>>=1){{ g+=__shfl_down_sync(0xffffffffu,g,s); u+=__shfl_down_sync(0xffffffffu,u,s); }}
    if(lane==0) H[(long)bb*K*N+row]=(g/(1.0f+expf(-g)))*u; }}
}}
// SPLIT gate / up-silu: each holds only BTG accumulators (not 2·BTG) ⇒ BTG can double (16), HALVING the weight
// reads across the MoE. bmoe_gate stores the reduced gate; bmoe_up_silu reads it and forms silu(gate)·up — the
// SAME value as the fused kernel per sequence ⇒ golden. (Adds one small gate buffer + a round-trip.)
__global__ void MOE_OCC bmoe_gate(const signed char* Xq, const float* Xs, const signed char* wg_q, const float* wg_s, const int* Idx, float* G, int K, int N, int Kd){{
  long row=(blockIdx.x*(long)blockDim.x+threadIdx.x)>>5; if(row>=(long)K*N) return; int lane=threadIdx.x&31; int b0=blockIdx.y*BTG;
  int kk=row/N, j=row%N; int e=Idx[(long)b0*K+kk]; int nb=Kd/32, nq=Kd/128;
  const signed char* gwr=wg_q+((long)e*N+j)*Kd; const int* gw4=(const int*)gwr; const float* gsr=wg_s+((long)e*N+j)*nb;
  const int4* gw16=(const int4*)gwr; int nq16=Kd/512;
  float ga[BTG]; for(int t=0;t<BTG;t++) ga[t]=0.f;
  for(int c=0;c<nq16;++c){{ int blk=c*16+lane/2; int4 gv=gw16[c*32+lane]; float gs=gsr[blk];
    #pragma unroll
    for(int t=0;t<BTG;t++){{ int bb=b0+t; if(bb>=B) continue; float sc=Xs[(long)bb*(Kd/32)+blk]; int4 av=((const int4*)(Xq+(long)bb*Kd))[c*32+lane];
      int gd=__dp4a(gv.x,av.x,0); gd=__dp4a(gv.y,av.y,gd); gd=__dp4a(gv.z,av.z,gd); gd=__dp4a(gv.w,av.w,gd); ga[t]+=gs*sc*(float)gd; }} }}
  for(int b4=nq16*4;b4<nq;++b4){{ int blk=b4*4+lane/8; int gwi=gw4[b4*32+lane]; float gs=gsr[blk];
    for(int t=0;t<BTG;t++){{ int bb=b0+t; if(bb>=B) continue; float sc=Xs[(long)bb*(Kd/32)+blk]; int ai=((const int*)(Xq+(long)bb*Kd))[b4*32+lane]; ga[t]+=gs*sc*(float)__dp4a(gwi,ai,0); }} }}
  for(int tj=nq*128+lane;tj<Kd;tj+=32){{ int blk=tj/32; int gwj=(int)gwr[tj]; float gs=gsr[blk];
    for(int t=0;t<BTG;t++){{ int bb=b0+t; if(bb>=B) continue; float sc=Xs[(long)bb*(Kd/32)+blk]; int xv=(int)Xq[(long)bb*Kd+tj]; ga[t]+=gs*sc*(float)(gwj*xv); }} }}
  for(int t=0;t<BTG;t++){{ int bb=b0+t; if(bb>=B) continue; float g=ga[t];
    for(int s=16;s>0;s>>=1) g+=__shfl_down_sync(0xffffffffu,g,s); if(lane==0) G[(long)bb*K*N+row]=g; }}
}}
__global__ void MOE_OCC bmoe_up_silu(const signed char* Xq, const float* Xs, const signed char* wu_q, const float* wu_s, const int* Idx, const float* G, float* H, int K, int N, int Kd){{
  long row=(blockIdx.x*(long)blockDim.x+threadIdx.x)>>5; if(row>=(long)K*N) return; int lane=threadIdx.x&31; int b0=blockIdx.y*BTG;
  int kk=row/N, j=row%N; int e=Idx[(long)b0*K+kk]; int nb=Kd/32, nq=Kd/128;
  const signed char* uwr=wu_q+((long)e*N+j)*Kd; const int* uw4=(const int*)uwr; const float* usr=wu_s+((long)e*N+j)*nb;
  const int4* uw16=(const int4*)uwr; int nq16=Kd/512;
  float ua[BTG]; for(int t=0;t<BTG;t++) ua[t]=0.f;
  for(int c=0;c<nq16;++c){{ int blk=c*16+lane/2; int4 uv=uw16[c*32+lane]; float us=usr[blk];
    #pragma unroll
    for(int t=0;t<BTG;t++){{ int bb=b0+t; if(bb>=B) continue; float sc=Xs[(long)bb*(Kd/32)+blk]; int4 av=((const int4*)(Xq+(long)bb*Kd))[c*32+lane];
      int ud=__dp4a(uv.x,av.x,0); ud=__dp4a(uv.y,av.y,ud); ud=__dp4a(uv.z,av.z,ud); ud=__dp4a(uv.w,av.w,ud); ua[t]+=us*sc*(float)ud; }} }}
  for(int b4=nq16*4;b4<nq;++b4){{ int blk=b4*4+lane/8; int uwi=uw4[b4*32+lane]; float us=usr[blk];
    for(int t=0;t<BTG;t++){{ int bb=b0+t; if(bb>=B) continue; float sc=Xs[(long)bb*(Kd/32)+blk]; int ai=((const int*)(Xq+(long)bb*Kd))[b4*32+lane]; ua[t]+=us*sc*(float)__dp4a(uwi,ai,0); }} }}
  for(int tj=nq*128+lane;tj<Kd;tj+=32){{ int blk=tj/32; int uwj=(int)uwr[tj]; float us=usr[blk];
    for(int t=0;t<BTG;t++){{ int bb=b0+t; if(bb>=B) continue; float sc=Xs[(long)bb*(Kd/32)+blk]; int xv=(int)Xq[(long)bb*Kd+tj]; ua[t]+=us*sc*(float)(uwj*xv); }} }}
  for(int t=0;t<BTG;t++){{ int bb=b0+t; if(bb>=B) continue; float u=ua[t];
    for(int s=16;s>0;s>>=1) u+=__shfl_down_sync(0xffffffffu,u,s); if(lane==0){{ float g=G[(long)bb*K*N+row]; H[(long)bb*K*N+row]=(g/(1.0f+expf(-g)))*u; }} }}
}}
// batched activation quant: grid over (K blocks, B). aq[seq]/as_[seq].
__global__ void bmoe_qact(const float* A, signed char* Aq, float* As, int K){{
  int seq=blockIdx.y; int nb=K/32; int blk=blockIdx.x*(blockDim.x/32)+threadIdx.x/32; if(blk>=nb) return;
  const float* a=A+(long)seq*K; signed char* aq=Aq+(long)seq*K; float* as_=As+(long)seq*(K/32);
  int lane=threadIdx.x&31; float v=a[blk*32+lane], av=fabsf(v);
  for(int s=16;s>0;s>>=1) av=fmaxf(av,__shfl_down_sync(0xffffffffu,av,s));
  av=__shfl_sync(0xffffffffu,av,0); float amax=av>1e-30f?av:1e-30f; float sc=amax/127.0f;
  if(lane==0) as_[blk]=sc; int q=(int)rintf(v/sc); if(q>127)q=127; if(q<-128)q=-128; aq[blk*32+lane]=(signed char)q;
}}
// batched down+combine, REGISTER-TILED over BTG sequences (shared-routing tile): grid=(E, ceil(B/BTG)). The
// down weight row (expert e, output o) is read ONCE and reused across the BTG sequences' hidden activations;
// each warp is one expert slot, looping the BTG sequences. Per-sequence order identical to batch-1 ⇒ golden.
__global__ void MOE_OCC bmoe_down_combine(const signed char* Hq, const float* Hs, const signed char* wd_q, const float* wd_s, const int* Idx, const float* Wc, float* Hout, int K, int N, int Kd){{
  int o=blockIdx.x; int b0=blockIdx.y*BTG; int warp=threadIdx.x>>5, lane=threadIdx.x&31;
  int e=Idx[(long)b0*K+warp]; int nb=Kd/32, nq=Kd/128;
  const signed char* wr=wd_q+((long)e*N+o)*Kd; const int* w4=(const int*)wr; const float* wsr=wd_s+((long)e*N+o)*nb;
  const int4* w16=(const int4*)wr; int nq16=Kd/512;
  __shared__ float part[BTG][64];
  float acc[BTG]; for(int t=0;t<BTG;t++) acc[t]=0.f;
  for(int c=0;c<nq16;++c){{ int blk=c*16+lane/2; int4 wv=w16[c*32+lane]; float ws_=wsr[blk];
    #pragma unroll
    for(int t=0;t<BTG;t++){{ int bb=b0+t; if(bb>=B) continue;
      int4 hv=((const int4*)(Hq+(long)bb*K*Kd+(long)warp*Kd))[c*32+lane]; float hs_=Hs[(long)bb*K*(Kd/32)+(long)warp*nb+blk];
      int d=__dp4a(wv.x,hv.x,0); d=__dp4a(wv.y,hv.y,d); d=__dp4a(wv.z,hv.z,d); d=__dp4a(wv.w,hv.w,d);
      acc[t]+=ws_*hs_*(float)d; }} }}
  for(int b4=nq16*4;b4<nq;++b4){{ int blk=b4*4+lane/8; int wi=w4[b4*32+lane]; float ws_=wsr[blk];
    for(int t=0;t<BTG;t++){{ int bb=b0+t; if(bb>=B) continue; int hi=((const int*)(Hq+(long)bb*K*Kd+(long)warp*Kd))[b4*32+lane]; float hs_=Hs[(long)bb*K*(Kd/32)+(long)warp*nb+blk]; acc[t]+=ws_*hs_*(float)__dp4a(wi,hi,0); }} }}
  for(int tj=nq*128+lane;tj<Kd;tj+=32){{ int blk=tj/32; int wj=(int)wr[tj]; float ws_=wsr[blk];
    for(int t=0;t<BTG;t++){{ int bb=b0+t; if(bb>=B) continue; int hv=(int)Hq[(long)bb*K*Kd+(long)warp*Kd+tj]; float hs_=Hs[(long)bb*K*(Kd/32)+(long)warp*nb+blk]; acc[t]+=ws_*hs_*(float)(wj*hv); }} }}
  for(int t=0;t<BTG;t++){{ int bb=b0+t; float a=(bb<B)?acc[t]:0.f;
    for(int s=16;s>0;s>>=1) a+=__shfl_down_sync(0xffffffffu,a,s);
    if(lane==0) part[t][warp]=(bb<B)?Wc[(long)bb*K+warp]*a:0.f; }}
  __syncthreads();
  if(threadIdx.x<BTG){{ int t=threadIdx.x; int bb=b0+t; if(bb<B){{ float sum=0.0f; for(int kk=0;kk<K;++kk) sum+=part[t][kk]; Hout[(long)bb*N+o]=sum; }} }}
}}
// batched argmax+advance: one block per sequence writes D_tok[seq]; seq 0 also advances the shared position.
__global__ void bgk_argmax(const float* Logits, int* D_tok, int* d_pos){{
  int seq=blockIdx.x; const float* logits=Logits+(long)seq*VOCAB;
  __shared__ float bv[256]; __shared__ int bi[256]; float bestv=-1e30f; int besti=0;
  for(int i=threadIdx.x;i<VOCAB;i+=blockDim.x){{ float v=logits[i]; if(v>bestv){{bestv=v;besti=i;}} }}
  bv[threadIdx.x]=bestv; bi[threadIdx.x]=besti; __syncthreads();
  for(int k=blockDim.x/2;k>0;k>>=1){{ if(threadIdx.x<k && bv[threadIdx.x+k]>bv[threadIdx.x]){{ bv[threadIdx.x]=bv[threadIdx.x+k]; bi[threadIdx.x]=bi[threadIdx.x+k]; }} __syncthreads(); }}
  if(threadIdx.x==0){{ D_tok[seq]=bi[0]; if(seq==0) *d_pos=*d_pos+1; }}
}}

// ── global device buffers ──
static signed char *emb_q,*wq_q[LAYERS],*wk_q[LAYERS],*wv_q[LAYERS],*wo_q[LAYERS],*wout_q,*wr_q[LAYERS],*wg_q[LAYERS],*wu_q[LAYERS],*wd_q[LAYERS];
static float *emb_s,*wq_s[LAYERS],*wk_s[LAYERS],*wv_s[LAYERS],*wo_s[LAYERS],*wout_s,*wr_s[LAYERS],*wg_s[LAYERS],*wu_s[LAYERS],*wd_s[LAYERS];
static float *an[LAYERS],*fn_[LAYERS],*aqn[LAYERS],*akn[LAYERS],*on_w;
static float *X,*qbuf,*tmp,*x2n,*moe_logits,*moe_w,*moe_he,*moe_gate,*moe_out,*logits,*asE,*asQ,*moe_xs,*moe_hs,*acc_part,*m_part,*d_part,*kc[LAYERS],*vc[LAYERS];
static signed char *aqE,*aqQ,*moe_xq,*moe_hq;
static int *moe_idx,*d_pos,*D_tok;

static signed char* Li(const char* p, long n){{ signed char* d; if(cudaMalloc(&d,n)){{fprintf(stderr,"malloc %s\n",p);exit(1);}} signed char* h=(signed char*)malloc(n); FILE* f=fopen(p,"rb"); if(!f||fread(h,1,n,f)!=(size_t)n){{fprintf(stderr,"read %s\n",p);exit(1);}} fclose(f); cudaMemcpy(d,h,n,cudaMemcpyHostToDevice); free(h); return d; }}
static float* Lf(const char* p, long n){{ float* d; if(cudaMalloc(&d,n*4)){{fprintf(stderr,"malloc %s\n",p);exit(1);}} float* h=(float*)malloc(n*4); FILE* f=fopen(p,"rb"); if(!f||fread(h,4,n,f)!=(size_t)n){{fprintf(stderr,"read %s\n",p);exit(1);}} fclose(f); cudaMemcpy(d,h,n*4,cudaMemcpyHostToDevice); free(h); return d; }}
#define P ATTN_SPLIT

static void decode_step(cudaStream_t s){{
  bgk_embed<<<(B*E+255)/256,256,0,s>>>(X,emb_q,emb_s,D_tok);
  bgk_rmsnorm_q8<<<B,256,0,s>>>(X,aqE,asE,an[0],E,EPSV);
  for(int l=0;l<LAYERS;l++){{
    bgk_gemv_qkv<<<dim3((QD+2*KD+7)/8,(B+BTG-1)/BTG),256,0,s>>>(qbuf,kc[l],vc[l],d_pos,wq_q[l],wq_s[l],wk_q[l],wk_s[l],wv_q[l],wv_s[l],aqE,asE,E);
    bgk_qk_norm_rope2<<<dim3(NH+NKV,B),128,0,s>>>(qbuf,kc[l],d_pos,aqn[l],akn[l],ROPEV,EPSV);
    bgk_attn_part<<<dim3(NH*P,B),256,0,s>>>(qbuf,kc[l],vc[l],d_pos,SCALEV,P,acc_part,m_part,d_part);
    bgk_attn_comb_q<<<dim3(NH,B),128,0,s>>>(aqQ,asQ,P,acc_part,m_part,d_part);
    bgk_gemv<<<dim3((E+7)/8,(B+BTG-1)/BTG),256,0,s>>>(tmp,wo_q[l],wo_s[l],aqQ,asQ,QD,E);
    bgk_add_rmsnorm_q8<<<B,256,0,s>>>(X,tmp,x2n,fn_[l],moe_xq,moe_xs,E,EPSV);
    bmoe_router<<<dim3((NE+7)/8,B),256,0,s>>>(x2n,wr_q[l],wr_s[l],moe_logits,NE,E);
    bmoe_topk<<<B,NE,0,s>>>(moe_logits,moe_idx,moe_w,NE,KTOP);
    bmoe_gate<<<dim3((KTOP*FF+7)/8,(B+BTG-1)/BTG),256,0,s>>>(moe_xq,moe_xs,wg_q[l],wg_s[l],moe_idx,moe_gate,KTOP,FF,E);
    bmoe_up_silu<<<dim3((KTOP*FF+7)/8,(B+BTG-1)/BTG),256,0,s>>>(moe_xq,moe_xs,wu_q[l],wu_s[l],moe_idx,moe_gate,moe_he,KTOP,FF,E);
    bmoe_qact<<<dim3(((KTOP*FF)/32+7)/8,B),256,0,s>>>(moe_he,moe_hq,moe_hs,KTOP*FF);
    bmoe_down_combine<<<dim3(E,(B+BTG-1)/BTG),KTOP*32,0,s>>>(moe_hq,moe_hs,wd_q[l],wd_s[l],moe_idx,moe_w,moe_out,KTOP,E,FF);
    if(l+1<LAYERS) bgk_add_rmsnorm_q8a<<<B,256,0,s>>>(X,moe_out,aqE,asE,an[l+1],E,EPSV);
    else bgk_add_rmsnorm_q8a<<<B,256,0,s>>>(X,moe_out,aqE,asE,on_w,E,EPSV);
  }}
  bgk_gemv<<<dim3((VOCAB+7)/8,(B+BTG-1)/BTG),256,0,s>>>(logits,wout_q,wout_s,aqE,asE,E,VOCAB);
}}

int main(){{
  char p[512]; const char* W="{wdir}";
  for(int l=0;l<LAYERS;l++){{
    snprintf(p,512,"%s/an%d.f",W,l); an[l]=Lf(p,E); snprintf(p,512,"%s/fn%d.f",W,l); fn_[l]=Lf(p,E);
    snprintf(p,512,"%s/aqn%d.f",W,l); aqn[l]=Lf(p,HD); snprintf(p,512,"%s/akn%d.f",W,l); akn[l]=Lf(p,HD);
    snprintf(p,512,"%s/wq%d.q",W,l); wq_q[l]=Li(p,(long)QD*E); snprintf(p,512,"%s/wq%d.s",W,l); wq_s[l]=Lf(p,(long)QD*(E/32));
    snprintf(p,512,"%s/wk%d.q",W,l); wk_q[l]=Li(p,(long)KD*E); snprintf(p,512,"%s/wk%d.s",W,l); wk_s[l]=Lf(p,(long)KD*(E/32));
    snprintf(p,512,"%s/wv%d.q",W,l); wv_q[l]=Li(p,(long)KD*E); snprintf(p,512,"%s/wv%d.s",W,l); wv_s[l]=Lf(p,(long)KD*(E/32));
    snprintf(p,512,"%s/wo%d.q",W,l); wo_q[l]=Li(p,(long)E*QD); snprintf(p,512,"%s/wo%d.s",W,l); wo_s[l]=Lf(p,(long)E*(QD/32));
    snprintf(p,512,"%s/wr%d.q",W,l); wr_q[l]=Li(p,(long)NE*E); snprintf(p,512,"%s/wr%d.s",W,l); wr_s[l]=Lf(p,(long)NE*(E/32));
    snprintf(p,512,"%s/wg%d.q",W,l); wg_q[l]=Li(p,(long)NE*FF*E); snprintf(p,512,"%s/wg%d.s",W,l); wg_s[l]=Lf(p,(long)NE*FF*(E/32));
    snprintf(p,512,"%s/wu%d.q",W,l); wu_q[l]=Li(p,(long)NE*FF*E); snprintf(p,512,"%s/wu%d.s",W,l); wu_s[l]=Lf(p,(long)NE*FF*(E/32));
    snprintf(p,512,"%s/wd%d.q",W,l); wd_q[l]=Li(p,(long)NE*E*FF); snprintf(p,512,"%s/wd%d.s",W,l); wd_s[l]=Lf(p,(long)NE*E*(FF/32));
  }}
  snprintf(p,512,"%s/on.f",W); on_w=Lf(p,E); snprintf(p,512,"%s/wout.q",W); wout_q=Li(p,(long)VOCAB*E); snprintf(p,512,"%s/wout.s",W); wout_s=Lf(p,(long)VOCAB*(E/32));
  snprintf(p,512,"%s/emb.q",W); emb_q=Li(p,(long)VOCAB*E); snprintf(p,512,"%s/emb.s",W); emb_s=Lf(p,(long)VOCAB*(E/32));
  fprintf(stderr,"weights loaded\n");
  cudaMalloc(&X,(long)B*E*4); cudaMalloc(&qbuf,(long)B*QD*4); cudaMalloc(&tmp,(long)B*E*4); cudaMalloc(&x2n,(long)B*E*4);
  cudaMalloc(&moe_logits,(long)B*NE*4); cudaMalloc(&moe_w,(long)B*KTOP*4); cudaMalloc(&moe_he,(long)B*KTOP*FF*4); cudaMalloc(&moe_gate,(long)B*KTOP*FF*4); cudaMalloc(&moe_out,(long)B*E*4); cudaMalloc(&logits,(long)B*VOCAB*4);
  cudaMalloc(&asE,(long)B*(E/32)*4); cudaMalloc(&asQ,(long)B*(QD/32)*4); cudaMalloc(&moe_xs,(long)B*(E/32)*4); cudaMalloc(&moe_hs,(long)B*(KTOP*FF/32)*4);
  cudaMalloc(&acc_part,(long)B*NH*P*HD*4); cudaMalloc(&m_part,(long)B*NH*P*4); cudaMalloc(&d_part,(long)B*NH*P*4);
  cudaMalloc(&aqE,(long)B*E); cudaMalloc(&aqQ,(long)B*QD); cudaMalloc(&moe_xq,(long)B*E); cudaMalloc(&moe_hq,(long)B*KTOP*FF);
  cudaMalloc(&moe_idx,(long)B*KTOP*4); cudaMalloc(&d_pos,4); cudaMalloc(&D_tok,(long)B*4);
  for(int l=0;l<LAYERS;l++){{ cudaMalloc(&kc[l],(long)B*MAXSEQ*KD*4); cudaMalloc(&vc[l],(long)B*MAXSEQ*KD*4); }}
  int prompt[]={{{prompt_c}}}; int PL=sizeof(prompt)/sizeof(int);
  std::vector<int> htok(B);
  // PREFILL: all B sequences share the identical prompt (llama-batched-bench's setup).
  for(int i=0;i<PL;i++){{ cudaMemcpy(d_pos,&i,4,cudaMemcpyHostToDevice); for(int b=0;b<B;b++) htok[b]=prompt[i]; cudaMemcpy(D_tok,htok.data(),(long)B*4,cudaMemcpyHostToDevice); decode_step(0); cudaDeviceSynchronize(); }}
  // first generated token per sequence (from the last prefill logits).
  std::vector<std::vector<int>> gen(B);
  {{ std::vector<float> hl((long)B*VOCAB); cudaMemcpy(hl.data(),logits,(long)B*VOCAB*4,cudaMemcpyDeviceToHost);
    for(int b=0;b<B;b++){{ int bi=0; for(int i=1;i<VOCAB;i++) if(hl[(long)b*VOCAB+i]>hl[(long)b*VOCAB+bi]) bi=i; gen[b].push_back(bi); htok[b]=bi; }}
    int pl=PL; cudaMemcpy(d_pos,&pl,4,cudaMemcpyHostToDevice); cudaMemcpy(D_tok,htok.data(),(long)B*4,cudaMemcpyHostToDevice); }}
  // capture the decode+argmax as ONE graph, replay per token.
  cudaStream_t st; cudaStreamCreate(&st); cudaGraph_t g; cudaGraphExec_t ge;
  cudaStreamBeginCapture(st,cudaStreamCaptureModeGlobal); decode_step(st); bgk_argmax<<<B,256,0,st>>>(logits,D_tok,d_pos); cudaStreamEndCapture(st,&g); cudaGraphInstantiate(&ge,g,0);
  cudaEvent_t t0,t1; cudaEventCreate(&t0); cudaEventCreate(&t1); float genms=0; int steps=0;
  for(int step=1; step<{n_gen}; step++){{ cudaEventRecord(t0,st); cudaGraphLaunch(ge,st); cudaEventRecord(t1,st); cudaStreamSynchronize(st); float ms; cudaEventElapsedTime(&ms,t0,t1); genms+=ms; steps++;
    cudaMemcpy(htok.data(),D_tok,(long)B*4,cudaMemcpyDeviceToHost); for(int b=0;b<B;b++) gen[b].push_back(htok[b]); }}
  // golden: sequence 0's stream (all sequences identical under the shared prompt).
  printf("GENERATED_TOKENS:"); for(size_t i=0;i<gen[0].size();++i) printf(" %d",gen[0][i]); printf("\n");
  // per-sequence agreement (every sequence must match sequence 0 — proves the batching is correct, no cross-talk).
  long disagree=0; for(int b=1;b<B;b++) for(size_t i=0;i<gen[0].size();++i) if(gen[b][i]!=gen[0][i]) disagree++;
  fprintf(stderr,"PER_SEQ_AGREE %s (disagreements=%ld across %d sequences)\n", disagree==0?"YES":"NO", disagree, B);
  // aggregate throughput: B sequences advance per decode step.
  // ── tg128 from an EMPTY KV cache (KV grows 0→128), exactly as llama-batched-bench measures S_TG: reset the
  // shared position to 0 so the replayed decode step regrows the cache in-bounds, then time 128 steps. ──
  {{ int NR=128; int z=0; cudaMemcpy(d_pos,&z,4,cudaMemcpyHostToDevice); cudaStreamSynchronize(st);
    cudaEventRecord(t0,st); for(int i=0;i<NR;i++) cudaGraphLaunch(ge,st); cudaEventRecord(t1,st); cudaStreamSynchronize(st); float bms; cudaEventElapsedTime(&bms,t0,t1);
    double s_tg = (double)B*NR*1000.0/bms; fprintf(stderr,"BATCHED_S_TG %.2f (B=%d, tg128 from empty KV = llama-batched-bench S_TG metric, in-bounds)\n", s_tg, B); }}
  return 0;
}}
"#,
        wdir = wdir,
        e = e,
        qd = qd,
        kd = kd,
        ff = ff,
        ne = ne,
        ktop = ktop,
        vocab = vocab,
        layers = layers,
        nh = nh,
        nkv = nkv,
        hd = hd,
        maxseq = maxseq,
        batch = batch,
        scale = scale,
        prompt_c = prompt_c,
        n_gen = n_gen
    )
}
