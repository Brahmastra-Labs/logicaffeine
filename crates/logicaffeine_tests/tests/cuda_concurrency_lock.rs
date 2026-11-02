//! ★ CUDA CONCURRENCY — THE NON-BLOCKING PROGRAMS — PIT OF SUCCESS ★ (RED-first, TDD).
//!
//! A GPU thread has no `ucontext`/`swapcontext`, so the C tier's coroutine scheduler cannot run on the
//! device. Every concurrent program lowers through the stepped tier instead: the C tier's own lowering with
//! each task a step function over a heap frame, under the C family's scheduler core, placed on the device
//! whole — so a program schedules on the GPU exactly as it does on the host C tier. This lock holds the
//! programs that never block (a fire-and-forget worker, a launch with an argument, a producer buffering into
//! a pipe, the `Try` forms): each lowers, compiles with `nvcc`, carries no coroutine machinery, and runs on
//! the GPU printing what the tree-walker prints. `cuda_stackless_concurrency_lock` holds the programs that
//! block, and `c_family_scheduler_conformance_lock` the scheduler corpus at every seed.
//!
//! Discipline (mirrors `cuda_aot_lock`):
//! - Each corpus program: `compile_to_cuda` succeeds → the `.cu` compiles with `nvcc` → runs on the GPU →
//!   stdout equals the deterministic oracle (`interpret_for_ui_sync`).
//! - The **refuse corpus** is what the C tier itself refuses — each MUST be a typed refusal, never
//!   miscompiled.
//! - The nvcc/GPU halves SKIP when `nvcc` / `/dev/nvidia0` is absent (never a failure); the
//!   oracle-agreement + refusal halves always run.
//!
//! ⚠️ RED tests are the spec. If a program fails ≡VM on the GPU, fix the device path (`codegen_cuda`'s
//! device lift, the stepped tier), NEVER weaken this file.

use logicaffeine_compile::compile::{compile_to_c, compile_to_cuda};
use logicaffeine_compile::interpret_for_ui_sync;

// ============================================================
//  Corpus — the programs that never block
// ============================================================

/// The non-blocking determinate concurrency corpus — copied verbatim from `c_concurrency_scheduler_lock`.
/// Each program NEVER truly blocks: the producer buffers into a pipe then exits; the consumer drains
/// buffered values; a fire-and-forget worker just `Show`s.
const NON_BLOCKING_CORPUS: &[(&str, &str)] = &[
    (
        "fire_and_forget",
        "## To worker:\n    Show \"work\".\n\n## Main\n    Launch a task to worker.\n    Show \"main\".\n",
    ),
    (
        "spawn_with_arg",
        "## To greet (n: Int):\n    Show n.\n\n## Main\n    Launch a task to greet with 7.\n    Show 0.\n",
    ),
    (
        "producer_consumer",
        "## To produce (ch: Int):\n    Send 1 into ch.\n    Send 2 into ch.\n    Send 3 into ch.\n\n\
         ## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to produce with jobs.\n    \
         Receive a from jobs.\n    Receive b from jobs.\n    Receive c from jobs.\n    Show a + b + c.\n",
    ),
    // Non-blocking `Try to receive` on an EMPTY channel binds Nothing (`Show x` → `nothing`), and a
    // `Try to send` + `Try to receive` round-trips the value (→ `42`). The value-or-nothing is the C tier's
    // `CType::Opt` / `logos_opt`.
    (
        "try_recv_empty",
        "## Main\n    Let ch be a Pipe of Int.\n    Try to receive x from ch.\n    Show x.\n",
    ),
    (
        "try_send_recv",
        "## Main\n    Let ch be a Pipe of Int.\n    Try to send 42 into ch.\n    \
         Try to receive x from ch.\n    Show x.\n",
    ),
];

