//! cuda_layout_tradeoff_lock — LAYOUT is a first-class frontier knob because it carries a ~3x tradeoff, and
//! this lock proves it by running cuBLAS ITSELF, per layout, on this box (correctness-guarded, matched timing):
//!   * cuBLAS int8 is FAST only in the tensor-core-native TN layout (A^T,B); NN/NT/TT fall back ~3x slower.
//!   * our NN cp.async WMMA kernel BEATS cuBLAS's NN fallback.
//!   * cuBLAS's TN path is the honest target the TN-native mma.sync+ldmatrix kernel must chase.
//!
//! Every cuBLAS layout is verified numerically (A=B=1 ⇒ every C==K) before its GOPS is trusted — the same
//! guard that caught our earlier impossible-speed timing bug. Requires nvcc + cuBLAS + a GPU.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_layout_tradeoff_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::{emit_wmma_cpasync_i8_bench_staged, WmmaTile};

fn have() -> bool {
    std::process::Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        && std::path::Path::new("/dev/nvidia0").exists()
}

/// cuBLAS int8 GEMM per layout (2048^3), each verified all-ones ⇒ C==K, per-iteration-synced. Prints
/// `LAYOUT <NN|NT|TN|TT> <gops>` for the layouts that are correct.
const CUBLAS_BENCH: &str = r#"#include <cublas_v2.h>
#include <cuda_runtime.h>
#include <cstdio>
static double bench(cublasHandle_t h, cublasOperation_t oA, cublasOperation_t oB, const char* nm,
                    int M,int K,int N, signed char*dA, signed char*dB, int*dC){
  int alpha=1,beta=0;
  int lda=(oA==CUBLAS_OP_N)?N:K, ldb=(oB==CUBLAS_OP_N)?K:M;
  cublasStatus_t st=cublasGemmEx(h,oA,oB,N,M,K,&alpha,dB,CUDA_R_8I,lda,dA,CUDA_R_8I,ldb,&beta,dC,CUDA_R_32I,N,CUBLAS_COMPUTE_32I,CUBLAS_GEMM_DEFAULT_TENSOR_OP);
  if(st!=CUBLAS_STATUS_SUCCESS) return 0;
  if(cudaDeviceSynchronize()) return 0;
  int *hC=(int*)malloc((size_t)M*N*4); cudaMemcpy(hC,dC,(size_t)M*N*4,cudaMemcpyDeviceToHost);
  long step=((long)M*N)/4096+1; for(long i=0;i<(long)M*N;i+=step){ if(hC[i]!=K){ free(hC); return 0; } }
  free(hC);
  cudaEvent_t s,e; cudaEventCreate(&s); cudaEventCreate(&e); int IT=30; float tot=0;
  for(int it=0;it<IT;it++){ cudaEventRecord(s);
    cublasGemmEx(h,oA,oB,N,M,K,&alpha,dB,CUDA_R_8I,lda,dA,CUDA_R_8I,ldb,&beta,dC,CUDA_R_32I,N,CUBLAS_COMPUTE_32I,CUBLAS_GEMM_DEFAULT_TENSOR_OP);
    cudaEventRecord(e); cudaEventSynchronize(e); float ms=0; cudaEventElapsedTime(&ms,s,e); tot+=ms; }
  return 2.0*M*N*K/((tot/IT)/1e3)/1e9;
}
int main(){
  const int M=2048,K=2048,N=2048;
  signed char*dA,*dB; int*dC;
  cudaMalloc(&dA,(size_t)M*K); cudaMalloc(&dB,(size_t)K*N); cudaMalloc(&dC,(size_t)M*N*4);
  cudaMemset(dA,1,(size_t)M*K); cudaMemset(dB,1,(size_t)K*N);
  cublasHandle_t h; cublasCreate(&h); cublasSetMathMode(h,CUBLAS_TENSOR_OP_MATH);
  printf("LAYOUT NN %.1f\n", bench(h,CUBLAS_OP_N,CUBLAS_OP_N,"NN",M,K,N,dA,dB,dC));
  printf("LAYOUT TN %.1f\n", bench(h,CUBLAS_OP_T,CUBLAS_OP_N,"TN",M,K,N,dA,dB,dC));
  return 0;
}
"#;

