// Grouped-MoE throughput probe at real Qwen3-30B-A3B dims (d=2048, dff=768, E=128, top-8). The MoE is ~52% of the
// decode and is what batches POORLY at batch-1: each token needs its own top-8 of 128 experts. The fix (Megablocks/
// vLLM) is expert-GROUPING: gather the tokens routed to each expert, read that expert's weight ONCE, dp4a it across
// all its tokens, scatter back. Because each (token,expert) dp4a uses the identical order as the batch-1 GEMV, the
// grouped result is BIT-IDENTICAL -> golden preserved. This measures the real batching curve the assembled decode
// will follow, and checks golden. Expert weights ~600 MB int8 (DRAM-bound).
//
// Build: nvcc -O3 -arch=sm_90 grouped_moe.cu -o /tmp/gm && /tmp/gm
#include <cstdio>
#include <cstdlib>
#include <cmath>
#include <vector>
#include <algorithm>
#include <cuda_runtime.h>
static void ck(cudaError_t e,const char* w){ if(e!=cudaSuccess){ fprintf(stderr,"CUDA %s: %s\n",w,cudaGetErrorString(e)); exit(1);} }
#define D    2048
#define DFF  768
#define E    128
#define KTOP 8

// batch-1 reference: one token, its gate GEMV against expert e (dp4a order = the golden order).
__global__ void gate_gemv1(int* y,const signed char* Wg,const signed char* xq,int e){
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=DFF) return; int lane=threadIdx.x&31;
  const int4* w16=(const int4*)(Wg+((long)e*DFF+o)*D); const int4* x16=(const int4*)xq; int acc=0; int nq=D/512;
  for(int c=0;c<nq;c++){ int4 w=w16[c*32+lane], xx=x16[c*32+lane]; acc+=__dp4a(w.x,xx.x,0)+__dp4a(w.y,xx.y,0)+__dp4a(w.z,xx.z,0)+__dp4a(w.w,xx.w,0); }
  for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s); if(lane==0) y[o]=acc;
}

// GROUPED gate GEMM: block.y = expert e; its tokens are etok[eoff[e]..eoff[e+1]) (sorted contiguous). Each warp
// reads Wg[e][o] ONCE (uint4) and dp4a's it across ALL of e's tokens -> weight reuse. Per-token dp4a == batch-1.
#define BT 16
__global__ void gate_grouped(int* Yg,const signed char* Wg,const signed char* Xq,const int* etok,const int* eoff){
  int e=blockIdx.y; int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=DFF) return; int lane=threadIdx.x&31;
  int lo=eoff[e], hi=eoff[e+1]; int b0=blockIdx.z*BT;                 // token tile within the expert
  if(lo+b0>=hi) return;
  const int4* w16=(const int4*)(Wg+((long)e*DFF+o)*D); int nq=D/512;
  int acc[BT]; for(int t=0;t<BT;t++) acc[t]=0;
  for(int c=0;c<nq;c++){ int4 w=w16[c*32+lane];
    #pragma unroll
    for(int t=0;t<BT;t++){ int slot=lo+b0+t; if(slot>=hi) continue; const int4* x16=(const int4*)(Xq+(long)etok[slot]*D);
      int4 xx=x16[c*32+lane]; acc[t]+=__dp4a(w.x,xx.x,0)+__dp4a(w.y,xx.y,0)+__dp4a(w.z,xx.z,0)+__dp4a(w.w,xx.w,0); } }
  for(int t=0;t<BT;t++){ int slot=lo+b0+t; if(slot>=hi) continue; int a=acc[t];
    for(int s=16;s>0;s>>=1) a+=__shfl_down_sync(0xffffffffu,a,s); if(lane==0) Yg[(long)slot*DFF+o]=a; }
}

