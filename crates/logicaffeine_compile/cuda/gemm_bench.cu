// The "both" GEMM lane: our own proven DP4A int8 kernel vs NVIDIA cuBLAS int8, head to head.
// Same GEMM O(M x N) = A(M x K) . W(N x K)^T in int8 (int32 accumulate); both verified against a CPU
// reference, then timed. Prints GOPS for each — proof (our kernel) vs speed (cuBLAS).
//
//   nvcc -O3 -arch=sm_86 gemm_bench.cu -lcublas -o gemm_bench && ./gemm_bench [M K N]

#include <cstdio>
#include <cstdint>
#include <cstdlib>
#include <vector>
#include <cuda_runtime.h>
#include <cublas_v2.h>

#define CK(x) do{cudaError_t e=(x);if(e!=cudaSuccess){fprintf(stderr,"cuda %d %s\n",__LINE__,cudaGetErrorString(e));exit(1);} }while(0)
#define CB(x) do{cublasStatus_t s=(x);if(s!=CUBLAS_STATUS_SUCCESS){fprintf(stderr,"cublas %d %d\n",__LINE__,(int)s);exit(1);} }while(0)

// Our DP4A int8 GEMM (the proven kernel shape): one thread per output, 4-int8 lanes via __dp4a.
__global__ void dp4a_gemm(int* O, const int8_t* A, const int8_t* W, int M, int K, int N){
    int j = blockIdx.x*blockDim.x + threadIdx.x; // output col (W row)
    int i = blockIdx.y*blockDim.y + threadIdx.y; // output row (A row)
    if(i>=M||j>=N) return;
    int acc=0;
    const int* a4=(const int*)(A + (long)i*K);
    const int* w4=(const int*)(W + (long)j*K);
    for(int t=0;t<K/4;t++) acc=__dp4a(a4[t], w4[t], acc);
    O[(long)i*N+j]=acc;
}

int main(int argc, char** argv){
    int M=4096,K=4096,N=4096;
    if(argc>=4){ M=atoi(argv[1]); K=atoi(argv[2]); N=atoi(argv[3]); }
    printf("int8 GEMM  M=%d K=%d N=%d  (O = A . W^T)\n", M,K,N);
    size_t szA=(size_t)M*K, szW=(size_t)N*K, szO=(size_t)M*N;
    std::vector<int8_t> hA(szA), hW(szW); std::vector<int> hRef(szO);
    for(size_t i=0;i<szA;i++) hA[i]=(int8_t)((i*7+3)%251-125);
    for(size_t i=0;i<szW;i++) hW[i]=(int8_t)((i*5+1)%251-125);

    int8_t *dA,*dW; int *dO; CK(cudaMalloc(&dA,szA)); CK(cudaMalloc(&dW,szW)); CK(cudaMalloc(&dO,szO*4));
    CK(cudaMemcpy(dA,hA.data(),szA,cudaMemcpyHostToDevice));
    CK(cudaMemcpy(dW,hW.data(),szW,cudaMemcpyHostToDevice));

    cudaEvent_t t0,t1; CK(cudaEventCreate(&t0)); CK(cudaEventCreate(&t1));
    double gops = 2.0*M*N*K/1e9;
    const int ITERS=20;

    // ---- our DP4A kernel ----
    dim3 blk(16,16), grd((N+15)/16,(M+15)/16);
    dp4a_gemm<<<grd,blk>>>(dO,dA,dW,M,K,N); CK(cudaDeviceSynchronize());
    CK(cudaEventRecord(t0)); for(int it=0;it<ITERS;it++) dp4a_gemm<<<grd,blk>>>(dO,dA,dW,M,K,N); CK(cudaEventRecord(t1)); CK(cudaEventSynchronize(t1));
    float ms_ours; cudaEventElapsedTime(&ms_ours,t0,t1); ms_ours/=ITERS;
    std::vector<int> hOurs(szO); CK(cudaMemcpy(hOurs.data(),dO,szO*4,cudaMemcpyDeviceToHost));

    // ---- cuBLAS int8 ----  O_row(M x N) = A_row(M x K) . W_row(N x K)^T  ==>  col-major: OP_T on W, OP_N on A
    cublasHandle_t h; CB(cublasCreate(&h));
    int alpha=1, beta=0;
    CB(cublasGemmEx(h, CUBLAS_OP_T, CUBLAS_OP_N, N, M, K, &alpha,
        dW, CUDA_R_8I, K, dA, CUDA_R_8I, K, &beta, dO, CUDA_R_32I, N,
        CUBLAS_COMPUTE_32I, CUBLAS_GEMM_DEFAULT)); CK(cudaDeviceSynchronize());
    CK(cudaEventRecord(t0)); for(int it=0;it<ITERS;it++) CB(cublasGemmEx(h,CUBLAS_OP_T,CUBLAS_OP_N,N,M,K,&alpha,dW,CUDA_R_8I,K,dA,CUDA_R_8I,K,&beta,dO,CUDA_R_32I,N,CUBLAS_COMPUTE_32I,CUBLAS_GEMM_DEFAULT)); CK(cudaEventRecord(t1)); CK(cudaEventSynchronize(t1));
    float ms_cub; cudaEventElapsedTime(&ms_cub,t0,t1); ms_cub/=ITERS;
    std::vector<int> hCub(szO); CK(cudaMemcpy(hCub.data(),dO,szO*4,cudaMemcpyDeviceToHost));

    // ---- verify both against a CPU reference on a few output cells ----
    int mism_ours=0, mism_cub=0;
    for(int s=0;s<64;s++){ int i=(s*37)%M, j=(s*53)%N; long ref=0; for(int t=0;t<K;t++) ref+=(int)hA[(long)i*K+t]*(int)hW[(long)j*K+t];
        if(hOurs[(long)i*N+j]!=ref) mism_ours++; if(hCub[(long)i*N+j]!=ref) mism_cub++; }

    printf("our DP4A kernel : %7.3f ms  %8.1f GOPS   %s\n", ms_ours, gops/(ms_ours/1e3), mism_ours?"WRONG":"correct");
    printf("cuBLAS int8     : %7.3f ms  %8.1f GOPS   %s\n", ms_cub,  gops/(ms_cub/1e3),  mism_cub?"WRONG":"correct");
    printf("speedup (cuBLAS / ours): %.1fx\n", ms_ours/ms_cub);
    return 0;
}
