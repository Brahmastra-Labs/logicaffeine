//! ★ THE BACKEND EXECUTION-DIFF LOCK ★ — Invariant B of the bytecode-IR-as-source-of-truth refactor:
//!
//!     VM(opt P) == backend(opt P)
//!
//! Every backend must FAITHFULLY REALIZE the optimized IR — not merely compile it. The gauntlet's op-matrix
//! proves each backend *compiles* every op; this lock proves each backend *runs* every real program to the
//! same answer the VM produces for the optimized program. `VM(opt P)` is `program_outcome(compile_to_program_
//! optimized(P))`; `backend(opt P)` is the compiled artifact actually executed. We diff them byte-for-byte over
//! the whole benchmark suite + a compute corpus.
//!
//! The distinction that makes this a pit-of-success trap, not a floor to game:
//!   - a MISMATCH (compiled, ran, but the answer or error-flag diverges) is NEVER tolerable → hard RED. A
//!     backend can no longer "pass" by compiling and mis-running.
//!   - a COMPILE-GAP (an op the tier cannot yet emit) is tolerable ONLY as an explicit, shrink-only known-gap.
//!
//! This is the trap that guards Phase 1 (re-basing `codegen_c` onto the bytecode): the moment the re-based
//! emitter produces a different answer than the VM for any program, this goes RED — before it can ship.
//! Execution is gated on `gcc` and SKIPS clean when the toolchain is absent (never a false failure).
#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use logicaffeine_compile::compile::{
    compile_to_c, compile_to_program_optimized, program_outcome, vm_outcome,
};

fn benchmarks_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/programs")
}

