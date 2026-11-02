// M1: register-dequant Marlin-class Q8_0 GEMM. mma.sync.m16n8k32.s8 (int8 tensor core, int32 accumulate). The
// mma's k=32 aligns EXACTLY with the Q8_0 32-scale-block, so after each mma the int32 fragment (4 values/thread,
// documented layout) is dequant-scaled by Ws[o]*Xs[b] IN REGISTERS and summed in f32 -- no shared round-trip
// (that round-trip is what capped v1 at 7.5x). Same float grouping as dp4a. golden-checked vs dp4a (the oracle
// that validates the fragment layout: if the (row,col) map is wrong, the result won't match).
//
// Build: nvcc -O3 -arch=sm_90 marlin_v2.cu -o /tmp/m2 && /tmp/m2
#include <cstdio>
#include <cstdlib>
#include <vector>
#include <cuda_runtime.h>
static void ck(cudaError_t e,const char* w){ if(e!=cudaSuccess){ fprintf(stderr,"CUDA %s: %s\n",w,cudaGetErrorString(e)); exit(1);} }
#define K 4096
#define N 32768
#define KB (K/32)

__global__ void dp4a_ref(float* y,const signed char* wq,const float* ws,const signed char* xq,const float* xs){
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31;
  const int4* w16=(const int4*)(wq+(long)o*K); const int4* x16=(const int4*)xq; const float* wsr=ws+(long)o*KB; float acc=0.f;
  for(int b=0;b<K/512;b++){ int4 w=w16[b*32+lane], xx=x16[b*32+lane]; int blk=b*16+lane/2;
    int d=__dp4a(w.x,xx.x,0)+__dp4a(w.y,xx.y,0)+__dp4a(w.z,xx.z,0)+__dp4a(w.w,xx.w,0);
    acc += (float)d * (wsr[blk]*xs[blk]); }
  for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s); if(lane==0) y[o]=acc;
}

// one warp -> 16 tokens x 8 outputs, full K, register dequant per 32-block.
__global__ void marlin2(const signed char* Xq,const float* Xs,const signed char* Wq,const float* Ws,float* Y,int B){
  int warp=(blockIdx.x*blockDim.x+threadIdx.x)>>5; int lane=threadIdx.x&31;
  int ntiles=N/8; int mtile=warp/ntiles, ntile=warp%ntiles; int m0=mtile*16, n0=ntile*8;
  int gid=lane>>2, tid=lane&3;
  float d0=0,d1=0,d2=0,d3=0;
  for(int kb=0;kb<KB;kb++){ int k0=kb*32;
    int a0=*(const int*)(Xq+(long)(m0+gid)*K+k0+tid*4);
    int a1=*(const int*)(Xq+(long)(m0+gid+8)*K+k0+tid*4);
    int a2=*(const int*)(Xq+(long)(m0+gid)*K+k0+16+tid*4);
    int a3=*(const int*)(Xq+(long)(m0+gid+8)*K+k0+16+tid*4);
    int b0=*(const int*)(Wq+(long)(n0+gid)*K+k0+tid*4);
    int b1=*(const int*)(Wq+(long)(n0+gid)*K+k0+16+tid*4);
    int c0=0,c1=0,c2=0,c3=0;
    asm volatile("mma.sync.aligned.m16n8k32.row.col.s32.s8.s8.s32 {%0,%1,%2,%3}, {%4,%5,%6,%7}, {%8,%9}, {%0,%1,%2,%3};"
      : "+r"(c0),"+r"(c1),"+r"(c2),"+r"(c3)
      : "r"(a0),"r"(a1),"r"(a2),"r"(a3), "r"(b0),"r"(b1));
    float xs0=Xs[(long)(m0+gid)*KB+kb], xs1=Xs[(long)(m0+gid+8)*KB+kb];
    float ws0=Ws[(long)(n0+tid*2)*KB+kb], ws1=Ws[(long)(n0+tid*2+1)*KB+kb];
    d0+=(float)c0*(ws0*xs0); d1+=(float)c1*(ws1*xs0);
    d2+=(float)c2*(ws0*xs1); d3+=(float)c3*(ws1*xs1);
  }
  int r0=m0+gid, r1=m0+gid+8, cc0=n0+tid*2, cc1=n0+tid*2+1;
  if(r0<B){ if(cc0<N) Y[(long)r0*N+cc0]=d0; if(cc1<N) Y[(long)r0*N+cc1]=d1; }
  if(r1<B){ if(cc0<N) Y[(long)r1*N+cc0]=d2; if(cc1<N) Y[(long)r1*N+cc1]=d3; }
}