int main(){
  int Bmax=512;
  std::vector<signed char> hWg((long)E*DFF*D), hX((long)Bmax*D);
  srand(9); for(auto&v:hWg) v=(rand()%15)-7; for(auto&v:hX) v=(rand()%15)-7;
  signed char *Wg,*X; int *Yg,*Y1,*d_etok,*d_eoff;
  ck(cudaMalloc(&Wg,(long)E*DFF*D),"Wg"); ck(cudaMalloc(&X,(long)Bmax*D),"X");
  ck(cudaMalloc(&Yg,(long)Bmax*KTOP*DFF*4),"Yg"); ck(cudaMalloc(&Y1,DFF*4),"Y1");
  ck(cudaMalloc(&d_etok,(long)Bmax*KTOP*4),"et"); ck(cudaMalloc(&d_eoff,(E+1)*4),"eo");
  ck(cudaMemcpy(Wg,hWg.data(),(long)E*DFF*D,cudaMemcpyHostToDevice),"cWg");
  ck(cudaMemcpy(X,hX.data(),(long)Bmax*D,cudaMemcpyHostToDevice),"cX");
  cudaEvent_t t0,t1; cudaEventCreate(&t0); cudaEventCreate(&t1); float ms;
  printf("grouped MoE gate at Qwen dims d=%d dff=%d E=%d top-%d, expert weight = %.0f MB int8\n", D,DFF,E,KTOP,(double)E*DFF*D/1e6);

  auto route=[&](int B, std::vector<int>& etok, std::vector<int>& eoff){
    // each token -> KTOP distinct experts (round-robin spread for even load); build sorted (expert -> token slots)
    std::vector<std::vector<int>> buckets(E);
    for(int b=0;b<B;b++) for(int k=0;k<KTOP;k++){ int e=(b*KTOP+k)%E; buckets[e].push_back(b); }
    eoff.assign(E+1,0); etok.clear();
    for(int e=0;e<E;e++){ eoff[e]=etok.size(); for(int b:buckets[e]) etok.push_back(b); } eoff[E]=etok.size();
  };

  // correctness: grouped slot result == batch-1 GEMV for that (token,expert)
  { int B=32; std::vector<int> etok,eoff; route(B,etok,eoff);
    cudaMemcpy(d_etok,etok.data(),etok.size()*4,cudaMemcpyHostToDevice); cudaMemcpy(d_eoff,eoff.data(),(E+1)*4,cudaMemcpyHostToDevice);
    int maxne=0; for(int e=0;e<E;e++) maxne=std::max(maxne,eoff[e+1]-eoff[e]);
    dim3 grd((DFF+7)/8,E,(maxne+BT-1)/BT); gate_grouped<<<grd,256>>>(Yg,Wg,X,d_etok,d_eoff); cudaDeviceSynchronize();
    std::vector<int> hYg(etok.size()*DFF); cudaMemcpy(hYg.data(),Yg,(long)etok.size()*DFF*4,cudaMemcpyDeviceToHost);
    long mism=0; std::vector<int> hY1(DFF);
    for(int e=0;e<E;e++) for(int slot=eoff[e]; slot<eoff[e+1] && slot<eoff[e]+3; slot++){ int tok=etok[slot];
      gate_gemv1<<<(DFF+7)/8,256>>>(Y1,Wg,X+(long)tok*D,e); cudaDeviceSynchronize(); cudaMemcpy(hY1.data(),Y1,DFF*4,cudaMemcpyDeviceToHost);
      for(int o=0;o<DFF;o++) if(hYg[(long)slot*DFF+o]!=hY1[o]) mism++; }
    printf("golden-preservation: grouped vs batch-1 GEMV (sampled slots)  mismatches = %ld  (%s)\n", mism, mism==0?"BIT-IDENTICAL":"DRIFT"); }

  const int REP=100, WARM=15;
  int Bs[]={1,8,32,128,256,512};
  printf("\naggregate token throughput of the grouped MoE gate stage (weight read amortized as tokens/expert grows):\n");
  double tps1=0;
  for(int bi=0; bi<6; bi++){ int B=Bs[bi]; std::vector<int> etok,eoff; route(B,etok,eoff);
    cudaMemcpy(d_etok,etok.data(),etok.size()*4,cudaMemcpyHostToDevice); cudaMemcpy(d_eoff,eoff.data(),(E+1)*4,cudaMemcpyHostToDevice);
    int maxne=0; for(int e=0;e<E;e++) maxne=std::max(maxne,eoff[e+1]-eoff[e]);
    dim3 grd((DFF+7)/8,E,(maxne+BT-1)/BT);
    for(int i=0;i<WARM;i++) gate_grouped<<<grd,256>>>(Yg,Wg,X,d_etok,d_eoff); cudaDeviceSynchronize();
    cudaEventRecord(t0); for(int i=0;i<REP;i++) gate_grouped<<<grd,256>>>(Yg,Wg,X,d_etok,d_eoff); cudaEventRecord(t1); cudaEventSynchronize(t1); cudaEventElapsedTime(&ms,t0,t1);
    double us=ms*1e3/REP, tps=B/(us*1e-6); if(bi==0) tps1=tps;
    printf("  B=%-4d : %8.1f us  %10.0f tok/s  (%.1fx)  [%.1f tokens/expert]\n", B, us, tps, tps/tps1, (double)B*KTOP/E);
  }
  printf("\ngrouped MoE: golden-preserving, and it batches as tokens/expert grows -- the 52%% that llama batches, now ours.\n");
  return 0;
}