fn discover_benchmarks() -> Vec<(String, String)> {
    let dir = benchmarks_dir();
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                let interp = p.join("interp.lg");
                if interp.is_file() {
                    let name = p.file_name().unwrap().to_string_lossy().to_string();
                    if let Ok(src) = std::fs::read_to_string(&interp) {
                        out.push((name, src));
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// A compute corpus spanning arithmetic / loops / functions / collections / text / the error path.
const CORPUS: &[(&str, &str)] = &[
    ("const_fold", "## Main\n    Show 2 + 3 * 4 - 1.\n"),
    ("loop_sum_5050", "## Main\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is at most 100:\n        Set acc to acc + i.\n        Set i to i + 1.\n    Show acc.\n"),
    ("factorial_bignum", "## Main\n    Let mutable acc be 1.\n    Let mutable i be 1.\n    While i is at most 25:\n        Set acc to acc * i.\n        Set i to i + 1.\n    Show acc.\n"),
    ("nested_loops", "## Main\n    Let mutable s be 0.\n    Let mutable i be 1.\n    While i is at most 20:\n        Let mutable j be 1.\n        While j is at most 20:\n            Set s to s + i * j.\n            Set j to j + 1.\n        Set i to i + 1.\n    Show s.\n"),
    ("recursion", "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\n    Show fib(20).\n"),
    ("collection_build", "## Main\n    Let mutable xs be a new Seq of Int.\n    Let mutable i be 1.\n    While i is at most 50:\n        Push i * i to xs.\n        Set i to i + 1.\n    Show length of xs.\n    Show item 50 of xs.\n"),
    ("text", "## Main\n    Show \"hello world\".\n"),
    ("div_by_zero_error", "## Main\n    Let a be 5.\n    Let b be 0.\n    Show a / b.\n"),
    ("floor_div", "## Main\n    Show 7 // 2.\n"),
];

/// The only benchmarks the C tier cannot yet EMIT (compile-gap). Shrink-only — an entry that starts compiling
/// must be removed, and a NEW compile-gap fails the lock. Empty = C emits every real program.
const C_KNOWN_COMPILE_GAPS: &[&str] = &[];

fn have_gcc() -> bool {
    std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

enum CRun {
    /// Compiled and ran: (trimmed stdout, exit-was-success).
    Ran(String, bool),
    /// `compile_to_c` or gcc refused it — a compile-gap.
    CompileFailed,
}

/// Compile a program to C, build with gcc, run, and report both stdout and whether the process exited cleanly.
fn c_run(src: &str, label: &str) -> CRun {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let c = match compile_to_c(src) {
        Ok(c) => c,
        Err(_) => return CRun::CompileFailed,
    };
    let dir = std::env::temp_dir().join(format!(
        "logos_execdiff_{}_{}_{}",
        std::process::id(),
        label,
        n
    ));
    let _ = std::fs::create_dir_all(&dir);
    let cfile = dir.join("prog.c");
    let bin = dir.join("prog");
    if std::fs::write(&cfile, &c).is_err() {
        return CRun::CompileFailed;
    }
    // `-Werror=incompatible-pointer-types`: a function-pointer type mismatch in the emitted C (an
    // `LInt (*)(LInt)` exact-HOF parameter fed an `int64_t (*)(int64_t)` closure) is only a WARNING to
    // gcc but an ERROR to nvcc — the CUDA tier tree-shakes this very oracle — so the C leg fails loud
    // on it too instead of letting the class hide behind gcc's leniency.
    let build = std::process::Command::new("gcc")
        .args(["-O2", "-Werror=incompatible-pointer-types", "-o"])
        .arg(&bin)
        .arg(&cfile)
        .arg("-lm")
        .output();
    let build = match build {
        Ok(b) => b,
        Err(_) => return CRun::CompileFailed,
    };
    if !build.status.success() {
        eprintln!(
            "  C compile-gap `{label}`:\n{}",
            String::from_utf8_lossy(&build.stderr)
                .lines()
                .take(2)
                .collect::<Vec<_>>()
                .join("\n")
        );
        return CRun::CompileFailed;
    }
    let run = std::process::Command::new(&bin).output();
    let _ = std::fs::remove_dir_all(&dir);
    match run {
        Ok(r) => CRun::Ran(
            String::from_utf8_lossy(&r.stdout).trim().to_string(),
            r.status.success(),
        ),
        Err(_) => CRun::CompileFailed,
    }
}

/// Diff one program's C execution against `VM(opt P)`. Returns a verdict string for the report.
enum Verdict {
    Match,
    CompileGap,
    Mismatch(String),
}

fn diff_c_vs_opt_ir(label: &str, src: &str) -> Verdict {
    let opt = match compile_to_program_optimized(src) {
        Ok(p) => program_outcome(&p),
        Err(e) => return Verdict::Mismatch(format!("compile_to_program_optimized failed: {e:?}")),
    };
    let want_out = opt.output.trim().to_string();
    let want_ok = opt.error.is_none();
    match c_run(src, label) {
        CRun::CompileFailed => Verdict::CompileGap,
        CRun::Ran(got_out, got_ok) => {
            if got_out == want_out && got_ok == want_ok {
                Verdict::Match
            } else {
                Verdict::Mismatch(format!(
                    "C ran but DIVERGED from VM(opt): want=({want_out:?},ok={want_ok}) got=({got_out:?},ok={got_ok})"
                ))
            }
        }
    }
}

/// Run the diff over a labelled set, tallying verdicts and asserting the invariant.
fn assert_execution_faithful(kind: &str, programs: &[(String, String)]) {
    let mut matched = Vec::new();
    let mut gaps = Vec::new();
    let mut mismatches = Vec::new();
    for (label, src) in programs {
        // Only diff programs the VM itself runs straight-line (a program needing argv isn't a subject here).
        if vm_outcome(src).error.is_some() && !label.contains("error") {
            continue;
        }
        match diff_c_vs_opt_ir(label, src) {
            Verdict::Match => matched.push(label.clone()),
            Verdict::CompileGap => gaps.push(label.clone()),
            Verdict::Mismatch(why) => mismatches.push(format!("{label}: {why}")),
        }
    }
    eprintln!(
        "[Invariant B / C tier / {kind}] matched={} compile-gaps={} mismatches={}",
        matched.len(),
        gaps.len(),
        mismatches.len()
    );
    for g in &gaps {
        eprintln!("  compile-gap: {g}");
    }
    for m in &mismatches {
        eprintln!("  MISMATCH: {m}");
    }

    // (1) A wrong answer is NEVER tolerable — Invariant B is hard here.
    assert!(
        mismatches.is_empty(),
        "[{kind}] C DIVERGED from VM(opt) on {} program(s) — Invariant B violated:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );

    // (2) Compile-gaps must be a subset of the explicit, shrink-only known-gap set.
    let allowed: std::collections::BTreeSet<&str> = C_KNOWN_COMPILE_GAPS.iter().copied().collect();
    let unexpected: Vec<&String> = gaps
        .iter()
        .filter(|g| !allowed.contains(g.as_str()))
        .collect();
    assert!(
        unexpected.is_empty(),
        "[{kind}] NEW C compile-gap(s) not in C_KNOWN_COMPILE_GAPS: {unexpected:?} — either fix codegen_c or \
         (with the user) add them to the shrink-only known-gap list"
    );

    // (3) The known-gap list may only shrink: every listed gap must actually still be a gap here (for the
    //     benchmark run, which sees every op) — a stale entry means the gap closed and must be removed.
    if kind == "benchmarks" {
        for known in C_KNOWN_COMPILE_GAPS {
            assert!(
                gaps.iter().any(|g| g == known),
                "[{kind}] `{known}` is listed in C_KNOWN_COMPILE_GAPS but now compiles — remove it (ratchet down)"
            );
        }
    }
}

#[test]
fn c_execution_matches_the_optimized_ir_over_the_compute_corpus() {
    if !have_gcc() {
        eprintln!("SKIP: gcc absent — cannot run the C execution-diff");
        return;
    }
    let programs: Vec<(String, String)> = CORPUS
        .iter()
        .map(|(l, s)| (l.to_string(), s.to_string()))
        .collect();
    assert_execution_faithful("corpus", &programs);
}

#[test]
fn c_execution_matches_the_optimized_ir_over_every_benchmark() {
    if !have_gcc() {
        eprintln!("SKIP: gcc absent — cannot run the C execution-diff");
        return;
    }
    let benches = discover_benchmarks();
    assert!(
        benches.len() >= 30,
        "expected the full benchmark suite, found {}",
        benches.len()
    );
    assert_execution_faithful("benchmarks", &benches);
}
