// Megakernel thesis probe: does fusing a long chain of dependent decode-like kernels into ONE cooperative-groups
// kernel (grid.sync() between stages) beat a CUDA graph of the same kernels? The decode step is ~15 kernels/layer
// x 48 layers = ~700 dependent nodes; the hypothesis is that per-node dispatch/drain overhead is a real slice of
// batch-1 time that a single cooperative launch removes. This tests it WITHOUT the 30B weights, on any GPU — the
// dispatch vs grid.sync overheads transfer qualitatively. Mirrors the decode's mix: a few big global-weight GEMVs
// per layer (the bandwidth-bound bulk) + many tiny elementwise ops (where dispatch overhead dominates).
//
// Build: nvcc -O3 -arch=sm_86 megakernel_probe.cu -o /tmp/mk && /tmp/mk
#include <cstdio>
#include <cstdlib>
#include <cmath>
#include <vector>
#include <cuda_runtime.h>
#include <cooperative_groups.h>
namespace cg = cooperative_groups;

#define E     2048          // hidden
#define LAYERS 48
#define BIGK  2048          // a "big" GEMV inner dim (mimics a weight read)
#define BIGN  768           // big GEMV outputs (mimics MoE gate width)
#define TINY_OPS 10         // tiny elementwise ops per layer (norms, rope, qacts, adds, router, topk...)

static void ck(cudaError_t e,const char* w){ if(e!=cudaSuccess){ fprintf(stderr,"CUDA %s: %s\n",w,cudaGetErrorString(e)); exit(1);} }

// ── the two primitive stages ──
// tiny: elementwise over E (one block, mimics a norm/qact/add — dispatch-dominated)
__global__ void k_tiny(float* x, const float* a, int n, float s){
  for(int i=blockIdx.x*blockDim.x+threadIdx.x; i<n; i+=blockDim.x*gridDim.x) x[i] = x[i]*s + a[i]*0.999f;
}
// big: a real GEMV reading BIGN*BIGK weights from global (bandwidth-bound bulk)
__global__ void k_big(float* y, const float* w, const float* x, int K, int N){
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31;
  const float* wr=w+(long)o*K; float acc=0.f;
  for(int j=lane;j<K;j+=32) acc+=wr[j]*x[j];
  for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s); if(lane==0) y[o]=acc;
}

// ── the SAME work as ONE cooperative megakernel: grid-stride each stage, grid.sync() between ──
__global__ void k_mega(float* x, float* y, const float* a, const float* w, int layers){
  cg::grid_group grid = cg::this_grid();
  int tid=blockIdx.x*blockDim.x+threadIdx.x, nth=blockDim.x*gridDim.x;
  for(int l=0;l<layers;l++){
    // TINY_OPS elementwise stages over E
    for(int t=0;t<TINY_OPS;t++){
      for(int i=tid;i<E;i+=nth) x[i]=x[i]*1.0001f + a[i]*0.999f;
      grid.sync();
    }
    // one big GEMV: warp-per-output over BIGN, reading BIGN*BIGK weights
    int warp=tid>>5, lane=tid&31, nwarp=nth>>5;
    for(int o=warp;o<BIGN;o+=nwarp){ const float* wr=w+(long)o*BIGK; float acc=0.f;
      for(int j=lane;j<BIGK;j+=32) acc+=wr[j]*x[j%E];
      for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s); if(lane==0) y[o]=acc; }
    grid.sync();
    for(int i=tid;i<E;i+=nth) x[i]=x[i]+y[i%BIGN]*0.001f;   // residual-ish
    grid.sync();
  }
}

int main(){
  float *x,*y,*a,*w;
  ck(cudaMalloc(&x,E*4),"x"); ck(cudaMalloc(&y,BIGN*4),"y"); ck(cudaMalloc(&a,E*4),"a");
  ck(cudaMalloc(&w,(long)BIGN*BIGK*4),"w");
  ck(cudaMemset(x,0,E*4),"mx"); ck(cudaMemset(a,0,E*4),"ma"); ck(cudaMemset(w,0,(long)BIGN*BIGK*4),"mw");
  cudaEvent_t t0,t1; cudaEventCreate(&t0); cudaEventCreate(&t1); float ms; const int REP=200, WARM=30;

  // ── A) graph of separate kernels (mirrors our decode: many nodes in a CUDA graph) ──
  cudaStream_t s; cudaStreamCreate(&s);
  auto build=[&](cudaStream_t st){
    for(int l=0;l<LAYERS;l++){
      for(int t=0;t<TINY_OPS;t++) k_tiny<<<8,256,0,st>>>(x,a,E,1.0001f);
      k_big<<<(BIGN+7)/8,256,0,st>>>(y,w,x,BIGK,BIGN);
      k_tiny<<<8,256,0,st>>>(x,a,E,1.0f);
    }
  };
  cudaGraph_t g; cudaGraphExec_t ge;
  cudaStreamBeginCapture(s,cudaStreamCaptureModeGlobal); build(s); cudaStreamEndCapture(s,&g); cudaGraphInstantiate(&ge,g,0);
  for(int i=0;i<WARM;i++) cudaGraphLaunch(ge,s); cudaStreamSynchronize(s);
  cudaEventRecord(t0,s); for(int i=0;i<REP;i++) cudaGraphLaunch(ge,s); cudaEventRecord(t1,s); cudaStreamSynchronize(s);
  cudaEventElapsedTime(&ms,t0,t1); double graph_us=ms*1e3/REP;
  int nodes=LAYERS*(TINY_OPS+2);
  printf("GRAPH  (%d nodes/step): %8.1f us/step\n", nodes, graph_us);

  // ── B) one cooperative megakernel (grid.sync between the same stages) ──
  int dev; cudaGetDevice(&dev); int sm; cudaDeviceGetAttribute(&sm,cudaDevAttrMultiProcessorCount,dev);
  int blocksPerSm=0; cudaOccupancyMaxActiveBlocksPerMultiprocessor(&blocksPerSm,(void*)k_mega,256,0);
  int grid=blocksPerSm*sm; if(grid<1) grid=sm;
  int coop=0; cudaDeviceGetAttribute(&coop,cudaDevAttrCooperativeLaunch,dev);
  printf("SMs=%d blocks/SM=%d coop-grid=%d coopSupported=%d\n", sm, blocksPerSm, grid, coop);
  int L=LAYERS; void* args[]={&x,&y,&a,&w,&L};
  auto launchMega=[&](cudaStream_t st){ ck(cudaLaunchCooperativeKernel((void*)k_mega, grid, 256, args, 0, st),"coop"); };
  for(int i=0;i<WARM;i++) launchMega(s); cudaStreamSynchronize(s);
  cudaEventRecord(t0,s); for(int i=0;i<REP;i++) launchMega(s); cudaEventRecord(t1,s); cudaStreamSynchronize(s);
  cudaEventElapsedTime(&ms,t0,t1); double mega_us=ms*1e3/REP;
  printf("MEGA   (1 launch, %d grid.syncs): %8.1f us/step\n", nodes, mega_us);
  printf("\nspeedup (graph/mega) = %.3fx  -> %s\n", graph_us/mega_us,
    mega_us<graph_us*0.97 ? "MEGAKERNEL WINS (dispatch overhead is real & removable)" :
    (mega_us>graph_us*1.03 ? "MEGAKERNEL LOSES (grid.sync costs more than dispatch)" : "TIE (dispatch overhead is negligible under graphs)"));
  return 0;
}
