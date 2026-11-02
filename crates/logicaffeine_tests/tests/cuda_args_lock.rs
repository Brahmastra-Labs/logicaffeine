//! ★ CUDA ARGV LOCK ★ — `args()` runs on the GPU byte-identical to the C oracle (which is proven ≡VM)
//! UNDER THE SAME COMMAND-LINE ARGUMENTS, so CUDA ≡ C ≡ VM for the argv op.
//!
//! A GPU thread has no `argc`/`argv`. So the CUDA HOST orchestrator (a CPU) receives its own argv, marshals
//! each string into device memory (`cudaMalloc`/`cudaMemcpy`), builds a device `char**`, and copies it into
//! the device `_logos_argc`/`_logos_argv` globals (`cudaMemcpyToSymbol`) BEFORE the launch. The lifted
//! `logos_args()` then reads them to build the `Seq of Text`, exactly as the C runtime does from `main`'s
//! argv. This lock pins CUDA == the C oracle for a program that reads its arguments, on real hardware.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. If it regresses, fix `codegen_cuda`, not the lock. Gated on
//! `nvcc` + `/dev/nvidia0`; when absent it SKIPS.

use logicaffeine_compile::compile::{compile_to_c, compile_to_cuda};

/// Reads its arguments: the count (`length of args()`) and the second one (`item 2` — 1-based, so the first
/// real argument after the program name). Device-clean once argv is the injected globals.
const ARGS_PROGRAM: &str =
    "## To native args () -> Seq of Text\n\n## Main\nLet arguments be args().\n\
     Show length of arguments.\nShow item 2 of arguments.\n";

/// The command-line arguments passed to BOTH binaries — the count and `item 2` must render identically.
const ARGV: &[&str] = &["alpha", "beta", "gamma"];

fn have_nvcc_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
        && std::process::Command::new("nvcc")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

/// Build `src` with `cc`, run it under `ARGV`, return trimmed stdout (or `None` on failure).
fn build_and_run(
    src_path: &std::path::Path,
    bin: &std::path::Path,
    compiler: &str,
    cargs: &[&str],
) -> Option<String> {
    let c = std::process::Command::new(compiler)
        .args(cargs)
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
    let r = std::process::Command::new(bin).args(ARGV).output().ok()?;
    r.status
        .success()
        .then(|| String::from_utf8_lossy(&r.stdout).trim().to_string())
}

#[test]
fn argv_runs_on_the_gpu_equiv_the_c_oracle_under_the_same_arguments() {
    if !have_nvcc_gpu() {
        eprintln!("SKIP: needs nvcc + /dev/nvidia0 — cannot run the argv program on the GPU");
        return;
    }
    let dir = std::env::temp_dir().join(format!("logos_cudaargv_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // The C oracle (gcc) is the ≡VM reference, run under the same argv.
    let c = compile_to_c(ARGS_PROGRAM).expect("compiles to C");
    std::fs::write(dir.join("o.c"), &c).unwrap();
    let c_out = build_and_run(&dir.join("o.c"), &dir.join("o"), "gcc", &["-O2", "-lm"])
        .expect("the C oracle must build + run under argv");

    // CUDA: the host marshals its argv into the device globals; the device reads them.
    let art = compile_to_cuda(ARGS_PROGRAM).expect("compiles to CUDA");
    std::fs::write(dir.join("k.cu"), &art.kernel_cu).unwrap();
    let gpu_out = build_and_run(
        &dir.join("k.cu"),
        &dir.join("k"),
        "nvcc",
        &["-O2", "-arch=sm_86", "-fmad=false"],
    )
    .expect("the emitted `.cu` must nvcc-build + run under argv");

    assert_eq!(
        gpu_out, c_out,
        "GPU argv output diverged from the C oracle under the same arguments"
    );
    // Sanity: `length` = ARGV.len()+1 (program name + args), `item 2` = the first real arg.
    assert_eq!(
        gpu_out,
        format!("{}\n{}", ARGV.len() + 1, ARGV[0]),
        "unexpected argv rendering"
    );
    eprintln!("ok argv ≡C≡VM on GPU: {:?}", gpu_out);
    let _ = std::fs::remove_dir_all(&dir);
}
