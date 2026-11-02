// The GPU's streaming read ceiling: a 4 GiB buffer, far past any L2, read once through with 16-byte loads by every
// launch shape below (blocks a multiprocessor by threads a block), the fastest of ten timed runs of each after two
// warm-ups. Prints each shape's rate and `READ_GB_S <the fastest>`, in GB/s (10⁹ bytes a second). A decode step
// that streams its weights at this rate is at the card's memory wall.
//
//   nvcc -O3 -arch=sm_86 -o read_ceiling scripts/accel/read_ceiling.cu && ./read_ceiling
#include <cstdio>
#include <cstdlib>
#include <cuda_runtime.h>

#define CK(x) do { cudaError_t e_ = (x); if(e_ != cudaSuccess){ fprintf(stderr, "cuda %d %s\n", __LINE__, cudaGetErrorString(e_)); exit(1); } } while(0)

// Every 16-byte word of `p` read once, four in flight a thread; the folded words never match the sentinel, so the
// loads stay and nothing is written.
__global__ void read_all(const int4* __restrict__ p, size_t n, int* out){
  int acc = 0;
  size_t stride = (size_t)gridDim.x * blockDim.x, i = (size_t)blockIdx.x * blockDim.x + threadIdx.x;
  for(; i + 3 * stride < n; i += 4 * stride){
    int4 v[4];
    #pragma unroll
    for(int u = 0; u < 4; ++u) v[u] = __ldg(p + i + u * stride);
    #pragma unroll
    for(int u = 0; u < 4; ++u) acc ^= v[u].x ^ v[u].y ^ v[u].z ^ v[u].w;
  }
  for(; i < n; i += stride){ int4 v = __ldg(p + i); acc ^= v.x ^ v.y ^ v.z ^ v.w; }
  if(acc == 0x12345678) out[0] = acc;
}

int main(){
  const size_t bytes = (size_t)4 << 30, n = bytes / 16;
  int4* p; int* o;
  CK(cudaMalloc(&p, bytes)); CK(cudaMalloc(&o, 4));
  CK(cudaMemset(p, 1, bytes));
  int sms; CK(cudaDeviceGetAttribute(&sms, cudaDevAttrMultiProcessorCount, 0));
  cudaEvent_t a, b; CK(cudaEventCreate(&a)); CK(cudaEventCreate(&b));
  double best = 0;
  for(int threads : {256, 512, 1024})
    for(int per : {1, 2, 4, 8, 16, 32}){
      int blocks = sms * per;
      for(int w = 0; w < 2; ++w) read_all<<<blocks, threads>>>(p, n, o);
      CK(cudaGetLastError());
      float fastest = 1e30f;
      for(int r = 0; r < 10; ++r){
        CK(cudaEventRecord(a)); read_all<<<blocks, threads>>>(p, n, o); CK(cudaEventRecord(b)); CK(cudaEventSynchronize(b));
        float ms; CK(cudaEventElapsedTime(&ms, a, b)); if(ms < fastest) fastest = ms;
      }
      double gbs = bytes / (fastest * 1e-3) / 1e9;
      if(gbs > best) best = gbs;
      printf("SHAPE %d blocks of %d threads: %.1f GB/s\n", blocks, threads, gbs);
    }
  printf("READ_GB_S %.2f\n", best);
  return 0;
}
