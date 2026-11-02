//! cuda_precision_tradeoff_lock — PRECISION is a first-class frontier knob because it carries a real
//! throughput-vs-range tradeoff on the SAME Ampere tensor cores, and this lock proves it by MEASURING both
//! cp.async WMMA kernels on this box (correctness-guarded first):
//!   * int8 IMMA runs ~2× the bf16 HMMA rate with HALF the memory traffic (1-byte vs 2-byte operands) — the
//!     SPEED side. On the 3060 the gap is ~3–4× at 1024³.
//!   * bf16 accumulates in fp32 and carries an 8-bit exponent (~1e38 dynamic range) that int8's [-128,127]
//!     integer domain cannot — the RANGE / accuracy side (structural, from the MmaKind metadata).
//!
//! Both kernels are verified ≡ the SAME scalar oracle (the fill is exact in int8 AND bf16) BEFORE their GOPS is
//! trusted — the measurement-is-truth guard that caught our earlier impossible-speed timing bug. So the knob is
//! honest: pick int8 for peak throughput on quantized data, bf16 when you need range. Requires nvcc + a GPU.
//!
//! ⚠️ If the speed assertion goes RED, tune the kernels (`codegen_cuda::gemm_tile`), never the assertion.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_precision_tradeoff_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::{
    default_wmma_tile, emit_wmma_cpasync_bench, emit_wmma_cpasync_program, MmaKind,
};
use logicaffeine_compile::codegen_cuda::CudaKnobs;

const STAGES: usize = 2;

fn have() -> bool {
    std::process::Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        && std::path::Path::new("/dev/nvidia0").exists()
}

/// The scalar oracle the deterministic int8-range fill computes — exact in int8, bf16, AND f16.
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
    let dir = std::env::temp_dir().join(format!("logos_prec_{}_{tag}", std::process::id()));
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

/// The RANGE side is a structural property of the MmaKind, not a benchmark: bf16 operands are 2 bytes and
/// accumulate in fp32 (wide exponent); int8 operands are 1 byte and accumulate in int (bounded integer domain).
#[test]
fn bf16_carries_range_int8_carries_density() {
    assert_eq!(
        MmaKind::WmmaI8.operand_bytes(),
        1,
        "int8 operand is 1 byte (density)"
    );
    assert_eq!(
        MmaKind::HmmaBf16.operand_bytes(),
        2,
        "bf16 operand is 2 bytes (range)"
    );
    assert_eq!(
        MmaKind::WmmaI8.acc_ctype(),
        "int",
        "int8 accumulates in the integer domain"
    );
    assert_eq!(
        MmaKind::HmmaBf16.acc_ctype(),
        "float",
        "bf16 accumulates in fp32 — the wide-exponent range"
    );
    // Both are genuine tensor-core kinds on the same silicon — the tradeoff is real, not a fallback artifact.
    assert!(MmaKind::WmmaI8.is_tensor_core() && MmaKind::HmmaBf16.is_tensor_core());
}

#[test]
fn precision_is_a_real_tradeoff_int8_is_faster_on_the_same_tensor_cores() {
    if !have() {
        eprintln!("SKIP precision tradeoff: needs nvcc + /dev/nvidia0.");
        return;
    }
    let arch = CudaKnobs::default().chip.nvcc_arch_flag();
    let tile = default_wmma_tile();

    // (1) Correctness guard — BOTH precisions' cp.async kernels ≡ the same scalar oracle at 256³ before any GOPS
    //     is trusted. The fill (values in [-3,3]·[-2,2]) is exact in int8 and bf16, so Σ C is identical.
    let (cm, ck, cn) = (256usize, 256usize, 256usize);
    let want = cpu_gemm_sum(cm, ck, cn);
    for kind in [MmaKind::WmmaI8, MmaKind::HmmaBf16] {
        let cu = emit_wmma_cpasync_program(kind, &tile, STAGES, cm, ck, cn);
        let got: i64 = compile_run(&cu, kind.tag(), &arch).parse().unwrap();
        assert_eq!(
            got,
            want,
            "cp.async {} Σ C must equal the scalar oracle before its speed is trusted",
            kind.tag()
        );
    }

    // (2) Speed side — at 1024³ (memory + compute both bite), int8 IMMA is meaningfully faster than bf16 HMMA on
    //     the same tensor cores: 2× the op rate × half the operand traffic.
    let (m, k, n) = (1024usize, 1024usize, 1024usize);
    let parse_gops = |s: String| {
        s.lines()
            .find_map(|l| l.strip_prefix("GOPS "))
            .unwrap()
            .trim()
            .parse::<f64>()
            .unwrap()
    };
    let i8_gops = parse_gops(compile_run(
        &emit_wmma_cpasync_bench(MmaKind::WmmaI8, &tile, STAGES, m, k, n),
        "i8",
        &arch,
    ));
    let bf16_gops = parse_gops(compile_run(
        &emit_wmma_cpasync_bench(MmaKind::HmmaBf16, &tile, STAGES, m, k, n),
        "bf16",
        &arch,
    ));
    eprintln!(
        "cp.async GEMM {m}³ on the 3060: int8 {i8_gops:.0} GOPS  vs  bf16 {bf16_gops:.0} GOPS  ({:.2}× — the precision tradeoff)",
        i8_gops / bf16_gops.max(1.0)
    );
    assert!(
        i8_gops > 1.5 * bf16_gops,
        "int8 cp.async ({i8_gops:.0} GOPS) must be >>1.5× bf16 cp.async ({bf16_gops:.0} GOPS) — that gap is why \
         PRECISION is a frontier knob: int8 for throughput on quantized data, bf16 for range"
    );
}
