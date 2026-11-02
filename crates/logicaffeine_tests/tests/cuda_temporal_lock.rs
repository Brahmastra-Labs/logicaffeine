//! ★ CUDA TEMPORAL LOCK ★ — `today` / `now` run on the GPU under a deterministic injected clock, byte
//! identical to the C oracle (which is proven ≡VM), so CUDA ≡ C ≡ VM for the temporal ops.
//!
//! A GPU thread has no `getenv`/`time`, so the CUDA tier cannot read the C runtime's injected-clock shim.
//! Instead the HOST orchestrator (a CPU) reads the SAME `LOGOS_FIXED_DAYS`/`LOGOS_FIXED_NANOS` env the C
//! runtime does and `cudaMemcpyToSymbol`s it into device globals before the launch; the device
//! `logos_today`/`logos_now` overrides return them, and `Show` renders the date/moment via device calendar
//! math (`show_date`/`show_moment` — `printf`, no host libc). This lock pins CUDA == the C oracle for both,
//! UNDER THE SAME FIXED CLOCK, on real hardware.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. If it regresses, fix `codegen_cuda`, not the lock. Gated on
//! `nvcc` + `/dev/nvidia0`; when absent it SKIPS.

use std::sync::atomic::{AtomicUsize, Ordering};

use logicaffeine_compile::compile::{compile_to_c, compile_to_cuda};

/// The injected fixed clock — a fixed day count and nanosecond instant. Both tiers read these; the exact
/// values do not matter, only that CUDA renders them identically to the C oracle.
const FIXED_DAYS: &str = "20000"; // 2024-10-04
const FIXED_NANOS: &str = "1728043200000000000"; // 2024-10-04 12:00 UTC-ish

/// Temporal programs. `today` → a `Date` (`YYYY-MM-DD`); `now` → a `Moment` (`YYYY-MM-DD HH:MM`). Each is
/// device-clean once the clock read is the injected global, so each must render byte-identically to the C
/// oracle under the same env.
const TEMPORAL: &[(&str, &str)] = &[
    ("today", "## Main\n    Let d be today.\n    Show d.\n"),
    ("now", "## Main\n    Let m be now.\n    Show m.\n"),
];

fn have_nvcc_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
        && std::process::Command::new("nvcc")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

/// Compile `src` with `cc`, run it under the fixed-clock env, return trimmed stdout (or `None` on failure).
fn build_and_run(
    src_path: &std::path::Path,
    bin: &std::path::Path,
    compiler: &str,
    args: &[&str],
) -> Option<String> {
    let c = std::process::Command::new(compiler)
        .args(args)
        .arg("-o")
        .arg(bin)
        .arg(src_path)
        .output()
        .ok()?;
    if !c.status.success() {
        eprintln!(
            "  {compiler} failed:\n{}",
            String::from_utf8_lossy(&c.stderr)
                .lines()
                .take(4)
                .collect::<Vec<_>>()
                .join("\n")
        );
        return None;
    }
    let r = std::process::Command::new(bin)
        .env("LOGOS_FIXED_DAYS", FIXED_DAYS)
        .env("LOGOS_FIXED_NANOS", FIXED_NANOS)
        .output()
        .ok()?;
    r.status
        .success()
        .then(|| String::from_utf8_lossy(&r.stdout).trim().to_string())
}

#[test]
fn temporal_runs_on_the_gpu_equiv_the_c_oracle_under_a_fixed_clock() {
    if !have_nvcc_gpu() {
        eprintln!("SKIP: needs nvcc + /dev/nvidia0 — cannot run the temporal programs on the GPU");
        return;
    }
    static N: AtomicUsize = AtomicUsize::new(0);
    for (label, src) in TEMPORAL {
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "logos_cudatemporal_{}_{n}_{label}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        // The C oracle (gcc) is the ≡VM reference under the same fixed clock.
        let c = compile_to_c(src).unwrap_or_else(|e| panic!("`{label}`: compiles to C: {e:?}"));
        let c_path = dir.join("o.c");
        std::fs::write(&c_path, &c).unwrap();
        let c_out = build_and_run(&c_path, &dir.join("o"), "gcc", &["-O2", "-lm"])
            .unwrap_or_else(|| panic!("`{label}`: the C oracle must build + run"));

        // CUDA: the host injects the same clock; the device renders it.
        let art =
            compile_to_cuda(src).unwrap_or_else(|e| panic!("`{label}`: compiles to CUDA: {e:?}"));
        let cu_path = dir.join("k.cu");
        std::fs::write(&cu_path, &art.kernel_cu).unwrap();
        let gpu_out = build_and_run(
            &cu_path,
            &dir.join("k"),
            "nvcc",
            &["-O2", "-arch=sm_86", "-fmad=false"],
        )
        .unwrap_or_else(|| panic!("`{label}`: the emitted `.cu` must nvcc-build + run"));

        assert_eq!(
            gpu_out, c_out,
            "`{label}`: GPU temporal output diverged from the C oracle under the fixed clock"
        );
        assert!(
            !gpu_out.is_empty(),
            "`{label}`: expected a rendered date/moment, got empty"
        );
        eprintln!("ok temporal ≡C≡VM on GPU: {label} → `{gpu_out}`");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
