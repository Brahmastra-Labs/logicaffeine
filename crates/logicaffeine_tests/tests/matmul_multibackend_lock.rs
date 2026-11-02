//! matmul_multibackend_lock — ONE LOGOS matmul, MANY backends, all ≡ the VM. The same ordinary triple-loop
//! GEMM a user writes (NO matmul primitive) produces a byte-identical checksum on every backend we ship:
//!   • the tree-walker (reference interpreter),
//!   • the register VM (the anchor every other tier is proven against),
//!   • RISC-V — our `codegen_riscv` AOT tier, assembled by `riscv64-linux-gnu-gcc` and run under `qemu-riscv64`,
//!   • CUDA — emitted by `compile_to_cuda_matmul`, compiled by `nvcc`, run on the RTX 3060.
//! This is the portable half of the tensor-compiler thesis: "write it in LOGOS → correct output on EVERY
//! backend." The optimized kernels (int4-dp4a GEMV, etc.) are refinements *of a backend* that must never
//! change the answer — this lock is the invariant they are held to. RISC-V and CUDA legs self-skip when
//! their toolchains are absent (they run in CI where present).
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(matmul_multibackend_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_riscv::RISCV_RUNTIME_C;
use logicaffeine_compile::compile::{
    compile_to_cuda_matmul, compile_to_riscv, tw_outcome, vm_outcome,
};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A row-major GEMM `C(M×N) = A(M×K)·B(K×N)` as an ordinary LOGOS program — int8-range deterministic fill,
/// checksum. The kind of code a user writes, with NO matmul primitive. (Identical shape to the CUDA/TT locks
/// so the backends are held to the same source.)
fn gemm_program(m: usize, n: usize, k: usize) -> String {
    format!(
        r#"## Main
Let rows be {m}.
Let cols be {n}.
Let mid be {k}.
Let mutable a be a new Seq of Int.
Let mutable b be a new Seq of Int.
Let mutable c be a new Seq of Int.
Let mutable p be 0.
While p is less than rows * mid:
    Push (p % 7) - 3 to a.
    Set p to p + 1.
Set p to 0.
While p is less than mid * cols:
    Push (p % 5) - 2 to b.
    Set p to p + 1.
Set p to 0.
While p is less than rows * cols:
    Push 0 to c.
    Set p to p + 1.
Let mutable i be 0.
While i is less than rows:
    Let mutable k be 0.
    While k is less than mid:
        Let mutable j be 0.
        While j is less than cols:
            Let idx be i * cols + j + 1.
            Set item idx of c to item idx of c + item (i * mid + k + 1) of a * item (k * cols + j + 1) of b.
            Set j to j + 1.
        Set k to k + 1.
    Set i to i + 1.
Let mutable checksum be 0.
Let mutable q be 1.
While q is at most rows * cols:
    Set checksum to checksum + item q of c.
    Set q to q + 1.
Show checksum.
"#
    )
}

/// The dims we sweep — small enough to run every backend fast, big enough to exercise the contraction.
const DIMS: &[(usize, usize, usize)] = &[(16, 16, 16), (24, 32, 40), (8, 64, 48)];

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
fn have_riscv() -> bool {
    let gcc = std::process::Command::new("riscv64-linux-gnu-gcc")
        .arg("--version")
        .output();
    let qemu = std::process::Command::new("qemu-riscv64")
        .arg("--version")
        .output();
    gcc.map(|o| o.status.success()).unwrap_or(false)
        && qemu.map(|o| o.status.success()).unwrap_or(false)
}

/// Assemble RISC-V asm + the runtime shim with the cross gcc and run it under qemu; return trimmed stdout.
fn riscv_run(asm: &str, label: &str) -> String {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("logos_mm_rv_{}_{n}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let (asm_path, rt_path, bin_path) =
        (dir.join("main.s"), dir.join("logos_rt.c"), dir.join("main"));
    std::fs::write(&asm_path, asm).expect("write .s");
    std::fs::write(&rt_path, RISCV_RUNTIME_C).expect("write runtime shim");

    let c = std::process::Command::new("riscv64-linux-gnu-gcc")
        .args(["-O2", "-static", "-o"])
        .arg(&bin_path)
        .arg(&asm_path)
        .arg(&rt_path)
        .arg("-lm")
        .output()
        .expect("run riscv64-linux-gnu-gcc");
    assert!(
        c.status.success(),
        "[{label}] riscv assemble/link failed:\n{}",
        String::from_utf8_lossy(&c.stderr)
    );

    let r = std::process::Command::new("qemu-riscv64")
        .arg(&bin_path)
        .output()
        .expect("run qemu-riscv64");
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&r.stdout).trim().to_string()
}

/// Compile the emitted CUDA matmul with nvcc and run it on the GPU; return trimmed stdout.
fn cuda_run(cu: &str, label: &str) -> String {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("logos_mm_cu_{}_{n}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let (s, b) = (dir.join("m.cu"), dir.join("m"));
    std::fs::write(&s, cu).unwrap();
    let c = std::process::Command::new("nvcc")
        .args(["-O2", "-arch=sm_86", "-o"])
        .arg(&b)
        .arg(&s)
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "[{label}] emitted CUDA must compile:\n{}\n\n{cu}",
        String::from_utf8_lossy(&c.stderr)
    );
    let r = std::process::Command::new(&b).output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&r.stdout).trim().to_string()
}