// M2-lite: 16 tokens x 32 outputs per warp (4 mma subtiles/k-block) -> 4 independent mma in flight -> ILP hides
// the mma+load latency that capped v2. Register dequant per 32-block, same golden grouping.
__global__ void marlin3(const signed char* Xq,const float* Xs,const signed char* Wq,const float* Ws,float* Y,int B){
  int warp=(blockIdx.x*blockDim.x+threadIdx.x)>>5; int lane=threadIdx.x&31;
  int ntiles=N/32; int mtile=warp/ntiles, ntile=warp%ntiles; int m0=mtile*16, n0=ntile*32;
  int gid=lane>>2, tid=lane&3;
  float d[4][4]; for(int j=0;j<4;j++){d[j][0]=d[j][1]=d[j][2]=d[j][3]=0;}
  for(int kb=0;kb<KB;kb++){ int k0=kb*32;
    int a0=*(const int*)(Xq+(long)(m0+gid)*K+k0+tid*4);
    int a1=*(const int*)(Xq+(long)(m0+gid+8)*K+k0+tid*4);
    int a2=*(const int*)(Xq+(long)(m0+gid)*K+k0+16+tid*4);
    int a3=*(const int*)(Xq+(long)(m0+gid+8)*K+k0+16+tid*4);
    float xs0=Xs[(long)(m0+gid)*KB+kb], xs1=Xs[(long)(m0+gid+8)*KB+kb];
    #pragma unroll
    for(int j=0;j<4;j++){ int nn=n0+j*8;
      int b0=*(const int*)(Wq+(long)(nn+gid)*K+k0+tid*4);
      int b1=*(const int*)(Wq+(long)(nn+gid)*K+k0+16+tid*4);
      int c0=0,c1=0,c2=0,c3=0;
      asm volatile("mma.sync.aligned.m16n8k32.row.col.s32.s8.s8.s32 {%0,%1,%2,%3}, {%4,%5,%6,%7}, {%8,%9}, {%0,%1,%2,%3};"
        : "+r"(c0),"+r"(c1),"+r"(c2),"+r"(c3) : "r"(a0),"r"(a1),"r"(a2),"r"(a3), "r"(b0),"r"(b1));
      float ws0=Ws[(long)(nn+tid*2)*KB+kb], ws1=Ws[(long)(nn+tid*2+1)*KB+kb];
      d[j][0]+=(float)c0*(ws0*xs0); d[j][1]+=(float)c1*(ws1*xs0);
      d[j][2]+=(float)c2*(ws0*xs1); d[j][3]+=(float)c3*(ws1*xs1);
    }
  }
  for(int j=0;j<4;j++){ int nn=n0+j*8; int r0=m0+gid,r1=m0+gid+8,cc0=nn+tid*2,cc1=nn+tid*2+1;
    if(r0<B){ if(cc0<N)Y[(long)r0*N+cc0]=d[j][0]; if(cc1<N)Y[(long)r0*N+cc1]=d[j][1]; }
    if(r1<B){ if(cc0<N)Y[(long)r1*N+cc0]=d[j][2]; if(cc1<N)Y[(long)r1*N+cc1]=d[j][3]; } }
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

  { int B=16, blk=256; long warps=(long)(B/16)*(N/8); dim3 grd((warps*32+blk-1)/blk); marlin2<<<grd,blk>>>(X,Xs,W,Ws,Y,B); cudaDeviceSynchronize();
    std::vector<float> hY((long)B*N); cudaMemcpy(hY.data(),Y,(long)B*N*4,cudaMemcpyDeviceToHost);
    long mism=0; double maxabs=0; std::vector<float> hYr(N);
    for(int b=0;b<4;b++){ dp4a_ref<<<(N+7)/8,256>>>(Yr,W,Ws,X+(long)b*K,Xs+(long)b*KB); cudaDeviceSynchronize(); cudaMemcpy(hYr.data(),Yr,N*4,cudaMemcpyDeviceToHost);
      for(int o=0;o<N;o++){ if(hY[(long)b*N+o]!=hYr[o]) mism++; double a=fabs(hY[(long)b*N+o]-hYr[o]); if(a>maxabs)maxabs=a; } }
    printf("golden: marlin-v2 (mma.sync reg-dequant) vs dp4a, 4 tokens x %d outs  mismatches=%ld maxabs=%.3e  (%s)\n",
      N, mism, maxabs, mism==0?"BIT-IDENTICAL":(maxabs<1e-2?"~match(fp-order, layout OK)":"LAYOUT WRONG")); }

  const int REP=200, WARM=30;
  printf("\nMarlin-v2 register-dequant, K=%d N=%d = %.0f MB\n", K,N,(double)N*K/1e6);
  for(int i=0;i<WARM;i++) dp4a_ref<<<(N+7)/8,256>>>(Yr,W,Ws,X,Xs); cudaDeviceSynchronize();
  cudaEventRecord(t0); for(int i=0;i<REP;i++) dp4a_ref<<<(N+7)/8,256>>>(Yr,W,Ws,X,Xs); cudaEventRecord(t1); cudaEventSynchronize(t1); cudaEventElapsedTime(&ms,t0,t1);
  double us1=ms*1e3/REP, tps1=1.0/(us1*1e-6); printf("  B=1    dp4a  : %8.1f us  %10.0f tok/s  (1.0x)\n", us1, tps1);
  int Bs[]={16,32,64,128,256,512};
  for(int bi=0; bi<6; bi++){ int B=Bs[bi]; int blk=256; long warps=(long)(B/16)*(N/8); dim3 grd((warps*32+blk-1)/blk);
    for(int i=0;i<WARM;i++) marlin2<<<grd,blk>>>(X,Xs,W,Ws,Y,B); cudaDeviceSynchronize();
    cudaEventRecord(t0); for(int i=0;i<REP;i++) marlin2<<<grd,blk>>>(X,Xs,W,Ws,Y,B); cudaEventRecord(t1); cudaEventSynchronize(t1); cudaEventElapsedTime(&ms,t0,t1);
    double us=ms*1e3/REP, tps=B/(us*1e-6); printf("  B=%-4d v2    : %8.1f us  %10.0f tok/s  (%.1fx)\n", B, us, tps, tps/tps1);
  }
  { int B=16, blk=256; long warps=(long)(B/16)*(N/32); dim3 grd((warps*32+blk-1)/blk); marlin3<<<grd,blk>>>(X,Xs,W,Ws,Y,B); cudaDeviceSynchronize();
    std::vector<float> hY((long)B*N); cudaMemcpy(hY.data(),Y,(long)B*N*4,cudaMemcpyDeviceToHost);
    long mism=0; double maxabs=0; std::vector<float> hYr(N);
    for(int b=0;b<2;b++){ dp4a_ref<<<(N+7)/8,256>>>(Yr,W,Ws,X+(long)b*K,Xs+(long)b*KB); cudaDeviceSynchronize(); cudaMemcpy(hYr.data(),Yr,N*4,cudaMemcpyDeviceToHost);
      for(int o=0;o<N;o++){ if(hY[(long)b*N+o]!=hYr[o]) mism++; double a=fabs(hY[(long)b*N+o]-hYr[o]); if(a>maxabs)maxabs=a; } }
    printf("\nv3 golden: maxabs=%.3e (%s)\n", maxabs, maxabs<1e-2?"layout OK":"WRONG"); }
  printf("Marlin-v3 (4 mma/warp ILP):\n");
  for(int bi=0; bi<6; bi++){ int B=Bs[bi]; int blk=256; long warps=(long)(B/16)*(N/32); dim3 grd((warps*32+blk-1)/blk);
    for(int i=0;i<WARM;i++) marlin3<<<grd,blk>>>(X,Xs,W,Ws,Y,B); cudaDeviceSynchronize();
    cudaEventRecord(t0); for(int i=0;i<REP;i++) marlin3<<<grd,blk>>>(X,Xs,W,Ws,Y,B); cudaEventRecord(t1); cudaEventSynchronize(t1); cudaEventElapsedTime(&ms,t0,t1);
    double us=ms*1e3/REP, tps=B/(us*1e-6); printf("  B=%-4d v3    : %8.1f us  %10.0f tok/s  (%.1fx)\n", B, us, tps, tps/tps1);
  }
  return 0;
}
