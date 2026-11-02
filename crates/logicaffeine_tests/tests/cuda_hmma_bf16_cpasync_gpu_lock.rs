//! cuda_hmma_bf16_cpasync_gpu_lock — the genuine bf16 speed climb: cp.async double-buffering beats basic.
//!
//! The plain shared-memory bf16 WMMA (cuda_hmma_bf16_tiled) did NOT beat the high-occupancy basic kernel on the
//! 3060 — its memory loads still serialize behind compute. `cp.async` streams the NEXT K-tile into a second
//! shared buffer WHILE the tensor cores compute the current one (double-buffered), so the WMMA units stop
//! starving. This lock proves the cp.async bf16 kernel (a) equals the scalar oracle and (b) is STRICTLY FASTER
//! than the basic bf16 kernel on the 3060 — the real climb. Gated on nvcc + a device.
//!
//! ⚠️ If the speed assertion goes RED, fix/tune the kernel (`codegen_cuda::gemm_tile`), never the assertion.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_hmma_bf16_cpasync_gpu_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::{
    default_wmma_tile, emit_wmma_basic_bench, emit_wmma_cpasync_bench, emit_wmma_cpasync_program,
    MmaKind,
};
use logicaffeine_compile::codegen_cuda::CudaKnobs;

const STAGES: usize = 2;

fn have_nvcc() -> bool {
    std::process::Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
fn have_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
}

fn cpu_gemm_sum(m: usize, k: usize, n: usize) -> i64 {
    let a = |mm: usize, kk: usize| ((mm * k + kk) as i64 % 7) - 3;
    let b = |kk: usize, nn: usize| ((kk * n + nn) as i64 % 5) - 2;
    let mut sum = 0i64;
    for mm in 0..m {
        for nn in 0..n {
            let mut c = 0i64;
            for kk in 0..k {
                c += a(mm, kk) * b(kk, nn);
            }
            sum += c;
        }
    }
    sum
}

fn compile_run(cu: &str, tag: &str, arch: &str) -> String {
    let dir = std::env::temp_dir().join(format!("logos_cpasync_{}_{tag}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (s, b) = (dir.join("g.cu"), dir.join("g"));
    std::fs::write(&s, cu).unwrap();
    let c = std::process::Command::new("nvcc")
        .args(["-O3", arch, "-o"])
        .arg(&b)
        .arg(&s)
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "[{tag}] must compile:\n{}\n\n{cu}",
        String::from_utf8_lossy(&c.stderr)
    );
    let out = String::from_utf8_lossy(&std::process::Command::new(&b).output().unwrap().stdout)
        .trim()
        .to_string();
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[test]
fn cpasync_bf16_gemm_equals_the_oracle_on_the_gpu() {
    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP cp.async HMMA GPU: needs nvcc + /dev/nvidia0.");
        return;
    }
    let arch = CudaKnobs::default().chip.nvcc_arch_flag();
    let tile = default_wmma_tile();
    let (m, k, n) = (256usize, 256usize, 256usize);
    let want = cpu_gemm_sum(m, k, n) as f64;
    for kind in [MmaKind::HmmaBf16, MmaKind::HmmaF16] {
        let cu = emit_wmma_cpasync_program(kind, &tile, STAGES, m, k, n);
        let got: f64 = compile_run(&cu, kind.tag(), &arch).parse().unwrap();
        assert!(
            (got - want).abs() < 0.5,
            "cp.async {} GEMM Σ C={got} must equal the oracle={want}",
            kind.tag()
        );
    }
}

#[test]
fn cpasync_bf16_is_faster_than_the_basic_bf16_kernel() {
    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP cp.async-vs-basic HMMA GPU: needs nvcc + /dev/nvidia0.");
        return;
    }
    let arch = CudaKnobs::default().chip.nvcc_arch_flag();
    let tile = default_wmma_tile();
    let (m, k, n) = (1024usize, 1024usize, 1024usize); // big enough that memory-latency hiding decides it
    let parse_gops = |s: String| {
        s.lines()
            .find_map(|l| l.strip_prefix("GOPS "))
            .unwrap()
            .trim()
            .parse::<f64>()
            .unwrap()
    };

    let basic = parse_gops(compile_run(
        &emit_wmma_basic_bench(MmaKind::HmmaBf16, m, k, n),
        "basic",
        &arch,
    ));
    let cpasync = parse_gops(compile_run(
        &emit_wmma_cpasync_bench(MmaKind::HmmaBf16, &tile, STAGES, m, k, n),
        "cpasync",
        &arch,
    ));
    eprintln!("bf16 GEMM {m}x{k}x{n} on the 3060: basic {basic:.0} GOPS -> cp.async {cpasync:.0} GOPS ({:.2}x)", cpasync / basic.max(1.0));
    assert!(
        cpasync > basic,
        "cp.async bf16 WMMA ({cpasync:.0} GOPS) must beat the basic bf16 WMMA ({basic:.0} GOPS)"
    );
}
