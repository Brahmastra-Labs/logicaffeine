// Golden-preserving AND fast batched GEMM, built from our shipped dp4a GEMV. Warp-per-output-row: the int8 weight
// row is read ONCE (uint4) and reused across B tokens, each token accumulated with the IDENTICAL __dp4a order as
// the batch-1 GEMV -> byte-identical per row (golden preserved) while weight bandwidth is amortized B-fold. This
// is the kernel the emitter will lower the batched decode to (same .lg, batched execution).
//
// Build: nvcc -O3 -arch=sm_90 batched_dp4a_gemm.cu -o /tmp/bd && /tmp/bd
#include <cstdio>
#include <cstdlib>
#include <cmath>
#include <vector>
#include <cuda_runtime.h>
static void ck(cudaError_t e,const char* w){ if(e!=cudaSuccess){ fprintf(stderr,"CUDA %s: %s\n",w,cudaGetErrorString(e)); exit(1);} }
#define K 4096
#define N 8192
#define BT 16                    // tokens per warp-pass (register-tiled); loop over B in BT chunks

// batch-1: our shipped warp-per-row uint4 dp4a GEMV (the reference order).
__global__ void gemv1(int* y,const signed char* wq,const signed char* xq){
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31;
  const int4* w16=(const int4*)(wq+(long)o*K); const int4* x16=(const int4*)xq; int acc=0; int nq=K/512;
  for(int c=0;c<nq;c++){ int4 w=w16[c*32+lane], xx=x16[c*32+lane];
    acc+=__dp4a(w.x,xx.x,0)+__dp4a(w.y,xx.y,0)+__dp4a(w.z,xx.z,0)+__dp4a(w.w,xx.w,0); }
  for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s); if(lane==0) y[o]=acc;
}
// batched: warp per output, weight row read once/uint4, reused across BT tokens; per-token dp4a order IDENTICAL
// to gemv1 -> byte-exact rows. grid.y tiles the batch by BT.
__global__ void bdp4a(const signed char* Xq,const signed char* Wq,int* Y,int B){
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31;
  int b0=blockIdx.y*BT; const int4* w16=(const int4*)(Wq+(long)o*K); int nq=K/512;
  int acc[BT]; for(int t=0;t<BT;t++) acc[t]=0;
  for(int c=0;c<nq;c++){ int4 w=w16[c*32+lane];              // weight read ONCE, reused across the BT tokens
    #pragma unroll
    for(int t=0;t<BT;t++){ int b=b0+t; if(b>=B) continue; const int4* x16=(const int4*)(Xq+(long)b*K);
      int4 xx=x16[c*32+lane]; acc[t]+=__dp4a(w.x,xx.x,0)+__dp4a(w.y,xx.y,0)+__dp4a(w.z,xx.z,0)+__dp4a(w.w,xx.w,0); } }
  for(int t=0;t<BT;t++){ int b=b0+t; if(b>=B) continue; int a=acc[t];
    for(int s=16;s>0;s>>=1) a+=__shfl_down_sync(0xffffffffu,a,s); if(lane==0) Y[(long)b*N+o]=a; }
}

int main(){
  int Bmax=256;
  std::vector<signed char> hW((long)N*K), hX((long)Bmax*K);
  srand(5); for(auto&v:hW) v=(rand()%15)-7; for(auto&v:hX) v=(rand()%15)-7;
  signed char *W,*X; int *Y,*Y1;
  ck(cudaMalloc(&W,(long)N*K),"W"); ck(cudaMalloc(&X,(long)Bmax*K),"X"); ck(cudaMalloc(&Y,(long)Bmax*N*4),"Y"); ck(cudaMalloc(&Y1,N*4),"Y1");
  ck(cudaMemcpy(W,hW.data(),(long)N*K,cudaMemcpyHostToDevice),"cW"); ck(cudaMemcpy(X,hX.data(),(long)Bmax*K,cudaMemcpyHostToDevice),"cX");
  cudaEvent_t t0,t1; cudaEventCreate(&t0); cudaEventCreate(&t1); float ms;

  // golden check: every batched row must be BIT-IDENTICAL to the batch-1 GEMV on that token
  { int B=32; dim3 blk(256), grd((N+7)/8,(B+BT-1)/BT); bdp4a<<<grd,blk>>>(X,W,Y,B); cudaDeviceSynchronize();
    std::vector<int> hYb((long)B*N); cudaMemcpy(hYb.data(),Y,(long)B*N*4,cudaMemcpyDeviceToHost);
    long mism=0; std::vector<int> hY1(N);
    for(int b=0;b<B;b++){ gemv1<<<(N+7)/8,256>>>(Y1,W,X+(long)b*K); cudaDeviceSynchronize(); cudaMemcpy(hY1.data(),Y1,N*4,cudaMemcpyDeviceToHost);
      for(int o=0;o<N;o++) if(hYb[(long)b*N+o]!=hY1[o]) mism++; }
    printf("golden-preservation: batched vs batch-1 GEMV over 32 tokens x %d outs  mismatches = %ld  (%s)\n", N, mism, mism==0?"BIT-IDENTICAL":"DRIFT"); }

  const int REP=300, WARM=40; double wMB=(double)N*K/1e6;
  printf("\nint8 dp4a (golden flash lane), K=%d N=%d = %.0f MB weight\n", K,N,wMB);
  for(int i=0;i<WARM;i++) gemv1<<<(N+7)/8,256>>>(Y,W,X); cudaDeviceSynchronize();
  cudaEventRecord(t0); for(int i=0;i<REP;i++) gemv1<<<(N+7)/8,256>>>(Y,W,X); cudaEventRecord(t1); cudaEventSynchronize(t1); cudaEventElapsedTime(&ms,t0,t1);
  double us1=ms*1e3/REP, tps1=1.0/(us1*1e-6);
  printf("  B=%-4d GEMV : %8.1f us  %10.0f tok/s  (1.0x)\n", 1, us1, tps1);
  int Bs[]={2,4,8,16,32,64,128,256};
  for(int bi=0; bi<8; bi++){ int B=Bs[bi]; dim3 blk(256), grd((N+7)/8,(B+BT-1)/BT);
    for(int i=0;i<WARM;i++) bdp4a<<<grd,blk>>>(X,W,Y,B); cudaDeviceSynchronize();
    cudaEventRecord(t0); for(int i=0;i<REP;i++) bdp4a<<<grd,blk>>>(X,W,Y,B); cudaEventRecord(t1); cudaEventSynchronize(t1); cudaEventElapsedTime(&ms,t0,t1);
    double us=ms*1e3/REP, tps=B/(us*1e-6); printf("  B=%-4d GEMM : %8.1f us  %10.0f tok/s  (%.1fx)\n", B, us, tps, tps/tps1);
  }
  printf("\ngolden-preserving batched dp4a: same weight read serves B tokens, each row bit-identical to batch-1.\n");
  return 0;
}
