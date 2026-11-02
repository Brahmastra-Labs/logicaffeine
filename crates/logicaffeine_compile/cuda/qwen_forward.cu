// End-to-end GPU forward of Qwen3-8B (Q8_0) on one CUDA device.
//
// Weights stay resident in VRAM as Q8_0 int8 (~8.2GB, fits a 12GB card); each matmul dequantizes the
// weight to f16 into a reused buffer and runs a cuBLAS tensor-core GEMM (f16 in, f32 accumulate). The
// transformer's elementwise stages (RMSNorm, QK-RMSNorm, RoPE-NEOX, GQA causal attention, SwiGLU) are
// custom kernels. Generates greedily and prints the token ids — validated against llama.cpp's golden.
//
//   nvcc -O3 -arch=sm_86 qwen_forward.cu -lcublas -o qwen_forward
//   ./qwen_forward MODEL.gguf t0 t1 t2 ...        # prompt token ids; prints N generated ids
//
// Self-contained: a minimal GGUF reader, no ggml/llama.cpp.

#include <cstdio>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <cmath>
#include <string>
#include <vector>
#include <unordered_map>

#include <fcntl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

#include <cuda_fp16.h>
#include <cuda_runtime.h>
#include <cublas_v2.h>

#define CK(x) do { cudaError_t e=(x); if(e!=cudaSuccess){fprintf(stderr,"cuda %s:%d %s\n",__FILE__,__LINE__,cudaGetErrorString(e));exit(1);} } while(0)
#define CB(x) do { cublasStatus_t s=(x); if(s!=CUBLAS_STATUS_SUCCESS){fprintf(stderr,"cublas %s:%d %d\n",__FILE__,__LINE__,(int)s);exit(1);} } while(0)

// ───────────────────────────── minimal GGUF reader (mmap) ─────────────────────────────
enum { T_U8,T_I8,T_U16,T_I16,T_U32,T_I32,T_F32,T_BOOL,T_STRING,T_ARRAY,T_U64,T_I64,T_F64 };
static const uint32_t GGML_Q8_0 = 8, GGML_F32 = 0;

struct Tensor { std::vector<uint64_t> dims; uint32_t type; uint64_t offset; };

struct Gguf {
    const uint8_t* base; size_t size; uint64_t data_start;
    std::unordered_map<std::string, Tensor> tensors;
    std::unordered_map<std::string, uint64_t> ints;
    std::unordered_map<std::string, float> floats;
    std::string arch;
};

struct Rd { const uint8_t* p; };
static uint32_t r32(Rd& r){ uint32_t v; memcpy(&v,r.p,4); r.p+=4; return v; }
static uint64_t r64(Rd& r){ uint64_t v; memcpy(&v,r.p,8); r.p+=8; return v; }
static std::string rstr(Rd& r){ uint64_t n=r64(r); std::string s((const char*)r.p,n); r.p+=n; return s; }
static void skip_val(Rd& r, uint32_t t);
static int vwidth(uint32_t t){ switch(t){case T_U8:case T_I8:case T_BOOL:return 1;case T_U16:case T_I16:return 2;case T_U32:case T_I32:case T_F32:return 4;case T_U64:case T_I64:case T_F64:return 8;default:return -1;} }
static void skip_val(Rd& r, uint32_t t){
    if(t==T_STRING){ uint64_t n=r64(r); r.p+=n; }
    else if(t==T_ARRAY){ uint32_t et=r32(r); uint64_t c=r64(r); int w=vwidth(et); if(w>0) r.p+=w*c; else for(uint64_t i=0;i<c;i++) skip_val(r,et); }
    else { int w=vwidth(t); r.p+=(w>0?w:0); }
}

static Gguf gguf_open(const char* path){
    int fd=open(path,O_RDONLY); if(fd<0){perror("open");exit(1);}
    struct stat st; fstat(fd,&st);
    const uint8_t* base=(const uint8_t*)mmap(nullptr,st.st_size,PROT_READ,MAP_SHARED,fd,0);
    if(base==MAP_FAILED){perror("mmap");exit(1);}
    Gguf g; g.base=base; g.size=st.st_size;
    Rd r{base+4}; /* skip "GGUF" */ uint32_t ver=r32(r); (void)ver;
    uint64_t n_tensors=r64(r), n_kv=r64(r);
    for(uint64_t i=0;i<n_kv;i++){
        std::string key=rstr(r); uint32_t vt=r32(r);
        if(key=="general.architecture" && vt==T_STRING) g.arch=rstr(r);
        else if(vt==T_F32){ float f; memcpy(&f,r.p,4); r.p+=4; g.floats[key]=f; }
        else if(vt==T_U32) g.ints[key]=r32(r);
        else if(vt==T_U64) g.ints[key]=r64(r);
        else if(vt==T_I32) g.ints[key]=(uint64_t)(int32_t)r32(r);
        else skip_val(r,vt);
    }
    for(uint64_t i=0;i<n_tensors;i++){
        std::string name=rstr(r); uint32_t nd=r32(r); Tensor t;
        for(uint32_t d=0;d<nd;d++) t.dims.push_back(r64(r));
        t.type=r32(r); t.offset=r64(r); g.tensors[name]=t;
    }
    uint64_t align = g.ints.count("general.alignment")?g.ints["general.alignment"]:32;
    uint64_t pos=(uint64_t)(r.p-base);
    g.data_start=((pos+align-1)/align)*align;
    return g;
}
static uint64_t gi(Gguf& g,const char* s){ return g.ints[g.arch+"."+s]; }
static float gf(Gguf& g,const char* s,float d){ auto k=g.arch+"."+s; return g.floats.count(k)?g.floats[k]:d; }