/// Concurrency programs the CUDA tier refuses — each MUST be a typed refusal, never miscompiled. The C tier
/// answers each with its own refusal written into its C, and the device lowers the same C, so the refusal is
/// the C tier's words.
///
/// NOTE: `sleep_virtual_time`, the determinate selects and `select_two_ready_recv_arms` all GRADUATED out of
/// this corpus: every concurrent program lowers through the stepped tier, whose scheduler breaks a tie
/// between two ready receive arms by the seed, as the runtime does. They are proven under the stronger
/// stdout-==-oracle biconditional in `cuda_stackless_concurrency_lock`.
const REFUSE: &[(&str, &str)] = &[
    (
        "pipe_of_records",
        "## A Point has:\n    an x, which is Int.\n\n## To consume (ch: Point):\n    Receive p from ch.\n    \
         Show p's x.\n\n## Main\n    Let ch be a Pipe of Point.\n    Launch a task to consume with ch.\n    \
         Let q be a new Point with x 1.\n    Send q into ch.\n",
    ),
];

// ============================================================
//  Static — oracle anchor, acceptance, refusal
// ============================================================

#[test]
fn the_oracle_pair_agrees_on_every_non_blocking_program() {
    // The target output must be unambiguous: the deterministic tree-walker is the oracle for the GPU
    // run below, so its output must be reproducible (no nondeterminate select in the corpus).
    for (label, src) in NON_BLOCKING_CORPUS {
        let tw = interpret_for_ui_sync(src);
        assert!(
            tw.error.is_none(),
            "`{label}`: the tree-walker oracle must not error: {:?}",
            tw.error
        );
        assert!(
            !tw.lines.is_empty(),
            "`{label}`: the oracle must produce output"
        );
    }
}

#[test]
fn the_non_blocking_corpus_compiles_to_cuda() {
    for (label, src) in NON_BLOCKING_CORPUS {
        assert!(
            compile_to_c(src).is_ok(),
            "`{label}`: the C oracle should compile"
        );
        assert!(
            compile_to_cuda(src).is_ok(),
            "`{label}`: a determinate concurrency program MUST lower to CUDA (the stepped tier)",
        );
    }
}

#[test]
fn the_cuda_is_wellformed_and_has_no_ucontext() {
    // The emitted `.cu` must be a well-formed single-file CUDA program AND must NOT carry any of the
    // ucontext machinery the coroutine tier emits (`ucontext_t`, `swapcontext`, `getcontext`, `makecontext`,
    // 262144-byte stacks) — nvcc cannot compile those. The stepped scheduler runs without them.
    for (label, src) in NON_BLOCKING_CORPUS {
        let art = compile_to_cuda(src)
            .unwrap_or_else(|e| panic!("`{label}` should compile to CUDA: {e:?}"));
        let cu = &art.kernel_cu;
        assert!(
            cu.contains("__global__ void logos_kernel()"),
            "`{label}`: kernel entry missing"
        );
        assert!(
            cu.contains("__device__ int logos_main(int argc, char **argv) {"),
            "`{label}`: device entry missing"
        );
        assert!(
            cu.contains("int main(int argc, char **argv) {"),
            "`{label}`: host orchestrator missing"
        );
        for banned in [
            "ucontext",
            "swapcontext",
            "getcontext",
            "makecontext",
            "LOGOS_STACK",
        ] {
            assert!(
                !cu.contains(banned),
                "`{label}`: the device program must NOT carry `{banned}` (nvcc cannot compile the ucontext scheduler)",
            );
        }
        assert!(
            art.makefile.contains("nvcc"),
            "`{label}`: Makefile must drive nvcc"
        );
    }
}

#[test]
fn unsupported_concurrency_is_refused() {
    for (label, src) in REFUSE {
        // The C backend answers these, writing its own refusal into its C — the device lowers the same C,
        // so its refusal is typed, never a program that fails at nvcc or runs wrong.
        assert!(
            compile_to_c(src).is_ok(),
            "`{label}`: the C oracle should answer"
        );
        assert!(
            compile_to_cuda(src).is_err(),
            "`{label}`: an unsupported concurrency program MUST be refused, never miscompiled",
        );
    }
}

