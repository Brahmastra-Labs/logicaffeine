//! ★ THE CUDA DEVICE FLOAT-FORMATTER LOCK ★ — proves the from-scratch device `logos_fmt_f64`
//! (Dragon4/FPP2, [`codegen_cuda::LOGOS_DEVICE_FLOAT_FMT`]) prints EVERY `f64` byte-identically to the
//! VM/tree-walker (Rust `{}` Display, via [`compile::display_float_like_logos`]). The device has no
//! `strtod`/`snprintf`, so the host round-trip formatter cannot be lifted; this lock is what lets us
//! trust the replacement. Edge cases + a wide random fuzz are formatted ON THE GPU and compared to the
//! Rust oracle byte-for-byte.
//!
//! Gated on `nvcc` + a GPU driver (`/dev/nvidia0`); SKIPS gracefully when absent (like the RISC-V/qemu
//! and CUDA execution gates). This is the authoritative correctness gate for the Phase 1 float lane.

use logicaffeine_compile::codegen_cuda::LOGOS_DEVICE_FLOAT_FMT;
use logicaffeine_compile::compile::display_float_like_logos;

fn have_nvcc_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
        && std::process::Command::new("nvcc")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

/// The doubles under test: hand-picked edge cases + a wide random fuzz (finite values across the whole
/// exponent range, including subnormals). Returned as raw bits so the `.cu` reconstructs each value
/// bit-exactly (no decimal-literal round-trip in the C source).
fn test_values() -> Vec<f64> {
    let mut v: Vec<f64> = vec![
        0.0,
        -0.0,
        1.0,
        -1.0,
        2.5,
        -2.5,
        0.1,
        0.2,
        0.3,
        1.0 / 3.0,
        2.0 / 3.0,
        10.0 / 4.0,
        3.141592653589793,
        2.718281828459045,
        100.0,
        1000.0,
        123456.789,
        0.000123456,
        1e10,
        1e-10,
        1e100,
        1e-100,
        1e300,
        1e-300,
        9007199254740992.0, // 2^53
        12345678901234567.0,
        f64::MAX,
        f64::MIN_POSITIVE,
        5e-324,            // smallest subnormal
        f64::from_bits(1), // subnormal
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ];
    // Deterministic fuzz — random finite doubles across the full exponent range (LCG, no wall clock).
    let mut st: u64 = 0x1234_5678_9abc_def0;
    let mut next = || {
        st = st
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        st
    };
    for _ in 0..4000 {
        let sign = next() & 1;
        let exp = next() % 2047; // 0..=2046 → finite (never all-ones)
        let mant = next() & 0xf_ffff_ffff_ffff;
        let bits = (sign << 63) | (exp << 52) | mant;
        v.push(f64::from_bits(bits));
    }
    v
}

#[test]
fn device_float_formatter_matches_the_vm_for_every_value() {
    if !have_nvcc_gpu() {
        eprintln!("SKIP cuda float lock: needs nvcc + a GPU driver (/dev/nvidia0).");
        return;
    }
    let values = test_values();
    let expected: Vec<String> = values
        .iter()
        .map(|&f| display_float_like_logos(f))
        .collect();

    // Build a self-contained .cu: the device formatter + a kernel that formats each value (rebuilt
    // bit-exactly from its u64 bits) and prints one line each.
    let mut bits_lines = String::new();
    for (i, f) in values.iter().enumerate() {
        if i % 8 == 0 {
            bits_lines.push_str("\n    ");
        }
        bits_lines.push_str(&format!("{}ull, ", f.to_bits()));
    }
    let cu = format!(
        r#"#include <cstdio>
#include <cstdint>
#include <cstring>
#include <cstdlib>
#include <cmath>
{fmt}
__device__ static const unsigned long long BITS[{n}] = {{{bits}}};
__global__ void k() {{
    for (int i = 0; i < {n}; i++) {{
        double x; memcpy(&x, &BITS[i], 8);
        char* s = logos_fmt_f64(x);
        printf("%s\n", s);
        free(s);
    }}
}}
int main() {{
    cudaDeviceSetLimit(cudaLimitStackSize, (size_t)64 * 1024);
    cudaDeviceSetLimit(cudaLimitMallocHeapSize, (size_t)256 * 1024 * 1024);
    k<<<1, 1>>>();
    cudaError_t e = cudaDeviceSynchronize();
    if (e != cudaSuccess) {{ fprintf(stderr, "exec: %s\n", cudaGetErrorString(e)); return 1; }}
    return 0;
}}
"#,
        fmt = LOGOS_DEVICE_FLOAT_FMT,
        n = values.len(),
        bits = bits_lines,
    );

    let dir = std::env::temp_dir().join(format!("logos_cuda_float_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("f.cu");
    let bin = dir.join("f");
    std::fs::write(&src, &cu).expect("write .cu");

    let compile = std::process::Command::new("nvcc")
        .args(["-O2", "-arch=sm_86", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .expect("run nvcc");
    assert!(
        compile.status.success(),
        "device float .cu must compile with nvcc.\nstderr:\n{}",
        String::from_utf8_lossy(&compile.stderr),
    );

    let run = std::process::Command::new(&bin)
        .output()
        .expect("run float binary");
    assert!(
        run.status.success(),
        "float binary should run.\nstderr:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let out = String::from_utf8_lossy(&run.stdout);
    let got: Vec<&str> = out.lines().collect();

    assert_eq!(
        got.len(),
        expected.len(),
        "device printed {} lines, expected {}",
        got.len(),
        expected.len()
    );

    let mut mismatches = Vec::new();
    for (i, (g, e)) in got.iter().zip(expected.iter()).enumerate() {
        if g != e {
            mismatches.push(format!(
                "  bits={:#018x}  device={:?}  vm={:?}",
                values[i].to_bits(),
                g,
                e
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        mismatches.is_empty(),
        "device float formatter diverged from the VM on {} / {} values:\n{}",
        mismatches.len(),
        values.len(),
        mismatches
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n"),
    );
}