// ───────────────────────────── kernels ─────────────────────────────
// Dequantize a whole Q8_0 tensor (out x in) to f16: W[o*in + t] = q8 * scale. One thread per element.
__global__ void deq_q8_f16(__half* out, const uint8_t* q8, int out_dim, int in_dim){
    long idx = blockIdx.x*(long)blockDim.x + threadIdx.x;
    long total = (long)out_dim*in_dim;
    if(idx>=total) return;
    int t = idx % in_dim;               // in-feature
    long o = idx / in_dim;              // out-feature
    int nb = in_dim/32;
    const uint8_t* row = q8 + (o*nb + t/32)*34;
    __half sc = *(const __half*)row;
    int8_t q = (int8_t)row[2 + (t%32)];
    out[idx] = __float2half((float)q * __half2float(sc));
}
// Dequantize a single (deinterleaved) Q8_0 row to f32 (token embedding).
__global__ void deq_q8_row_f32(float* out, const int8_t* wqrow, const __half* wsrow, int K){
    int t = blockIdx.x*blockDim.x + threadIdx.x; if(t>=K) return;
    out[t] = (float)wqrow[t] * __half2float(wsrow[t/32]);
}
// RMSNorm per row: y = x/sqrt(mean(x^2)+eps)*w. One block per row.
__global__ void rmsnorm(float* out, const float* x, const float* w, int n, float eps){
    int row=blockIdx.x; const float* xr=x+(long)row*n; float* yr=out+(long)row*n;
    __shared__ float sh[256];
    float s=0; for(int i=threadIdx.x;i<n;i+=blockDim.x) s+=xr[i]*xr[i];
    sh[threadIdx.x]=s; __syncthreads();
    for(int k=blockDim.x/2;k>0;k>>=1){ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }
    float scale=rsqrtf(sh[0]/n+eps);
    for(int i=threadIdx.x;i<n;i+=blockDim.x) yr[i]=xr[i]*scale*w[i];
}
// Per-head QK-RMSNorm (over head_dim, weight[hd]) then RoPE-NEOX at ABSOLUTE position `pos0+tpos`.
__global__ void qk_norm_rope(float* v, const float* nw, int n_heads, int hd, int T, int pos0, float base, float eps){
    int head=blockIdx.x, tpos=blockIdx.y; if(head>=n_heads||tpos>=T) return;
    int pos=pos0+tpos;
    float* h = v + ((long)tpos*n_heads + head)*hd;
    __shared__ float sh[256];
    float s=0; for(int i=threadIdx.x;i<hd;i+=blockDim.x) s+=h[i]*h[i];
    sh[threadIdx.x]=s; __syncthreads();
    for(int k=blockDim.x/2;k>0;k>>=1){ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }
    float scale=rsqrtf(sh[0]/hd+eps); __syncthreads();
    for(int i=threadIdx.x;i<hd;i+=blockDim.x) h[i]=h[i]*scale*nw[i];
    __syncthreads();
    int half=hd/2;
    for(int i=threadIdx.x;i<half;i+=blockDim.x){
        float freq=powf(base,-2.0f*i/hd); float ang=pos*freq; float c=cosf(ang),si=sinf(ang);
        float x0=h[i],x1=h[i+half]; h[i]=x0*c-x1*si; h[i+half]=x0*si+x1*c;
    }
}
// GQA causal attention over the KV CACHE. q:[Tn,n_head,hd]; kc,vc:[seqlen,n_kv,hd]. New query local i has
// absolute position pos0+i and attends cached keys 0..pos0+i. One block per (head, local query i).
__global__ void attention_kv(float* out, const float* q, const float* kc, const float* vc,
                          int Tn, int pos0, int n_head, int n_kv, int hd, float scale){
    int head=blockIdx.x, i=blockIdx.y; if(head>=n_head||i>=Tn) return;
    int kvh=head/(n_head/n_kv);
    int nabs=pos0+i;  // attend j = 0..nabs
    const float* qi = q + ((long)i*n_head+head)*hd;
    extern __shared__ float sc[];
    for(int j=threadIdx.x;j<=nabs;j+=blockDim.x){
        const float* kj = kc + ((long)j*n_kv+kvh)*hd;
        float d=0; for(int e=0;e<hd;e++) d+=qi[e]*kj[e];
        sc[j]=d*scale;
    }
    __syncthreads();
    if(threadIdx.x==0){ float m=-1e30f; for(int j=0;j<=nabs;j++) m=fmaxf(m,sc[j]);
        float sum=0; for(int j=0;j<=nabs;j++){ sc[j]=expf(sc[j]-m); sum+=sc[j]; }
        for(int j=0;j<=nabs;j++) sc[j]/=sum; }
    __syncthreads();
    float* oi = out + ((long)i*n_head+head)*hd;
    for(int e=threadIdx.x;e<hd;e+=blockDim.x){ float a=0; for(int j=0;j<=nabs;j++) a+=sc[j]*vc[((long)j*n_kv+kvh)*hd+e]; oi[e]=a; }
}
// ── device-state kernels for the CUDA-graph generation loop (T=1) — position + token live in device
// memory so a single captured graph replays every token with zero per-launch overhead ──
__global__ void embed_dev(float* x, const int8_t* wq, const __half* ws, const int* d_tok, int E){
    int t=blockIdx.x*blockDim.x+threadIdx.x; if(t>=E) return;
    long tok=*d_tok; const int8_t* wr=wq+tok*E; const __half* sr=ws+tok*(E/32);
    x[t]=(float)wr[t]*__half2float(sr[t/32]);
}
// K/V GEMV writing into the cache at row *d_pos (dynamic offset computed on device).
__global__ void q8_gemv_kv(float* kc_base, const int* d_pos, int KD, const int8_t* __restrict__ wq,
                           const __half* __restrict__ ws, const float* __restrict__ x, int K, int N){
    int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31;
    const char4* w4=(const char4*)(wq+(long)o*K); const __half* sr=ws+(long)o*(K/32);
    float acc=0.f; int nq=K/128;
    for(int b4=0;b4<nq;b4++){ char4 w=w4[b4*32+lane]; float sc=__half2float(sr[b4*4+lane/8]);
        const float* xt=x+b4*128+lane*4; acc+=sc*((float)w.x*xt[0]+(float)w.y*xt[1]+(float)w.z*xt[2]+(float)w.w*xt[3]); }
    for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s);
    if(lane==0) kc_base[(long)(*d_pos)*KD + o]=acc;
}
// Per-head QK-RMSNorm + RoPE at device position *d_pos (T=1). `at_pos` already points at the row.
__global__ void qk_norm_rope_dev(float* base, const int* d_pos, int stride, const float* nw,
                                 int n_heads, int hd, float rbase, float eps){
    int head=blockIdx.x; if(head>=n_heads) return;
    float* h = base + (long)(*d_pos)*stride + head*hd;
    __shared__ float sh[128];
    float s=0; for(int i=threadIdx.x;i<hd;i+=blockDim.x) s+=h[i]*h[i]; sh[threadIdx.x]=s; __syncthreads();
    for(int k=blockDim.x/2;k>0;k>>=1){ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }
    float sc=rsqrtf(sh[0]/hd+eps); __syncthreads();
    for(int i=threadIdx.x;i<hd;i+=blockDim.x) h[i]=h[i]*sc*nw[i]; __syncthreads();
    int half=hd/2; int pos=*d_pos;
    for(int i=threadIdx.x;i<half;i+=blockDim.x){ float f=powf(rbase,-2.0f*i/hd); float a=pos*f,c=cosf(a),si=sinf(a);
        float x0=h[i],x1=h[i+half]; h[i]=x0*c-x1*si; h[i+half]=x0*si+x1*c; }
}
// GQA causal attention for the single new query at device position *d_pos, over cache 0..*d_pos.
__global__ void attn_dev(float* out, const float* q, const float* kc, const float* vc, const int* d_pos,
                         int n_head, int n_kv, int hd, float scale){
    int head=blockIdx.x; if(head>=n_head) return; int kvh=head/(n_head/n_kv); int nabs=*d_pos;
    const float* qi=q+head*hd; extern __shared__ float sc[];
    for(int j=threadIdx.x;j<=nabs;j+=blockDim.x){ const float* kj=kc+((long)j*n_kv+kvh)*hd;
        float d=0; for(int e=0;e<hd;e++) d+=qi[e]*kj[e]; sc[j]=d*scale; }
    __syncthreads();
    if(threadIdx.x==0){ float m=-1e30f; for(int j=0;j<=nabs;j++) m=fmaxf(m,sc[j]);
        float sum=0; for(int j=0;j<=nabs;j++){ sc[j]=expf(sc[j]-m); sum+=sc[j]; } for(int j=0;j<=nabs;j++) sc[j]/=sum; }
    __syncthreads();
    float* oi=out+head*hd;
    for(int e=threadIdx.x;e<hd;e+=blockDim.x){ float a=0; for(int j=0;j<=nabs;j++) a+=sc[j]*vc[((long)j*n_kv+kvh)*hd+e]; oi[e]=a; }
}
// Argmax of logits -> *d_tok, and advance the position (one block).
__global__ void argmax_advance(const float* logits, int vocab, int* d_tok, int* d_pos){
    __shared__ float bv[256]; __shared__ int bi[256];
    float bestv=-1e30f; int besti=0;
    for(int i=threadIdx.x;i<vocab;i+=blockDim.x){ float v=logits[i]; if(v>bestv){bestv=v;besti=i;} }
    bv[threadIdx.x]=bestv; bi[threadIdx.x]=besti; __syncthreads();
    for(int k=blockDim.x/2;k>0;k>>=1){ if(threadIdx.x<k && bv[threadIdx.x+k]>bv[threadIdx.x]){ bv[threadIdx.x]=bv[threadIdx.x+k]; bi[threadIdx.x]=bi[threadIdx.x+k]; } __syncthreads(); }
    if(threadIdx.x==0){ *d_tok=bi[0]; *d_pos=*d_pos+1; }
}