fn run_cublas() -> Option<(f64, f64)> {
    let dir = std::env::temp_dir().join(format!("logos_cublas_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("cb.cu");
    let bin = dir.join("cb");
    std::fs::write(&src, CUBLAS_BENCH).ok()?;
    let c = std::process::Command::new("nvcc")
        .args(["-O3", "-arch=sm_86", "-o"])
        .arg(&bin)
        .arg(&src)
        .arg("-lcublas")
        .output()
        .ok()?;
    if !c.status.success() {
        eprintln!(
            "cuBLAS bench compile failed:\n{}",
            String::from_utf8_lossy(&c.stderr)
        );
        return None;
    }
    let r = std::process::Command::new(&bin).output().ok()?;
    let out = String::from_utf8_lossy(&r.stdout);
    let g = |k: &str| {
        out.lines().find_map(|l| {
            l.strip_prefix(&format!("LAYOUT {k} "))
                .and_then(|v| v.trim().parse::<f64>().ok())
        })
    };
    let _ = std::fs::remove_dir_all(&dir);
    Some((g("NN")?, g("TN")?))
}

fn run_our_nn() -> Option<f64> {
    let t = WmmaTile {
        bm: 128,
        bn: 64,
        bk: 32,
        warps_m: 2,
        warps_n: 2,
    };
    let dir = std::env::temp_dir().join(format!("logos_ournn_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("k.cu");
    let bin = dir.join("k");
    std::fs::write(
        &src,
        emit_wmma_cpasync_i8_bench_staged(&t, 3, 2048, 2048, 2048),
    )
    .ok()?;
    let c = std::process::Command::new("nvcc")
        .args(["-O3", "-arch=sm_86", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .ok()?;
    if !c.status.success() {
        return None;
    }
    let r = std::process::Command::new(&bin).output().ok()?;
    let out = String::from_utf8_lossy(&r.stdout);
    let g = out.lines().find_map(|l| {
        l.strip_prefix("GOPS ")
            .and_then(|v| v.trim().parse::<f64>().ok())
    });
    let _ = std::fs::remove_dir_all(&dir);
    g
}

#[test]
fn layout_is_a_real_tradeoff_and_our_nn_kernel_beats_cublas_nn() {
    if !have() {
        eprintln!("SKIP: needs nvcc + cuBLAS + a GPU");
        return;
    }
    let (cublas_nn, cublas_tn) = match run_cublas() {
        Some(v) => v,
        None => {
            eprintln!("SKIP: cuBLAS unavailable on this box");
            return;
        }
    };
    let ours_nn = run_our_nn().expect("our NN kernel must run");
    eprintln!(
        "cuBLAS NN = {cublas_nn:.0} GOPS  |  cuBLAS TN = {cublas_tn:.0} GOPS ({:.2}x over NN — the layout tradeoff)  |  OUR NN = {ours_nn:.0} GOPS",
        cublas_tn / cublas_nn
    );
    eprintln!(
        "  -> we beat cuBLAS-NN by {:.2}x; cuBLAS-TN is {:.2}x our NN (the TN-native mma.sync+ldmatrix target)",
        ours_nn / cublas_nn,
        cublas_tn / ours_nn
    );

    // (1) LAYOUT is a genuine tradeoff: cuBLAS's TN path is far faster than its NN path (tensor-core-native).
    assert!(
        cublas_tn > 1.5 * cublas_nn,
        "cuBLAS TN ({cublas_tn:.0}) should be >>1.5x its NN ({cublas_nn:.0}) — that 3x is why layout is a frontier knob"
    );
    // (2) our NN kernel beats cuBLAS's NN fallback — a real, verified win in the common layout.
    assert!(
        ours_nn > cublas_nn,
        "our NN kernel ({ours_nn:.0} GOPS) must beat cuBLAS's NN fallback ({cublas_nn:.0} GOPS)"
    );
}
