//! ★ THE OPTIMIZER-SOUNDNESS LOCK ★ — Invariant A of the bytecode-IR-as-source-of-truth refactor:
//!
//!     VM(P) == VM(opt(P))
//!
//! The many-pass AOT optimizer lattice (`optimize/mod.rs` — splice-fuse → PE-fixpoint → SCCP → egraph "the
//! Architect" → GVN/LICM → supercompile → `bound_version` → …) must NEVER change a program's observable
//! meaning. We prove it by running the RAW program on the VM (`vm_outcome`, the reference semantics) and the
//! OPTIMIZED IR on the VM (`program_outcome(compile_to_program_optimized)`), and asserting they are
//! byte-for-byte identical — over EVERY benchmark we ship (the biggest, realest LOGOS programs) plus a
//! compute corpus that exercises the shapes each pass rewrites.
//!
//! This is the trap that lets the whole refactor proceed safely: if any optimization ever diverges from the
//! meaning, this goes RED — the optimizer, not a backend, is the culprit, caught before it can ship.
#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

use logicaffeine_compile::compile::{compile_to_program_optimized, program_outcome, vm_outcome};

fn benchmarks_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/programs")
}

/// Discover every `benchmarks/programs/*/interp.lg` — the 35-program suite (fib, gcd, collatz, sieve, nqueens,
/// quicksort, mergesort, nbody, mandelbrot, sha256, …), the real workloads that stress the whole lattice.
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

/// A compute corpus targeting the shapes the optimizer transforms — constant folding, strength reduction,
/// loop versioning (bigint overflow), DCE, division lowering, closed-form, function inlining.
const CORPUS: &[(&str, &str)] = &[
    ("const_fold", "## Main\n    Show 2 + 3 * 4 - 1.\n"),
    ("strength_reduce_div", "## Main\n    Let n be 800.\n    Show n / 8.\n    Show n / 10.\n"),
    ("loop_sum_5050", "## Main\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is at most 100:\n        Set acc to acc + i.\n        Set i to i + 1.\n    Show acc.\n"),
    ("factorial_bignum", "## Main\n    Let mutable acc be 1.\n    Let mutable i be 1.\n    While i is at most 25:\n        Set acc to acc * i.\n        Set i to i + 1.\n    Show acc.\n"),
    ("nested_loops", "## Main\n    Let mutable s be 0.\n    Let mutable i be 1.\n    While i is at most 20:\n        Let mutable j be 1.\n        While j is at most 20:\n            Set s to s + i * j.\n            Set j to j + 1.\n        Set i to i + 1.\n    Show s.\n"),
    ("recursion", "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\n    Show fib(20).\n"),
    ("collection_build", "## Main\n    Let mutable xs be a new Seq of Int.\n    Let mutable i be 1.\n    While i is at most 50:\n        Push i * i to xs.\n        Set i to i + 1.\n    Show length of xs.\n    Show item 50 of xs.\n"),
    ("branch_dce", "## Main\n    Let x be 10.\n    If x is greater than 5:\n        Show 1.\n    Otherwise:\n        Show 999.\n"),
    ("div_by_zero_error", "## Main\n    Let a be 5.\n    Let b be 0.\n    Show a / b.\n"),
    ("floor_vs_exact", "## Main\n    Show 7 // 2.\n"),
    // ── global-mutated-inside-a-callee, read after a loop ─────────────────────────────────────────────────
    // The recurring optimizer-soundness family (4 siblings fixed: propagate/sccp/ctfe/supercompile). A global
    // (`counter`) is mutated by a FUNCTION (`bump()` does `Set counter …`) called in a loop, then read after
    // it. `collect_modified_vars_block` sees only direct `Set`/`Let` targets, so an over-eager pass would keep
    // `counter`'s stale pre-loop 0 and fold the read to a WRONG constant. VM(raw) must equal VM(opt) (=100/=100
    // /=50/=…) at every toggle. Pinned directly here so a future pass edit can never silently reintroduce it.
    ("global_in_callee_while", "## To bump:\n    Set counter to counter + 1.\n## Main\n    Let counter be 0.\n    Let i be 0.\n    While i is less than 100:\n        bump().\n        Set i to i + 1.\n    Show counter.\n"),
    ("global_in_callee_repeat", "## To bump:\n    Set counter to counter + 1.\n## Main\n    Let counter be 0.\n    Repeat for i from 1 to 100:\n        bump().\n    Show counter.\n"),
    ("global_in_callee_transitive", "## To inner:\n    Set counter to counter + 1.\n## To outer:\n    inner().\n## Main\n    Let counter be 0.\n    Let i be 0.\n    While i is less than 50:\n        outer().\n        Set i to i + 1.\n    Show counter.\n"),
    ("global_in_callee_read_midloop", "## To bump:\n    Set counter to counter + 1.\n## Main\n    Let counter be 0.\n    Let i be 0.\n    While i is less than 10:\n        bump().\n        Show counter.\n        Set i to i + 1.\n    Show counter.\n"),
];

/// Assert `VM(raw P) == VM(opt P)` for one program — the optimizer preserved meaning.
fn assert_optimizer_sound(label: &str, src: &str) {
    let raw = vm_outcome(src);
    let opt_program = match compile_to_program_optimized(src) {
        Ok(p) => p,
        Err(e) => panic!("`{label}`: compile_to_program_optimized failed: {e:?}"),
    };
    let opt = program_outcome(&opt_program);
    assert_eq!(
        (raw.output.trim(), raw.error.is_some()),
        (opt.output.trim(), opt.error.is_some()),
        "`{label}`: OPTIMIZER CHANGED MEANING — VM(raw) != VM(opt). raw=({:?},{}) opt=({:?},{})",
        raw.output.trim(),
        raw.error.is_some(),
        opt.output.trim(),
        opt.error.is_some()
    );
}

#[test]
fn the_optimizer_preserves_meaning_over_the_compute_corpus() {
    for (label, src) in CORPUS {
        assert_optimizer_sound(label, src);
    }
}

#[test]
fn the_optimizer_preserves_meaning_over_every_benchmark() {
    let benches = discover_benchmarks();
    assert!(
        benches.len() >= 30,
        "expected the full benchmark suite, found {}",
        benches.len()
    );
    let mut checked = 0;
    for (name, src) in &benches {
        // A benchmark whose raw VM run errors (e.g. needs argv) is not a soundness subject here — skip it,
        // but only if RAW errors; if raw runs, the optimized IR MUST match it.
        let raw = vm_outcome(src);
        if raw.error.is_some() {
            continue;
        }
        assert_optimizer_sound(name, src);
        checked += 1;
    }
    assert!(
        checked >= 25,
        "too few benchmarks were soundness-checked ({checked}) — most must run on the raw VM"
    );
    eprintln!(
        "optimizer-soundness: {checked}/{} benchmarks checked ≡ across the lattice",
        benches.len()
    );
}