// Quantize a length-K f32 activation to int8 per 32-block (symmetric, like Q8_0): xq[i]=round(x/scale),
// xs[b]=max|x| over the block / 127. One warp per 32-block. Enables the int8 dp4a GEMV below.
__global__ void quant_act(const float* __restrict__ x, int8_t* __restrict__ xq, float* __restrict__ xs, int K){
    int blk=blockIdx.x*(blockDim.x/32) + threadIdx.x/32; if((long)blk*32>=K) return;
    int lane=threadIdx.x&31;
    float v=x[blk*32+lane], a=fabsf(v);
    for(int s=16;s>0;s>>=1) a=fmaxf(a,__shfl_down_sync(0xffffffffu,a,s));
    a=__shfl_sync(0xffffffffu,a,0);
    float scale = a>0 ? a/127.0f : 1.0f;
    if(lane==0) xs[blk]=scale;
    xq[blk*32+lane]=(int8_t)__float2int_rn(v/scale);
}
// Fused RMSNorm + int8 quantize (single row): y = x·rms·w, then quantize per 32-block → int8+scale.
// One block; each of 8 warps owns a stride of 32-blocks. Removes the separate quant pass + f32 round-trip.
__global__ void rmsnorm_q8(const float* __restrict__ x, int8_t* __restrict__ xq, float* __restrict__ xs,
                           const float* __restrict__ w, int n, float eps){
    __shared__ float red[256];
    float s=0; for(int i=threadIdx.x;i<n;i+=blockDim.x) s+=x[i]*x[i];
    red[threadIdx.x]=s; __syncthreads();
    for(int k=128;k>0;k>>=1){ if(threadIdx.x<k) red[threadIdx.x]+=red[threadIdx.x+k]; __syncthreads(); }
    float rms=rsqrtf(red[0]/n+eps);
    int warp=threadIdx.x>>5, lane=threadIdx.x&31, nb=n/32;
    for(int b=warp;b<nb;b+=8){
        float y=x[b*32+lane]*rms*w[b*32+lane], a=fabsf(y);
        for(int sh=16;sh>0;sh>>=1) a=fmaxf(a,__shfl_down_sync(0xffffffffu,a,sh));
        a=__shfl_sync(0xffffffffu,a,0); float sc=a>0?a/127.0f:1.0f;
        if(lane==0) xs[b]=sc; xq[b*32+lane]=(int8_t)__float2int_rn(y/sc);
    }
}
// Fused SwiGLU + int8 quantize: h = silu(gate)·up, quantized per 32-block. One block per 32-block group.
__global__ void swiglu_q8(const float* __restrict__ gate, const float* __restrict__ up, int8_t* __restrict__ hq,
                          float* __restrict__ hs, int n){
    int b=blockIdx.x*(blockDim.x/32)+threadIdx.x/32; if(b*32>=n) return; int lane=threadIdx.x&31;
    float g=gate[b*32+lane]; float y=(g/(1.0f+expf(-g)))*up[b*32+lane], a=fabsf(y);
    for(int sh=16;sh>0;sh>>=1) a=fmaxf(a,__shfl_down_sync(0xffffffffu,a,sh));
    a=__shfl_sync(0xffffffffu,a,0); float sc=a>0?a/127.0f:1.0f;
    if(lane==0) hs[b]=sc; hq[b*32+lane]=(int8_t)__float2int_rn(y/sc);
}
// ★ int8×int8 fused Q8_0 GEMV via __dp4a — the SOTA lane. Weight int8 + activation int8, integer dot
// (4 MACs/instruction), scaled by (weight block scale)·(activation block scale). Frees the ALU to hide
// memory latency → higher effective bandwidth than the float-MAC path.
__global__ void q8_gemv_dp4a(float* __restrict__ y, const int8_t* __restrict__ wq, const __half* __restrict__ ws,
                             const int8_t* __restrict__ xq, const float* __restrict__ xs, int K, int N){
    int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31;
    const int4* w16=(const int4*)(wq+(long)o*K); const int4* x16=(const int4*)xq; const __half* wsr=ws+(long)o*(K/32);
    float acc=0.f; int nq=K/512;  // 512B (int4=16 int8) per lane per iter — more memory in flight ⇒ better saturation
    for(int b=0;b<nq;b++){
        int4 w=w16[b*32+lane], xx=x16[b*32+lane]; int blk=b*16+lane/2; // all 16 int8 fall in one 32-block ⇒ one scale
        int d=__dp4a(w.x,xx.x,0)+__dp4a(w.y,xx.y,0)+__dp4a(w.z,xx.z,0)+__dp4a(w.w,xx.w,0);
        acc += (float)d * (__half2float(wsr[blk]) * xs[blk]);
    }
    for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s);
    if(lane==0) y[o]=acc;
}
// dp4a K/V GEMV writing into the cache at device row *d_pos.
__global__ void q8_gemv_kv_dp4a(float* kc_base, const int* d_pos, int KD, const int8_t* __restrict__ wq,
                                const __half* __restrict__ ws, const int8_t* __restrict__ xq,
                                const float* __restrict__ xs, int K, int N){
    int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31;
    const int4* w16=(const int4*)(wq+(long)o*K); const int4* x16=(const int4*)xq; const __half* wsr=ws+(long)o*(K/32);
    float acc=0.f; int nq=K/512;
    for(int b=0;b<nq;b++){ int4 w=w16[b*32+lane], xx=x16[b*32+lane]; int blk=b*16+lane/2;
        int d=__dp4a(w.x,xx.x,0)+__dp4a(w.y,xx.y,0)+__dp4a(w.z,xx.z,0)+__dp4a(w.w,xx.w,0);
        acc += (float)d * (__half2float(wsr[blk]) * xs[blk]); }
    for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s);
    if(lane==0) kc_base[(long)(*d_pos)*KD + o]=acc;
}

