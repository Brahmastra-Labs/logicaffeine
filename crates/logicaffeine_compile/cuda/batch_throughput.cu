// Batched-decode throughput demonstration. The batch-1 wall is that one DRAM weight read serves ONE token. At
// batch-B the SAME weight read serves B tokens (GEMV -> GEMM, weight reuse), so aggregate tokens/sec multiplies
// until the kernel turns compute-bound. To measure the REAL effect the weight must exceed L2 (else it's cached
// and we measure L2, not the decode's DRAM wall), and the GEMM must use dp4a (else it's compute-bound and slow).
// Weight = N*K int8 = 128 MB here (>> any L2). Verified against a CPU reference.
//
// Build: nvcc -O3 -arch=sm_86 batch_throughput.cu -o /tmp/bt && /tmp/bt
#include <cstdio>
#include <cstdlib>
#include <cmath>
#include <vector>
#include <cuda_runtime.h>
#include <cublas_v2.h>

#define K 4096
#define N 32768                 // N*K = 128 MB int8 weight -> DRAM-bound on any GPU
static void ck(cudaError_t e,const char* w){ if(e!=cudaSuccess){ fprintf(stderr,"CUDA %s: %s\n",w,cudaGetErrorString(e)); exit(1);} }

// Best batch-1: warp-per-row uint4 dp4a (the shipped GEMV shape) — serves ONE token per full weight read.
__global__ void gemv1(int* y,const signed char* wq,const signed char* xq){
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31;
  const int4* w16=(const int4*)(wq+(long)o*K); const int4* x16=(const int4*)xq; int acc=0; int nq=K/512;
  for(int b=0;b<nq;b++){ int4 w=w16[b*32+lane], xx=x16[b*32+lane];
    acc+=__dp4a(w.x,xx.x,0)+__dp4a(w.y,xx.y,0)+__dp4a(w.z,xx.z,0)+__dp4a(w.w,xx.w,0); }
  for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s); if(lane==0) y[o]=acc;
}

// Batched GEMM Y[B,N] = X[B,K].W[N,K]^T, dp4a, shared-tiled. A W tile is loaded once and REUSED across BM tokens
// (and an X tile across BN outputs) — that reuse breaks the batch-1 weight-bandwidth wall. int8 packed as int4.
#define BM 16
#define BN 16
#define BKI 8                 // 8 ints = 32 int8 per K-tile
__global__ void bgemm(const int* X,const int* W,int* Y,int B){
  int ty=threadIdx.y, tx=threadIdx.x; int bm=blockIdx.y*BM+ty, bn=blockIdx.x*BN+tx;
  __shared__ int sX[BM][BKI], sW[BN][BKI];
  int acc=0; int Ki=K/4;                       // K in ints
  for(int k0=0;k0<Ki;k0+=BKI){
    int lid=ty*BN+tx;
    for(int i=lid;i<BM*BKI;i+=256){ int r=i/BKI,c=i%BKI; int tok=blockIdx.y*BM+r; sX[r][c]= tok<B? X[(long)tok*Ki+k0+c]:0; }
    for(int i=lid;i<BN*BKI;i+=256){ int r=i/BKI,c=i%BKI; int out=blockIdx.x*BN+r; sW[r][c]= out<N? W[(long)out*Ki+k0+c]:0; }
    __syncthreads();
    #pragma unroll
    for(int c=0;c<BKI;c++) acc=__dp4a(sX[ty][c],sW[tx][c],acc);
    __syncthreads();
  }
  if(bm<B && bn<N) Y[(long)bm*N+bn]=acc;
}

