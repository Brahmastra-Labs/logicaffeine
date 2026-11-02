//! ★ THE "BOTH" GEMM LANE ★ — compiles + runs `cuda/gemm_bench.cu` on the GPU: our own proven DP4A int8
//! kernel vs NVIDIA cuBLAS int8, same GEMM, both verified against a CPU reference, then timed. Asserts
//! BOTH are numerically correct on the device (the point: our auditable kernel and the black-box tensor-core
//! path land the same answer); prints the GOPS for the record.
//!
//! Gated on `nvcc` + a live GPU; SKIPS cleanly otherwise.

use std::path::PathBuf;

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile")
}
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

#[test]
fn our_dp4a_int8_gemm_and_cublas_agree_on_the_gpu() {
    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP gemm bench: needs nvcc + a live GPU (/dev/nvidia0).");
        return;
    }
    let dir = std::env::temp_dir().join(format!("logos_gemm_bench_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let bin = dir.join("gemm_bench");
    let src = manifest().join("cuda/gemm_bench.cu");

    let compile = std::process::Command::new("nvcc")
        .args(["-O3", "-arch=sm_86"])
        .arg(&src)
        .args(["-lcublas", "-o"])
        .arg(&bin)
        .output()
        .expect("run nvcc");
    assert!(
        compile.status.success(),
        "gemm bench must compile:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );

    // A smaller shape keeps the test quick while still exercising both paths.
    let run = std::process::Command::new(&bin)
        .args(["1024", "1024", "1024"])
        .output()
        .expect("run gemm_bench");
    let out = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success(),
        "gemm bench run failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );

    // Both lanes must be numerically correct against the CPU reference.
    let our_line = out.lines().find(|l| l.contains("our DP4A")).unwrap_or("");
    let cub_line = out.lines().find(|l| l.contains("cuBLAS")).unwrap_or("");
    assert!(
        our_line.contains("correct"),
        "our DP4A kernel must be correct on the GPU. got: `{our_line}`\n{out}"
    );
    assert!(
        cub_line.contains("correct"),
        "cuBLAS int8 must be correct on the GPU. got: `{cub_line}`\n{out}"
    );
    eprintln!("{out}");
    let _ = std::fs::remove_dir_all(&dir);
}
