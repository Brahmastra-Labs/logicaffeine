//! ★ THE C CRDT COUNTERS — PIT OF SUCCESS ★ (RED-first, TDD).
//!
//! LOGOS's `ConvergentCount` (a grow-only G-Counter) and `Tally` (a PN-Counter) are convergent CRDTs.
//! In a single-process AOT program they are a SINGLE-REPLICA in-memory value: a per-replica map of
//! counts whose OBSERVABLE is the SUM, with an idempotent per-replica-MAX merge. For a lone replica
//! that behaves like an ordinary monotone counter — `Increase … by n` bumps this replica's own slot,
//! `Decrease` (Tally only) bumps it negative, reading the field SUMS the map, and `Merge b into a`
//! takes the per-replica MAX (so re-merging the same state never double-counts, matching the VM).
//!
//! This is the EXACT semantics the tree-walker (`crate::semantics::crdt::CrdtValue`) and the RISC-V AOT
//! tier (`codegen_riscv::runtime`'s `logos_crdt_*`) already run; the C backend is brought to parity here.
//! The C runtime uses ONLY the heap + integer ops (a `logos_crdt` per-replica map, malloc/realloc, no
//! host-only libc), so the CUDA tier — which tree-shakes + lifts `codegen_c` — can lift it verbatim.
//!
//! Discipline (mirrors `c_temporal_lock` + `riscv_aot_lock`'s CRDT half):
//! - The oracle pair (`tw_outcome` + `vm_outcome`) must AGREE on every corpus program (so the target
//!   output is unambiguous).
//! - A `Supported` program MUST compile to C and run byte-for-byte identical to that oracle.
//! - `SUPPORTED_FLOOR` only rises; `KNOWN_GAPS` only shrinks.
//!
//! ⚠️ RED tests are the spec: if a `Supported` program fails ≡VM, fix `codegen_c`, NEVER weaken this
//! file. The execution half is gated on `gcc` and SKIPS when absent; the oracle-agreement + ratchet
//! always run.
#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};

use logicaffeine_compile::compile::{compile_to_c, tw_outcome, vm_outcome};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    /// Lowered to C + proven byte-for-byte ≡ the VM/tree-walker oracle.
    Supported,
    /// Not yet lowered — a documented, shrink-only gap.
    #[allow(dead_code)]
    Gap,
}

/// The CRDT-counter corpus + its per-program status. Each program creates a counter, mutates it, and
/// `Show`s the observable — the C output must equal the oracle byte-for-byte.
const CORPUS: &[(&str, &str, Status)] = &[
    // A grow-only `ConvergentCount`: two increments accumulate in this replica's slot, the read SUMS
    // the map → 5 + 3 = 8.
    (
        "counter_add_show",
        "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## Main\n    \
         Let mutable c be a new Counter.\n    Increase c's points by 5.\n    Increase c's points by 3.\n    \
         Show c's points.\n",
        Status::Supported,
    ),
    // A `Tally` (PN-Counter) tracks BOTH directions: 10 up, 4 down → 6. `Decrease` is the increment/
    // decrement bias a Tally has and a ConvergentCount refuses.
    (
        "tally_inc_dec",
        "## Definition\nA Wallet is Shared and has:\n    balance: Tally.\n## Main\n    \
         Let mutable w be a new Wallet.\n    Increase w's balance by 10.\n    Decrease w's balance by 4.\n    \
         Show w's balance.\n",
        Status::Supported,
    ),
    // The idempotent lattice JOIN: `Merge b into a` takes the per-replica MAX, so two disjoint replicas
    // sum (5 + 3 = 8) and a REPEATED merge is a no-op (still 8 — never double-counts, exactly the VM).
    (
        "counter_merge",
        "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## Main\n    \
         Let mutable a be a new Counter.\n    Increase a's points by 5.\n    \
         Let mutable b be a new Counter.\n    Increase b's points by 3.\n    \
         Merge b into a.\n    Merge b into a.\n    Show a's points.\n",
        Status::Supported,
    ),
    // Two DISTINCT counter types in one program (a ConvergentCount and a Tally), each read as a plain
    // Int — the counter handles never alias across struct types.
    (
        "two_counter_types",
        "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n\
         A Wallet is Shared and has:\n    balance: Tally.\n## Main\n    \
         Let mutable c be a new Counter.\n    Increase c's points by 5.\n    Increase c's points by 3.\n    \
         Show c's points.\n    \
         Let mutable w be a new Wallet.\n    Increase w's balance by 10.\n    Decrease w's balance by 4.\n    \
         Show w's balance.\n",
        Status::Supported,
    ),
    // Four increments on one replica accumulate monotonically in its own slot → 1+2+3+4 = 10.
    (
        "counter_many_adds",
        "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## Main\n    \
         Let mutable c be a new Counter.\n    Increase c's points by 1.\n    Increase c's points by 2.\n    \
         Increase c's points by 3.\n    Increase c's points by 4.\n    Show c's points.\n",
        Status::Supported,
    ),
    // A `Tally` can go NEGATIVE (unlike a grow-only ConvergentCount): 3 up, 10 down → -7.
    (
        "tally_goes_negative",
        "## Definition\nA Wallet is Shared and has:\n    balance: Tally.\n## Main\n    \
         Let mutable w be a new Wallet.\n    Increase w's balance by 3.\n    Decrease w's balance by 10.\n    \
         Show w's balance.\n",
        Status::Supported,
    ),
];

