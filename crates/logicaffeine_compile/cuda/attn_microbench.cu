// Attention-kernel microbench at the real Qwen3-30B-A3B decode dims (hd=128, n_head=32, n_kv=4/GQA).
//
// Purpose: pick the fastest single-query (batch-1, growing-KV) decode-attention layout WITHOUT an A100, by
// measuring every candidate head-to-head on whatever CUDA GPU is present and verifying each against a CPU
// softmax(QK^T·scale)·V reference. Decode attention is LATENCY-bound, so relative kernel latency transfers
// across GPUs; the ONE effect that does NOT transfer is grid occupancy (SM count differs), so we hold the
// grid shape fixed per family and compare per-block layout efficiency — the axis llama's `fattn-vec` wins on
// (small threads-per-key ⇒ many keys in flight per warp).
//
// Candidates:
//   baseline  — full-warp-per-key, multi-pass shared-score softmax (our gk_attn).
//   split     — full-warp-per-key, register online-softmax, split-K + combine (our gk_attn_part/comb).
//   kp<TKQ>   — KEY-PARALLEL: TKQ threads per key ⇒ 32/TKQ keys per warp, register-tiled strided Q,
//               in-block flash-combine, split-K + combine (llama fattn-vec method). TKQ tuned here.
//
// Build:  nvcc -O3 -arch=sm_86 attn_microbench.cu -o /tmp/attn_mb && /tmp/attn_mb
#include <cstdio>
#include <cstdlib>
#include <cmath>
#include <vector>
#include <cuda_runtime.h>

#define HD   128
#define NH   32
#define NKV  4
#define GRP  (NH/NKV)
#define KD   (NKV*HD)
#define SCALE (0.08838834764831845f)   // 1/sqrt(128)

static void ck(cudaError_t e, const char* w){ if(e!=cudaSuccess){ fprintf(stderr,"CUDA %s: %s\n",w,cudaGetErrorString(e)); exit(1);} }

static void cpu_attn(const float* q,const float* kc,const float* vc,int L,float* out){
  for(int h=0;h<NH;h++){ int kvh=h/GRP; const float* qh=q+(long)h*HD;
    std::vector<float> s(L); float mx=-1e30f;
    for(int j=0;j<L;j++){ const float* kj=kc+(long)j*KD+kvh*HD; float d=0; for(int e=0;e<HD;e++) d+=qh[e]*kj[e]; s[j]=d*SCALE; if(s[j]>mx) mx=s[j]; }
    float den=0; for(int j=0;j<L;j++){ s[j]=expf(s[j]-mx); den+=s[j]; }
    for(int e=0;e<HD;e++){ float a=0; for(int j=0;j<L;j++) a+=s[j]*vc[(long)j*KD+kvh*HD+e]; out[(long)h*HD+e]=a/den; }
  }
}

__global__ void k_baseline(float* out,const float* q,const float* kc,const float* vc,int L,float scale){
  int head=blockIdx.x; int kvh=head/GRP; const float* qh=q+(long)head*HD;
  extern __shared__ float sc[]; __shared__ float red[32];
  int lane=threadIdx.x&31, wp=threadIdx.x>>5, nw=(blockDim.x+31)>>5;
  for(int j=wp;j<L;j+=nw){ const float* kj=kc+(long)j*KD+kvh*HD; float s=0.f; for(int e=lane;e<HD;e+=32) s+=qh[e]*kj[e];
    for(int r=16;r>0;r>>=1) s+=__shfl_down_sync(0xffffffffu,s,r); if(lane==0) sc[j]=s*scale; }
  __syncthreads();
  float m=-1e30f; for(int j=threadIdx.x;j<L;j+=blockDim.x) m=fmaxf(m,sc[j]);
  for(int r=16;r>0;r>>=1) m=fmaxf(m,__shfl_down_sync(0xffffffffu,m,r)); if(lane==0) red[wp]=m; __syncthreads();
  if(threadIdx.x==0){ float mm=red[0]; for(int i=1;i<nw;i++) mm=fmaxf(mm,red[i]); red[0]=mm; } __syncthreads(); float mx=red[0];
  float ss=0.f; for(int j=threadIdx.x;j<L;j+=blockDim.x){ float e=expf(sc[j]-mx); sc[j]=e; ss+=e; }
  for(int r=16;r>0;r>>=1) ss+=__shfl_down_sync(0xffffffffu,ss,r); if(lane==0) red[wp]=ss; __syncthreads();
  if(threadIdx.x==0){ float su=0.f; for(int i=0;i<nw;i++) su+=red[i]; red[0]=su; } __syncthreads(); float den=red[0];
  for(int e=threadIdx.x;e<HD;e+=blockDim.x){ float a=0.f; for(int j=0;j<L;j++) a+=sc[j]*vc[(long)j*KD+kvh*HD+e]; out[(long)head*HD+e]=a/den; }
}

