//! ★ THE C TEMPORAL CLOCK — PIT OF SUCCESS ★ (RED-first, TDD).
//!
//! `now` (a `Moment`, nanoseconds since the epoch) and `today` (a `Date`, whole days) are the two
//! wall-clock BUILTINS. They are nondeterministic by nature, so the whole differential rests on an
//! INJECTED FIXED CLOCK: the VM/tree-walker oracle reads it via [`set_fixed_clock`], and the AOT-C
//! binary reads the SAME instant from the `LOGOS_FIXED_NANOS` / `LOGOS_FIXED_DAYS` environment
//! variables the runtime shim (`logos_now` / `logos_today`) consults — byte-for-byte the RISC-V shim,
//! so the two AOT tiers cannot diverge. Under that shared instant the C output MUST equal the oracle
//! exactly (`now` → `YYYY-MM-DD HH:MM`, `today` → `YYYY-MM-DD`).
//!
//! Discipline (mirrors `riscv_aot_lock`'s temporal half + `c_concurrency_scheduler_lock`'s ratchet):
//! - The oracle pair (`tw_outcome` + `vm_outcome`) must AGREE on every corpus program (so the target
//!   output is unambiguous) — both read the injected clock, so the instant is pinned on both.
//! - A `Supported` program MUST compile to C and run byte-for-byte identical to that oracle.
//! - `SUPPORTED_FLOOR` only rises; `KNOWN_GAPS` only shrinks. The temporal builtins are FULLY lowered,
//!   so `KNOWN_GAPS` is EMPTY.
//!
//! ⚠️ RED tests are the spec: if a `Supported` program fails ≡VM, fix `codegen_c`, NEVER weaken this
//! file. The execution half is gated on `gcc` and SKIPS when absent; the oracle-agreement + ratchet
//! always run.
#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};

use logicaffeine_compile::compile::{compile_to_c, tw_outcome, vm_outcome};

/// The harness instant ([`fixed_clock`]): the VM oracle reads it through `set_fixed_clock`, the AOT-C binary
/// through the `LOGOS_FIXED_*` environment its runtime shim reads — the Moment `2024-10-04 12:34` and the Date
/// `2024-10-04`.
#[path = "support/fixed_clock.rs"]
mod fixed_clock;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    /// Lowered to C + proven byte-for-byte ≡ the fixed-clock oracle.
    Supported,
    /// Not yet lowered — a documented, shrink-only gap.
    #[allow(dead_code)]
    Gap,
}

/// The temporal corpus + its per-program status. Each program `Show`s a `now`/`today`-derived value under
/// the injected clock; the C output must equal the oracle byte-for-byte.
const CORPUS: &[(&str, &str, Status)] = &[
    // `now` — a Moment (nanoseconds since epoch); `Show` renders `YYYY-MM-DD HH:MM`.
    (
        "now_let",
        "## Main\n    Let m be now.\n    Show m.\n",
        Status::Supported,
    ),
    // `today` — a Date (days since epoch); `Show` renders `YYYY-MM-DD`.
    (
        "today_let",
        "## Main\n    Let d be today.\n    Show d.\n",
        Status::Supported,
    ),
    // Both in one program — the two builtins interleave and keep their distinct render formats.
    (
        "now_and_today",
        "## Main\n    Let d be today.\n    Show d.\n    Let m be now.\n    Show m.\n",
        Status::Supported,
    ),
    // The builtins shown DIRECTLY (no intermediate `Let`): the NAME resolves to the clock in expression
    // position too, so `Show now` / `Show today` render the Moment / Date, not an undefined identifier.
    ("now_direct", "## Main\n    Show now.\n", Status::Supported),
    (
        "today_direct",
        "## Main\n    Show today.\n",
        Status::Supported,
    ),
];

/// The count of `Supported` temporal programs — a MONOTONE FLOOR. RAISE it as coverage grows; NEVER lower.
const SUPPORTED_FLOOR: usize = 5;

/// The EXACT set of currently-`Gap` program labels — a shrink-only ledger. The temporal builtins are
/// fully lowered on host-C, so the frontier is EMPTY. Moving a program to `Supported` requires removing
/// it here AND raising the floor.
const KNOWN_GAPS: &[&str] = &[];

/// Pin BOTH oracles to the injected instant. Idempotent; every test that reads `now`/`today` calls it so
/// the tree-walker and VM see the same clock the C binary reads from the env.
fn pin_clock() {
    fixed_clock::set_vm();
}

fn have_gcc() -> bool {
    std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Compile a program to C, build with gcc UNDER the fixed-clock env (so the runtime shim reads the same
/// instant the VM oracle was pinned to), run, and return trimmed stdout. `None` if the C fails to compile
/// or the binary fails to run (a `Supported` program must do both).
fn c_output(src: &str, label: &str) -> Option<String> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let c = compile_to_c(src).ok()?;
    let dir = std::env::temp_dir().join(format!(
        "logos_ctemporal_{}_{}_{}",
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
        return None;
    }
    let run = std::process::Command::new(&bin)
        .envs(fixed_clock::env())
        .output()
        .ok()?;
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
fn the_temporal_oracles_agree_on_every_program() {
    // Under the pinned clock, the tree-walker and VM agree on every corpus program, so "≡ VM" and
    // "≡ tree-walker" are the same unambiguous target.
    pin_clock();
    for (label, src, _) in CORPUS {
        let tw = tw_outcome(src);
        let vm = vm_outcome(src);
        assert_eq!(
            (tw.output.trim(), tw.error.is_some()),
            (vm.output.trim(), vm.error.is_some()),
            "temporal oracle divergence on `{label}` — tw={:?} vm={:?}",
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
        "NEW temporal gap(s) not in KNOWN_GAPS: {new_gaps:?} — lower them or list them consciously"
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
        "C-temporal supported count fell to {n} (floor {SUPPORTED_FLOOR}) — fix codegen_c, do not lower the floor"
    );
}

// ============================================================
//  Behavioural — the ≡VM biconditional (gated on gcc)
// ============================================================

#[test]
fn every_supported_temporal_program_runs_equiv_vm_on_c() {
    if !have_gcc() {
        eprintln!("SKIP c-temporal execution: gcc not installed");
        return;
    }
    pin_clock();
    for (label, src, status) in CORPUS {
        if *status != Status::Supported {
            continue;
        }
        let want = tw_outcome(src).output.trim().to_string();
        let got = c_output(src, label).unwrap_or_else(|| {
            panic!("`{label}` is Supported but its C failed to compile/run — implement the clock shim in codegen_c")
        });
        assert_eq!(
            got, want,
            "`{label}`: C temporal output must equal the fixed-clock oracle (want {want:?}, got {got:?})"
        );
    }
}
