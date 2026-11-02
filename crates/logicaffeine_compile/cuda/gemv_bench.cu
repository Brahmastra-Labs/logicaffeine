// Isolated Q8_0 GEMV bandwidth probe — how close to the 3060's ~360 GB/s peak does our fused int8 kernel
// get on a big matrix? This is the saturation ceiling for the whole engine (the forward is GEMV-bound).
//   nvcc -O3 -arch=sm_86 gemv_bench.cu -o gemv_bench && ./gemv_bench
#include <cstdio>
#include <cstdint>
#include <cstdlib>
#include <cuda_fp16.h>
#include <cuda_runtime.h>
#define CK(x) do{cudaError_t e=(x);if(e!=cudaSuccess){fprintf(stderr,"cuda %d %s\n",__LINE__,cudaGetErrorString(e));exit(1);} }while(0)

// V1: warp-per-row, one char4 (128B) per lane per iter.
__global__ void gemv_v1(float* y, const int8_t* wq, const __half* ws, const int8_t* xq, const float* xs, int K, int N){
    int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31;
    const int* w4=(const int*)(wq+(long)o*K); const int* x4=(const int*)xq; const __half* sr=ws+(long)o*(K/32);
    float acc=0; int nq=K/128;
    for(int b4=0;b4<nq;b4++){ int d=__dp4a(w4[b4*32+lane],x4[b4*32+lane],0); int b=b4*4+lane/8;
        acc+=(float)d*(__half2float(sr[b])*xs[b]); }
    for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s); if(lane==0) y[o]=acc;
}
// V2: warp-per-row, int4 (512B) per lane per iter — 4× fewer iterations, 4 dp4a, more memory in flight.
__global__ void gemv_v2(float* y, const int8_t* wq, const __half* ws, const int8_t* xq, const float* xs, int K, int N){
    int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31;
    const int4* w16=(const int4*)(wq+(long)o*K); const int4* x16=(const int4*)xq; const __half* sr=ws+(long)o*(K/32);
    float acc=0; int nq=K/512;
    for(int b=0;b<nq;b++){
        int4 w=w16[b*32+lane]; int4 xx=x16[b*32+lane];
        int base=b*16+lane/2;  // this lane's 16 int8 all fall in ONE 32-block (half of it) -> one scale
        int dd=__dp4a(w.x,xx.x,0)+__dp4a(w.y,xx.y,0)+__dp4a(w.z,xx.z,0)+__dp4a(w.w,xx.w,0);
        acc+=(float)dd*(__half2float(sr[base])*xs[base]);
    }
    for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s); if(lane==0) y[o]=acc;
}
// V3: TWO rows per warp (each lane holds 2 accumulators) — reuse the activation read across rows,
// double the compute per memory transaction issued, more rows in flight per block.
__global__ void gemv_v3(float* y, const int8_t* wq, const __half* ws, const int8_t* xq, const float* xs, int K, int N){
    int row2=((blockIdx.x*blockDim.x+threadIdx.x)>>5)*2; if(row2>=N) return; int lane=threadIdx.x&31;
    const int* x4=(const int*)xq; int nq=K/128;
    const int* wa=(const int*)(wq+(long)row2*K); const __half* sa=ws+(long)row2*(K/32);
    const int* wb=(const int*)(wq+(long)(row2+1)*K); const __half* sb=ws+(long)(row2+1)*(K/32);
    float acca=0, accb=0;
    for(int b4=0;b4<nq;b4++){ int xv=x4[b4*32+lane]; int b=b4*4+lane/8; float xsc=xs[b];
        acca+=(float)__dp4a(wa[b4*32+lane],xv,0)*(__half2float(sa[b])*xsc);
        accb+=(float)__dp4a(wb[b4*32+lane],xv,0)*(__half2float(sb[b])*xsc); }
    for(int s=16;s>0;s>>=1){ acca+=__shfl_down_sync(0xffffffffu,acca,s); accb+=__shfl_down_sync(0xffffffffu,accb,s); }
    if(lane==0){ y[row2]=acca; if(row2+1<N) y[row2+1]=accb; }
}

int main(int argc,char**argv){
    int N=16384, K=4096; if(argc>=3){N=atoi(argv[1]);K=atoi(argv[2]);}
    size_t wbytes=(size_t)N*K, sbytes=(size_t)N*(K/32)*2;
    double rd = (wbytes + sbytes)/1e9; // GB read per GEMV (weights dominate)
    int8_t *wq,*xq; __half* ws; float *xs,*y;
    CK(cudaMalloc(&wq,wbytes)); CK(cudaMalloc(&xq,K)); CK(cudaMalloc(&ws,sbytes)); CK(cudaMalloc(&xs,(K/32)*4)); CK(cudaMalloc(&y,N*4));
    CK(cudaMemset(wq,1,wbytes)); CK(cudaMemset(xq,1,K)); CK(cudaMemset(ws,0x3c,sbytes)); CK(cudaMemset(xs,0,(K/32)*4));
    cudaEvent_t t0,t1; CK(cudaEventCreate(&t0)); CK(cudaEventCreate(&t1)); const int IT=100;
    auto run=[&](const char* nm, void(*k)(float*,const int8_t*,const __half*,const int8_t*,const float*,int,int)){
        unsigned g=(N+7)/8; k<<<g,256>>>(y,wq,ws,xq,xs,K,N); CK(cudaDeviceSynchronize());
        CK(cudaEventRecord(t0)); for(int i=0;i<IT;i++) k<<<g,256>>>(y,wq,ws,xq,xs,K,N); CK(cudaEventRecord(t1)); CK(cudaEventSynchronize(t1));
        float ms; cudaEventElapsedTime(&ms,t0,t1); ms/=IT;
        printf("%-8s %6.3f ms  %6.1f GB/s  (%.0f%% of 360 peak)\n", nm, ms, rd/(ms/1e3), 100*rd/(ms/1e3)/360);
    };
    printf("GEMV  N=%d K=%d  reads %.3f GB/call\n", N, K, rd);
    run("v1", gemv_v1); run("v2", gemv_v2); run("v3", gemv_v3);
    return 0;
}
