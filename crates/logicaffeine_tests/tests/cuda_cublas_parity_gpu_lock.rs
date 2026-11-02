//! cuda_cublas_parity_gpu_lock — the DEFINITIVE, noise-free parity metric: our compiler's champion int8 GEMM
//! kernel and cuBLAS run in the SAME process, interleaved, best-of-100 (which rejects clock/thermal throttle),
//! on the exact same 2048^3 data. Reports OUR GOPS / cuBLAS GOPS. This is how we honestly track the last mile
//! to 100% parity — cross-run variance (~10% on this low-occupancy kernel) is factored out entirely.
//!
//! Kernel comes from the COMPILER (emit_mma_tn_k64_kernel), so the lock tracks the real codegen, not a copy.
//! Requires nvcc + cuBLAS + a GPU.
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::emit_mma_tn_k64_kernel;
use std::process::Command;

fn have() -> bool {
    Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        && std::path::Path::new("/dev/nvidia0").exists()
}

// Host harness: correctness (varied-fill oracle spot-check) then interleaved best-of-100 vs cuBLAS TN int8.
const HARNESS: &str = r#"
#include <cublas_v2.h>
#include <cstdio>
#include <cstdlib>
static double best(float* ms,int n){double b=1e30;for(int i=0;i<n;i++)if(ms[i]<b)b=ms[i];return b;}
int main(){
  const int M=2048,K=2048,N=2048;
  signed char *dA,*dB; int *dC;
  cudaMalloc(&dA,(size_t)M*K);cudaMalloc(&dB,(size_t)K*N);cudaMalloc(&dC,(size_t)M*N*4);
  // correctness: varied fill (B col-major) vs the scalar oracle — a race would fail here.
  signed char *hA=(signed char*)malloc((size_t)M*K),*hB=(signed char*)malloc((size_t)K*N);
  for(long i=0;i<(long)M*K;++i)hA[i]=(signed char)(((i*7+3)%251)-125);
  for(long nn=0;nn<N;++nn)for(long kk=0;kk<K;++kk){long bkn=(long)kk*N+nn;hB[nn*K+kk]=(signed char)(((bkn*5+1)%251)-125);}
  cudaMemcpy(dA,hA,(size_t)M*K,cudaMemcpyHostToDevice);cudaMemcpy(dB,hB,(size_t)K*N,cudaMemcpyHostToDevice);
  dim3 g(N/128,M/128); int NTH=128;
  gemm_mma_tn_k64<<<g,NTH>>>(dA,dB,dC,M,N,K); cudaDeviceSynchronize();
  int *hC=(int*)malloc((size_t)M*N*4); cudaMemcpy(hC,dC,(size_t)M*N*4,cudaMemcpyDeviceToHost);
  for(long q=0;q<10;q++){long i=(q*7919)%M,j=(q*104729)%N;long acc=0;for(long kk=0;kk<K;kk++){long av=((i*K+kk)*7+3)%251-125;long bv=((kk*N+j)*5+1)%251-125;acc+=av*bv;}if(hC[i*N+j]!=acc){printf("INCORRECT\n");return 1;}}
  cudaMemset(dA,1,(size_t)M*K);cudaMemset(dB,1,(size_t)K*N);
  cublasHandle_t h;cublasCreate(&h);cublasSetMathMode(h,CUBLAS_TENSOR_OP_MATH);int al=1,be=0;
  cudaEvent_t s,e;cudaEventCreate(&s);cudaEventCreate(&e);
  int IT=100; float mo[100],mc[100];
  for(int i=0;i<10;i++){gemm_mma_tn_k64<<<g,NTH>>>(dA,dB,dC,M,N,K);cublasGemmEx(h,CUBLAS_OP_T,CUBLAS_OP_N,N,M,K,&al,dB,CUDA_R_8I,K,dA,CUDA_R_8I,K,&be,dC,CUDA_R_32I,N,CUBLAS_COMPUTE_32I,CUBLAS_GEMM_DEFAULT_TENSOR_OP);}
  cudaDeviceSynchronize();
  for(int i=0;i<IT;i++){
    cudaEventRecord(s);gemm_mma_tn_k64<<<g,NTH>>>(dA,dB,dC,M,N,K);cudaEventRecord(e);cudaEventSynchronize(e);cudaEventElapsedTime(&mo[i],s,e);
    cudaEventRecord(s);cublasGemmEx(h,CUBLAS_OP_T,CUBLAS_OP_N,N,M,K,&al,dB,CUDA_R_8I,K,dA,CUDA_R_8I,K,&be,dC,CUDA_R_32I,N,CUBLAS_COMPUTE_32I,CUBLAS_GEMM_DEFAULT_TENSOR_OP);cudaEventRecord(e);cudaEventSynchronize(e);cudaEventElapsedTime(&mc[i],s,e);
  }
  double go=2.0*M*N*K/(best(mo,IT)/1e3)/1e9, gc=2.0*M*N*K/(best(mc,IT)/1e3)/1e9;
  printf("OURS %.0f\nCUBLAS %.0f\nPARITY %.1f\n", go, gc, 100.0*go/gc);
  return 0;
}
"#;

#[test]
fn our_gemm_holds_cublas_parity_head_to_head() {
    if !have() {
        eprintln!("SKIP: needs nvcc + cuBLAS + a GPU");
        return;
    }
    let src = format!("{}{}", emit_mma_tn_k64_kernel(128, 128, 2, 2, 3), HARNESS);
    let dir = std::env::temp_dir().join(format!("logos_parity_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let s = dir.join("p.cu");
    let b = dir.join("p");
    std::fs::write(&s, &src).expect("write");
    let c = Command::new("nvcc")
        .args(["-O3", "-arch=sm_86", "-o"])
        .arg(&b)
        .arg(&s)
        .arg("-lcublas")
        .output()
        .expect("nvcc");
    assert!(
        c.status.success(),
        "compile:\n{}",
        String::from_utf8_lossy(&c.stderr)
    );
    let r = Command::new(&b).output().expect("run");
    let out = String::from_utf8_lossy(&r.stdout);
    assert!(
        !out.contains("INCORRECT"),
        "our kernel must be numerically correct vs the oracle"
    );
    let get = |k: &str| {
        out.lines()
            .find_map(|l| l.strip_prefix(k).and_then(|v| v.trim().parse::<f64>().ok()))
    };
    let ours = get("OURS ").expect("OURS");
    let cublas = get("CUBLAS ").expect("CUBLAS");
    let parity = get("PARITY ").expect("PARITY");
    eprintln!("HEAD-TO-HEAD (best-of-100, same process): OURS {ours:.0} GOPS | cuBLAS {cublas:.0} GOPS | PARITY {parity:.1}%");
    let _ = std::fs::remove_dir_all(&dir);

    // Our from-scratch, compiler-emitted kernel must hold high parity with cuBLAS's hand-tuned int8 GEMM.
    // Floor is set below the measured ~91% to absorb driver/GPU differences, and RATCHETS up as we close in.
    assert!(
        parity >= 85.0,
        "cuBLAS parity ({parity:.1}%) dropped below the 85% floor — a regression"
    );
}