__global__ void swiglu(float* out, const float* gate, const float* up, long n){
    long i=blockIdx.x*(long)blockDim.x+threadIdx.x; if(i<n){ float g=gate[i]; out[i]=(g/(1.0f+expf(-g)))*up[i]; }
}
__global__ void add_inplace(float* x, const float* y, long n){ long i=blockIdx.x*(long)blockDim.x+threadIdx.x; if(i<n) x[i]+=y[i]; }

// ★ SOTA fused Q8_0 matmul: y(T x N) = x(T x K) . W(N x K)^T, reading DEINTERLEAVED int8 weights directly
// (contiguous int8 `wq` + separate per-32-block f16 scales `ws`) — no dequant, no f16 temp. One warp per
// output row; each lane loads a 128-byte-coalesced `char4` (4 contraction elements, same 32-block → same
// scale), folds the scale, accumulates against all T activation columns; one warp-reduce per column.
// Weight-bandwidth-bound: streams the 8GB of int8 exactly once per token at full coalesced bandwidth.
#define MAXT 16
__global__ void q8_matmul(float* y, const int8_t* __restrict__ wq, const __half* __restrict__ ws,
                          const float* __restrict__ x, int T, int K, int N){
    int o = (blockIdx.x*blockDim.x + threadIdx.x) >> 5; if(o>=N) return;
    int lane = threadIdx.x & 31;
    const int8_t* wrow = wq + (long)o*K;
    const __half* srow = ws + (long)o*(K/32);
    const char4* w4row = (const char4*)wrow;
    float acc[MAXT];
    #pragma unroll
    for(int t=0;t<MAXT;t++) acc[t]=0.f;
    int nq = K/128;                    // 128 contraction elements per warp-iteration
    for(int b4=0;b4<nq;b4++){
        char4 w = w4row[b4*32 + lane];               // 128 contiguous int8, coalesced
        float sc = __half2float(srow[b4*4 + lane/8]); // the 4 elements share one 32-block scale
        int base = b4*128 + lane*4;
        for(int t=0;t<T;t++){
            const float* xt = x + (long)t*K + base;
            acc[t] += sc*((float)w.x*xt[0] + (float)w.y*xt[1] + (float)w.z*xt[2] + (float)w.w*xt[3]);
        }
    }
    for(int t=0;t<T;t++){
        float a=acc[t];
        #pragma unroll
        for(int s=16;s>0;s>>=1) a += __shfl_down_sync(0xffffffffu, a, s);
        if(lane==0) y[(long)t*N + o] = a;
    }
}

