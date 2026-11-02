// What one dependent stage costs a GPU beyond its own work: the launch-to-completion overhead of one node of a
// CUDA graph whose nodes run one after another, as a batch-1 decode's kernels do.
//
// A graph of N empty kernels captured on one stream is a chain of N dependent nodes. Its replay time is
// a + b·N: `a` is the graph launch, `b` what each further dependent node adds. The least-squares slope over
// N = 1 … 4096 is `b`, the per-stage overhead the decode model charges; the same fit over plain stream
// launches is printed beside it for reference. Every time is the median of REPS replays after WARM.
//
//   nvcc -O2 -o graph_node_overhead scripts/accel/graph_node_overhead.cu && ./graph_node_overhead
#include <cstdio>
#include <vector>
#include <algorithm>
#include <cuda_runtime.h>

#define CK(x) do { cudaError_t e = (x); if (e != cudaSuccess) { std::fprintf(stderr, "%s: %s\n", #x, cudaGetErrorString(e)); return 1; } } while (0)

__global__ void empty_stage(int* sink) {
    if (sink != nullptr && threadIdx.x == 0) sink[blockIdx.x] = 1;
}

static const int REPS = 51;
static const int WARM = 5;

int main() {
    cudaDeviceProp prop;
    CK(cudaGetDeviceProperties(&prop, 0));
    cudaStream_t s;
    CK(cudaStreamCreateWithFlags(&s, cudaStreamNonBlocking));
    cudaEvent_t t0, t1;
    CK(cudaEventCreate(&t0));
    CK(cudaEventCreate(&t1));
    const int ns[] = {1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096};
    std::vector<double> xs, graph_ms, stream_ms;
    for (int n : ns) {
        cudaGraph_t g;
        cudaGraphExec_t ge;
        CK(cudaStreamBeginCapture(s, cudaStreamCaptureModeGlobal));
        for (int i = 0; i < n; ++i) empty_stage<<<1, 32, 0, s>>>(nullptr);
        CK(cudaStreamEndCapture(s, &g));
        CK(cudaGraphInstantiate(&ge, g, 0));
        std::vector<float> gt, st;
        for (int r = 0; r < WARM + REPS; ++r) {
            float ms = 0;
            CK(cudaEventRecord(t0, s));
            CK(cudaGraphLaunch(ge, s));
            CK(cudaEventRecord(t1, s));
            CK(cudaEventSynchronize(t1));
            CK(cudaEventElapsedTime(&ms, t0, t1));
            if (r >= WARM) gt.push_back(ms);
            CK(cudaEventRecord(t0, s));
            for (int i = 0; i < n; ++i) empty_stage<<<1, 32, 0, s>>>(nullptr);
            CK(cudaEventRecord(t1, s));
            CK(cudaEventSynchronize(t1));
            CK(cudaEventElapsedTime(&ms, t0, t1));
            if (r >= WARM) st.push_back(ms);
        }
        std::sort(gt.begin(), gt.end());
        std::sort(st.begin(), st.end());
        xs.push_back(n);
        graph_ms.push_back(gt[gt.size() / 2]);
        stream_ms.push_back(st[st.size() / 2]);
        CK(cudaGraphExecDestroy(ge));
        CK(cudaGraphDestroy(g));
    }
    auto fit = [&](const std::vector<double>& ys, double& slope, double& icept) {
        double n = xs.size(), sx = 0, sy = 0, sxx = 0, sxy = 0;
        for (size_t i = 0; i < xs.size(); ++i) { sx += xs[i]; sy += ys[i]; sxx += xs[i] * xs[i]; sxy += xs[i] * ys[i]; }
        slope = (n * sxy - sx * sy) / (n * sxx - sx * sx);
        icept = (sy - slope * sx) / n;
    };
    double gb, ga, sb, sa;
    fit(graph_ms, gb, ga);
    fit(stream_ms, sb, sa);
    std::printf("{\n  \"device\": \"%s\",\n  \"sm\": \"%d.%d\",\n  \"reps\": %d,\n  \"rows\": [\n", prop.name, prop.major, prop.minor, REPS);
    for (size_t i = 0; i < xs.size(); ++i)
        std::printf("    {\"nodes\": %d, \"graph_median_us\": %.3f, \"stream_median_us\": %.3f}%s\n", (int)xs[i], graph_ms[i] * 1e3, stream_ms[i] * 1e3, i + 1 < xs.size() ? "," : "");
    std::printf("  ],\n  \"graph_node_us\": %.4f,\n  \"graph_launch_us\": %.3f,\n  \"stream_launch_us\": %.4f,\n  \"stream_fixed_us\": %.3f\n}\n", gb * 1e3, ga * 1e3, sb * 1e3, sa * 1e3);
    return 0;
}
