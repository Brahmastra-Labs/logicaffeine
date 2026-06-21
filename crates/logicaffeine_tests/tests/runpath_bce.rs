//! RUN-PATH BOUNDS-CHECK ELIMINATION — the array-cluster coverage ratchet
//! (full VM+JIT run path).
//!
//! The VM compiler emits `Op::IndexUnchecked` / `Op::SetIndexUnchecked` (the
//! JIT lowers these WITHOUT the bounds branch) instead of the checked
//! `Op::Index` / `Op::SetIndex` exactly when the Oracle's range analysis
//! (`index_provably_in_bounds`, kernel-LIA-backed) proves the access in
//! `[1, length]` for ALL executions. An unchecked load on an out-of-bounds
//! index is UNDEFINED BEHAVIOR (it reads the heap buffer past its end — the
//! frame canary cannot catch it), so this is a SAFETY-critical decision, not a
//! mere correctness one. Each test pins two properties:
//!
//!   1. DECISION SOUNDNESS — a STRUCTURAL assertion on the compiled bytecode
//!      that only the provably in-bounds hot accesses become unchecked. The
//!      NEGATIVE companions pin that an access the analysis cannot soundly prove
//!      (a user-supplied index, a nonlinear/variable-stride index, a
//!      data-dependent function-parameter index) stays CHECKED.
//!   2. EXECUTION SOUNDNESS — the eliding VM+JIT run path produces output
//!      IDENTICAL to the independent (always-checked) tree-walker, INCLUDING
//!      the out-of-bounds error path.
//!
//! Everything runs through the EXACT live run path (`with_optimized_program` →
//! `oracle_analyze_with` on the residual → `compile_with_oracle` → tiered VM
//! with a `ForgeTier`), so the structural counts and the parity are byte-for-
//! byte what `interpret_for_ui_sync` hands the engine. Correctness of these
//! exact programs is also covered by `vm_opt_differential::bench_corpus`; here
//! the point is that the elision FIRES (and where it must not, that it does
//! NOT). The VM-only mirror lives in
//! `logicaffeine_compile::tests::runpath_bce_vm` (no JIT dependency).

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::tw_outcome_with_args;
use logicaffeine_compile::optimize::oracle_analyze_with;
use logicaffeine_compile::ui_bridge::with_optimized_program;
use logicaffeine_compile::vm::{Compiler, NativeTier, Op};
use logicaffeine_jit::ForgeTier;

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// (unchecked reads, checked reads, unchecked stores, checked stores) across
/// the WHOLE compiled program — every function body is emitted into the single
/// `program.code` array — compiled through the exact run path the live engine
/// uses (`optimize_for_run` residual → `oracle_analyze_with` → `compile_with_oracle`).
fn bce_counts(src: &str) -> (usize, usize, usize, usize) {
    with_optimized_program(src, |parsed, interner| {
        let (stmts, types, _policies) = parsed.expect("program parses");
        let oracle = oracle_analyze_with(stmts, interner);
        let program = Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
            .expect("program compiles");
        let (mut ru, mut rc, mut su, mut sc) = (0, 0, 0, 0);
        for op in &program.code {
            match op {
                Op::IndexUnchecked { .. } => ru += 1,
                Op::Index { .. } => rc += 1,
                Op::SetIndexUnchecked { .. } => su += 1,
                Op::SetIndex { .. } => sc += 1,
                _ => {}
            }
        }
        (ru, rc, su, sc)
    })
}

/// The tiered VM+JIT run path (private `ForgeTier` so its compile counters stay
/// isolated) against the raw tree-walker on the same argv. Returns the
/// normalized eliding output for an exact-value assertion.
fn assert_runpath_parity(src: &str, size: &str) -> String {
    let argv = vec!["bench".to_string(), size.to_string()];
    let tier = ForgeTier::new();
    let (out, err) = with_optimized_program(src, |parsed, interner| match parsed {
        Ok((stmts, types, policies)) => logicaffeine_compile::vm::run_to_outcome_with_args(
            stmts,
            interner,
            Some(types),
            Some(&policies),
            &argv,
            Some(&tier as &dyn NativeTier),
        ),
        Err(advice) => (String::new(), Some(advice)),
    });
    let tw = tw_outcome_with_args(src, &argv);
    assert_eq!(
        (norm(&out), &err),
        (norm(&tw.output), &tw.error),
        "eliding VM+JIT run path diverged from the checked tree-walker on:\n{src}"
    );
    norm(&out)
}