// ★ The generation hot path: T=1 fused Q8_0 GEMV. A SINGLE accumulator → minimal registers → maximum
// occupancy → weight-bandwidth-bound (streams the 8GB of int8 once at near-peak coalesced bandwidth).
__global__ void q8_gemv(float* __restrict__ y, const int8_t* __restrict__ wq, const __half* __restrict__ ws,
                        const float* __restrict__ x, int K, int N){
    int o = (blockIdx.x*blockDim.x + threadIdx.x) >> 5; if(o>=N) return;
    int lane = threadIdx.x & 31;
    const char4* w4row = (const char4*)(wq + (long)o*K);
    const __half* srow = ws + (long)o*(K/32);
    float acc=0.f; int nq=K/128;
    for(int b4=0;b4<nq;b4++){
        char4 w = w4row[b4*32 + lane];
        float sc = __half2float(srow[b4*4 + lane/8]);
        const float* xt = x + b4*128 + lane*4;
        acc += sc*((float)w.x*xt[0] + (float)w.y*xt[1] + (float)w.z*xt[2] + (float)w.w*xt[3]);
    }
    #pragma unroll
    for(int s=16;s>0;s>>=1) acc += __shfl_down_sync(0xffffffffu, acc, s);
    if(lane==0) y[o]=acc;
}

// ───────────────────────────── host engine ─────────────────────────────
struct Model {
    Gguf g;
    int n_layer,n_embd,n_head,n_kv,hd,n_ff,vocab; float eps,rope_base;
    std::unordered_map<std::string,int8_t*> wq;   // deinterleaved int8 weights (contiguous rows)
    std::unordered_map<std::string,__half*> ws;    // per-32-block f16 scales
    std::unordered_map<std::string,float*> f32w;    // f32 norm weights
};

