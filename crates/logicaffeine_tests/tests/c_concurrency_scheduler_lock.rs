//! ★ THE C CONCURRENCY SCHEDULER — PIT OF SUCCESS ★ (RED-first, TDD).
//!
//! This lock DRIVES the deterministic offline scheduler into `codegen_c` (channels / tasks / select /
//! virtual-time), so that C, Tenstorrent, and — via the stackless lowering ([`concurrency`
//! knob](logicaffeine_compile::codegen_c::concurrency)) — CUDA all inherit concurrency. `codegen_c` STUBS
//! these ops today (`/* unsupported stmt */`), so the corpus below is RED until the scheduler lands.
//!
//! Discipline (mirrors `riscv_aot_lock`'s concurrency half):
//! - The **concurrent oracle pair** — `interpret_for_ui_sync` (tree-walker) + `run_vm_concurrent` (VM),
//!   sharing one deterministic scheduler — must AGREE on every corpus program (so the target output is
//!   unambiguous). Every program here is DETERMINATE (no >1-ready-select race), so its output is unique.
//! - A program classified `Supported` MUST compile to C and run byte-for-byte identical to that oracle.
//! - A program classified `Gap` is a documented, shrink-only frontier: `KNOWN_GAPS` must equal the live
//!   `Gap` census exactly, and `SUPPORTED_FLOOR` only rises — coverage can never silently regress.
//!
//! ⚠️ RED tests are the spec: if a `Supported` program fails ≡VM, fix `codegen_c`, NEVER weaken this file.
//! The execution half is gated on `gcc` and SKIPS when absent; the oracle-agreement + ratchet always run.

use std::collections::BTreeSet;

use logicaffeine_compile::compile::compile_to_c;
use logicaffeine_compile::{interpret_for_ui_sync, run_vm_concurrent};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    /// Lowered to C + proven byte-for-byte ≡ the deterministic concurrent oracle.
    Supported,
    /// Not yet lowered (codegen_c stubs it) — a documented, shrink-only gap.
    Gap,
}

/// The determinate concurrency corpus + its per-program status. Drive the frontier by moving a program
/// `Gap → Supported` (and raising `SUPPORTED_FLOOR`) once `codegen_c` runs it ≡VM.
const CORPUS: &[(&str, &str, Status)] = &[
    // The frontier the scheduler build drives to `Supported`, simplest first. `codegen_c` stubs these
    // today (supports 0 concurrency programs), so all are `Gap` — the honest baseline. Flip one to
    // `Supported` (and raise SUPPORTED_FLOOR) as the deterministic offline scheduler lands, RED→GREEN.
    (
        "fire_and_forget",
        "## To worker:\n    Show \"work\".\n\n## Main\n    Launch a task to worker.\n    Show \"main\".\n",
        Status::Supported,
    ),
    (
        "spawn_with_arg",
        "## To greet (n: Int):\n    Show n.\n\n## Main\n    Launch a task to greet with 7.\n    Show 0.\n",
        Status::Supported,
    ),
    (
        "producer_consumer",
        "## To produce (ch: Int):\n    Send 1 into ch.\n    Send 2 into ch.\n    Send 3 into ch.\n\n\
         ## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to produce with jobs.\n    \
         Receive a from jobs.\n    Receive b from jobs.\n    Receive c from jobs.\n    Show a + b + c.\n",
        Status::Supported,
    ),
    (
        "sleep_virtual_time",
        "## Main\n    Show 1.\n    Sleep 5.\n    Show 2.\n",
        Status::Supported,
    ),
    (
        // Determinate: the producer (spawn-to-front) sends 9 before Main selects, so the recv arm is
        // ready and wins → `9`. The Select STATEMENT lowers to the runtime's `logos_select_*` protocol
        // (reset → arm each branch in declaration order → wait → dispatch on the winning arm index).
        "select_recv_timeout",
        "## To produce (ch: Int):\n    Send 9 into ch.\n\n## Main\n    Let ch be a Pipe of Int.\n    \
         Launch a task to produce with ch.\n    Await the first of:\n        Receive v from ch:\n            \
         Show v.\n        After 100 seconds:\n            Show 0.\n",
        Status::Supported,
    ),
    (
        // Determinate the OTHER way: no producer ever sends, so the recv arm can never fire and the
        // TIMEOUT arm wins → `42`. This behaviorally locks the timeout tick lowering (`After 5 seconds`
        // = a 5·10⁹-nanos Duration → that tick count, matching the VM's `eval_select_timeout_ticks`) and
        // the arm-dispatch's non-zero index path — not just the recv-ready fast path.
        "select_timeout_wins",
        "## Main\n    Let ch be a Pipe of Int.\n    Await the first of:\n        Receive v from ch:\n            \
         Show v.\n        After 5 seconds:\n            Show 42.\n",
        Status::Supported,
    ),
];

