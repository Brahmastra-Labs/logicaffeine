//! tensor_sugar_all_backends_lock — the Phase-2 tensor sugar runs on EVERY backend we ship.
//!
//! `Let A be a R by C tensor of Int` / `Let C be A matmul B` desugar (in the parser) to ordinary
//! `Seq` + loop + `Show` statements — primitives every tier already agrees on — so the SAME source
//! produces the SAME answer everywhere. Two honest classes of claim, both locked here:
//!
//!   • **Portable execution (≡ the VM, byte-for-byte on real data):** the whole-program compilers run the
//!     actual `Show`s — tree-walker · VM · AOT-C (gcc) · RISC-V (gcc + qemu) · AOT-Rust (cargo). Each emits
//!     `19 22 43 50` for the 2×2 product. (WASM lowers cleanly here; its `Seq`-program execution parity is
//!     the domain of `wasm_aot_lock`, whose 150-line host-import harness we don't duplicate.)
//!   • **Accelerator recognition (→ the optimized kernel):** the sugar desugars to the exact GEMM idiom the
//!     backend-agnostic `recognize_matmul` lifts, so the CUDA and Tenstorrent backends RECOGNIZE it and emit
//!     their optimized kernels (the emitter extracts the dims; Phase 1's locks prove the kernel's numerics on
//!     the synthetic-fill programs). This is "readable LOGOS → the fast kernel" for the accelerators.
//!
//! Backend legs self-skip when their toolchain is absent, so this runs anywhere (and fully in CI).
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(tensor_sugar_all_backends_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_riscv::RISCV_RUNTIME_C;
use logicaffeine_compile::compile::{
    compile_to_c, compile_to_cuda_matmul, compile_to_cuda_tensor_op, compile_to_riscv,
    compile_to_tenstorrent_matmul, compile_to_tenstorrent_tensor_op, compile_to_wasm,
    run_logos_source, tw_outcome, vm_outcome,
};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A 2×2 · 2×2 matmul in tensor SUGAR that `Show`s each of C's four entries (real data, so the whole-program
/// compilers all print `19 22 43 50`). `A=[[1,2],[3,4]]`, `B=[[5,6],[7,8]]`.
const SUGAR: &str = r#"## Main
Let A be a 2 by 2 tensor of Int.
Set item 1 of A to 1.
Set item 2 of A to 2.
Set item 3 of A to 3.
Set item 4 of A to 4.
Let B be a 2 by 2 tensor of Int.
Set item 1 of B to 5.
Set item 2 of B to 6.
Set item 3 of B to 7.
Set item 4 of B to 8.
Let C be A matmul B.
Show item 1 of C.
Show item 2 of C.
Show item 3 of C.
Show item 4 of C.
"#;

/// The product, whitespace-normalized — the single oracle every backend is held to.
fn want() -> Vec<String> {
    vm_outcome(SUGAR)
        .output
        .split_whitespace()
        .map(str::to_string)
        .collect()
}
fn norm(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_string).collect()
}

// ---- toolchain gates -----------------------------------------------------------------------------------
fn have(cmd: &str) -> bool {
    std::process::Command::new(cmd)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
fn have_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
}