// Deinterleave a Q8_0 tensor (34-byte blocks: [f16 scale][32 int8]) into contiguous int8 + separate f16
// scales, and upload both — the layout the coalesced fused kernel needs.
static void upload_q8(Model& m, const std::string& name){
    Gguf& g=m.g; Tensor& t=g.tensors[name]; int in=t.dims[0], out=t.dims[1]; int nb=in/32;
    const uint8_t* raw = g.base+g.data_start+t.offset;
    std::vector<int8_t> hq((size_t)out*in);
    std::vector<uint16_t> hs((size_t)out*nb);
    for(long o=0;o<out;o++) for(int b=0;b<nb;b++){
        const uint8_t* blk = raw + ((size_t)o*nb+b)*34;
        memcpy(&hs[(size_t)o*nb+b], blk, 2);
        memcpy(&hq[(size_t)o*in + b*32], blk+2, 32);
    }
    int8_t* dq; __half* ds;
    CK(cudaMalloc(&dq,(size_t)out*in)); CK(cudaMalloc(&ds,(size_t)out*nb*2));
    CK(cudaMemcpy(dq,hq.data(),(size_t)out*in,cudaMemcpyHostToDevice));
    CK(cudaMemcpy(ds,hs.data(),(size_t)out*nb*2,cudaMemcpyHostToDevice));
    m.wq[name]=dq; m.ws[name]=ds;
}
static float* upload_f32(Gguf& g, const std::string& name){
    Tensor& t=g.tensors[name]; size_t n=1; for(auto d:t.dims) n*=d;
    float* d; CK(cudaMalloc(&d,n*4)); CK(cudaMemcpy(d, g.base+g.data_start+t.offset, n*4, cudaMemcpyHostToDevice)); return d;
}

// y(T x out) = x(T x in) . W(out x in)^T, via OUR fused Q8_0 kernel (int8 read directly, no dequant).
// x is f32; one warp per output row. Weight-bandwidth-bound.
static void matmul(Model& m, const std::string& wname, const float* x, float* y, int T, int in, int out){
    const int WPB=8; // warps per block -> 256 threads
    long grid=((long)out + WPB-1)/WPB;
    if(T==1) q8_gemv<<<grid, WPB*32>>>(y, m.wq[wname], m.ws[wname], x, in, out);
    else     q8_matmul<<<grid, WPB*32>>>(y, m.wq[wname], m.ws[wname], x, T, in, out);
}