__global__ void k_split_part(const float* q,const float* kc,const float* vc,int L,float scale,int P,float* accp,float* mp,float* dp){
  int b=blockIdx.x, head=b/P, p=b%P; int kvh=head/GRP; const float* qh=q+(long)head*HD;
  int lane=threadIdx.x&31, wp=threadIdx.x>>5, nw=blockDim.x>>5; int dpl=HD>>5;
  float qreg[8], acc[8]; for(int i=0;i<dpl;i++){ qreg[i]=qh[lane+(i<<5)]; acc[i]=0.f; }
  float m=-1e30f, dsum=0.f;
  for(int idx=wp; ; idx+=nw){ int j=p+idx*P; if(j>=L) break;
    const float* kj=kc+(long)j*KD+kvh*HD; float pt=0.f; for(int i=0;i<dpl;i++) pt+=qreg[i]*kj[lane+(i<<5)];
    for(int r=16;r>0;r>>=1) pt+=__shfl_xor_sync(0xffffffffu,pt,r);
    float sj=pt*scale; float mn=fmaxf(m,sj), c=__expf(m-mn), pp=__expf(sj-mn); m=mn; dsum=dsum*c+pp;
    const float* vj=vc+(long)j*KD+kvh*HD; for(int i=0;i<dpl;i++) acc[i]=acc[i]*c+pp*vj[lane+(i<<5)];
  }
  __shared__ float sm[8], sd[8], sa[8][HD];
  if(lane==0){ sm[wp]=m; sd[wp]=dsum; } for(int i=0;i<dpl;i++) sa[wp][lane+(i<<5)]=acc[i]; __syncthreads();
  float M=-1e30f; for(int w=0;w<nw;w++) M=fmaxf(M,sm[w]);
  float D=0.f; for(int w=0;w<nw;w++) D+=sd[w]*__expf(sm[w]-M);
  if(threadIdx.x==0){ mp[b]=M; dp[b]=D; }
  for(int d=threadIdx.x; d<HD; d+=blockDim.x){ float a=0.f; for(int w=0;w<nw;w++) a+=sa[w][d]*__expf(sm[w]-M); accp[(long)b*HD+d]=a; }
}
__global__ void k_comb(float* out,int P,const float* accp,const float* mp,const float* dp){
  int head=blockIdx.x; __shared__ float GM,GD;
  if(threadIdx.x==0){ float M=-1e30f; for(int p=0;p<P;p++) M=fmaxf(M,mp[head*P+p]); GM=M;
    float D=0.f; for(int p=0;p<P;p++) D+=dp[head*P+p]*__expf(mp[head*P+p]-M); GD=D; }
  __syncthreads(); float M=GM,D=GD;
  for(int d=threadIdx.x; d<HD; d+=blockDim.x){ float a=0.f; for(int p=0;p<P;p++) a+=accp[(long)(head*P+p)*HD+d]*__expf(mp[head*P+p]-M); out[(long)head*HD+d]=a/D; }
}