// ---- whole-program executors (real Show output) --------------------------------------------------------
fn c_run(program: &str) -> String {
    let c = compile_to_c(program).expect("sugar must lower to C");
    let dir = std::env::temp_dir().join(format!("logos_tsab_c_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (src, bin) = (dir.join("out.c"), dir.join("out"));
    std::fs::write(&src, &c).unwrap();
    let compiled = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&src)
        .arg("-lm")
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "emitted C must compile:\n{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let r = std::process::Command::new(&bin).output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&r.stdout).to_string()
}

fn riscv_run(program: &str) -> String {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let asm = compile_to_riscv(program).expect("sugar must lower to RISC-V");
    let dir = std::env::temp_dir().join(format!("logos_tsab_rv_{}_{n}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (a, rt, bin) = (dir.join("main.s"), dir.join("logos_rt.c"), dir.join("main"));
    std::fs::write(&a, &asm).unwrap();
    std::fs::write(&rt, RISCV_RUNTIME_C).unwrap();
    let c = std::process::Command::new("riscv64-linux-gnu-gcc")
        .args(["-O2", "-static", "-o"])
        .arg(&bin)
        .arg(&a)
        .arg(&rt)
        .arg("-lm")
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "emitted RISC-V must assemble:\n{}",
        String::from_utf8_lossy(&c.stderr)
    );
    let r = std::process::Command::new("qemu-riscv64")
        .arg(&bin)
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&r.stdout).to_string()
}

#[test]
fn sugar_runs_on_the_tree_walker_and_vm() {
    assert_eq!(norm(&tw_outcome(SUGAR).output), want(), "tree-walker");
    assert_eq!(
        want(),
        ["19", "22", "43", "50"],
        "the product itself (VM oracle)"
    );
}

#[test]
fn sugar_runs_on_the_c_backend() {
    if !have("gcc") {
        eprintln!("SKIP C: no gcc.");
        return;
    }
    assert_eq!(
        norm(&c_run(SUGAR)),
        want(),
        "AOT-C ≡ VM on the tensor sugar"
    );
}

#[test]
fn sugar_runs_on_riscv() {
    if !(have("riscv64-linux-gnu-gcc") && have("qemu-riscv64")) {
        eprintln!("SKIP RISC-V: no cross toolchain.");
        return;
    }
    assert_eq!(
        norm(&riscv_run(SUGAR)),
        want(),
        "RISC-V (qemu) ≡ VM on the tensor sugar"
    );
}

#[test]
fn sugar_runs_on_aot_rust() {
    // The AOT-Rust tier: emit a Rust program, build it with cargo, run it. Heavier (a real build), but the
    // genuine native tier — no shortcut. Skips only if the emit itself fails to produce a project.
    match run_logos_source(SUGAR) {
        Ok(out) => assert_eq!(norm(&out), want(), "AOT-Rust ≡ VM on the tensor sugar"),
        Err(e) => panic!("AOT-Rust must build+run the tensor sugar: {e}"),
    }
}

#[test]
fn sugar_lowers_to_wasm() {
    // The WASM backend accepts and lowers the desugared sugar (Seq + loops + Show are all supported ops);
    // full Seq-program execution parity on WASM is locked by wasm_aot_lock.
    assert!(
        compile_to_wasm(SUGAR).is_ok(),
        "the WASM backend must lower the tensor sugar"
    );
}

/// An elementwise `plus` program in tensor sugar (2×2), `Show`ing C's four entries.
const SUGAR_PLUS: &str = r#"## Main
Let A be a 2 by 2 tensor of Int.
Set item 1 of A to 1.
Set item 2 of A to 2.
Set item 3 of A to 3.
Set item 4 of A to 4.
Let B be a 2 by 2 tensor of Int.
Set item 1 of B to 5.
Set item 2 of B to 6.
Set item 3 of B to 7.
Set item 4 of B to 8.
Let C be A plus B.
Show item 1 of C.
Show item 2 of C.
Show item 3 of C.
Show item 4 of C.
"#;

#[test]
fn sugar_is_recognized_by_the_accelerator_backends() {
    // The payoff for the accelerators: readable LOGOS → the optimized kernel. The matmul sugar is the exact
    // GEMM idiom recognize_matmul lifts; the elementwise sugar is the idiom recognize_elementwise lifts. Both
    // CUDA and Tenstorrent recognize BOTH ops and emit their optimized kernels — no missing cells.
    let (cu_spec, _cu) =
        compile_to_cuda_matmul(SUGAR).expect("CUDA backend must recognize the sugar's GEMM");
    assert_eq!(
        (cu_spec.m, cu_spec.n, cu_spec.k),
        (2, 2, 2),
        "CUDA recovered the dims from the sugar"
    );
    let (tt_spec, _tt) = compile_to_tenstorrent_matmul(SUGAR)
        .expect("Tenstorrent backend must recognize the sugar's GEMM");
    assert_eq!(
        (tt_spec.m, tt_spec.n, tt_spec.k),
        (2, 2, 2),
        "Tenstorrent recovered the dims from the sugar"
    );
    // Elementwise: both accelerators recognize `plus` and emit a real kernel (contains the elementwise op).
    let cu_ew =
        compile_to_cuda_tensor_op(SUGAR_PLUS).expect("CUDA must recognize the elementwise sugar");
    assert!(
        cu_ew.contains("c[p] = a[p] + b[p]"),
        "CUDA emitted the elementwise-add kernel"
    );
    let tt_ew = compile_to_tenstorrent_tensor_op(SUGAR_PLUS)
        .expect("Tenstorrent must recognize the elementwise sugar");
    assert!(
        tt_ew.contains("add_tiles"),
        "Tenstorrent emitted the elementwise-add tile kernel"
    );
}

#[test]
fn the_elementwise_cuda_kernel_runs_and_is_correct_on_the_gpu() {
    // Parity with the matmul GPU test: the emitted elementwise kernel COMPILES with nvcc and runs on the
    // RTX 3060, and its checksum equals the emitter's synthetic-fill semantics (a[p]=(p%7)-3, b[p]=(p%5)-2,
    // c[p]=a[p]+b[p], Σc over the recognized N=4). Verifies the emitter is correct, not merely recognized.
    if !(have("nvcc") && have_gpu()) {
        eprintln!("SKIP elementwise CUDA GPU: needs nvcc + /dev/nvidia0.");
        return;
    }
    let cu = compile_to_cuda_tensor_op(SUGAR_PLUS).unwrap();
    let expected: i64 = (0..4).map(|p| ((p % 7) - 3) + ((p % 5) - 2)).sum();
    let dir = std::env::temp_dir().join(format!("logos_tsab_ew_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (s, b) = (dir.join("e.cu"), dir.join("e"));
    std::fs::write(&s, &cu).unwrap();
    let c = std::process::Command::new("nvcc")
        .args(["-O2", "-arch=sm_86", "-o"])
        .arg(&b)
        .arg(&s)
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "elementwise CUDA must compile:\n{}\n\n{cu}",
        String::from_utf8_lossy(&c.stderr)
    );
    let r = std::process::Command::new(&b).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&r.stdout).trim(),
        expected.to_string(),
        "elementwise GPU checksum"
    );
}

