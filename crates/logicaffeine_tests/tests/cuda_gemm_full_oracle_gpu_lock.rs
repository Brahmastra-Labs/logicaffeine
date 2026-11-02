//! The flagship int8 GEMM is numerically correct on EVERY element, EVERY run.
//!
//! The existing GPU locks spot-check 10–16 sampled elements once; a pipeline race that corrupts a
//! few tiles a few percent of the time sails through them ~95% of the time (measured: 2 INCORRECT in
//! 40 runs of `emit_mma_tn_k64_kernel(128,128,2,2,3)` under the varied-fill oracle). This lock
//! compares the FULL 2048³ result matrix against a host int64 reference and repeats the launch 300
//! times (the race sits in the LAST k-stage of a tile and lands only occasionally), so a corrupted tile cannot hide behind sampling or luck. GPU-gated (nvcc + /dev/nvidia0),
//! in the serialized `cuda_*` group. No retries — a single mismatch is a real kernel bug.
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::emit_mma_tn_k64_kernel;
use std::path::Path;
use std::process::Command;

fn have() -> bool {
    Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        && Path::new("/dev/nvidia0").exists()
}

/// Driver-API runner: host int64 reference computed once; the kernel is launched `RUNS` times and every
/// element of every result is compared. Prints `MISMATCH run=<r> i=<i> j=<j> got=<g> exp=<e>` on the
/// first bad element of a run, then `RUNS_BAD <n>` / `RUNS_OK <n>` at the end.
const RUNNER: &str = r#"#include <cuda.h>
#include <cstdio>
#include <cstdlib>
#include <cstdint>
static void ck(CUresult r,const char*w){ if(r!=CUDA_SUCCESS){ const char*s; cuGetErrorString(r,&s); printf("ERR %s: %s\n",w,s); exit(2);} }
int main(int argc,char**argv){
  const int M=2048,K=2048,N=2048; const int RUNS = argc > 2 ? atoi(argv[2]) : 300;
  ck(cuInit(0),"init"); CUdevice d; ck(cuDeviceGet(&d,0),"dev"); CUcontext c; ck(cuCtxCreate(&c,0,d),"ctx");
  CUmodule mod; ck(cuModuleLoad(&mod,argv[1]),"load"); CUfunction f; ck(cuModuleGetFunction(&f,mod,"gemm_mma_tn_k64"),"fn");
  CUdeviceptr dA,dB,dC; ck(cuMemAlloc(&dA,(size_t)M*K),"a"); ck(cuMemAlloc(&dB,(size_t)K*N),"b"); ck(cuMemAlloc(&dC,(size_t)M*N*4),"c");
  signed char*hA=(signed char*)malloc((size_t)M*K),*hB=(signed char*)malloc((size_t)K*N);
  for(long i=0;i<(long)M*K;++i)hA[i]=(signed char)(((i*7+3)%251)-125);
  for(long nn=0;nn<N;++nn)for(long kk=0;kk<K;++kk){long bkn=(long)kk*N+nn;hB[nn*K+kk]=(signed char)(((bkn*5+1)%251)-125);}
  ck(cuMemcpyHtoD(dA,hA,(size_t)M*K),"h2d a"); ck(cuMemcpyHtoD(dB,hB,(size_t)K*N),"h2d b");
  // Full host reference (B is col-major: hB[n*K+k]).
  int64_t*ref=(int64_t*)malloc((size_t)M*N*8);
  for(long i=0;i<M;i++){for(long j=0;j<N;j++){int64_t acc=0; const signed char*a=hA+i*K; const signed char*b=hB+j*K; for(long kk=0;kk<K;kk++)acc+=(int64_t)a[kk]*(int64_t)b[kk]; ref[i*N+j]=acc;}}
  int m=M,n=N,k=K; void*args[]={&dA,&dB,&dC,&m,&n,&k}; unsigned gx=N/128,gy=M/128;
  int*hC=(int*)malloc((size_t)M*N*4); int bad=0;
  for(int r=0;r<RUNS;r++){
    ck(cuMemsetD8(dC,0,(size_t)M*N*4),"memset");
    ck(cuLaunchKernel(f,gx,gy,1,128,1,1,0,0,args,0),"launch"); ck(cuCtxSynchronize(),"sync");
    ck(cuMemcpyDtoH(hC,dC,(size_t)M*N*4),"d2h");
    long first=-1; for(long q=0;q<(long)M*N;q++){ if((int64_t)hC[q]!=ref[q]){ first=q; break; } }
    if(first>=0){ bad++; printf("MISMATCH run=%d i=%ld j=%ld got=%d exp=%lld\n", r, first/N, first%N, hC[first], (long long)ref[first]); }
  }
  printf("RUNS_BAD %d\nRUNS_OK %d\n", bad, RUNS-bad);
  return 0;
}
"#;

#[test]
fn the_flagship_int8_gemm_is_correct_on_every_element_across_repeated_runs() {
    if !have() {
        eprintln!("SKIP: needs nvcc + a GPU");
        return;
    }
    let dir = std::env::temp_dir().join(format!("logos_gemm_full_oracle_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let runner_src = dir.join("run.cu");
    let runner = dir.join("run");
    std::fs::write(&runner_src, RUNNER).expect("write runner");
    let o = Command::new("nvcc")
        .args(["-O3", "-o"])
        .arg(&runner)
        .arg(&runner_src)
        .arg("-lcuda")
        .output()
        .expect("nvcc runner");
    assert!(
        o.status.success(),
        "runner build:\n{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let ksrc = dir.join("k.cu");
    let cubin = dir.join("k.cubin");
    std::fs::write(&ksrc, emit_mma_tn_k64_kernel(128, 128, 2, 2, 3)).expect("write kernel");
    let o = Command::new("nvcc")
        .args(["-O3", "-arch=sm_86", "-cubin", "-o"])
        .arg(&cubin)
        .arg(&ksrc)
        .output()
        .expect("nvcc cubin");
    assert!(
        o.status.success(),
        "kernel build:\n{}",
        String::from_utf8_lossy(&o.stderr)
    );

    let o = Command::new(&runner)
        .arg(&cubin)
        .arg("300")
        .output()
        .expect("run");
    let out = String::from_utf8_lossy(&o.stdout);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!out.contains("ERR "), "runner error:\n{out}");
    let bad: usize = out
        .lines()
        .find_map(|l| {
            l.strip_prefix("RUNS_BAD ")
                .and_then(|v| v.trim().parse().ok())
        })
        .expect("RUNS_BAD line");
    assert_eq!(
        bad, 0,
        "the int8 GEMM produced a wrong element in {bad} of 300 full-matrix runs — a kernel race, not noise:\n{}",
        out.lines().filter(|l| l.starts_with("MISMATCH")).take(5).collect::<Vec<_>>().join("\n")
    );
}