// KEY-PARALLEL (llama fattn-vec). TKQ threads per key ⇒ 32/TKQ keys per warp; strided register-tiled Q; each
// (warp,key-group) is an independent online-softmax stream; in-block flash-combine → split-K partial. TKQ and P
// are runtime args here so one kernel covers the sweep (the emitter bakes the winning TKQ as a constant). BLK is
// fixed 128 (4 warps). Static shared sized for the worst case (TKQ=4 ⇒ 8 keys/warp ⇒ 32 streams).
__global__ void k_kp_part(const float* q,const float* kc,const float* vc,int L,float scale,int TKQ,int P,float* accp,float* mp,float* dp){
  const int MAXS=32;
  __shared__ float smax[MAXS], sden[MAXS], snum[MAXS*HD];
  int KPW=32/TKQ, DPL=HD/TKQ;
  int b=blockIdx.x, head=b/P, p=b%P; int kvh=head/GRP; const float* qh=q+(long)head*HD;
  int lane=threadIdx.x&31, warp=threadIdx.x>>5, nwarps=blockDim.x>>5;
  int sub=lane%TKQ, kgrp=lane/TKQ; int stream=warp*KPW+kgrp, nstream=nwarps*KPW;
  // Per-key-group shuffle mask: within a warp only the TKQ lanes of THIS key-group reduce together (they share
  // the same key j, so they never diverge). A full-warp 0xffffffff mask deadlocks — different key-groups process
  // different keys and hit different trip counts. The loop advances the WHOLE block in lockstep over key-batches
  // (base += nstream) so the shuffle is always convergent; out-of-range keys ride along inert (valid=0).
  unsigned gmask = (TKQ>=32)?0xffffffffu : (((1u<<TKQ)-1u) << (kgrp*TKQ));
  float qreg[32], acc[32]; for(int i=0;i<DPL;i++){ qreg[i]=qh[sub+i*TKQ]; acc[i]=0.f; }
  float m=-1e30f, dsum=0.f;
  for(int base=0; p + base*P < L; base += nstream){ int t=base+stream; int j=p+t*P; int valid=(j<L);
    float pt=0.f; if(valid){ const float* kj=kc+(long)j*KD+kvh*HD; for(int i=0;i<DPL;i++) pt+=qreg[i]*kj[sub+i*TKQ]; }
    for(int r=TKQ/2;r>0;r>>=1) pt+=__shfl_xor_sync(gmask,pt,r);
    if(valid){ float sj=pt*scale; float mn=fmaxf(m,sj), c=__expf(m-mn), pp=__expf(sj-mn); m=mn; dsum=dsum*c+pp;
      const float* vj=vc+(long)j*KD+kvh*HD; for(int i=0;i<DPL;i++) acc[i]=acc[i]*c+pp*vj[sub+i*TKQ]; }
  }
  if(sub==0){ smax[stream]=m; sden[stream]=dsum; }
  for(int i=0;i<DPL;i++) snum[stream*HD + sub+i*TKQ]=acc[i];
  __syncthreads();
  float M=-1e30f; for(int s=0;s<nstream;s++) M=fmaxf(M,smax[s]);
  float D=0.f; for(int s=0;s<nstream;s++) D+=sden[s]*__expf(smax[s]-M);
  if(threadIdx.x==0){ mp[b]=M; dp[b]=D; }
  for(int d=threadIdx.x; d<HD; d+=blockDim.x){ float a=0.f; for(int s=0;s<nstream;s++) a+=snum[s*HD+d]*__expf(smax[s]-M); accp[(long)b*HD+d]=a; }
}

static float maxerr(const std::vector<float>& a,const std::vector<float>& b){ float e=0; for(size_t i=0;i<a.size();i++) e=fmaxf(e,fabsf(a[i]-b[i])); return e; }