int main(int argc, char** argv){
    if(argc<3){ fprintf(stderr,"usage: %s model.gguf tok0 tok1 ...\n",argv[0]); return 1; }
    std::vector<uint32_t> prompt; for(int i=2;i<argc;i++) prompt.push_back((uint32_t)atoi(argv[i]));
    const int N_GEN=5;

    Model m; m.g=gguf_open(argv[1]);
    Gguf& g=m.g;
    m.n_layer=gi(g,"block_count"); m.n_embd=gi(g,"embedding_length"); m.n_head=gi(g,"attention.head_count");
    m.n_kv=gi(g,"attention.head_count_kv"); m.hd=gi(g,"attention.key_length"); m.n_ff=gi(g,"feed_forward_length");
    m.eps=gf(g,"attention.layer_norm_rms_epsilon",1e-6f); m.rope_base=gf(g,"rope.freq_base",1e6f);
    m.vocab=g.tensors["output.weight"].dims[1];
    fprintf(stderr,"qwen3: %d layers d=%d heads=%d/%d hd=%d ff=%d vocab=%d eps=%g rope=%g\n",
        m.n_layer,m.n_embd,m.n_head,m.n_kv,m.hd,m.n_ff,m.vocab,m.eps,m.rope_base);

    // Upload weights resident.
    fprintf(stderr,"uploading weights...\n");
    upload_q8(m,"token_embd.weight");
    upload_q8(m,"output.weight");
    m.f32w["output_norm.weight"]=upload_f32(g,"output_norm.weight");
    for(int l=0;l<m.n_layer;l++){ char b[64];
        #define L(n) (snprintf(b,64,"blk.%d." n,l),std::string(b))
        upload_q8(m,L("attn_q.weight"));
        upload_q8(m,L("attn_k.weight"));
        upload_q8(m,L("attn_v.weight"));
        upload_q8(m,L("attn_output.weight"));
        upload_q8(m,L("ffn_gate.weight"));
        upload_q8(m,L("ffn_up.weight"));
        upload_q8(m,L("ffn_down.weight"));
        m.f32w[L("attn_norm.weight")]=upload_f32(g,L("attn_norm.weight"));
        m.f32w[L("attn_q_norm.weight")]=upload_f32(g,L("attn_q_norm.weight"));
        m.f32w[L("attn_k_norm.weight")]=upload_f32(g,L("attn_k_norm.weight"));
        m.f32w[L("ffn_norm.weight")]=upload_f32(g,L("ffn_norm.weight"));
    }
    // Device scratch — sized for the NEW tokens per step (prefill = the largest batch); all f32.
    int E=m.n_embd, HD=m.hd, QD=m.n_head*m.hd, KD=m.n_kv*m.hd, FF=m.n_ff;
    int maxSeq=prompt.size()+N_GEN, maxTn=prompt.size();
    float *x,*xn,*q,*attn,*tmp,*gate,*up,*hbuf,*logits;
    CK(cudaMalloc(&x,(long)maxTn*E*4)); CK(cudaMalloc(&xn,(long)maxTn*E*4));
    CK(cudaMalloc(&q,(long)maxTn*QD*4)); CK(cudaMalloc(&attn,(long)maxTn*QD*4)); CK(cudaMalloc(&tmp,(long)maxTn*E*4));
    CK(cudaMalloc(&gate,(long)maxTn*FF*4)); CK(cudaMalloc(&up,(long)maxTn*FF*4)); CK(cudaMalloc(&hbuf,(long)maxTn*FF*4));
    CK(cudaMalloc(&logits,(long)m.vocab*4));
    // int8 activation-quant scratch (dp4a lane): aqE covers all E-sized inputs (xn/xn2/attn: QD==E), aqFF the FFN.
    int8_t *aqE,*aqFF; float *asE,*asFF;
    CK(cudaMalloc(&aqE,E)); CK(cudaMalloc(&asE,(long)(E/32)*4));
    CK(cudaMalloc(&aqFF,FF)); CK(cudaMalloc(&asFF,(long)(FF/32)*4));
    // KV cache per layer: [maxSeq][KD], persisted across generation steps.
    std::vector<float*> kc(m.n_layer), vc(m.n_layer);
    for(int l=0;l<m.n_layer;l++){ CK(cudaMalloc(&kc[l],(long)maxSeq*KD*4)); CK(cudaMalloc(&vc[l],(long)maxSeq*KD*4)); }
    float scale=1.f/sqrtf((float)HD);

    std::vector<uint32_t> gen; int seqlen=0;
    cudaEvent_t t0,t1; CK(cudaEventCreate(&t0)); CK(cudaEventCreate(&t1)); float gen_ms=0; int gen_steps=0;
    auto G=[&](long N)->unsigned{ return (unsigned)((N+7)/8); }; // q8_gemv grid: 8 output rows / 256-thread block

    // ── PREFILL (host-driven, one-time): run the prompt, fill the KV cache, take the first token. ──
    {
        int Tn=prompt.size(), pos0=0;
        for(int i=0;i<Tn;i++) deq_q8_row_f32<<<(E+255)/256,256>>>(x+(long)i*E,
            m.wq["token_embd.weight"]+(long)prompt[i]*E, m.ws["token_embd.weight"]+(long)prompt[i]*(E/32), E);
        for(int l=0;l<m.n_layer;l++){ char b[64];
            rmsnorm<<<Tn,256>>>(xn,x,m.f32w[L("attn_norm.weight")],E,m.eps);
            matmul(m,L("attn_q.weight"),xn,q,Tn,E,QD);
            matmul(m,L("attn_k.weight"),xn, kc[l]+(long)pos0*KD, Tn,E,KD);
            matmul(m,L("attn_v.weight"),xn, vc[l]+(long)pos0*KD, Tn,E,KD);
            qk_norm_rope<<<dim3(m.n_head,Tn),128>>>(q, m.f32w[L("attn_q_norm.weight")], m.n_head,HD,Tn,pos0, m.rope_base,m.eps);
            qk_norm_rope<<<dim3(m.n_kv,Tn),128>>>(kc[l]+(long)pos0*KD, m.f32w[L("attn_k_norm.weight")], m.n_kv,HD,Tn,pos0, m.rope_base,m.eps);
            attention_kv<<<dim3(m.n_head,Tn),128,(size_t)(pos0+Tn)*4>>>(attn,q,kc[l],vc[l],Tn,pos0,m.n_head,m.n_kv,HD,scale);
            matmul(m,L("attn_output.weight"),attn,tmp,Tn,QD,E);
            add_inplace<<<((long)Tn*E+255)/256,256>>>(x,tmp,(long)Tn*E);
            rmsnorm<<<Tn,256>>>(xn,x,m.f32w[L("ffn_norm.weight")],E,m.eps);
            matmul(m,L("ffn_gate.weight"),xn,gate,Tn,E,FF);
            matmul(m,L("ffn_up.weight"),xn,up,Tn,E,FF);
            swiglu<<<((long)Tn*FF+255)/256,256>>>(hbuf,gate,up,(long)Tn*FF);
            matmul(m,L("ffn_down.weight"),hbuf,tmp,Tn,FF,E);
            add_inplace<<<((long)Tn*E+255)/256,256>>>(x,tmp,(long)Tn*E);
        }
        seqlen=Tn;
        rmsnorm<<<1,256>>>(xn, x+(long)(Tn-1)*E, m.f32w["output_norm.weight"], E, m.eps);
        matmul(m,"output.weight",xn,logits,1,E,m.vocab);
        std::vector<float> hl(m.vocab); CK(cudaMemcpy(hl.data(),logits,m.vocab*4,cudaMemcpyDeviceToHost));
        int best=0; for(int i=1;i<m.vocab;i++) if(hl[i]>hl[best]) best=i;
        gen.push_back(best);
        fprintf(stderr,"prefill %d tokens done, first gen token=%d\n", Tn, best);
    }

    // ── device generation state, then CAPTURE the T=1 step as a CUDA graph (position + token in device
    //    memory) and REPLAY it per token — collapsing ~540 kernel launches/token into one graph launch. ──
    int *d_pos,*d_tok; CK(cudaMalloc(&d_pos,4)); CK(cudaMalloc(&d_tok,4));
    CK(cudaMemcpy(d_pos,&seqlen,4,cudaMemcpyHostToDevice));
    { int b0=(int)gen.back(); CK(cudaMemcpy(d_tok,&b0,4,cudaMemcpyHostToDevice)); }
    cudaStream_t stream; CK(cudaStreamCreate(&stream));
    cudaGraph_t graph; cudaGraphExec_t exec;
    CK(cudaStreamBeginCapture(stream, cudaStreamCaptureModeGlobal));
    embed_dev<<<(E+255)/256,256,0,stream>>>(x, m.wq["token_embd.weight"], m.ws["token_embd.weight"], d_tok, E);
    for(int l=0;l<m.n_layer;l++){ char b[64];
        rmsnorm_q8<<<1,256,0,stream>>>(x, aqE, asE, m.f32w[L("attn_norm.weight")], E, m.eps);
        q8_gemv_dp4a<<<G(QD),256,0,stream>>>(q, m.wq[L("attn_q.weight")], m.ws[L("attn_q.weight")], aqE, asE, E, QD);
        q8_gemv_kv_dp4a<<<G(KD),256,0,stream>>>(kc[l], d_pos, KD, m.wq[L("attn_k.weight")], m.ws[L("attn_k.weight")], aqE, asE, E, KD);
        q8_gemv_kv_dp4a<<<G(KD),256,0,stream>>>(vc[l], d_pos, KD, m.wq[L("attn_v.weight")], m.ws[L("attn_v.weight")], aqE, asE, E, KD);
        qk_norm_rope_dev<<<m.n_head,128,0,stream>>>(q, d_pos, 0, m.f32w[L("attn_q_norm.weight")], m.n_head,HD, m.rope_base,m.eps);
        qk_norm_rope_dev<<<m.n_kv,128,0,stream>>>(kc[l], d_pos, KD, m.f32w[L("attn_k_norm.weight")], m.n_kv,HD, m.rope_base,m.eps);
        attn_dev<<<m.n_head,128,(size_t)maxSeq*4,stream>>>(attn, q, kc[l], vc[l], d_pos, m.n_head,m.n_kv,HD,scale);
        quant_act<<<(QD+255)/256,256,0,stream>>>(attn, aqE, asE, QD);
        q8_gemv_dp4a<<<G(E),256,0,stream>>>(tmp, m.wq[L("attn_output.weight")], m.ws[L("attn_output.weight")], aqE, asE, QD, E);
        add_inplace<<<(E+255)/256,256,0,stream>>>(x, tmp, E);
        rmsnorm_q8<<<1,256,0,stream>>>(x, aqE, asE, m.f32w[L("ffn_norm.weight")], E, m.eps);
        q8_gemv_dp4a<<<G(FF),256,0,stream>>>(gate, m.wq[L("ffn_gate.weight")], m.ws[L("ffn_gate.weight")], aqE, asE, E, FF);
        q8_gemv_dp4a<<<G(FF),256,0,stream>>>(up, m.wq[L("ffn_up.weight")], m.ws[L("ffn_up.weight")], aqE, asE, E, FF);
        swiglu_q8<<<(FF+255)/256,256,0,stream>>>(gate, up, aqFF, asFF, FF);
        q8_gemv_dp4a<<<G(E),256,0,stream>>>(tmp, m.wq[L("ffn_down.weight")], m.ws[L("ffn_down.weight")], aqFF, asFF, FF, E);
        add_inplace<<<(E+255)/256,256,0,stream>>>(x, tmp, E);
    }
    rmsnorm_q8<<<1,256,0,stream>>>(x, aqE, asE, m.f32w["output_norm.weight"], E, m.eps);
    q8_gemv_dp4a<<<G(m.vocab),256,0,stream>>>(logits, m.wq["output.weight"], m.ws["output.weight"], aqE, asE, E, m.vocab);
    argmax_advance<<<1,256,0,stream>>>(logits, m.vocab, d_tok, d_pos);
    CK(cudaStreamEndCapture(stream, &graph));
    CK(cudaGraphInstantiate(&exec, graph, 0));

    for(int step=1; step<N_GEN; step++){
        CK(cudaEventRecord(t0,stream));
        CK(cudaGraphLaunch(exec, stream));
        CK(cudaEventRecord(t1,stream));
        CK(cudaStreamSynchronize(stream));
        float ms; cudaEventElapsedTime(&ms,t0,t1); gen_ms+=ms; gen_steps++;
        int tok; CK(cudaMemcpy(&tok,d_tok,4,cudaMemcpyDeviceToHost)); gen.push_back((uint32_t)tok);
        fprintf(stderr,"gen %d: %6.2f ms\n",step,ms);
    }
    if(gen_steps) fprintf(stderr,"GEN %.2f ms/token = %.1f tok/s  (CUDA-graph replay, fused Q8_0 GEMV, no dequant)\n",
        gen_ms/gen_steps, 1000.0*gen_steps/gen_ms);
    printf("generated:");
    for(uint32_t t:gen) printf(" %u",t);
    printf("\n");
    return 0;
}