const SUPPORTED_FLOOR: usize = 6;

/// The EXACT set of currently-`Gap` program labels — a shrink-only ledger. Channels + tasks +
/// virtual-time sleep + select all run ≡VM on host-C, so the frontier is empty. Moving a program to
/// `Supported` requires removing it here AND raising the floor.
const KNOWN_GAPS: &[&str] = &[];

fn have_gcc() -> bool {
    std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Compile a program to C via `codegen_c`, build with gcc, run, and return stdout lines. `None` if the C
/// fails to compile or the binary fails to run (a `Supported` program must do both).
fn c_lines(src: &str, label: &str) -> Option<Vec<String>> {
    let c = compile_to_c(src).ok()?;
    let dir = std::env::temp_dir().join(format!("logos_cconc_{}_{}", std::process::id(), label));
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
        return None;
    }
    let run = std::process::Command::new(&bin).output().ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    if !run.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&run.stdout)
            .lines()
            .map(|s| s.to_string())
            .collect(),
    )
}

// ============================================================
//  Static ratchet — the pit-of-success census
// ============================================================

#[test]
fn the_concurrent_oracles_agree_on_every_program() {
    // The target output must be unambiguous: the two deterministic drivers agree (else the program is
    // nondeterminate and does not belong in this corpus).
    for (label, src, _) in CORPUS {
        let tw = interpret_for_ui_sync(src);
        let vm = run_vm_concurrent(src);
        assert_eq!(
            (&tw.lines, tw.error.is_some()),
            (&vm.lines, vm.error.is_some()),
            "concurrent oracle divergence on `{label}` — not a determinate program: tw={:?} vm={:?}",
            tw.lines, vm.lines,
        );
    }
}

#[test]
fn the_gap_ledger_is_pinned_and_shrink_only() {
    let live: BTreeSet<&str> = CORPUS
        .iter()
        .filter(|(_, _, s)| *s == Status::Gap)
        .map(|(l, _, _)| *l)
        .collect();
    let declared: BTreeSet<&str> = KNOWN_GAPS.iter().copied().collect();
    assert_eq!(
        declared.len(),
        KNOWN_GAPS.len(),
        "KNOWN_GAPS has duplicate entries"
    );
    let new_gaps: Vec<_> = live.difference(&declared).collect();
    let closed_but_listed: Vec<_> = declared.difference(&live).collect();
    assert!(new_gaps.is_empty(), "NEW concurrency gap(s) not in KNOWN_GAPS: {new_gaps:?} — lower them or list them consciously");
    assert!(closed_but_listed.is_empty(), "KNOWN_GAPS lists now-Supported program(s): {closed_but_listed:?} — remove them and raise SUPPORTED_FLOOR");
}

#[test]
fn the_supported_count_never_regresses() {
    let n = CORPUS
        .iter()
        .filter(|(_, _, s)| *s == Status::Supported)
        .count();
    assert!(n >= SUPPORTED_FLOOR, "C-concurrency supported count fell to {n} (floor {SUPPORTED_FLOOR}) — fix codegen_c, do not lower the floor");
}

// ============================================================
//  Behavioural — the ≡VM biconditional (gated on gcc)
// ============================================================

#[test]
fn every_supported_concurrency_program_runs_equiv_vm_on_c() {
    if !have_gcc() {
        eprintln!("SKIP c-concurrency execution: gcc not installed");
        return;
    }
    for (label, src, status) in CORPUS {
        if *status != Status::Supported {
            continue;
        }
        let want = interpret_for_ui_sync(src).lines;
        let got = c_lines(src, label)
            .unwrap_or_else(|| panic!("`{label}` is Supported but its C failed to compile/run — implement the scheduler in codegen_c"));
        assert_eq!(
            got, want,
            "`{label}`: C concurrency output must equal the deterministic scheduler oracle"
        );
    }
}
