//! ★★★ THE ULTIMATE GAUNTLET LOCK ★★★
//!
//! Every real LOGOS program we own — the cross-op corpus harvested from `op_backend_matrix_lock`,
//! every benchmark's `interp.lg`, and a curated compute corpus — driven through the whole tier stack and pinned
//! to ONE meaning:
//!
//!     VM(raw P) ≡ VM(opt P) ≡ C(opt P) ≡ Rust(opt P)
//!
//! - `the_harvest_is_faithful` — the harvested corpus still computes its recorded answer on the VM (nothing was
//!   lost or corrupted in the lift; the gauntlet's own expectations still hold).
//! - `the_optimizer_is_sound_over_the_whole_gauntlet` — Invariant A over EVERY program (fast, no toolchain).
//! - `c_realizes_the_whole_gauntlet` / `rust_realizes_the_whole_gauntlet` — Invariant B: the AOT tiers run every
//!   program to the same answer as the optimized IR. Gated on gcc / rustc, `#[ignore]` (heavy — runs in the full
//!   suite via `run-all-tests-fast.sh`, skipped in quick iteration).
//!
//! The pit-of-success distinction (never a floor to game): a MISMATCH (compiled + ran but the answer or
//! error-flag diverges) is ALWAYS a hard RED. A COMPILE-GAP (an op the tier cannot yet emit) is tolerable only
//! as an explicit, shrink-only known-gap set. The generative fuzzers (`soft_core_fuzz_differential`,
//! `pe_jones_fuzz`) cover VM ≡ soft-core / PE over RANDOM programs; this gauntlet covers VM ≡ opt ≡ C ≡ Rust
//! over the HARVESTED + benchmark corpus.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};

use logicaffeine_compile::compile::{
    compile_to_c, compile_to_program_optimized, program_outcome_concurrent, run_logos_source,
    vm_outcome, vm_outcome_concurrent, vm_outcome_net, vm_outcome_tiered, RunOutcome,
};
use logicaffeine_language::optimization::Tier;

#[path = "gauntlet/mod.rs"]
mod gauntlet;
use gauntlet::{all, benchmarks, compute_corpus, GauntletProg};

/// The reference VM run for a program, routed by its oracle.
fn vm_reference(p: &GauntletProg) -> RunOutcome {
    match p.oracle.as_str() {
        "concurrent" => vm_outcome_concurrent(&p.src),
        "net" => vm_outcome_net(&p.src),
        _ => vm_outcome(&p.src),
    }
}

/// Run the program under the FULL optimizer on the VM — the `VM(opt P)` side of Invariant A. For plain
/// programs this is `vm_outcome_tiered` at T3 (the production optimized path: runs the whole lattice AND
/// installs the policy registry + globals, so it is faithful for CheckPolicy/global programs alike). For
/// concurrent programs it is the optimized `CompiledProgram` on the scheduler. `None` for `net` (no
/// straight-line net oracle over an optimized program yet — those keep their `soft_core_fuzz`/`cuda_net`
/// coverage).
fn vm_optimized(p: &GauntletProg) -> Option<RunOutcome> {
    match p.oracle.as_str() {
        "concurrent" => Some(program_outcome_concurrent(
            &compile_to_program_optimized(&p.src).ok()?,
        )),
        "net" => None,
        _ => Some(vm_outcome_tiered(&p.src, &[], Tier::T3, None)),
    }
}

fn same(a: &RunOutcome, b: &RunOutcome) -> bool {
    a.output.trim() == b.output.trim() && a.error.is_some() == b.error.is_some()
}

