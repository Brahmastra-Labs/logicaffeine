//! ★ THE TENSTORRENT AOT PIT OF SUCCESS ★ — the authoritative behavioural lock for the
//! LOGOS → Tenstorrent (TT-Metalium) tier, the sibling of `riscv_aot_lock.rs`.
//!
//! The Tenstorrent tier runs a whole LOGOS program on ONE Tensix baby RISC-V core. Its compute
//! body is the SAME self-contained C the C backend emits — byte-for-byte ≡ the VM. This file
//! proves three things over one corpus, WITHOUT needing a card:
//!
//! 1. **Compute ≡ VM (executable, now).** For every on-core corpus program, the emitted
//!    `oracle_c` — the exact compute the kernel runs — compiles with `gcc` and runs byte-for-byte
//!    identical to the VM (and to the tree-walker). This is the correctness core; it needs only gcc.
//! 2. **Well-formed TT-Metalium artifact.** The emitted kernel carries the `kernel_main` entry,
//!    the DPRINT show-bridge, the renamed compute entry, and no leftover glibc includes; the host
//!    carries the `MeshDevice`/`RISCV_0` data-movement API surface; the CMake needs `TT_METAL_HOME`.
//! 3. **Sound refusal.** A program using CONCURRENCY (channels/tasks/select — a `ucontext`
//!    scheduler + multiple contexts, off a single baby core) is REFUSED with a specific reason —
//!    never miscompiled. (Offline networking is instead ACCEPTED: its C runtime is newlib-clean and
//!    wraps verbatim, proven by the `net_loopback` on-core corpus entry.)
//!
//! An `#[ignore]`d on-hardware test builds the host+kernel with tt-metal and runs it on a Tensix
//! core when `TT_METAL_HOME` (a built SDK) is present — the Phase-5 gate. It fabricates nothing:
//! with no SDK it does not run.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. If a lock goes RED, fix `codegen_tt`, never the assertion.

use std::sync::atomic::{AtomicUsize, Ordering};

use logicaffeine_compile::codegen_tt::KERNEL_FILE_NAME;
use logicaffeine_compile::compile::{
    compile_to_tenstorrent, tw_outcome, vm_outcome, vm_outcome_net,
};

// ============================================================
//  Corpus
// ============================================================

/// On-core programs: each MUST emit, its `oracle_c` compute run byte-for-byte ≡ the VM, and its
/// kernel be well-formed. A representative scalar/text/seq/struct spread (a superset lands in Phase 2).
const CORPUS: &[(&str, &str)] = &[
    ("show_int", "## Main\n    Show 7.\n"),
    ("multi_show", "## Main\n    Show 1.\n    Show 2.\n    Show 3.\n"),
    ("sq_call", "## To sq (x: Int) -> Int:\n    Return x * x.\n## Main\n    Show sq(9).\n"),
    (
        "sumsq",
        "## To sq (x: Int) -> Int:\n    Return x * x.\n\
         ## To sumsq (a: Int) (b: Int) -> Int:\n    Return sq(a) + sq(b).\n\
         ## Main\n    Show sumsq(3, 4).\n",
    ),
    (
        "while_tri",
        "## To tri (n: Int) -> Int:\n    Let acc be 0.\n    Let i be n.\n    \
         While i is greater than 0:\n        Set acc to acc + i.\n        Set i to i - 1.\n    \
         Return acc.\n## Main\n    Show tri(100).\n",
    ),
    (
        "fib",
        "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    \
         Return fib(n - 1) + fib(n - 2).\n## Main\n    Show fib(10).\n",
    ),
    (
        "divmod",
        "## To dm (a: Int) (b: Int) -> Int:\n    Let q be a / b.\n    Let r be a % b.\n    Return q + r.\n\
         ## Main\n    Show dm(17, 5).\n",
    ),
    (
        "bitwise",
        "## To bits (a: Int) (b: Int) -> Int:\n    Let x be a & b.\n    Let y be a | b.\n    \
         Let z be a xor b.\n    Return x + y + z.\n## Main\n    Show bits(12, 10).\n",
    ),
    ("text_show", "## Main\n    Show \"hello\".\n    Show \"world\".\n"),
    ("text_interp_int", "## Main\n    Let n be 42.\n    Show \"n is {n}\".\n"),
    (
        "seq_build_sum",
        "## Main\n    Let mutable xs be a new Seq of Int.\n    Push 10 to xs.\n    Push 20 to xs.\n    \
         Push 30 to xs.\n    Show xs.\n    Show length of xs.\n    Show item 2 of xs.\n    \
         Let mutable t be 0.\n    Repeat for x in xs:\n        Set t to t + x.\n    Show t.\n",
    ),
    (
        "struct_point",
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet mutable p be a new Point with x 10 and y 20.\n\
         Show p.\nSet p's x to 5.\nShow p's x.\nShow p.\n",
    ),
];

