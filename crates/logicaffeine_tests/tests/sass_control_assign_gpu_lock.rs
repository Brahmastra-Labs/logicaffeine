//! sass_control_assign_gpu_lock — the control-code assigner, correctness-first, ON REAL SILICON. We recompute
//! every instruction's stall from OUR latency model (keeping ptxas's memory barriers), re-emit the machine
//! code, run it through the driver API, and prove it stays numerically EXACT vs the varied-fill oracle. This
//! validates that our control-code model produces WORKING machine code — the correctness half of a SASS
//! scheduler's control-code assignment. (Perf comes after: this conservative pass may be slower; correctness
//! first, exactly as asked.)
#![cfg(feature = "codegen")]

mod common;

use logicaffeine_compile::codegen_cuda::gemm_tile::emit_mma_tn_k64_kernel;
use logicaffeine_compile::codegen_sass::assign_stalls_by_latency;
use std::path::Path;
use std::process::Command;

fn have() -> bool {
    common::require_tool("nvcc") && common::require_gpu()
}

// Driver-API runner: load a cubin, run gemm_mma_tn_k64, varied-fill oracle, then best-of-50 timing.
const RUNNER: &str = r#"#include <cuda.h>
#include <cstdio>
#include <cstdlib>
static void ck(CUresult r,const char*w){ if(r!=CUDA_SUCCESS){ const char*s; cuGetErrorString(r,&s); printf("ERR %s: %s\n",w,s); exit(2);} }
static double best(float*m,int n){double b=1e30;for(int i=0;i<n;i++)if(m[i]<b)b=m[i];return b;}
int main(int argc,char**argv){
  const int M=2048,K=2048,N=2048;
  ck(cuInit(0),"init"); CUdevice d; ck(cuDeviceGet(&d,0),"dev"); CUcontext c; ck(cuCtxCreate(&c,0,d),"ctx");
  CUmodule mod; if(cuModuleLoad(&mod,argv[1])!=CUDA_SUCCESS){printf("LOADFAIL\n");return 3;}
  CUfunction f; ck(cuModuleGetFunction(&f,mod,"gemm_mma_tn_k64"),"fn");
  CUdeviceptr dA,dB,dC; ck(cuMemAlloc(&dA,(size_t)M*K),"a"); ck(cuMemAlloc(&dB,(size_t)K*N),"b"); ck(cuMemAlloc(&dC,(size_t)M*N*4),"c");
  signed char*hA=(signed char*)malloc((size_t)M*K),*hB=(signed char*)malloc((size_t)K*N);
  for(long i=0;i<(long)M*K;++i)hA[i]=(signed char)(((i*7+3)%251)-125);
  for(long nn=0;nn<N;++nn)for(long kk=0;kk<K;++kk){long bkn=(long)kk*N+nn;hB[nn*K+kk]=(signed char)(((bkn*5+1)%251)-125);}
  cuMemcpyHtoD(dA,hA,(size_t)M*K); cuMemcpyHtoD(dB,hB,(size_t)K*N);
  int m=M,n=N,k=K; void*args[]={&dA,&dB,&dC,&m,&n,&k}; unsigned gx=N/128,gy=M/128;
  if(cuLaunchKernel(f,gx,gy,1,128,1,1,0,0,args,0)!=CUDA_SUCCESS){printf("LAUNCHFAIL\n");return 4;}
  if(cuCtxSynchronize()!=CUDA_SUCCESS){printf("SYNCFAIL\n");return 5;}
  int*hC=(int*)malloc((size_t)M*N*4); cuMemcpyDtoH(hC,dC,(size_t)M*N*4);
  for(long q=0;q<16;q++){long i=(q*7919)%M,j=(q*104729)%N;long acc=0;for(long kk=0;kk<K;kk++){long av=((i*K+kk)*7+3)%251-125;long bv=((kk*N+j)*5+1)%251-125;acc+=av*bv;}if(hC[i*N+j]!=acc){printf("INCORRECT\n");return 1;}}
  cuMemsetD8(dA,1,(size_t)M*K); cuMemsetD8(dB,1,(size_t)K*N);
  CUevent s,e; cuEventCreate(&s,0); cuEventCreate(&e,0); int IT=50; float mo[50];
  for(int i=0;i<8;i++)cuLaunchKernel(f,gx,gy,1,128,1,1,0,0,args,0);
  cuCtxSynchronize();
  for(int i=0;i<IT;i++){cuEventRecord(s,0);cuLaunchKernel(f,gx,gy,1,128,1,1,0,0,args,0);cuEventRecord(e,0);cuEventSynchronize(e);cuEventElapsedTime(&mo[i],s,e);}
  printf("GOPS %.0f\n", 2.0*M*N*K/(best(mo,IT)/1e3)/1e9);
  return 0;
}
"#;