#[test]
fn the_optimized_cuda_kernel_from_sugar_runs_on_the_gpu() {
    // And it actually RUNS on the GPU: the kernel the sugar's recognized GEMM lowers to compiles with nvcc
    // and executes on the RTX 3060. (Its checksum reflects the emitter's synthetic fill — the dims are what
    // flow from the sugar; Phase 1's cuda_matmul_lowering_lock proves the numeric equivalence.)
    if !(have("nvcc") && have_gpu()) {
        eprintln!("SKIP CUDA GPU: needs nvcc + /dev/nvidia0.");
        return;
    }
    let (_spec, cu) = compile_to_cuda_matmul(SUGAR).unwrap();
    let dir = std::env::temp_dir().join(format!("logos_tsab_cu_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (s, b) = (dir.join("m.cu"), dir.join("m"));
    std::fs::write(&s, &cu).unwrap();
    let c = std::process::Command::new("nvcc")
        .args(["-O2", "-arch=sm_86", "-o"])
        .arg(&b)
        .arg(&s)
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "sugar's CUDA kernel must compile:\n{}",
        String::from_utf8_lossy(&c.stderr)
    );
    let r = std::process::Command::new(&b).output().unwrap();
    assert!(
        r.status.success() && !r.stdout.is_empty(),
        "sugar's CUDA kernel must run on the GPU"
    );
}