int main(){
  int Ls[]={8,130,512}; const int REP=2000, WARM=200;
  size_t qn=(size_t)NH*HD;
  std::vector<float> hq(qn); srand(1234);
  for(auto&x:hq) x=(rand()/(float)RAND_MAX-0.5f)*2.f;
  float *dq,*dout; ck(cudaMalloc(&dq,qn*4),"dq"); ck(cudaMalloc(&dout,qn*4),"dout");
  ck(cudaMemcpy(dq,hq.data(),qn*4,cudaMemcpyHostToDevice),"cpq");
  cudaEvent_t a,z; cudaEventCreate(&a); cudaEventCreate(&z); float ms;
  int TKQs[]={16,8,4};
  printf("dims: hd=%d n_head=%d n_kv=%d  (grid held fixed per family)\n",HD,NH,NKV);
  for(int li=0; li<3; li++){
    int L=Ls[li]; size_t cn=(size_t)L*KD; std::vector<float> hk(cn),hv(cn);
    for(auto&x:hk) x=(rand()/(float)RAND_MAX-0.5f)*2.f;
    for(auto&x:hv) x=(rand()/(float)RAND_MAX-0.5f)*2.f;
    float *dk,*dv; ck(cudaMalloc(&dk,cn*4),"dk"); ck(cudaMalloc(&dv,cn*4),"dv");
    ck(cudaMemcpy(dk,hk.data(),cn*4,cudaMemcpyHostToDevice),"cpk");
    ck(cudaMemcpy(dv,hv.data(),cn*4,cudaMemcpyHostToDevice),"cpv");
    std::vector<float> ref(qn); cpu_attn(hq.data(),hk.data(),hv.data(),L,ref.data());
    std::vector<float> got(qn);
    printf("\n=== L=%d ===\n",L);
    for(int i=0;i<WARM;i++) k_baseline<<<NH,256,(size_t)L*4>>>(dout,dq,dk,dv,L,SCALE);
    cudaDeviceSynchronize(); cudaEventRecord(a);
    for(int i=0;i<REP;i++) k_baseline<<<NH,256,(size_t)L*4>>>(dout,dq,dk,dv,L,SCALE);
    cudaEventRecord(z); cudaEventSynchronize(z); cudaEventElapsedTime(&ms,a,z);
    cudaMemcpy(got.data(),dout,qn*4,cudaMemcpyDeviceToHost);
    printf("  baseline (warp/key, multipass) : %7.1f ns   err=%.2e\n", ms*1e6/REP, maxerr(got,ref));
    for(int P=1;P<=8;P*=2){
      float *accp,*mp,*dp; ck(cudaMalloc(&accp,(size_t)NH*P*HD*4),"accp"); ck(cudaMalloc(&mp,(size_t)NH*P*4),"mp"); ck(cudaMalloc(&dp,(size_t)NH*P*4),"dp");
      for(int i=0;i<WARM;i++){ k_split_part<<<NH*P,256>>>(dq,dk,dv,L,SCALE,P,accp,mp,dp); k_comb<<<NH,128>>>(dout,P,accp,mp,dp); }
      cudaDeviceSynchronize(); cudaEventRecord(a);
      for(int i=0;i<REP;i++){ k_split_part<<<NH*P,256>>>(dq,dk,dv,L,SCALE,P,accp,mp,dp); k_comb<<<NH,128>>>(dout,P,accp,mp,dp); }
      cudaEventRecord(z); cudaEventSynchronize(z); cudaEventElapsedTime(&ms,a,z);
      cudaMemcpy(got.data(),dout,qn*4,cudaMemcpyDeviceToHost);
      printf("  split   P=%d (warp/key)         : %7.1f ns   err=%.2e\n",P, ms*1e6/REP, maxerr(got,ref));
      for(int ti=0;ti<3;ti++){ int TKQ=TKQs[ti];
        for(int i=0;i<WARM;i++){ k_kp_part<<<NH*P,128>>>(dq,dk,dv,L,SCALE,TKQ,P,accp,mp,dp); k_comb<<<NH,128>>>(dout,P,accp,mp,dp); }
        cudaDeviceSynchronize(); cudaEventRecord(a);
        for(int i=0;i<REP;i++){ k_kp_part<<<NH*P,128>>>(dq,dk,dv,L,SCALE,TKQ,P,accp,mp,dp); k_comb<<<NH,128>>>(dout,P,accp,mp,dp); }
        cudaEventRecord(z); cudaEventSynchronize(z); cudaEventElapsedTime(&ms,a,z);
        cudaMemcpy(got.data(),dout,qn*4,cudaMemcpyDeviceToHost);
        printf("  kp TKQ=%-2d P=%d (%d keys/warp)     : %7.1f ns   err=%.2e\n",TKQ,P,32/TKQ, ms*1e6/REP, maxerr(got,ref));
      }
      cudaFree(accp); cudaFree(mp); cudaFree(dp);
    }
    cudaFree(dk); cudaFree(dv);
  }
  return 0;
}
