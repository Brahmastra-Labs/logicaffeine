//! ★ CUDA FAILWITH DEVICE-TRAP LOCK ★ — a program that ERRORS at runtime on the VM exits NONZERO on the
//! GPU too, with stdout equal to the partial output printed BEFORE the error.
//!
//! The C runtime's fail sites are `fprintf(stderr, msg); exit(1);`. A GPU thread has none of
//! `fprintf`/`stderr`/`exit`, so the CUDA tier's [`CUDA_DEVICE_FAULT_SHIM`] lowers them: the stderr
//! diagnostic is elided (it is NOT part of the ≡VM comparison) and `exit(nonzero)`/`abort()` become a PTX
//! `trap` — the kernel faults, `cudaDeviceSynchronize` returns non-success, and the host orchestrator
//! exits nonzero, exactly mirroring the VM's `error.is_some()`. Crucially, the device `printf` buffer is
//! flushed BEFORE the trap, so the partial stdout survives. This lock pins that end-to-end on real
//! hardware: the enum corpus in `cuda_aot_lock` only exercises the DEAD (unreachable) fail path; THIS
//! proves a LIVE trap.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. If it regresses, fix `codegen_cuda`, not the lock. Gated on
//! `nvcc` + `/dev/nvidia0`; when absent it SKIPS (never fails), mirroring the RISC-V/qemu error gate.

use std::sync::atomic::{AtomicUsize, Ordering};

use logicaffeine_compile::compile::{compile_to_cuda, vm_outcome};

/// Programs that ERROR at runtime on the VM, via a fail path the C oracle emits a clean `exit(1)` for
/// (so the divergence is a real trap, not undefined behaviour). Each must: compile to CUDA, and on the
/// GPU exit NONZERO with stdout == the partial output the VM printed before erroring.
const ERROR_CORPUS: &[(&str, &str)] = &[
    // A runtime NEGATIVE integer exponent has no exact integer value → `logos_ipow` errors (VM + C).
    // Runtime exponent (a param) so it is not folded to a compile-time constant. `Show 111` first, so the
    // partial stdout (`111`) must survive the device-printf flush that precedes the trap.
    (
        "negexp",
        "## To p (b: Int) -> Int:\n    Return 2 ** b.\n## Main\n    Show 111.\n    Show p(0 - 1).\n",
    ),
];

fn have_nvcc_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
        && std::process::Command::new("nvcc")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

/// nvcc-compile a `.cu`, run it, and return `(stdout, exit_success)`. `None` only if compilation failed.
fn nvcc_compile_and_run(cu: &str, label: &str) -> Option<(String, bool)> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("logos_cudatrap_{}_{n}_{label}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let src = dir.join("k.cu");
    let bin = dir.join("k");
    std::fs::write(&src, cu).ok()?;
    let c = std::process::Command::new("nvcc")
        .args(["-O2", "-arch=sm_86", "-fmad=false", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .ok()?;
    if !c.status.success() {
        eprintln!(
            "  nvcc failed for {label}:\n{}",
            String::from_utf8_lossy(&c.stderr)
                .lines()
                .take(4)
                .collect::<Vec<_>>()
                .join("\n")
        );
        let _ = std::fs::remove_dir_all(&dir);
        return None;
    }
    let r = std::process::Command::new(&bin).output().ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    Some((
        String::from_utf8_lossy(&r.stdout).to_string(),
        r.status.success(),
    ))
}

#[test]
fn a_runtime_error_traps_on_the_gpu_with_partial_stdout() {
    if !have_nvcc_gpu() {
        eprintln!("SKIP: needs nvcc + /dev/nvidia0 — cannot prove the device trap on the GPU");
        return;
    }
    for (label, src) in ERROR_CORPUS {
        // The VM reference: it MUST error, and its `.output` is the partial stdout before the error.
        let vm = vm_outcome(src);
        assert!(
            vm.error.is_some(),
            "`{label}`: the reference VM run must ERROR (else this is not a trap test)"
        );
        let partial = vm.output.trim().to_string();

        let art = compile_to_cuda(src).unwrap_or_else(|e| {
            panic!("`{label}`: must compile to CUDA (the fault path lowers to a trap): {e:?}")
        });
        let (stdout, success) = nvcc_compile_and_run(&art.kernel_cu, label)
            .unwrap_or_else(|| panic!("`{label}`: the emitted `.cu` must nvcc-compile"));

        assert!(
            !success,
            "`{label}`: the kernel must FAULT (nonzero host exit) where the VM errors, not exit 0"
        );
        assert_eq!(
            stdout.trim(),
            partial,
            "`{label}`: the partial stdout before the trap must equal the VM's (device printf flushes before the trap)"
        );
        eprintln!("ok trap ≡VM on GPU: {label} (stdout `{partial}`, nonzero exit)");
    }
}