int main(){
  int Bmax=256;
  std::vector<signed char> hW((long)N*K), hX((long)Bmax*K);
  srand(7); for(auto&v:hW) v=(rand()%15)-7; for(auto&v:hX) v=(rand()%15)-7;
  signed char *W,*X; int *Y;
  ck(cudaMalloc(&W,(long)N*K),"W"); ck(cudaMalloc(&X,(long)Bmax*K),"X"); ck(cudaMalloc(&Y,(long)Bmax*N*4),"Y");
  ck(cudaMemcpy(W,hW.data(),(long)N*K,cudaMemcpyHostToDevice),"cW");
  ck(cudaMemcpy(X,hX.data(),(long)Bmax*K,cudaMemcpyHostToDevice),"cX");
  cudaEvent_t t0,t1; cudaEventCreate(&t0); cudaEventCreate(&t1); float ms;
  cublasHandle_t cb; cublasCreate(&cb); int one=1, zero=0;
  // cuBLAS int8 tensor-core GEMM, TN: C[B,N]col-major = X^T.W where X is [K,B]col (=[B,K]row), W is [K,N]col (=[N,K]row).
  auto cublasBatched=[&](int B){ return cublasGemmEx(cb, CUBLAS_OP_T, CUBLAS_OP_N, B, N, K,
      &one, X, CUDA_R_8I, K, W, CUDA_R_8I, K, &zero, Y, CUDA_R_32I, B, CUBLAS_COMPUTE_32I, CUBLAS_GEMM_DEFAULT); };

  // correctness: cuBLAS batched vs CPU on B=4, first 64 outputs (C col-major [B,N] -> C[b + o*B])
  { cublasStatus_t st=cublasBatched(4); cudaDeviceSynchronize();
    if(st!=CUBLAS_STATUS_SUCCESS){ printf("cublas int8 GEMM status=%d\n",(int)st); }
    std::vector<int> hY(4*N); cudaMemcpy(hY.data(),Y,4*N*4,cudaMemcpyDeviceToHost);
    long err=0; for(int b=0;b<4;b++) for(int o=0;o<64;o++){ long ref=0; for(int k=0;k<K;k++) ref+=(int)hX[(long)b*K+k]*(int)hW[(long)o*K+k]; err+=llabs((long)hY[b+o*4]-ref); }
    printf("cuBLAS int8 GEMM correctness vs CPU (B=4, 64 cols): abs err = %ld  (%s)\n", err, err==0?"EXACT":"MISMATCH"); }

  const int REP=200, WARM=30; int dev; cudaGetDevice(&dev); int sm; cudaDeviceGetAttribute(&sm,cudaDevAttrMultiProcessorCount,dev);
  double wMB=(double)N*K/1e6;
  printf("\nDRAM-bound weight: K=%d N=%d = %.0f MB int8, SMs=%d\n", K,N,wMB,sm);
  for(int i=0;i<WARM;i++) gemv1<<<(N+7)/8,256>>>(Y,W,X); cudaDeviceSynchronize();
  cudaEventRecord(t0); for(int i=0;i<REP;i++) gemv1<<<(N+7)/8,256>>>(Y,W,X); cudaEventRecord(t1); cudaEventSynchronize(t1); cudaEventElapsedTime(&ms,t0,t1);
  double us1=ms*1e3/REP, tps1=1.0/(us1*1e-6);
  printf("  B=%-4d GEMV (ours)   : %8.1f us  %10.0f tok/s  %6.0f GB/s  (1.0x)\n", 1, us1, tps1, wMB*1e6/(us1*1e3));
  int Bs[]={2,4,8,16,32,64,128,256};
  for(int bi=0; bi<8; bi++){ int B=Bs[bi];
    for(int i=0;i<WARM;i++) cublasBatched(B); cudaDeviceSynchronize();
    cudaEventRecord(t0); for(int i=0;i<REP;i++) cublasBatched(B); cudaEventRecord(t1); cudaEventSynchronize(t1); cudaEventElapsedTime(&ms,t0,t1);
    double us=ms*1e3/REP, tps=B/(us*1e-6);
    printf("  B=%-4d GEMM (tcore) : %8.1f us  %10.0f tok/s  %6.0f GB/s  (%.1fx)\n", B, us, tps, wMB*1e6/(us*1e3), tps/tps1);
  }
  printf("\ntok/s scaling with batch = the multi-x that breaks the batch-1 weight-bandwidth wall (one weight read serves B tokens).\n");
  return 0;
}