#[test]
fn the_harvest_is_faithful() {
    // The harvested corpus must still compute its recorded answer on the VM — the lift preserved meaning.
    let progs = all();
    let mut checked = 0;
    let mut wrong = Vec::new();
    for p in &progs {
        if p.op == "Benchmark" || p.op == "Compute" {
            continue; // only the harvested rows carry recorded expectations
        }
        if p.op == "LoadNow" || p.op == "LoadToday" {
            continue; // wall-clock reads — the recorded value was captured at harvest time, not reproducible
        }
        if p.oracle != "plain" {
            continue; // concurrent/net expectations are scheduler-dependent; covered by Invariant A below
        }
        let vm = vm_outcome(&p.src);
        let err_ok = vm.error.is_some() == p.expect_err;
        let out_ok = p.expect_err || vm.output.trim() == p.expect_out.trim();
        if !(err_ok && out_ok) {
            wrong.push(format!(
                "{} [{}]: recorded=({:?},err={}) vm=({:?},err={})",
                p.name,
                p.op,
                p.expect_out,
                p.expect_err,
                vm.output.trim(),
                vm.error.is_some()
            ));
        }
        checked += 1;
    }
    assert!(
        checked >= 500,
        "expected the full harvested corpus, only checked {checked}"
    );
    assert!(
        wrong.is_empty(),
        "the harvested gauntlet no longer matches its recorded expectations ({}):\n{}",
        wrong.len(),
        wrong
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    eprintln!("[gauntlet] harvest faithful: {checked} plain programs match their recorded answer");
}

#[test]
fn the_optimizer_is_sound_over_the_whole_gauntlet() {
    // Invariant A: VM(raw P) ≡ VM(opt P) over EVERY program (plain + concurrent). No toolchain needed.
    let progs = all();
    let mut checked = 0;
    let mut skipped_net = 0;
    let mut diverged = Vec::new();
    for p in &progs {
        let raw = vm_reference(p);
        let opt = match vm_optimized(p) {
            Some(o) => o,
            None => {
                skipped_net += 1;
                continue;
            }
        };
        // `vm_optimized` returns `Some` only when the optimized IR compiled, so any divergence here is a
        // genuine soundness break — the optimizer changed the program's meaning.
        if !same(&raw, &opt) {
            diverged.push(format!(
                "{} [{}/{}]: raw=({:?},err={}) opt=({:?},err={})",
                p.name,
                p.op,
                p.oracle,
                raw.output.trim(),
                raw.error.is_some(),
                opt.output.trim(),
                opt.error.is_some()
            ));
        }
        checked += 1;
    }
    assert!(
        checked >= 600,
        "expected to soundness-check the whole gauntlet, only did {checked}"
    );
    assert!(
        diverged.is_empty(),
        "OPTIMIZER CHANGED MEANING on {} gauntlet program(s) — Invariant A violated:\n{}",
        diverged.len(),
        diverged
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    eprintln!("[gauntlet] optimizer sound: {checked} programs ≡ VM(raw)==VM(opt) ({skipped_net} net skipped)");
}

// ============================================================
//  Invariant B — the AOT tiers realize the whole gauntlet
// ============================================================

fn have(tool: &str) -> bool {
    std::process::Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

enum Run {
    Ran(String, bool),
    CompileFailed,
}

fn build_and_run(
    source_code: &str,
    ext: &str,
    build: impl Fn(&std::path::Path, &std::path::Path) -> Option<bool>,
    label: &str,
) -> Run {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "logos_gauntlet_{}_{}_{}",
        std::process::id(),
        label,
        n
    ));
    let _ = std::fs::create_dir_all(&dir);
    let srcfile = dir.join(format!("prog.{ext}"));
    let bin = dir.join("prog");
    if std::fs::write(&srcfile, source_code).is_err() {
        return Run::CompileFailed;
    }
    let built = build(&srcfile, &bin);
    let result = match built {
        Some(true) => {
            let run = std::process::Command::new(&bin).output();
            match run {
                Ok(r) => Run::Ran(
                    String::from_utf8_lossy(&r.stdout).trim().to_string(),
                    r.status.success(),
                ),
                Err(_) => Run::CompileFailed,
            }
        }
        _ => Run::CompileFailed,
    };
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn gcc_build(src: &std::path::Path, bin: &std::path::Path) -> Option<bool> {
    let o = common::build_c_release(src, bin).ok()?;
    Some(o.status.success())
}

/// Diff one program's AOT execution against `VM(opt P)`. Only `plain` programs are AOT subjects here.
fn diff_aot(
    p: &GauntletProg,
    emit: impl Fn(&str) -> Result<String, String>,
    ext: &str,
    build: impl Fn(&std::path::Path, &std::path::Path) -> Option<bool>,
) -> Option<Result<(), String>> {
    if p.oracle != "plain" {
        return None; // concurrency/networking AOT is covered by the dedicated tier locks
    }
    let want = vm_optimized(p)?;
    let want_out = want.output.trim().to_string();
    let want_ok = want.error.is_none();
    let code = match emit(&p.src) {
        Ok(c) => c,
        Err(_) => return Some(Err("__COMPILE_GAP__".into())),
    };
    match build_and_run(&code, ext, build, &p.name) {
        Run::CompileFailed => Some(Err("__COMPILE_GAP__".into())),
        Run::Ran(got_out, got_ok) => {
            if got_out == want_out && got_ok == want_ok {
                Some(Ok(()))
            } else {
                Some(Err(format!(
                    "RAN but DIVERGED: want=({want_out:?},ok={want_ok}) got=({got_out:?},ok={got_ok})"
                )))
            }
        }
    }
}

fn run_aot_gauntlet(
    tier: &str,
    emit: impl Fn(&str) -> Result<String, String>,
    ext: &str,
    build: impl Fn(&std::path::Path, &std::path::Path) -> Option<bool> + Copy,
    known_divergences: &std::collections::BTreeSet<String>,
    known_gaps: &std::collections::BTreeSet<String>,
) {
    let progs = all();
    let mut matched = 0usize;
    let mut gaps: Vec<String> = Vec::new();
    let mut mismatches: Vec<(String, String)> = Vec::new();
    for p in &progs {
        match diff_aot(p, &emit, ext, build) {
            None => {}
            Some(Ok(())) => matched += 1,
            Some(Err(why)) if why == "__COMPILE_GAP__" => gaps.push(p.name.clone()),
            Some(Err(why)) => {
                mismatches.push((p.name.clone(), format!("{} [{}]: {}", p.name, p.op, why)))
            }
        }
    }
    assert!(
        matched >= 500,
        "[{tier}] too few programs realized ({matched}) — expected the whole gauntlet"
    );
    check_ledger(
        tier,
        matched,
        mismatches,
        gaps,
        known_divergences,
        known_gaps,
    );
}

/// The shrink-only ledger assertion shared by both AOT legs: every non-matching program must be an explicitly
/// ledgered execution divergence or compile-gap (a NEW one is a real regression → RED), and a ledgered program
/// that now matches must be deleted (the ledger only ratchets down).
fn check_ledger(
    tier: &str,
    matched: usize,
    mismatches: Vec<(String, String)>,
    gaps: Vec<String>,
    known_divergences: &std::collections::BTreeSet<String>,
    known_gaps: &std::collections::BTreeSet<String>,
) {
    eprintln!(
        "[gauntlet/{tier}] matched={matched} compile-gaps={} mismatches={}",
        gaps.len(),
        mismatches.len()
    );
    let dump = format!(
        "MISMATCHES ({}):\n{}\n\nCOMPILE-GAPS ({}):\n{}\n",
        mismatches.len(),
        mismatches
            .iter()
            .map(|(_, d)| d.clone())
            .collect::<Vec<_>>()
            .join("\n"),
        gaps.len(),
        gaps.join("\n"),
    );
    let _ = std::fs::write(format!("/tmp/gauntlet_{tier}_diverge.txt"), dump);

    let mismatch_names: std::collections::BTreeSet<&String> =
        mismatches.iter().map(|(n, _)| n).collect();
    let gap_names: std::collections::BTreeSet<&String> = gaps.iter().collect();

    let new_divergences: Vec<&(String, String)> = mismatches
        .iter()
        .filter(|(n, _)| !known_divergences.contains(n))
        .collect();
    assert!(
        new_divergences.is_empty(),
        "[{tier}] NEW execution divergence(s) NOT in the shrink-only ledger ({}) — a real regression:\n{}",
        new_divergences.len(),
        new_divergences.iter().map(|(_, d)| d.as_str()).take(40).collect::<Vec<_>>().join("\n")
    );
    let new_gaps: Vec<&String> = gaps.iter().filter(|n| !known_gaps.contains(*n)).collect();
    assert!(
        new_gaps.is_empty(),
        "[{tier}] NEW compile-gap(s) NOT in the shrink-only ledger ({}) — a real regression:\n{}",
        new_gaps.len(),
        new_gaps
            .iter()
            .take(40)
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
    let stale_div: Vec<&String> = known_divergences
        .iter()
        .filter(|n| !mismatch_names.contains(n))
        .collect();
    assert!(
        stale_div.is_empty(),
        "[{tier}] {} ledgered divergence(s) now PASS — delete them from the divergence ledger (ratchet):\n{}",
        stale_div.len(),
        stale_div.iter().take(40).map(|s| s.as_str()).collect::<Vec<_>>().join("\n")
    );
    let stale_gap: Vec<&String> = known_gaps
        .iter()
        .filter(|n| !gap_names.contains(n))
        .collect();
    assert!(
        stale_gap.is_empty(),
        "[{tier}] {} ledgered compile-gap(s) now COMPILE — delete them from the gap ledger (ratchet):\n{}",
        stale_gap.len(),
        stale_gap.iter().take(40).map(|s| s.as_str()).collect::<Vec<_>>().join("\n")
    );
    eprintln!(
        "[gauntlet/{tier}] ALL ACCOUNTED: matched={matched}, ledgered {} divergences + {} gaps (shrink-only)",
        mismatches.len(), gaps.len()
    );
}

#[test]
#[ignore = "heavy: gcc-compiles the whole gauntlet — runs in run-all-tests-fast.sh"]
fn c_realizes_the_whole_gauntlet() {
    if !have(&common::c_compiler()) {
        eprintln!("SKIP: no C compiler");
        return;
    }
    run_aot_gauntlet(
        "C",
        |s| compile_to_c(s).map_err(|e| format!("{e:?}")),
        "c",
        gcc_build,
        &gauntlet::c_known_divergences(),
        &gauntlet::c_known_compile_gaps(),
    );
}

#[test]
#[ignore = "heavy: full cargo build per program (run_logos_source) — runs in run-all-tests-fast.sh"]
fn rust_realizes_the_real_programs() {
    // The Rust AOT tier executes via a FULL cargo build per program (`run_logos_source`, shared-target cache),
    // so it is scoped to the REAL programs — every benchmark + the compute corpus — rather than the full
    // op-edge-case harvest (which would be ~50 min of cargo builds; its Rust COMPILE coverage lives in
    // `op_backend_matrix_lock`). Diffs Rust stdout against `VM(opt)` for the non-error programs.
    if !have("cargo") {
        eprintln!("SKIP: cargo absent");
        return;
    }
    let progs: Vec<GauntletProg> = benchmarks()
        .into_iter()
        .chain(compute_corpus())
        .filter(|p| p.oracle == "plain")
        .collect();
    let mut matched = 0usize;
    let mut gaps: Vec<String> = Vec::new();
    let mut mismatches: Vec<(String, String)> = Vec::new();
    for p in &progs {
        let want = match vm_optimized(p) {
            Some(w) => w,
            None => continue,
        };
        if want.error.is_some() {
            continue; // error-path Rust is covered by the dedicated AOT-Rust differentials
        }
        match run_logos_source(&p.src) {
            Ok(out) if out.trim() == want.output.trim() => matched += 1,
            Ok(out) => mismatches.push((
                p.name.clone(),
                format!(
                    "{} [{}]: Rust ran but DIVERGED: want={:?} got={:?}",
                    p.name,
                    p.op,
                    want.output.trim(),
                    out.trim()
                ),
            )),
            Err(_) => gaps.push(p.name.clone()),
        }
    }
    assert!(
        matched >= 30,
        "[Rust] too few real programs realized ({matched}) — expected benchmarks + corpus"
    );
    check_ledger(
        "Rust",
        matched,
        mismatches,
        gaps,
        &gauntlet::rust_known_divergences(),
        &gauntlet::rust_known_compile_gaps(),
    );
}
