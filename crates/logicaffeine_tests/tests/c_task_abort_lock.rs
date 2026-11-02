//! ★ THE C TASK-ABORT (`Stop h`) — PIT OF SUCCESS ★ (RED-first, TDD).
//!
//! `Stop h` cancels a spawned task (Op::TaskAbort): the scheduler marks it aborted, its post-`Receive` code
//! never runs, and a later `Send` to its channel does NOT wake it (lazy teardown — byte-for-byte the VM's
//! `VmBlock::Abort`). This lock proves the AOT-C ucontext backend lowers `Stop` FAITHFULLY. Before the fix
//! `codegen_c` silently NO-OP'd the statement, so an `abort`-then-`Send` wrongly RESUMED the "aborted" task
//! in C — a real C≠VM divergence. The `abort_then_send` program below is the DISTINGUISHING witness: its
//! output is "main" iff the abort actually took (a no-op would leak "got").
//!
//! Discipline (mirrors `c_temporal_lock`):
//! - The oracle pair (`tw_outcome` + `vm_outcome`) AGREES on every corpus program.
//! - Each program compiles to C and runs byte-for-byte ≡ that oracle under gcc.
//!
//! Scope: the HOST-C / Tenstorrent ucontext tier (both use the stackful scheduler, which has
//! `logos_task_abort`). The CUDA tier lowers concurrency via the STACKLESS transform, which does not yet
//! serve `Stop` (it is a typed refusal there — never a silent miscompile); lifting it to the GPU is a
//! separate stackless-scheduler extension (lazy-teardown across dispatch/channels/timers).
//!
//! ⚠️ RED tests are the spec: if a program fails ≡VM, fix `codegen_c`, NEVER weaken this file. The
//! execution half is gated on `gcc` and SKIPS when absent; the oracle-agreement always runs.
#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::{AtomicUsize, Ordering};

use logicaffeine_compile::compile::{
    compile_to_c, tw_outcome, vm_outcome_concurrent as vm_outcome,
};

/// Stop a task blocked on `Receive` — its post-receive `Show "got"` never runs; only "main" prints.
/// (This alone does NOT distinguish the fix from a no-op: nothing sends to `ch`, so the parked task never
/// resumes either way. `ABORT_THEN_SEND` is the discriminator.)
const ABORT_BLOCKED: &str = "## To blocker (ch: Int):\n    Receive x from ch.\n    Show \"got\".\n\n\
## Main\n    Let ch be a Pipe of Int.\n    Let h be Launch a task to blocker with ch.\n    Stop h.\n    Show \"main\".\n";

/// The DISCRIMINATOR: abort the blocked task, THEN send to its channel. A faithful abort drops the aborted
/// receiver (VM), so "got" never prints → "main". A no-op abort would wake it → "got" leaks.
const ABORT_THEN_SEND: &str = "## To blocker (ch: Int):\n    Receive x from ch.\n    Show \"got\".\n\n\
## Main\n    Let ch be a Pipe of Int.\n    Let h be Launch a task to blocker with ch.\n    Stop h.\n    Send 9 into ch.\n    Show \"main\".\n";

const CORPUS: &[(&str, &str)] = &[
    ("abort_blocked", ABORT_BLOCKED),
    ("abort_then_send", ABORT_THEN_SEND),
];

fn have_gcc() -> bool {
    std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Compile to C, build with gcc, run; trimmed stdout (`None` if compile or run fails).
fn c_output(src: &str, label: &str) -> Option<String> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let c = compile_to_c(src).ok()?;
    let dir = std::env::temp_dir().join(format!(
        "logos_ctaskabort_{}_{}_{}",
        std::process::id(),
        label,
        n
    ));
    let _ = std::fs::create_dir_all(&dir);
    let cfile = dir.join("prog.c");
    let bin = dir.join("prog");
    std::fs::write(&cfile, &c).ok()?;
    let build = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&cfile)
        .arg("-lm")
        .output()
        .ok()?;
    if !build.status.success() {
        eprintln!(
            "gcc failed for {label}:\n{}",
            String::from_utf8_lossy(&build.stderr)
        );
        return None;
    }
    let run = std::process::Command::new(&bin).output().ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    if !run.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&run.stdout).trim().to_string())
}

#[test]
fn the_abort_oracles_agree() {
    for (label, src) in CORPUS {
        let tw = tw_outcome(src);
        let vm = vm_outcome(src);
        assert_eq!(
            (tw.output.trim(), tw.error.is_some()),
            (vm.output.trim(), vm.error.is_some()),
            "`{label}`: the tree-walker and VM oracles must agree",
        );
    }
    // The discriminator's target is unambiguous: the aborted task never resumes.
    let vm = vm_outcome(ABORT_THEN_SEND);
    assert!(
        vm.error.is_none(),
        "abort_then_send must not error in the oracle"
    );
    assert_eq!(
        vm.output.trim(),
        "main",
        "the aborted task's post-receive `Show` must NOT run"
    );
    assert!(
        !vm.output.contains("got"),
        "a faithful abort drops the receiver — `got` must not print"
    );
}

#[test]
fn emitted_c_lowers_the_abort() {
    // Anti-floor-gaming: `Stop` really lowers to the abort call, not a silent `/* unsupported stmt */`.
    let c = compile_to_c(ABORT_THEN_SEND).expect("compile");
    assert!(
        c.contains("logos_task_abort("),
        "`Stop h` must lower to logos_task_abort"
    );
    assert!(
        !c.contains("/* unsupported stmt */"),
        "no statement in the corpus should be unsupported"
    );
}

#[test]
fn c_backend_matches_the_oracle_on_every_abort_program() {
    if !have_gcc() {
        eprintln!("skip: gcc absent");
        return;
    }
    for (label, src) in CORPUS {
        let want = vm_outcome(src).output.trim().to_string();
        let got = c_output(src, label).unwrap_or_else(|| panic!("`{label}`: C must compile + run"));
        assert_eq!(
            got, want,
            "`{label}`: the C backend must match the VM oracle byte-for-byte"
        );
    }
}
