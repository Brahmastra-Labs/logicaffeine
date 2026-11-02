// Batch-1 int8 GEMV shootout at the MoE gate/up dims (K=2048 inner, N=768 outputs) — the dominant decode kernel
// (~52% of time) currently at ~30% of peak bandwidth. Hunt for a design that reads the int8 weights faster than
// our shipped warp-per-row uint4 dp4a. Whatever wins here ports straight into moe_gate_up_silu. Weight-bandwidth
// is the floor: N*K int8 = 1.5 MB/expert; the metric is effective GB/s (higher = closer to the memory ceiling).
//
// Build: nvcc -O3 -arch=sm_86 gemv_shootout.cu -o /tmp/gv && /tmp/gv
#include <cstdio>
#include <cstdlib>
#include <cmath>
#include <vector>
#include <cuda_runtime.h>

#define K 2048
#define N 768
#define NB (K/32)

static void ck(cudaError_t e,const char* w){ if(e!=cudaSuccess){ fprintf(stderr,"CUDA %s: %s\n",w,cudaGetErrorString(e)); exit(1);} }

// A) SHIPPED: warp-per-row, 32 lanes stride K, uint4 (int4 vec) loads, 4 dp4a/load. Weight read once, coalesced.
__global__ void gv_shipped(float* y,const signed char* wq,const float* ws,const signed char* xq,const float* xs,int nrep){
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31;
  const int4* w16=(const int4*)(wq+(long)o*K); const int4* x16=(const int4*)xq; const float* wsr=ws+(long)o*NB;
  float acc=0.f; int nq=K/512;
  for(int r=0;r<nrep;r++) for(int b=0;b<nq;b++){ int4 w=w16[b*32+lane], xx=x16[b*32+lane]; int blk=b*16+lane/2;
    int d=__dp4a(w.x,xx.x,0)+__dp4a(w.y,xx.y,0)+__dp4a(w.z,xx.z,0)+__dp4a(w.w,xx.w,0);
    acc += (float)d * (wsr[blk]*xs[blk]); }
  for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s); if(lane==0) y[o]=acc;
}
// B) 2 OUTPUTS/WARP: register-block 2 rows, reuse the activation load, ~2x memory-level parallelism per warp.
__global__ void gv_2out(float* y,const signed char* wq,const float* ws,const signed char* xq,const float* xs,int nrep){
  int o0=((blockIdx.x*blockDim.x+threadIdx.x)>>5)*2; if(o0>=N) return; int lane=threadIdx.x&31;
  const int4* x16=(const int4*)xq; int nq=K/512;
  const int4* w0=(const int4*)(wq+(long)o0*K); const int4* w1=(const int4*)(wq+(long)(o0+1)*K);
  const float* ws0=ws+(long)o0*NB; const float* ws1=ws+(long)(o0+1)*NB;
  float a0=0.f,a1=0.f;
  for(int r=0;r<nrep;r++) for(int b=0;b<nq;b++){ int4 xx=x16[b*32+lane]; int4 wa=w0[b*32+lane], wb=w1[b*32+lane]; int blk=b*16+lane/2; float sc=xs[blk];
    int da=__dp4a(wa.x,xx.x,0)+__dp4a(wa.y,xx.y,0)+__dp4a(wa.z,xx.z,0)+__dp4a(wa.w,xx.w,0);
    int db=__dp4a(wb.x,xx.x,0)+__dp4a(wb.y,xx.y,0)+__dp4a(wb.z,xx.z,0)+__dp4a(wb.w,xx.w,0);
    a0+=(float)da*(ws0[blk]*sc); a1+=(float)db*(ws1[blk]*sc); }
  for(int s=16;s>0;s>>=1){ a0+=__shfl_down_sync(0xffffffffu,a0,s); a1+=__shfl_down_sync(0xffffffffu,a1,s); }
  if(lane==0){ y[o0]=a0; if(o0+1<N) y[o0+1]=a1; }
}
// C) 4 OUTPUTS/WARP: register-block 4 rows, one activation load feeds 4 rows (max reuse + MLP).
__global__ void gv_4out(float* y,const signed char* wq,const float* ws,const signed char* xq,const float* xs,int nrep){
  int o0=((blockIdx.x*blockDim.x+threadIdx.x)>>5)*4; if(o0>=N) return; int lane=threadIdx.x&31;
  const int4* x16=(const int4*)xq; int nq=K/512;
  const int4* w[4]; const float* wsr[4]; float acc[4];
  for(int i=0;i<4;i++){ w[i]=(const int4*)(wq+(long)(o0+i)*K); wsr[i]=ws+(long)(o0+i)*NB; acc[i]=0.f; }
  for(int r=0;r<nrep;r++) for(int b=0;b<nq;b++){ int4 xx=x16[b*32+lane]; int blk=b*16+lane/2; float sc=xs[blk];
    #pragma unroll
    for(int i=0;i<4;i++){ int4 wi=w[i][b*32+lane]; int d=__dp4a(wi.x,xx.x,0)+__dp4a(wi.y,xx.y,0)+__dp4a(wi.z,xx.z,0)+__dp4a(wi.w,xx.w,0); acc[i]+=(float)d*(wsr[i][blk]*sc); } }
  for(int s=16;s>0;s>>=1) for(int i=0;i<4;i++) acc[i]+=__shfl_down_sync(0xffffffffu,acc[i],s);
  if(lane==0) for(int i=0;i<4;i++) if(o0+i<N) y[o0+i]=acc[i];
}
// D) SPLIT-K x2: two warps per output split K, partial then atomicAdd. More blocks -> more MLP if occupancy-limited.
__global__ void gv_splitk(float* y,const signed char* wq,const float* ws,const signed char* xq,const float* xs,int nrep){
  int gw=(blockIdx.x*blockDim.x+threadIdx.x)>>5; int o=gw>>1, half=gw&1; if(o>=N) return; int lane=threadIdx.x&31;
  const int4* w16=(const int4*)(wq+(long)o*K); const int4* x16=(const int4*)xq; const float* wsr=ws+(long)o*NB;
  float acc=0.f; int nq=K/512; int b0=half*(nq/2), b1=half? nq : nq/2;
  for(int r=0;r<nrep;r++) for(int b=b0;b<b1;b++){ int4 w=w16[b*32+lane], xx=x16[b*32+lane]; int blk=b*16+lane/2;
    int d=__dp4a(w.x,xx.x,0)+__dp4a(w.y,xx.y,0)+__dp4a(w.z,xx.z,0)+__dp4a(w.w,xx.w,0); acc += (float)d * (wsr[blk]*xs[blk]); }
  for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s); if(lane==0) atomicAdd(&y[o],acc);
}

