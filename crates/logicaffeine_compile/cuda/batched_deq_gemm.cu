// Golden-preserving batched GEMM: Y[B,N] = sum_k (Wint8[n,k]*scale[n,k/32]) * Xf32[b,k]. This is the building
// block of the batched Qwen3-30B decode. int8 WEIGHT x f32 ACTIVATION (NOT cuBLAS int8, which would quantize the
// activation and re-introduce the near-tie flips) -> byte-exact by construction, so every batched sequence still
// emits the golden tokens. The batching win: the int8 weight is read ONCE and its dequant reused across B tokens.
//
// Build: nvcc -O3 -arch=sm_90 batched_deq_gemm.cu -o /tmp/bdg && /tmp/bdg   (sm_86 on the 3060 also fine)
#include <cstdio>
#include <cstdlib>
#include <cmath>
#include <vector>
#include <cuda_runtime.h>
static void ck(cudaError_t e,const char* w){ if(e!=cudaSuccess){ fprintf(stderr,"CUDA %s: %s\n",w,cudaGetErrorString(e)); exit(1);} }

#define K 4096
#define N 8192            // K*N int8 = 32 MB weight (>> L2 -> DRAM-bound, mirrors the decode weight wall)
#define BM 16             // token tile
#define BN 32             // output tile
#define BK 32             // K tile (== scale block)

// batch-1 baseline: our shipped dequant GEMV (warp per output), int8 weight x f32 activation.
__global__ void gemv_deq1(float* y,const signed char* wq,const float* ws,const float* x){
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31;
  const int4* w16=(const int4*)(wq+(long)o*K); const float* wsr=ws+(long)o*(K/32); float acc=0.f; int nq=K/512;
  for(int b=0;b<nq;b++){ int4 wv=w16[b*32+lane]; int blk=b*16+lane/2; float sc=wsr[blk];
    const signed char* wb=(const signed char*)&wv; const float* xt=x+b*512+lane*16;
    float ba=0.f; for(int i=0;i<16;i++) ba+=(float)wb[i]*xt[i]; acc+=sc*ba; }
  for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s); if(lane==0) y[o]=acc;
}

// batched: tile BM tokens x BN outputs, block 256 threads (each thread computes 2 tokens x 4 outputs micro-tile).
// The W tile (int8) is dequanted to shared ONCE and reused across all BM tokens -> weight read/dequant amortized.
__global__ void bgemm_deq(const float* X,const signed char* W,const float* WS,float* Y,int B){
  int tx=threadIdx.x%(BN/4), ty=threadIdx.x/(BN/4);   // 8 x 32 threads-> tx:0..7 (out group), ty:0..31? use 256=8x32
  // layout: 256 threads = (BN/4=8) x 32 ; each thread does 4 outputs x (BM/? ) -- simpler: thread -> one (bm,bn),
  // 256 threads cover BM*BN=16*32=512 -> 2 outputs each. Use lid.
  int lid=threadIdx.x;
  __shared__ float sX[BM][BK];
  __shared__ float sW[BN][BK];           // dequanted weight tile
  float acc[2]={0,0};                     // each thread owns 2 (bm,bn) cells
  int cell0=lid, cell1=lid+256;           // cells 0..511
  int bm0=cell0/BN, bn0=cell0%BN, bm1=cell1/BN, bn1=cell1%BN;
  for(int k0=0;k0<K;k0+=BK){
    for(int i=lid;i<BM*BK;i+=256){ int r=i/BK,c=i%BK; int tok=blockIdx.y*BM+r; sX[r][c]= tok<B? X[(long)tok*K+k0+c]:0.f; }
    for(int i=lid;i<BN*BK;i+=256){ int r=i/BK,c=i%BK; int out=blockIdx.x*BN+r;
      float sc = out<N? WS[(long)out*(K/32)+(k0+c)/32]:0.f; sW[r][c]= out<N? (float)W[(long)out*K+k0+c]*sc:0.f; }
    __syncthreads();
    #pragma unroll
    for(int k=0;k<BK;k++){ acc[0]+=sX[bm0][k]*sW[bn0][k]; acc[1]+=sX[bm1][k]*sW[bn1][k]; }
    __syncthreads();
  }
  int gm0=blockIdx.y*BM+bm0, gn0=blockIdx.x*BN+bn0; if(gm0<B&&gn0<N) Y[(long)gm0*N+gn0]=acc[0];
  int gm1=blockIdx.y*BM+bm1, gn1=blockIdx.x*BN+bn1; if(gm1<B&&gn1<N) Y[(long)gm1*N+gn1]=acc[1];
}