/// OFFLINE-NETWORKING loopback programs — now ACCEPTED on-core (see `net_loopback_lifts_...` below).
/// The offline model needs the net-driving VM oracle (`vm_outcome_net`), not the plain `vm_outcome`
/// the scalar CORPUS uses, so these live in a dedicated corpus. `Connect`/`Listen` establish the
/// inbox; `Send X to peer; Await … into y` round-trips through the per-address FIFO on one node.
const NET_CORPUS: &[(&str, &str)] = &[(
    "net_scalar",
    "## Main\n    Connect to \"ws://127.0.0.1:9\".\n    Listen on \"me\".\n    \
     Let peer be a PeerAgent at \"me\".\n    Send 7 to peer.\n    \
     Await response from peer into got.\n    Show got.\n",
)];

/// A program that ERRORS at runtime on the VM (divide by zero). The oracle must exit NONZERO where
/// the VM errors. (Partial-output-before-error fidelity is a `codegen_c` buffering concern — a
/// SIGFPE drops the block-buffered stdout — tracked as a Phase-2 item, not asserted here.)
const ERROR_CORPUS: &[(&str, &str)] = &[(
    "div_zero",
    "## To dz (a: Int) (b: Int) -> Int:\n    Return a / b.\n## Main\n    Show 111.\n    Show dz(5, 0).\n",
)];

/// Programs using an op outside the single-baby-core fragment — the backend must REFUSE (return
/// `Err`), never emit. CONCURRENCY (channels/tasks/select need a cooperative scheduler + multiple
/// contexts; the C concurrency runtime is `ucontext`-based, which a baby core has no libc for) is
/// refused here — the genuine multi-baby-core MESH tier lowers it instead. OFFLINE networking is now
/// ACCEPTED (see the `net_loopback` on-core entry above), so it is no longer in this corpus.
const REFUSE_CORPUS: &[(&str, &str)] = &[(
    "producer_consumer",
    "## To produce (ch: Int):\n    Send 1 into ch.\n    Send 2 into ch.\n\n\
         ## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to produce with jobs.\n    \
         Receive a from jobs.\n    Receive b from jobs.\n    Show a.\n    Show b.\n",
)];

// ============================================================
//  Helpers
// ============================================================

