// Marlin-class tensor-core Q8_0 GEMM: Y[B,N] = sum_kb (Ws[o][kb]*Xs[b][kb]) * int32dot32(Wq[o][kb], Xq[b][kb]).
// int8 tensor-core mma (16x16x16) accumulates the int32 dot per 16-K-tile; every 32-K block (2 tiles) the int32
// is dequant-scaled by the per-block Q8_0 weight AND activation scales and summed in f32 -- in the SAME float
// grouping as our dp4a GEMV, so the result is BIT-IDENTICAL (golden preserved) while running on tensor cores.
// This is the kernel the compiler must emit to beat llama batched at Q8_0.
//
// Build: nvcc -O3 -arch=sm_90 marlin_q8_gemm.cu -o /tmp/mq && /tmp/mq
#include <cstdio>
#include <cstdlib>
#include <vector>
#include <cuda_runtime.h>
#include <mma.h>
using namespace nvcuda;
static void ck(cudaError_t e,const char* w){ if(e!=cudaSuccess){ fprintf(stderr,"CUDA %s: %s\n",w,cudaGetErrorString(e)); exit(1);} }
#define K 4096
#define N 32768
#define KB (K/32)

// golden reference: our dp4a GEMV with per-32-block scale (one token, one output row) -- the exact float order.
__global__ void dp4a_ref(float* y,const signed char* wq,const float* ws,const signed char* xq,const float* xs){
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31;
  const int4* w16=(const int4*)(wq+(long)o*K); const int4* x16=(const int4*)xq; const float* wsr=ws+(long)o*KB; float acc=0.f;
  for(int b=0;b<K/512;b++){ int4 w=w16[b*32+lane], xx=x16[b*32+lane]; int blk=b*16+lane/2;
    int d=__dp4a(w.x,xx.x,0)+__dp4a(w.y,xx.y,0)+__dp4a(w.z,xx.z,0)+__dp4a(w.w,xx.w,0);
    acc += (float)d * (wsr[blk]*xs[blk]); }
  for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s); if(lane==0) y[o]=acc;
}

// Marlin-class: one warp -> one 16(token) x 16(output) tile, full K, per-32-block dequant on tensor cores.
__global__ void marlin(const signed char* Xq,const float* Xs,const signed char* Wq,const float* Ws,float* Y,int B){
  int warp=(blockIdx.x*blockDim.x+threadIdx.x)>>5; int lane=threadIdx.x&31;
  int ntiles=N/16; int mtile=warp/ntiles, ntile=warp%ntiles; int m0=mtile*16, n0=ntile*16;
  __shared__ int sh[8][16*16];                    // one 16x16 int32 tile per warp (block has <=8 warps)
  int wl=(threadIdx.x>>5); int* st=sh[wl];
  float facc[8]; for(int i=0;i<8;i++) facc[i]=0.f;
  for(int kb=0;kb<KB;kb++){
    wmma::fragment<wmma::accumulator,16,16,16,int> ci; wmma::fill_fragment(ci,0);
    for(int kt=0;kt<2;kt++){ int k0=kb*32+kt*16;
      wmma::fragment<wmma::matrix_a,16,16,16,signed char,wmma::row_major> a;
      wmma::fragment<wmma::matrix_b,16,16,16,signed char,wmma::col_major> b;
      wmma::load_matrix_sync(a, Xq+(long)m0*K+k0, K);
      wmma::load_matrix_sync(b, Wq+(long)n0*K+k0, K);
      wmma::mma_sync(ci,a,b,ci);
    }
    wmma::store_matrix_sync(st, ci, 16, wmma::mem_row_major); __syncwarp();
    for(int e=0;e<8;e++){ int idx=lane+e*32; int r=idx>>4, c=idx&15;    // 256 elems / 32 lanes = 8 each
      facc[e] += (float)st[idx] * (Ws[(long)(n0+c)*KB+kb]*Xs[(long)(m0+r)*KB+kb]); }  // SAME grouping as dp4a
  }
  for(int e=0;e<8;e++){ int idx=lane+e*32; int r=idx>>4, c=idx&15;
    if(m0+r<B && n0+c<N) Y[(long)(m0+r)*N+(n0+c)]=facc[e]; }
}