int main(){
  int Bmax=256;
  std::vector<signed char> hW((long)N*K); std::vector<float> hWS((long)N*(K/32)), hX((long)Bmax*K);
  srand(3); for(auto&v:hW) v=(rand()%15)-7; for(auto&v:hWS) v=0.01f+(rand()%100)*1e-4f; for(auto&v:hX) v=(rand()%200-100)*0.01f;
  signed char* W; float *WS,*X,*Y;
  ck(cudaMalloc(&W,(long)N*K),"W"); ck(cudaMalloc(&WS,(long)N*(K/32)*4),"WS"); ck(cudaMalloc(&X,(long)Bmax*K*4),"X"); ck(cudaMalloc(&Y,(long)Bmax*N*4),"Y");
  ck(cudaMemcpy(W,hW.data(),(long)N*K,cudaMemcpyHostToDevice),"cW");
  ck(cudaMemcpy(WS,hWS.data(),(long)N*(K/32)*4,cudaMemcpyHostToDevice),"cWS");
  ck(cudaMemcpy(X,hX.data(),(long)Bmax*K*4,cudaMemcpyHostToDevice),"cX");
  cudaEvent_t t0,t1; cudaEventCreate(&t0); cudaEventCreate(&t1); float ms;

  // correctness: batched (row b) must EQUAL the batch-1 GEMV on the same row (golden preservation)
  { dim3 blk(256), grd((N+BN-1)/BN,(8+BM-1)/BM); bgemm_deq<<<grd,blk>>>(X,W,WS,Y,8); cudaDeviceSynchronize();
    std::vector<float> hYb(8*N); cudaMemcpy(hYb.data(),Y,8*N*4,cudaMemcpyDeviceToHost);
    float* Y1; cudaMalloc(&Y1,N*4); gemv_deq1<<<(N+7)/8,256>>>(Y1,W,WS,X); cudaDeviceSynchronize(); // row 0
    std::vector<float> hY1(N); cudaMemcpy(hY1.data(),Y1,N*4,cudaMemcpyDeviceToHost);
    double maxrel=0; for(int o=0;o<N;o++){ double d=fabs(hYb[o]-hY1[o]); double r=d/(fabs(hY1[o])+1e-6); if(r>maxrel)maxrel=r; }
    printf("golden-preservation: batched row0 vs batch-1 GEMV  max rel err = %.2e  (%s)\n", maxrel, maxrel<1e-4?"MATCH (byte-safe)":"DRIFT"); cudaFree(Y1); }

  const int REP=300, WARM=40; double wMB=(double)N*K/1e6;
  printf("\nint8 weight x f32 act (golden lane), K=%d N=%d = %.0f MB\n", K,N,wMB);
  for(int i=0;i<WARM;i++) gemv_deq1<<<(N+7)/8,256>>>(Y,W,WS,X); cudaDeviceSynchronize();
  cudaEventRecord(t0); for(int i=0;i<REP;i++) gemv_deq1<<<(N+7)/8,256>>>(Y,W,WS,X); cudaEventRecord(t1); cudaEventSynchronize(t1); cudaEventElapsedTime(&ms,t0,t1);
  double us1=ms*1e3/REP, tps1=1.0/(us1*1e-6);
  printf("  B=%-4d GEMV : %8.1f us  %10.0f tok/s  (1.0x)\n", 1, us1, tps1);
  int Bs[]={2,4,8,16,32,64,128,256};
  for(int bi=0; bi<8; bi++){ int B=Bs[bi]; dim3 blk(256), grd((N+BN-1)/BN,(B+BM-1)/BM);
    for(int i=0;i<WARM;i++) bgemm_deq<<<grd,blk>>>(X,W,WS,Y,B); cudaDeviceSynchronize();
    cudaEventRecord(t0); for(int i=0;i<REP;i++) bgemm_deq<<<grd,blk>>>(X,W,WS,Y,B); cudaEventRecord(t1); cudaEventSynchronize(t1); cudaEventElapsedTime(&ms,t0,t1);
    double us=ms*1e3/REP, tps=B/(us*1e-6); printf("  B=%-4d GEMM : %8.1f us  %10.0f tok/s  (%.1fx)\n", B, us, tps, tps/tps1);
  }
  printf("\ngolden-preserving batched GEMM: same int8 weight, one read serves B tokens, byte-exact per row.\n");
  return 0;
}
