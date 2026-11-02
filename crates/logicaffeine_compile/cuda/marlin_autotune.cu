// The compiler auto-tunes the Marlin-class Q8_0 GEMM: one PARAMETERIZED kernel (NT = output-subtiles per warp),
// swept over the config space, each candidate GOLDEN-CHECKED vs dp4a (won't corrupt the weights), the fastest
// CORRECT config picked per (shape, GPU). That's "the compiler does the work + right-sizes" -- not a hand-tuned
// magic constant. Register-dequant mma.sync.m16n8k32, per-32-block Q8_0 scale in registers, same golden grouping.
//
// Build: nvcc -O3 -arch=sm_90 marlin_autotune.cu -o /tmp/at && /tmp/at
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

// parameterized: NT output-subtiles (each 8 wide) per warp -> 16 tokens x (8*NT) outputs, NT mma/k-block.
template<int NT>
__global__ void marlin(const signed char* Xq,const float* Xs,const signed char* Wq,const float* Ws,float* Y,int B){
  int warp=(blockIdx.x*blockDim.x+threadIdx.x)>>5; int lane=threadIdx.x&31;
  int ntiles=N/(8*NT); int mtile=warp/ntiles, ntile=warp%ntiles; int m0=mtile*16, n0=ntile*8*NT;
  int gid=lane>>2, tid=lane&3;
  float d[NT][4]; for(int j=0;j<NT;j++){d[j][0]=d[j][1]=d[j][2]=d[j][3]=0;}
  for(int kb=0;kb<KB;kb++){ int k0=kb*32;
    int a0=*(const int*)(Xq+(long)(m0+gid)*K+k0+tid*4);
    int a1=*(const int*)(Xq+(long)(m0+gid+8)*K+k0+tid*4);
    int a2=*(const int*)(Xq+(long)(m0+gid)*K+k0+16+tid*4);
    int a3=*(const int*)(Xq+(long)(m0+gid+8)*K+k0+16+tid*4);
    float xs0=Xs[(long)(m0+gid)*KB+kb], xs1=Xs[(long)(m0+gid+8)*KB+kb];
    #pragma unroll
    for(int j=0;j<NT;j++){ int nn=n0+j*8;
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
  for(int j=0;j<NT;j++){ int nn=n0+j*8; int r0=m0+gid,r1=m0+gid+8,cc0=nn+tid*2,cc1=nn+tid*2+1;
    if(r0<B){ if(cc0<N)Y[(long)r0*N+cc0]=d[j][0]; if(cc1<N)Y[(long)r0*N+cc1]=d[j][1]; }
    if(r1<B){ if(cc0<N)Y[(long)r1*N+cc0]=d[j][2]; if(cc1<N)Y[(long)r1*N+cc1]=d[j][3]; } }
}

static signed char *W,*X; static float *Ws,*Xs,*Y,*Yr;
static cudaEvent_t t0,t1;
template<int NT> double run(int B,int REP,int WARM,double& maxabs){
  int blk=256; long warps=(long)((B+15)/16)*(N/(8*NT)); dim3 grd((warps*32+blk-1)/blk);
  marlin<NT><<<grd,blk>>>(X,Xs,W,Ws,Y,B); cudaDeviceSynchronize();
  // golden vs dp4a on token 0
  std::vector<float> hY(N),hYr(N); cudaMemcpy(hY.data(),Y,N*4,cudaMemcpyDeviceToHost);
  dp4a_ref<<<(N+7)/8,256>>>(Yr,W,Ws,X,Xs); cudaDeviceSynchronize(); cudaMemcpy(hYr.data(),Yr,N*4,cudaMemcpyDeviceToHost);
  maxabs=0; for(int o=0;o<N;o++){ double a=fabs(hY[o]-hYr[o]); if(a>maxabs)maxabs=a; }
  float ms; for(int i=0;i<WARM;i++) marlin<NT><<<grd,blk>>>(X,Xs,W,Ws,Y,B); cudaDeviceSynchronize();
  cudaEventRecord(t0); for(int i=0;i<REP;i++) marlin<NT><<<grd,blk>>>(X,Xs,W,Ws,Y,B); cudaEventRecord(t1); cudaEventSynchronize(t1); cudaEventElapsedTime(&ms,t0,t1);
  return B/((ms*1e3/REP)*1e-6);
}

int main(){
  int Bmax=256;
  std::vector<signed char> hW((long)N*K), hX((long)Bmax*K); std::vector<float> hWs((long)N*KB), hXs((long)Bmax*KB);
  srand(11); for(auto&v:hW)v=(rand()%15)-7; for(auto&v:hX)v=(rand()%15)-7;
  for(auto&v:hWs)v=0.008f+(rand()%50)*1e-4f; for(auto&v:hXs)v=0.01f+(rand()%50)*1e-4f;
  ck(cudaMalloc(&W,(long)N*K),"W"); ck(cudaMalloc(&X,(long)Bmax*K),"X"); ck(cudaMalloc(&Ws,(long)N*KB*4),"Ws"); ck(cudaMalloc(&Xs,(long)Bmax*KB*4),"Xs");
  ck(cudaMalloc(&Y,(long)Bmax*N*4),"Y"); ck(cudaMalloc(&Yr,N*4),"Yr");
  cudaMemcpy(W,hW.data(),(long)N*K,cudaMemcpyHostToDevice); cudaMemcpy(X,hX.data(),(long)Bmax*K,cudaMemcpyHostToDevice);
  cudaMemcpy(Ws,hWs.data(),(long)N*KB*4,cudaMemcpyHostToDevice); cudaMemcpy(Xs,hXs.data(),(long)Bmax*KB*4,cudaMemcpyHostToDevice);
  cudaEventCreate(&t0); cudaEventCreate(&t1);
  printf("AUTO-TUNE Marlin Q8_0 GEMM (K=%d N=%d): sweep NT in {1,2,4,8,16}, golden-check each, pick fastest CORRECT\n",K,N);
  int Bs[]={16,32,64,128,256};
  for(int bi=0; bi<5; bi++){ int B=Bs[bi]; double best=0; int bestNT=0; double bestErr=0;
    double ma; double t;
    t=run<1>(B,200,30,ma);  if(ma<1e-5&&t>best){best=t;bestNT=1;bestErr=ma;}  printf("  B=%-4d NT=1 : %8.0f tok/s  err=%.1e\n",B,t,ma);
    t=run<2>(B,200,30,ma);  if(ma<1e-5&&t>best){best=t;bestNT=2;bestErr=ma;}  printf("         NT=2 : %8.0f tok/s  err=%.1e\n",t,ma);
    t=run<4>(B,200,30,ma);  if(ma<1e-5&&t>best){best=t;bestNT=4;bestErr=ma;}  printf("         NT=4 : %8.0f tok/s  err=%.1e\n",t,ma);
    t=run<8>(B,200,30,ma);  if(ma<1e-5&&t>best){best=t;bestNT=8;bestErr=ma;}  printf("         NT=8 : %8.0f tok/s  err=%.1e\n",t,ma);
    t=run<16>(B,200,30,ma); if(ma<1e-5&&t>best){best=t;bestNT=16;bestErr=ma;} printf("         NT=16: %8.0f tok/s  err=%.1e\n",t,ma);
    printf("  >> AUTO-TUNER PICKS B=%d: NT=%d @ %.0f tok/s (golden err %.1e)\n\n", B, bestNT, best, bestErr);
  }
  return 0;
}