fn prog_src(name: &str) -> String {
    let path = format!(
        "{}/../../benchmarks/programs/{name}/main.lg",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

// ─────────────────────────── REAL BENCHMARK SHAPES ────────────────────────

/// bubble_sort: the expression-bound guard `j <= n - 1 - i` proves BOTH reads
/// `item j of arr` / `item (j+1) of arr` (the `+1` fits the `-1-i` headroom)
/// AND both swap STORES.
#[test]
fn bubble_sort_reads_and_stores_elide() {
    let src = prog_src("bubble_sort");
    let (ru, _rc, su, _sc) = bce_counts(&src);
    assert!(ru >= 2, "bubble_sort: both adjacent reads must elide (got {ru})");
    assert!(su >= 2, "bubble_sort: both swap STORES must elide (got {su})");
    assert_eq!(assert_runpath_parity(&src, "30"), "1071");
}

/// knapsack 0/1 DP: `item (w+1) of prev` (bare offset) AND the multi-variable
/// `item (w-wi+1) of prev` (kernel LIA: path guard `w >= wi`, element bound
/// `wi ∈ [1,50]`, `length(prev) = cols = capacity + 1`) plus the row-build
/// reads `item (i+1) of weights/vals` all elide.
#[test]
fn knapsack_dp_reads_elide() {
    let src = prog_src("knapsack");
    let (ru, _rc, _su, _sc) = bce_counts(&src);
    assert!(
        ru >= 4,
        "knapsack: weights/vals reads + prev[w+1] + prev[w-wi+1] must elide (got {ru})"
    );
    assert_eq!(assert_runpath_parity(&src, "12"), "260");
}

/// heap_sort Main: the sift/checksum reads/stores on the locally-built `arr`
/// (length `n`) elide — `item 1 of arr`, `item (end+1) of arr` (guard
/// `end <= n-1`), `item i of arr` (checksum `i <= n`) plus the two in-place
/// swap stores. The siftDown helper's data-dependent param accesses correctly
/// stay CHECKED (see `heap_sort_helper_param_reads_stay_checked`).
#[test]
fn heap_sort_main_reads_and_stores_elide() {
    let src = prog_src("heap_sort");
    let (ru, _rc, su, _sc) = bce_counts(&src);
    assert!(ru >= 3, "heap_sort Main: item 1/item (end+1)/item i must elide (got {ru})");
    assert!(su >= 2, "heap_sort Main: the two swap STORES must elide (got {su})");
    assert_eq!(assert_runpath_parity(&src, "20"), "1093 30133 319067");
}

/// graph_bfs: the CSR/dist reads through the locally-built length-`n` arrays
/// (`item (i+1) of adjStarts/adjCounts`, `item front of queue`,
/// `item (v+1) of adjStarts/adjCounts/dist`, `item (u+1) of dist`) elide, as
/// does the `dist[u+1]` store. The `adj` accesses (element-sourced base into a
/// `5*n`-length array) correctly stay CHECKED.
#[test]
fn graph_bfs_csr_reads_elide() {
    let src = prog_src("graph_bfs");
    let (ru, _rc, su, _sc) = bce_counts(&src);
    assert!(ru >= 6, "graph_bfs: the CSR/dist reads must elide (got {ru})");
    assert!(su >= 1, "graph_bfs: the dist[u+1] store must elide (got {su})");
    assert_eq!(assert_runpath_parity(&src, "40"), "40 98");
}

// ───────────────────────── NEGATIVE / SOUNDNESS BOUNDARY ──────────────────

/// matrix_mult's hot inner accesses index by `i*n + j + 1` where BOTH `i` and
/// `n` are variables — a nonlinear (variable-stride) index a linear-integer
/// prover cannot bound, and `length(c) = n*n` is a product the linear length
/// fact cannot hold. The inner reads/stores stay CHECKED; the program still
/// runs correctly and matches the tree-walker.
#[test]
fn matrix_mult_nonlinear_index_stays_checked() {
    let src = prog_src("matrix_mult");
    let (ru, rc, su, sc) = bce_counts(&src);
    assert_eq!(
        (ru, su),
        (0, 0),
        "matrix_mult's variable-stride indices must NOT be elided (got {ru} reads, {su} stores)"
    );
    assert!(rc >= 4 && sc >= 1, "the nonlinear accesses stay checked ({rc} reads, {sc} stores)");
    assert_eq!(assert_runpath_parity(&src, "6"), "66780");
}

/// heap_sort's `siftDown` helper reads `arr`/`result` PARAMETERS at indices
/// `2*root+1`, `root+1`, `swapIdx+1` with `root := swapIdx` (non-monotone,
/// data-dependent), and the parameter's length relative to `end`/`start` is
/// only known interprocedurally — a fact the VM/JIT path does not enforce (the
/// entry-guard precondition is AOT-only). Every helper access stays CHECKED.
#[test]
fn heap_sort_helper_param_reads_stay_checked() {
    let src = "## To siftDown (arr: Seq of Int, start: Int, end: Int) -> Seq of Int:\n\
               \x20   Let mutable result be arr.\n\
               \x20   Let mutable root be start.\n\
               \x20   While 2 * root + 1 is at most end:\n\
               \x20       Let child be 2 * root + 1.\n\
               \x20       Let mutable swapIdx be root.\n\
               \x20       If item (swapIdx + 1) of result is less than item (child + 1) of result:\n\
               \x20           Set swapIdx to child.\n\
               \x20       If swapIdx equals root:\n\
               \x20           Return result.\n\
               \x20       Let tmp be item (root + 1) of result.\n\
               \x20       Set item (root + 1) of result to item (swapIdx + 1) of result.\n\
               \x20       Set item (swapIdx + 1) of result to tmp.\n\
               \x20       Set root to swapIdx.\n\
               \x20   Return result.\n\
               ## Main\n\
               Let mutable a be a new Seq of Int.\n\
               Push 5 to a. Push 3 to a. Push 8 to a. Push 1 to a.\n\
               Let r be siftDown(a, 0, 3).\n\
               Show item 1 of r.\n";
    let (ru, _rc, su, _sc) = bce_counts(src);
    assert_eq!(
        (ru, su),
        (0, 0),
        "siftDown's data-dependent param accesses must NOT be elided ({ru} reads, {su} stores)"
    );
    let tier = ForgeTier::new();
    let (out, err) = with_optimized_program(src, |parsed, interner| match parsed {
        Ok((stmts, types, policies)) => logicaffeine_compile::vm::run_to_outcome_with_args(
            stmts,
            interner,
            Some(types),
            Some(&policies),
            &[],
            Some(&tier as &dyn NativeTier),
        ),
        Err(advice) => (String::new(), Some(advice)),
    });
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!((norm(&out), &err), (norm(&tw.output), &tw.error));
}

/// A USER-SUPPLIED index `item k of arr` (`k = parseInt(argv)`, opaque to the
/// optimizer) must stay CHECKED, and an OUT-OF-BOUNDS value must raise the
/// kernel's exact error on BOTH the eliding VM+JIT and the tree-walker. The
/// direct UB guard: were the access elided, the OOB read would be undefined
/// behavior instead of a clean error.
#[test]
fn user_supplied_oob_index_stays_checked_and_errors_identically() {
    let src = "## To native args () -> Seq of Text\n\
               ## To native parseInt (s: Text) -> Int\n\
               ## Main\n\
               Let arguments be args().\n\
               Let k be parseInt(item 2 of arguments).\n\
               Let mutable arr be a new Seq of Int.\n\
               Let mutable i be 1.\n\
               While i is at most 10:\n\
               \x20   Push i * i to arr.\n\
               \x20   Set i to i + 1.\n\
               Show item k of arr.\n";
    let (ru, rc, _su, _sc) = bce_counts(src);
    assert_eq!(ru, 0, "a user-supplied index must never be proven in-bounds (got {ru})");
    assert!(rc >= 1, "the user-index read must stay CHECKED (got {rc})");

    let in_argv = vec!["bench".to_string(), "3".to_string()];
    assert_eq!(assert_runpath_parity_argv(src, &in_argv), ("9".to_string(), None));

    // OUT OF BOUNDS: the kept check must error IDENTICALLY on both engines.
    let oob_argv = vec!["bench".to_string(), "99".to_string()];
    let (out, err) = assert_runpath_parity_argv(src, &oob_argv);
    assert!(err.is_some(), "index 99 of a 10-element array must error, not read OOB (out={out:?})");
}

/// Run the tiered VM+JIT run path AND the tree-walker on `argv`, assert they
/// agree, and return the (normalized output, error) for the caller's
/// value/error assertions.
fn assert_runpath_parity_argv(src: &str, argv: &[String]) -> (String, Option<String>) {
    let tier = ForgeTier::new();
    let (out, err) = with_optimized_program(src, |parsed, interner| match parsed {
        Ok((stmts, types, policies)) => logicaffeine_compile::vm::run_to_outcome_with_args(
            stmts,
            interner,
            Some(types),
            Some(&policies),
            argv,
            Some(&tier as &dyn NativeTier),
        ),
        Err(advice) => (String::new(), Some(advice)),
    });
    let tw = tw_outcome_with_args(src, argv);
    assert_eq!(
        (norm(&out), &err),
        (norm(&tw.output), &tw.error),
        "eliding VM+JIT diverged from the checked tree-walker on argv {argv:?}:\n{src}"
    );
    (norm(&out), err)
}