#[test]
fn the_treewalker_matches_the_vm_over_the_matmul() {
    for &(m, n, k) in DIMS {
        let src = gemm_program(m, n, k);
        let vm = vm_outcome(&src).output.trim().to_string();
        let tw = tw_outcome(&src).output.trim().to_string();
        assert_eq!(tw, vm, "({m}×{n}×{k}) tree-walker must equal the VM");
    }
}

#[test]
fn riscv_matmul_equals_the_vm_under_qemu() {
    if !have_riscv() {
        eprintln!("SKIP riscv matmul exec: needs riscv64-linux-gnu-gcc + qemu-riscv64.");
        return;
    }
    for &(m, n, k) in DIMS {
        let src = gemm_program(m, n, k);
        let vm = vm_outcome(&src).output.trim().to_string();
        let asm = compile_to_riscv(&src)
            .expect("the matmul's ops (Seq/Index/While/Show) are RISC-V-supported");
        let got = riscv_run(&asm, &format!("{m}x{n}x{k}"));
        assert_eq!(
            got, vm,
            "({m}×{n}×{k}) RISC-V (qemu) checksum must equal the VM"
        );
    }
}

#[test]
fn cuda_matmul_equals_the_vm_on_the_gpu() {
    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP cuda matmul exec: needs nvcc + a GPU driver (/dev/nvidia0).");
        return;
    }
    for &(m, n, k) in DIMS {
        let src = gemm_program(m, n, k);
        let vm = vm_outcome(&src).output.trim().to_string();
        let (_spec, cu) = compile_to_cuda_matmul(&src).unwrap();
        let got = cuda_run(&cu, &format!("{m}x{n}x{k}"));
        assert_eq!(
            got, vm,
            "({m}×{n}×{k}) emitted-CUDA checksum must equal the VM"
        );
    }
}

/// The headline: on any box, EVERY backend whose toolchain is present agrees with the VM on the very same
/// source. (The tree-walker always runs; RISC-V/CUDA join when their toolchains exist.) One source, one
/// answer, many machines.
#[test]
fn every_available_backend_agrees_with_the_vm() {
    for &(m, n, k) in DIMS {
        let src = gemm_program(m, n, k);
        let vm = vm_outcome(&src).output.trim().to_string();
        assert!(!vm.is_empty(), "the VM must produce a checksum");

        let mut ran = vec!["tree-walker"];
        assert_eq!(
            tw_outcome(&src).output.trim(),
            vm,
            "({m}×{n}×{k}) tree-walker disagrees"
        );

        if have_riscv() {
            let asm = compile_to_riscv(&src).expect("riscv compile");
            assert_eq!(
                riscv_run(&asm, "agree"),
                vm,
                "({m}×{n}×{k}) RISC-V disagrees"
            );
            ran.push("riscv");
        }
        if have_nvcc() && have_gpu() {
            let (_s, cu) = compile_to_cuda_matmul(&src).unwrap();
            assert_eq!(cuda_run(&cu, "agree"), vm, "({m}×{n}×{k}) CUDA disagrees");
            ran.push("cuda");
        }
        eprintln!(
            "({m}×{n}×{k}) checksum {vm} agreed across: {}",
            ran.join(", ")
        );
    }
}
