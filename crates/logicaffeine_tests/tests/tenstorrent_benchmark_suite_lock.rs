//! ★ TENSTORRENT BENCHMARK-SUITE LOCK (Phase 2) ★ — the WHOLE benchmark suite runs on the
//! Tenstorrent single-baby-core tier, byte-for-byte ≡ the VM.
//!
//! Data-driven: it DISCOVERS every `benchmarks/programs/*/interp.lg` (the self-contained,
//! hardcoded-size form of each benchmark), emits a TT-Metalium bundle, compiles the kernel's
//! compute (the `oracle_c`) with gcc, runs it, and asserts the output equals `vm_outcome`. A
//! benchmark the tier cannot yet run is a shrink-only `KNOWN_GAPS` entry with a written reason —
//! never a silent skip. The set of benchmarks that RUN is a monotone floor.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. If a benchmark regresses, fix `codegen_tt`, not the lock.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use logicaffeine_compile::compile::{compile_to_tenstorrent, vm_outcome};

fn benchmarks_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/programs")
}

/// Benchmarks the single-baby-core tier does not yet run, each with a written reason. Every reason
/// here is a PRE-EXISTING `codegen_c` bug that the kernel inherits (the tier reuses the C compute
/// body verbatim), NOT a wrap defect — so the fix lands in the C backend and this ledger SHRINKS.
/// Shrink-only: removing an entry is a conscious edit; a new deferral must be added deliberately.
/// EMPTY — every benchmark runs ≡VM on the Tenstorrent tier. All seven pre-existing `codegen_c` bugs
/// this lock surfaced were fixed (with ZERO regression to the C backend's 1246-test suite):
///   pi_leibniz    — float Show now shortest-round-trip (logos_fmt_f64), not `%g`.
///   sha256        — the word32 bit-op family (word32/word_and/word_not/rotr/word32Shr/intOfWord32)
///                   now lowers as 32-bit unsigned ops.
///   strings       — `item k of <Text>` lowers to a 1-char string (logos_str_char_at), not seq_i64_get.
///   string_search — `item k of <Text>` infers String, so `!=` between chars lowers as !str_equals.
///   mandelbrot,   — an integral float literal (2.0) now emits `2.0`, not `2` (the old form turned
///   spectral_norm   `2.0 * x / n` into truncating integer division).
///   nbody         — a per-C-block declared-name scope stack (CContext::{enter,exit}_scope) makes an
///                   UNROLLED loop's duplicate `Let dx …` an assignment, not a C redefinition.
const KNOWN_GAPS: &[(&str, &str)] = &[];

fn have_gcc() -> bool {
    std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Compile `oracle_c` with gcc and run it; `Some(stdout)` on success, `None` if it failed to compile
/// or exited nonzero.
fn compile_and_run_oracle(oracle_c: &str) -> Option<String> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("logos_ttbench_{}_{n}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let c_path = dir.join("oracle.c");
    let bin_path = dir.join("oracle");
    std::fs::write(&c_path, oracle_c).ok()?;
    let c = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin_path)
        .arg(&c_path)
        .arg("-lm")
        .output()
        .ok()?;
    if !c.status.success() {
        eprintln!("  gcc failed:\n{}", String::from_utf8_lossy(&c.stderr));
        let _ = std::fs::remove_dir_all(&dir);
        return None;
    }
    let r = std::process::Command::new(&bin_path).output().ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    if r.status.success() {
        Some(String::from_utf8_lossy(&r.stdout).to_string())
    } else {
        None
    }
}

/// Discover every benchmark that ships an `interp.lg`, sorted by name.
fn discover_benchmarks() -> Vec<(String, String)> {
    let dir = benchmarks_dir();
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            let interp = p.join("interp.lg");
            if interp.is_file() {
                let name = p.file_name().unwrap().to_string_lossy().to_string();
                let src = std::fs::read_to_string(&interp).expect("read interp.lg");
                out.push((name, src));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn whole_benchmark_suite_runs_on_tenstorrent_equiv_vm() {
    if !have_gcc() {
        eprintln!("SKIP: gcc not present — cannot run the Tenstorrent benchmark oracle");
        return;
    }
    let gaps: BTreeSet<&str> = KNOWN_GAPS.iter().map(|(b, _)| *b).collect();
    let benches = discover_benchmarks();
    assert!(
        benches.len() >= 30,
        "expected the full benchmark suite, found {}",
        benches.len()
    );

    let mut ran = 0usize;
    let mut failures: Vec<String> = Vec::new(); // undocumented gaps/mismatches (RED)
    let mut now_working_gaps: Vec<String> = Vec::new(); // documented gaps that now pass (must shrink)

    for (name, src) in &benches {
        let is_gap = gaps.contains(name.as_str());
        let vm = vm_outcome(src);
        if vm.error.is_some() {
            // A benchmark that errors on the VM is not a compute oracle — treat as a mismatch.
            if !is_gap {
                failures.push(format!("{name}: VM errors ({:?})", vm.error));
            }
            continue;
        }
        // Does the emitted kernel's compute (the C oracle) reproduce the VM byte-for-byte?
        let matches = match compile_to_tenstorrent(src) {
            Err(_) => false,
            Ok(art) => compile_and_run_oracle(&art.oracle_c)
                .map(|out| out.trim() == vm.output.trim())
                .unwrap_or(false),
        };
        match (matches, is_gap) {
            (true, false) => {
                ran += 1;
                eprintln!("ok ≡VM: {name}");
            }
            (true, true) => now_working_gaps.push(name.clone()), // shrink-only: remove from KNOWN_GAPS
            (false, true) => eprintln!("gap (documented): {name}"),
            (false, false) => failures.push(format!("{name}: does not run ≡VM (undocumented)")),
        }
    }

    eprintln!(
        "\nTenstorrent benchmark suite: {ran}/{} run ≡VM, {} documented codegen_c-inherited gaps",
        benches.len(),
        gaps.len()
    );

    assert!(
        now_working_gaps.is_empty(),
        "these benchmarks are in KNOWN_GAPS but now run ≡VM — remove them (shrink-only): {now_working_gaps:?}"
    );
    assert!(
        failures.is_empty(),
        "Tenstorrent benchmark suite has undocumented gaps/mismatches:\n{}",
        failures.join("\n")
    );
    // Monotone floor: ALL 35 benchmarks run ≡VM (KNOWN_GAPS is empty). Raised 28 → 35 this session.
    assert!(
        ran >= 35,
        "benchmark ≡VM floor regressed: only {ran} run (expected ≥ 35, all benchmarks)"
    );
}