/// The count of `Supported` CRDT-counter programs — a MONOTONE FLOOR. RAISE it as coverage grows; NEVER
/// lower.
const SUPPORTED_FLOOR: usize = 6;

/// The EXACT set of currently-`Gap` program labels — a shrink-only ledger. The counter CRDTs are fully
/// lowered on host-C, so the frontier is EMPTY. Moving a program to `Supported` requires removing it here
/// AND raising the floor.
const KNOWN_GAPS: &[&str] = &[];

fn have_gcc() -> bool {
    std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Compile a program to C, build with gcc, run, and return trimmed stdout. `None` if the C fails to
/// compile or the binary fails to run (a `Supported` program must do both).
fn c_output(src: &str, label: &str) -> Option<String> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let c = compile_to_c(src).ok()?;
    let dir = std::env::temp_dir().join(format!(
        "logos_ccrdt_{}_{}_{}",
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
            "gcc failed for `{label}`:\n{}",
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

// ============================================================
//  Static ratchet — the pit-of-success census
// ============================================================

#[test]
fn the_crdt_oracles_agree_on_every_program() {
    // The tree-walker and VM must agree on every corpus program, so "≡ VM" and "≡ tree-walker" name the
    // same unambiguous target.
    for (label, src, _) in CORPUS {
        let tw = tw_outcome(src);
        let vm = vm_outcome(src);
        assert_eq!(
            (tw.output.trim(), tw.error.is_some()),
            (vm.output.trim(), vm.error.is_some()),
            "CRDT oracle divergence on `{label}` — tw={:?} vm={:?}",
            tw.output,
            vm.output,
        );
        assert!(
            tw.error.is_none(),
            "`{label}` oracle errored: {:?}",
            tw.error
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
    assert!(
        new_gaps.is_empty(),
        "NEW CRDT gap(s) not in KNOWN_GAPS: {new_gaps:?} — lower them or list them consciously"
    );
    assert!(
        closed_but_listed.is_empty(),
        "KNOWN_GAPS lists now-Supported program(s): {closed_but_listed:?} — remove them and raise SUPPORTED_FLOOR"
    );
}

#[test]
fn the_supported_count_never_regresses() {
    let n = CORPUS
        .iter()
        .filter(|(_, _, s)| *s == Status::Supported)
        .count();
    assert!(
        n >= SUPPORTED_FLOOR,
        "C-CRDT supported count fell to {n} (floor {SUPPORTED_FLOOR}) — fix codegen_c, do not lower the floor"
    );
}

// ============================================================
//  Behavioural — the ≡VM biconditional (gated on gcc)
// ============================================================

#[test]
fn every_supported_crdt_program_runs_equiv_vm_on_c() {
    if !have_gcc() {
        eprintln!("SKIP c-crdt execution: gcc not installed");
        return;
    }
    for (label, src, status) in CORPUS {
        if *status != Status::Supported {
            continue;
        }
        let want = tw_outcome(src).output.trim().to_string();
        let got = c_output(src, label).unwrap_or_else(|| {
            panic!("`{label}` is Supported but its C failed to compile/run — implement the CRDT runtime in codegen_c")
        });
        assert_eq!(
            got, want,
            "`{label}`: C CRDT output must equal the VM oracle (want {want:?}, got {got:?})"
        );
    }
}