fn have_gcc() -> bool {
    std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

struct COracle {
    stdout: String,
    success: bool,
}

/// Compile `oracle_c` with gcc and run it — the exact compute the Tensix kernel runs, exercised on
/// the host (the pre-hardware ≡VM proof).
fn compile_and_run_oracle(oracle_c: &str, label: &str) -> COracle {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("logos_tt_{}_{n}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let c_path = dir.join("oracle.c");
    let bin_path = dir.join("oracle");
    std::fs::write(&c_path, oracle_c).expect("write oracle.c");

    let c = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin_path)
        .arg(&c_path)
        .arg("-lm")
        .output()
        .expect("run gcc");
    assert!(
        c.status.success(),
        "[{label}] gcc failed to compile the oracle:\n{}\n\nOracle C:\n{oracle_c}",
        String::from_utf8_lossy(&c.stderr)
    );

    let r = std::process::Command::new(&bin_path)
        .output()
        .expect("run oracle binary");
    let out = COracle {
        stdout: String::from_utf8_lossy(&r.stdout).to_string(),
        success: r.status.success(),
    };
    let _ = std::fs::remove_dir_all(&dir);
    out
}

/// Every kernel/host/CMake structural invariant a well-formed TT-Metalium artifact must carry.
fn assert_wellformed(art: &logicaffeine_compile::codegen_tt::TtArtifact, label: &str) {
    let k = &art.kernel_cpp;
    assert!(
        k.contains("void kernel_main()"),
        "[{label}] kernel missing kernel_main entry"
    );
    assert!(
        k.contains("LOGOS_TT_SHOW_BRIDGE"),
        "[{label}] kernel missing DPRINT show-bridge"
    );
    assert!(
        k.contains("static int logos_main("),
        "[{label}] compute entry not renamed"
    );
    assert!(
        k.contains("logos_main(0, (char **)0)"),
        "[{label}] kernel_main does not invoke the compute"
    );
    assert!(
        !k.contains("#include <stdio.h>"),
        "[{label}] glibc include leaked into the kernel"
    );
    assert!(
        k.contains("#include \"debug/dprint.h\""),
        "[{label}] kernel missing tt-metal DPRINT header"
    );

    let h = &art.host_cpp;
    for needle in [
        "create_unit_mesh",
        "CreateProgram",
        "CreateKernel",
        "DataMovementProcessor::RISCV_0",
        "EnqueueMeshWorkload",
        KERNEL_FILE_NAME,
    ] {
        assert!(h.contains(needle), "[{label}] host missing `{needle}`");
    }
    assert!(
        art.cmake.contains("TT_METAL_HOME"),
        "[{label}] CMake does not require TT_METAL_HOME"
    );
    assert!(
        art.oracle_c.contains("int main("),
        "[{label}] oracle is not a self-contained C program"
    );
}

// ============================================================
//  1. Compute ≡ VM (executable, needs only gcc)
// ============================================================

#[test]
fn every_oncore_program_compute_matches_the_vm() {
    if !have_gcc() {
        eprintln!("SKIP: gcc not present — cannot run the Tenstorrent compute oracle");
        return;
    }
    for (label, src) in CORPUS {
        // The two oracles agree on visible output (`tw_outcome`/`vm_outcome` differ only by a
        // trailing newline, so compare trimmed — the same convention as `assert_c_output`).
        let tw = tw_outcome(src);
        let vm = vm_outcome(src);
        assert_eq!(
            tw.output.trim(),
            vm.output.trim(),
            "[{label}] tree-walker and VM disagree"
        );
        assert_eq!(
            tw.error.is_some(),
            vm.error.is_some(),
            "[{label}] oracle error-state disagreement"
        );

        let art = compile_to_tenstorrent(src).unwrap_or_else(|e| {
            panic!("[{label}] Tenstorrent emission failed: {e:?}\n\nSource:\n{src}")
        });
        assert_wellformed(&art, label);

        let run = compile_and_run_oracle(&art.oracle_c, label);
        assert!(
            run.success,
            "[{label}] oracle exited nonzero for a non-erroring program"
        );
        assert_eq!(
            run.stdout.trim(),
            vm.output.trim(),
            "[{label}] kernel compute (via the C oracle) diverged from the VM"
        );
    }
}

/// OFFLINE NETWORKING lifts onto the single Tensix baby core: the emitted kernel is well-formed, its
/// `oracle_c` really calls the loopback transport (`logos_net_send`/`logos_net_await`), and that
/// compute runs byte-for-byte ≡ the net-driving VM oracle (`vm_outcome_net`). The C net runtime is
/// newlib-clean (no `ucontext`), so it wraps verbatim — the same body the C/CUDA net tiers prove ≡VM.
#[test]
fn net_loopback_lifts_and_its_compute_matches_the_vm() {
    if !have_gcc() {
        eprintln!("SKIP: gcc not present — cannot run the Tenstorrent net oracle");
        return;
    }
    for (label, src) in NET_CORPUS {
        // The net-driving oracles agree, and the round-trip is observable (7 comes back).
        let tw = tw_outcome(src);
        let vm = vm_outcome_net(src);
        assert_eq!(
            tw.output.trim(),
            vm.output.trim(),
            "[{label}] tree-walker and offline-VM disagree"
        );
        assert_eq!(
            vm.output.trim(),
            "7",
            "[{label}] the loopback must round-trip the sent value"
        );

        let art = compile_to_tenstorrent(src).unwrap_or_else(|e| {
            panic!("[{label}] Tenstorrent net emission failed: {e:?}\n\nSource:\n{src}")
        });
        assert_wellformed(&art, label);
        // Anti-floor-gaming: the Net statements really lower to the transport, not a silent no-op.
        assert!(
            art.oracle_c.contains("logos_net_send("),
            "[{label}] Send must lower to logos_net_send"
        );
        assert!(
            art.oracle_c.contains("logos_net_await("),
            "[{label}] Await must lower to logos_net_await"
        );
        // And the on-core (kernel) body carries that transport too — it wraps the same C body.
        assert!(
            art.kernel_cpp.contains("logos_net_await("),
            "[{label}] kernel must carry the net transport"
        );

        let run = compile_and_run_oracle(&art.oracle_c, label);
        assert!(
            run.success,
            "[{label}] net oracle exited nonzero for a non-erroring program"
        );
        assert_eq!(
            run.stdout.trim(),
            vm.output.trim(),
            "[{label}] kernel net compute (via the C oracle) diverged from the offline VM"
        );
    }
}

#[test]
fn erroring_program_oracle_exits_nonzero_where_the_vm_errors() {
    if !have_gcc() {
        eprintln!("SKIP: gcc not present");
        return;
    }
    for (label, src) in ERROR_CORPUS {
        let vm = vm_outcome(src);
        assert!(
            vm.error.is_some(),
            "[{label}] ERROR_CORPUS program must error on the VM"
        );
        let art = compile_to_tenstorrent(src)
            .unwrap_or_else(|e| panic!("[{label}] emission failed: {e:?}"));
        let run = compile_and_run_oracle(&art.oracle_c, label);
        assert!(
            !run.success,
            "[{label}] oracle should exit nonzero where the VM errors"
        );
    }
}

// ============================================================
//  2. Sound refusal
// ============================================================

#[test]
fn concurrency_and_networking_are_refused_not_miscompiled() {
    for (label, src) in REFUSE_CORPUS {
        let got = compile_to_tenstorrent(src);
        assert!(
            got.is_err(),
            "[{label}] a program outside the single-baby-core fragment must be REFUSED, got Ok"
        );
    }
}

// ============================================================
//  3. On-hardware (Phase 5) — gated on a built tt-metal SDK
// ============================================================

/// Build the emitted host+kernel against a real tt-metal checkout and run it on a Tensix core,
/// comparing DPRINT output to the VM. SKIPS unless `TT_METAL_HOME` points at a built SDK, so no
/// on-silicon result is ever fabricated. Un-`ignore` it in Phase 5 (a card + SDK present).
#[test]
#[ignore = "requires a built tt-metal SDK (TT_METAL_HOME) and a Tensix card — Phase 5"]
fn on_hardware_kernel_output_matches_the_vm() {
    use logicaffeine_compile::codegen_tt;

    let Ok(_home) = std::env::var("TT_METAL_HOME") else {
        eprintln!("SKIP: TT_METAL_HOME not set — no tt-metal SDK to build against");
        return;
    };
    let (label, src) = CORPUS[0]; // "show_int" → "7"
    let vm = vm_outcome(src);
    let art = compile_to_tenstorrent(src).expect("emit bundle");

    // Write the bundle: kernel + host + CMake beside each other.
    let dir = std::env::temp_dir().join(format!("logos_tt_hw_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(codegen_tt::KERNEL_FILE_NAME), &art.kernel_cpp).unwrap();
    std::fs::write(dir.join("logos_tt_host.cpp"), &art.host_cpp).unwrap();
    std::fs::write(dir.join("CMakeLists.txt"), &art.cmake).unwrap();

    // Build against the SDK.
    let cfg = std::process::Command::new("cmake")
        .args(["-B", "build", "-S", "."])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        cfg.status.success(),
        "[{label}] cmake configure failed:\n{}",
        String::from_utf8_lossy(&cfg.stderr)
    );
    let bld = std::process::Command::new("cmake")
        .args(["--build", "build"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        bld.status.success(),
        "[{label}] cmake build failed:\n{}",
        String::from_utf8_lossy(&bld.stderr)
    );

    // Run on the Tensix core under the DPRINT server; the streamed `Show` output must equal the VM.
    let run = std::process::Command::new(dir.join("build/logos_tt_host"))
        .env("TT_METAL_DPRINT_CORES", "0,0")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "[{label}] on-card run failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let got = String::from_utf8_lossy(&run.stdout);
    // DPRINT prefixes lines with the core coord; compare the trailing `Show` payload.
    assert!(
        got.contains(vm.output.trim()),
        "[{label}] on-card DPRINT output {got:?} does not contain the VM output {:?}",
        vm.output.trim()
    );
    let _ = std::fs::remove_dir_all(&dir);
}