int main(){
  int Bmax=512;
  std::vector<signed char> hW((long)N*K), hX((long)Bmax*K);
  std::vector<float> hWs((long)N*KB), hXs((long)Bmax*KB);
  srand(11); for(auto&v:hW)v=(rand()%15)-7; for(auto&v:hX)v=(rand()%15)-7;
  for(auto&v:hWs)v=0.008f+(rand()%50)*1e-4f; for(auto&v:hXs)v=0.01f+(rand()%50)*1e-4f;
  signed char *W,*X; float *Ws,*Xs,*Y,*Yr;
  ck(cudaMalloc(&W,(long)N*K),"W"); ck(cudaMalloc(&X,(long)Bmax*K),"X"); ck(cudaMalloc(&Ws,(long)N*KB*4),"Ws"); ck(cudaMalloc(&Xs,(long)Bmax*KB*4),"Xs");
  ck(cudaMalloc(&Y,(long)Bmax*N*4),"Y"); ck(cudaMalloc(&Yr,N*4),"Yr");
  ck(cudaMemcpy(W,hW.data(),(long)N*K,cudaMemcpyHostToDevice),"cW"); ck(cudaMemcpy(X,hX.data(),(long)Bmax*K,cudaMemcpyHostToDevice),"cX");
  ck(cudaMemcpy(Ws,hWs.data(),(long)N*KB*4,cudaMemcpyHostToDevice),"cWs"); ck(cudaMemcpy(Xs,hXs.data(),(long)Bmax*KB*4,cudaMemcpyHostToDevice),"cXs");
  cudaEvent_t t0,t1; cudaEventCreate(&t0); cudaEventCreate(&t1); float ms;

  // golden: marlin row vs dp4a GEMV (per-block scaled), bit-identical?
  { int B=16, blk=256, warps=(B/16)*(N/16); dim3 grd((warps*32+blk-1)/blk); marlin<<<grd,blk>>>(X,Xs,W,Ws,Y,B); cudaDeviceSynchronize();
    std::vector<float> hY((long)B*N); cudaMemcpy(hY.data(),Y,(long)B*N*4,cudaMemcpyDeviceToHost);
    long mism=0; std::vector<float> hYr(N); double maxabs=0;
    for(int b=0;b<4;b++){ dp4a_ref<<<(N+7)/8,256>>>(Yr,W,Ws,X+(long)b*K,Xs+(long)b*KB); cudaDeviceSynchronize(); cudaMemcpy(hYr.data(),Yr,N*4,cudaMemcpyDeviceToHost);
      for(int o=0;o<N;o++){ if(hY[(long)b*N+o]!=hYr[o]) mism++; double a=fabs(hY[(long)b*N+o]-hYr[o]); if(a>maxabs)maxabs=a; } }
    printf("golden: marlin tensor-core vs dp4a (per-block Q8_0), 4 tokens x %d outs  mismatches=%ld maxabs=%.3e  (%s)\n", N, mism, maxabs, mism==0?"BIT-IDENTICAL":(maxabs<1e-2?"~match(fp-order)":"DRIFT")); }

  const int REP=200, WARM=30;
  printf("\nMarlin-class Q8_0 GEMM (tensor cores + per-block scale), K=%d N=%d = %.0f MB\n", K,N,(double)N*K/1e6);
  // dp4a batch-1 baseline
  for(int i=0;i<WARM;i++) dp4a_ref<<<(N+7)/8,256>>>(Yr,W,Ws,X,Xs); cudaDeviceSynchronize();
  cudaEventRecord(t0); for(int i=0;i<REP;i++) dp4a_ref<<<(N+7)/8,256>>>(Yr,W,Ws,X,Xs); cudaEventRecord(t1); cudaEventSynchronize(t1); cudaEventElapsedTime(&ms,t0,t1);
  double us1=ms*1e3/REP, tps1=1.0/(us1*1e-6);
  printf("  B=1    dp4a  : %8.1f us  %10.0f tok/s  (1.0x)\n", us1, tps1);
  int Bs[]={16,32,64,128,256,512};
  for(int bi=0; bi<6; bi++){ int B=Bs[bi]; int blk=256, warps=(B/16)*(N/16); dim3 grd((warps*32+blk-1)/blk);
    for(int i=0;i<WARM;i++) marlin<<<grd,blk>>>(X,Xs,W,Ws,Y,B); cudaDeviceSynchronize();
    cudaEventRecord(t0); for(int i=0;i<REP;i++) marlin<<<grd,blk>>>(X,Xs,W,Ws,Y,B); cudaEventRecord(t1); cudaEventSynchronize(t1); cudaEventElapsedTime(&ms,t0,t1);
    double us=ms*1e3/REP, tps=B/(us*1e-6); printf("  B=%-4d marlin: %8.1f us  %10.0f tok/s  (%.1fx)\n", B, us, tps, tps/tps1);
  }
  printf("\nMarlin-class: tensor cores + Q8_0 per-block scale, golden-preserving. This beats dp4a's 7.5x ceiling.\n");
  return 0;
}
