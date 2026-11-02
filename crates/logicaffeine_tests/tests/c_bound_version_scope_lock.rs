//! ★ BOUND-VERSION SCOPE LOCK ★ — a counted-loop index variable that is REUSED after a
//! bound-versioned nest must stay declared in the enclosing scope.
//!
//! `optimize::bound_version` splits a counted nest whose index arithmetic could overflow i64 into a
//! fast `if (bound <= B) { i64 loop } else { bigint loop }` guard. The index variable's `Let` (a C
//! *declaration*, `int64_t i = 0;`) must NOT be cloned into the guard branches: a program routinely
//! reuses the same `mutable` index after the nest (`Set i to 0` before a later loop), and a `Let`
//! scoped to the branch leaves that reuse referencing an undeclared variable. The VM has no block
//! scoping so it runs regardless — the divergence is a C/CUDA-codegen-only `'i' undeclared` error,
//! which is exactly how `matrix_mult` broke on gcc AND nvcc. The pass hoists the declaration before
//! the guard; this lock pins that so the whole C-derived tier family (C / Tenstorrent / CUDA) keeps
//! compiling it.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. If it regresses, fix `optimize/bound_version`, not the lock.

use std::sync::atomic::{AtomicUsize, Ordering};

use logicaffeine_compile::compile::{compile_to_c, vm_outcome};

/// A `Let mutable i` drives a counted nest whose index `i * n + j` (with `n` folded to a literal)
/// escapes i64 for unbounded leaves — so `bound_version` fires and emits its `bound <= 2^57` guard —
/// and is then REUSED via `Set i to 0` in a second counted loop. Pre-fix, the guard branches
/// re-declared `i`, so the reuse hit `'i' undeclared` under gcc/nvcc while the VM ran fine.
const REUSED_INDEX_ACROSS_VERSIONED_NEST: &str = "\
## Main
Let n be 50.
Let mutable a be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Let mutable j be 0.
    While j is less than n:
        Push (i * n + j) % 100 to a.
        Set j to j + 1.
    Set i to i + 1.
Let mutable checksum be 0.
Set i to 0.
While i is less than n * n:
    Set checksum to (checksum + item (i + 1) of a) % 1000000007.
    Set i to i + 1.
Show checksum.
";

/// The bound-version guard threshold for the `i * 50 + j` chain: the largest `2^k` (k in 16..=62)
/// whose worst-case magnitude `51 * 2^k` still fits i64 is `2^57`. Its presence in the emitted C
/// proves the versioning pass actually fired — so a gcc-green result is never vacuous.
const GUARD_THRESHOLD_2_POW_57: &str = "144115188075855872";

fn have_gcc() -> bool {
    std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Compile C with gcc and run it; `Some(stdout)` on success, `None` if it failed to compile or ran
/// nonzero. The compile step is the real proof — a mis-scoped `Let` is a hard `'i' undeclared` error.
fn gcc_compile_and_run(c_src: &str) -> Option<String> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("logos_bvscope_{}_{n}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let c_path = dir.join("k.c");
    let bin_path = dir.join("k");
    std::fs::write(&c_path, c_src).ok()?;
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
    r.status
        .success()
        .then(|| String::from_utf8_lossy(&r.stdout).to_string())
}

/// NON-VACUITY (always runs, no toolchain): the versioning pass must actually fire on this program —
/// otherwise the gcc gate below would be testing an un-versioned nest and prove nothing.
#[test]
fn the_bound_version_guard_actually_fires() {
    let c = compile_to_c(REUSED_INDEX_ACROSS_VERSIONED_NEST).expect("compiles to C");
    assert!(
        c.contains(GUARD_THRESHOLD_2_POW_57),
        "bound_version did not fire on the reused-index nest — this lock would be vacuous. If the \
         pass legitimately moved/renamed its guard, update GUARD_THRESHOLD_2_POW_57 to match; do not \
         delete the non-vacuity check."
    );
}

/// THE INVARIANT: the reused index survives C codegen. Pre-fix this failed to compile with
/// `'i' undeclared`; post-fix the declaration is hoisted before the guard and gcc builds it ≡VM.
#[test]
fn reused_index_across_versioned_nest_compiles_and_runs_equiv_vm() {
    let vm = vm_outcome(REUSED_INDEX_ACROSS_VERSIONED_NEST);
    assert!(
        vm.error.is_none(),
        "the reference VM run must succeed: {:?}",
        vm.error
    );

    let c = compile_to_c(REUSED_INDEX_ACROSS_VERSIONED_NEST).expect("compiles to C");

    if !have_gcc() {
        eprintln!("SKIP: gcc not present — cannot prove the emitted C compiles");
        return;
    }
    let out = gcc_compile_and_run(&c)
        .expect("the emitted C must gcc-compile (a mis-scoped `Let` is `'i' undeclared`) and run");
    assert_eq!(
        out.trim(),
        vm.output.trim(),
        "C output diverged from the VM for the reused-index nest"
    );
}