fn run(runner: &Path, cubin: &Path) -> (bool, f64) {
    let o = Command::new(runner).arg(cubin).output().expect("run");
    let out = String::from_utf8_lossy(&o.stdout);
    if out.contains("INCORRECT") || out.contains("FAIL") || out.contains("ERR ") {
        eprintln!("run: {}", out.trim());
        return (false, 0.0);
    }
    let g = out
        .lines()
        .find_map(|l| {
            l.strip_prefix("GOPS ")
                .and_then(|v| v.trim().parse::<f64>().ok())
        })
        .unwrap_or(0.0);
    (true, g)
}

#[test]
fn our_control_code_stalls_produce_correct_machine_code() {
    if !have() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("logos_ctl_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    // build the driver-API runner
    let rsrc = dir.join("run.cu");
    let runner = dir.join("run");
    std::fs::write(&rsrc, RUNNER).unwrap();
    let o = Command::new("nvcc")
        .args(["-O3", "-o"])
        .arg(&runner)
        .arg(&rsrc)
        .arg("-lcuda")
        .output()
        .expect("runner");
    assert!(
        o.status.success(),
        "runner: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    // our champion kernel -> cubin
    let src = dir.join("k.cu");
    let cubin = dir.join("k.cubin");
    std::fs::write(&src, emit_mma_tn_k64_kernel(128, 128, 2, 2, 3)).unwrap();
    let o = Command::new("nvcc")
        .args(["-O3", "-arch=sm_86", "-cubin", "-o"])
        .arg(&cubin)
        .arg(&src)
        .output()
        .expect("cubin");
    assert!(o.status.success());

    // baseline (ptxas control codes)
    let (base_ok, base_g) = run(&runner, &cubin);
    assert!(base_ok, "baseline kernel must be correct");

    // OUR control-code assignment: recompute all stalls from our latency model, keep ptxas barriers.
    let patched = assign_stalls_by_latency(&cubin).expect("assign stalls");
    let patched_path = dir.join("k_ctl.cubin");
    std::fs::write(&patched_path, &patched).unwrap();
    let (ok, g) = run(&runner, &patched_path);

    eprintln!(
        "CONTROL-CODE ASSIGNER (our stalls, ptxas barriers): correct={ok}  {:.0} GOPS  (ptxas baseline {:.0})",
        g, base_g
    );
    let _ = std::fs::remove_dir_all(&dir);

    // CORRECTNESS-FIRST: our recomputed control codes must produce numerically-correct machine code on-device.
    assert!(ok, "our control-code (stall) assignment must keep the kernel numerically CORRECT — the whole point");
}

#[test]
fn our_minimal_stall_assignment_is_correct_and_targets_ptxas() {
    use logicaffeine_compile::codegen_sass::assign_minimal_stalls;
    if !have() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("logos_minstall_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let rsrc = dir.join("run.cu");
    let runner = dir.join("run");
    std::fs::write(&rsrc, RUNNER).unwrap();
    assert!(Command::new("nvcc")
        .args(["-O3", "-o"])
        .arg(&runner)
        .arg(&rsrc)
        .arg("-lcuda")
        .output()
        .unwrap()
        .status
        .success());
    let src = dir.join("k.cu");
    let cubin = dir.join("k.cubin");
    std::fs::write(&src, emit_mma_tn_k64_kernel(128, 128, 2, 2, 3)).unwrap();
    assert!(Command::new("nvcc")
        .args(["-O3", "-arch=sm_86", "-cubin", "-o"])
        .arg(&cubin)
        .arg(&src)
        .output()
        .unwrap()
        .status
        .success());
    let (bok, bg) = run(&runner, &cubin);
    assert!(bok);
    let patched = assign_minimal_stalls(&cubin).expect("minimal stalls");
    let pp = dir.join("k_min.cubin");
    std::fs::write(&pp, &patched).unwrap();
    let (ok, g) = run(&runner, &pp);
    eprintln!("LATENCY+THROUGHPUT stall model (floored at ptxas): correct={ok}  {:.0} GOPS  (ptxas baseline {:.0}, {:.1}%)", g, bg, 100.0*g/bg);
    eprintln!("  FINDING: a below-ptxas minimal schedule UNDER-STALLS (hangs) — our latency+tensor-throughput model");
    eprintln!("  is incomplete (misses barrier-wait timing / dual-issue / bank conflicts). Flooring at ptxas keeps");
    eprintln!("  it correct. Beating ptxas needs the COMPLETE hazard model — the remaining MaxAs reverse-engineering.");
    let _ = std::fs::remove_dir_all(&dir);
    // Sound: floored at ptxas, our assigner produces CORRECT machine code (the mechanism works end-to-end).
    assert!(
        ok,
        "the (sound, ptxas-floored) stall assignment must keep the kernel numerically correct"
    );
}

#[test]
fn pure_calibrated_stall_model_after_measuring_imma_latency() {
    use logicaffeine_compile::codegen_sass::assign_minimal_stalls_opt;
    if !have() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("logos_pure_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let rsrc = dir.join("run.cu");
    let runner = dir.join("run");
    std::fs::write(&rsrc, RUNNER).unwrap();
    assert!(Command::new("nvcc")
        .args(["-O3", "-o"])
        .arg(&runner)
        .arg(&rsrc)
        .arg("-lcuda")
        .output()
        .unwrap()
        .status
        .success());
    let src = dir.join("k.cu");
    let cubin = dir.join("k.cubin");
    std::fs::write(&src, emit_mma_tn_k64_kernel(128, 128, 2, 2, 3)).unwrap();
    assert!(Command::new("nvcc")
        .args(["-O3", "-arch=sm_86", "-cubin", "-o"])
        .arg(&cubin)
        .arg(&src)
        .output()
        .unwrap()
        .status
        .success());
    let (_, bg) = run(&runner, &cubin);
    // PURE model (no ptxas floor) with the MEASURED IMMA latency=24 (was 8 => hung).
    let patched = assign_minimal_stalls_opt(&cubin, false).expect("pure");
    let pp = dir.join("k_pure.cubin");
    std::fs::write(&pp, &patched).unwrap();
    let (ok, g) = run(&runner, &pp);
    eprintln!("PURE CALIBRATED model (IMMA lat=24 measured, no ptxas floor): correct={ok}  {:.0} GOPS  (ptxas {:.0}, {:.1}%)", g, bg, if bg>0.0 {100.0*g/bg} else {0.0});
    let _ = std::fs::remove_dir_all(&dir);
    // Does the measured-latency calibration alone (no ptxas crutch) produce correct machine code? Report truth.
    eprintln!(
        "  => the empirical IMMA-latency=24 correction {}",
        if ok {
            "FIXED the hang: our own calibrated model is CORRECT"
        } else {
            "still incomplete: more hazards (barrier-timing/bank/dual-issue) remain to model"
        }
    );
}