// ============================================================
//  Behavioural — the ≡VM-on-GPU biconditional (gated on nvcc + GPU)
// ============================================================

fn have_nvcc() -> bool {
    std::process::Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A runnable GPU (the kernel driver is loaded). `/dev/nvidia0` exists exactly when the driver is up.
fn have_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
}

/// nvcc-compile a `.cu` to a binary WITHOUT running it — verifies the emitted CUDA is valid
/// `sm_86` code even on a box with no GPU driver.
fn nvcc_compiles(cu: &str, label: &str) {
    let dir = std::env::temp_dir().join(format!(
        "logos_cuda_conc_c_{}_{}",
        std::process::id(),
        label
    ));
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("logos_cuda.cu");
    let bin = dir.join("logos_cuda");
    std::fs::write(&src, cu).expect("write .cu");
    let compile = std::process::Command::new("nvcc")
        .args(["-O2", "-arch=sm_86", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .expect("run nvcc");
    assert!(
        compile.status.success(),
        "`{label}`: emitted CUDA must compile with nvcc.\nstderr:\n{}\n\n.cu:\n{cu}",
        String::from_utf8_lossy(&compile.stderr),
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_non_blocking_corpus_compiles_with_nvcc() {
    if !have_nvcc() {
        eprintln!("SKIP cuda concurrency compile: nvcc not installed");
        return;
    }
    for (label, src) in NON_BLOCKING_CORPUS {
        let art =
            compile_to_cuda(src).unwrap_or_else(|e| panic!("`{label}` should compile: {e:?}"));
        nvcc_compiles(&art.kernel_cu, label);
    }
}

/// Compile a single-file `.cu` with `nvcc`, run it, return trimmed stdout lines. Panics with the nvcc
/// stderr + source on failure (a supported program MUST compile AND run).
fn nvcc_compile_and_run(cu: &str, label: &str) -> Vec<String> {
    let dir =
        std::env::temp_dir().join(format!("logos_cuda_conc_{}_{}", std::process::id(), label));
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("logos_cuda.cu");
    let bin = dir.join("logos_cuda");
    std::fs::write(&src, cu).expect("write .cu");

    let compile = std::process::Command::new("nvcc")
        .args(["-O2", "-arch=sm_86", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .expect("run nvcc");
    assert!(
        compile.status.success(),
        "`{label}`: emitted CUDA must compile with nvcc.\nstderr:\n{}\n\n.cu:\n{cu}",
        String::from_utf8_lossy(&compile.stderr),
    );

    let run = std::process::Command::new(&bin)
        .output()
        .expect("run cuda binary");
    assert!(
        run.status.success(),
        "`{label}`: the CUDA binary should run.\nstderr:\n{}",
        String::from_utf8_lossy(&run.stderr),
    );
    let out: Vec<String> = String::from_utf8_lossy(&run.stdout)
        .lines()
        .map(|s| s.to_string())
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    out
}

/// ★ THE KEYSTONE ★ — the stepped scheduler runs each determinate concurrency program on the GPU
/// byte-for-byte identical to the deterministic tree-walker oracle.
#[test]
fn cuda_equals_tree_walker_on_the_gpu() {
    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP cuda concurrency execution: needs nvcc + a GPU driver (/dev/nvidia0). Compile is \
                   covered by the_non_blocking_corpus_compiles_with_nvcc; running awaits the driver.");
        return;
    }
    for (label, src) in NON_BLOCKING_CORPUS {
        let art =
            compile_to_cuda(src).unwrap_or_else(|e| panic!("`{label}` should compile: {e:?}"));
        let got = nvcc_compile_and_run(&art.kernel_cu, label);
        let want = interpret_for_ui_sync(src).lines;
        assert_eq!(
            got, want,
            "`{label}`: CUDA output must equal the deterministic scheduler oracle"
        );
    }
}