static signed char *wq,*xq; static float *ws,*xs,*y;
static cudaEvent_t t0,t1;
static double g_bytes;
#define TIME(NAME, LAUNCH) do{ const int REP=3000, WARM=300; float ms; \
  for(int i=0;i<WARM;i++){ LAUNCH; } cudaDeviceSynchronize(); \
  cudaEventRecord(t0); for(int i=0;i<REP;i++){ LAUNCH; } cudaEventRecord(t1); cudaEventSynchronize(t1); cudaEventElapsedTime(&ms,t0,t1); \
  double us=ms*1e3/REP; printf("  %-24s %7.3f us/GEMV   %7.1f GB/s\n", NAME, us, g_bytes/(us*1e3)); }while(0)

int main(){
  ck(cudaMalloc(&wq,(long)N*K),"wq"); ck(cudaMalloc(&xq,K),"xq"); ck(cudaMalloc(&ws,(long)N*NB*4),"ws"); ck(cudaMalloc(&xs,NB*4),"xs"); ck(cudaMalloc(&y,N*4),"y");
  ck(cudaMemset(wq,1,(long)N*K),"m"); ck(cudaMemset(xq,1,K),"m"); ck(cudaMemset(ws,0,(long)N*NB*4),"m"); ck(cudaMemset(xs,0,NB*4),"m");
  cudaEventCreate(&t0); cudaEventCreate(&t1);
  g_bytes = (double)N*K;   // int8 weights read once per GEMV (the bandwidth floor)
  int dev; cudaGetDevice(&dev); int smc; cudaDeviceGetAttribute(&smc,cudaDevAttrMultiProcessorCount,dev);
  printf("MoE gate GEMV: K=%d N=%d batch-1, weight read=%.2f MB, SMs=%d\n", K, N, g_bytes/1e6, smc);
  TIME("A) shipped warp/row",  (gv_shipped<<<(N+7)/8,256>>>(y,wq,ws,xq,xs,1)));
  TIME("B) 2 outputs/warp",    (gv_2out<<<((N+1)/2+7)/8,256>>>(y,wq,ws,xq,xs,1)));
  TIME("C) 4 outputs/warp",    (gv_4out<<<((N+3)/4+7)/8,256>>>(y,wq,ws,xq,xs,1)));
  TIME("D) split-K x2",        (cudaMemset(y,0,N*4), gv_splitk<<<(2*N+7)/8,256>>>(y,wq,ws,xq,xs,1)));
  printf("\nHigher GB/s = faster MoE. If B/C beat A, register-blocking wins (more MLP hides batch-1 latency) -> port to moe_gate_up_silu.\n");
  return 0;
}
