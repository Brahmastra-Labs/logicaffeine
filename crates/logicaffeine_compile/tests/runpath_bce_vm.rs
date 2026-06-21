//! RUN-PATH BOUNDS-CHECK ELIMINATION — the array-cluster coverage ratchet
//! (VM tier).
//!
//! This is the VM-only mirror of `logicaffeine_tests::runpath_bce` (which adds
//! the JIT tier). It lives in this crate because `logicaffeine_compile` carries
//! the Oracle + bytecode compiler with NO dependency on the JIT/forge crates,
//! so the structural decision (`IndexUnchecked`/`SetIndexUnchecked` vs the
//! checked forms) and the VM≡tree-walker parity it must preserve can be pinned
//! here independent of the codegen tier.
//!
//! Two properties on every shape, both load-bearing for SAFETY (an unchecked
//! load on an out-of-bounds index is undefined behavior — reading the heap
//! buffer past its end, which the frame canary cannot see):
//!
//!   1. DECISION SOUNDNESS — the structural assertion that ONLY a provably
//!      in-bounds access becomes unchecked. A negative companion pins that an
//!      access the analysis cannot prove (a user-supplied / data-dependent
//!      index, an off-by-one) stays CHECKED.
//!   2. EXECUTION SOUNDNESS — the eliding VM produces output IDENTICAL to the
//!      independent tree-walker (a wrong elision would yield a wrong value or
//!      miss an error → parity fails), including the OOB error path.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::optimize::oracle_analyze_with;
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::{Compiler, Op};

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// (unchecked Index reads, checked Index reads, unchecked stores, checked
/// stores) in the program's compiled bytecode, via the SAME Oracle path the
/// live engine uses (`oracle_analyze_with` over the parsed program).
fn bce_counts(src: &str) -> (usize, usize, usize, usize) {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _) = parsed.expect("parse");
        let oracle = oracle_analyze_with(stmts, interner);
        let prog = Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
            .expect("compile");
        let (mut ru, mut rc, mut su, mut sc) = (0, 0, 0, 0);
        for op in &prog.code {
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

/// VM (with BCE-driven unchecked ops) ≡ tree-walker (always checked) on the
/// same argv. Returns the normalized VM output for an exact-value assertion.
fn assert_parity_args(src: &str, size: &str) -> String {
    let argv = vec!["bench".to_string(), size.to_string()];
    let vm = vm_outcome_with_args(src, &argv, None);
    let tw = tw_outcome_with_args(src, &argv);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "eliding VM diverged from the checked tree-walker on:\n{src}"
    );
    norm(&vm.output)
}

fn prog_src(name: &str) -> String {
    let path = format!(
        "{}/../../benchmarks/programs/{name}/main.lg",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

// ─────────────────────────── REAL BENCHMARK SHAPES ────────────────────────
// Each test pins (a) the structural elision count for that program's hot
// array accesses and (b) the eliding VM's exact output equals the tree-walker.
// These ratchet the win: a future change that silently returns one of these
// hot loops to checked indexing fails the structural assertion loudly.

/// bubble_sort: adjacent-pair compare-and-swap. The expression-bound guard
/// `j <= n - 1 - i` proves BOTH reads `item j of arr` / `item (j+1) of arr`
/// (the `+1` fits inside the `-1-i` headroom) AND both swap STORES.
#[test]
fn bubble_sort_reads_and_stores_elide() {
    let src = prog_src("bubble_sort");
    let (ru, _rc, su, _sc) = bce_counts(&src);
    assert!(
        ru >= 2,
        "bubble_sort: both `item j` and `item (j+1)` reads must elide (got {ru})"
    );
    assert!(
        su >= 2,
        "bubble_sort: both swap STORES `item j`/`item (j+1)` must elide (got {su})"
    );
    assert_eq!(assert_parity_args(&src, "30"), "1071");
}

/// knapsack 0/1 DP: the suffix reads `item (w+1) of prev` (bare-offset) and
/// `item (w-wi+1) of prev` (multi-variable, proven by the kernel LIA prover
/// from the path guard `w >= wi`, the element bound `wi ∈ [1,50]`, and
/// `length(prev) = cols = capacity + 1`) plus the row-build reads
/// `item (i+1) of weights` / `item (i+1) of vals` all elide.
#[test]
fn knapsack_dp_reads_elide() {
    let src = prog_src("knapsack");
    let (ru, _rc, _su, _sc) = bce_counts(&src);
    assert!(
        ru >= 4,
        "knapsack: weights/vals row reads + prev[w+1] + prev[w-wi+1] must elide (got {ru})"
    );
    assert_eq!(assert_parity_args(&src, "12"), "260");
}

/// heap_sort: the Main-phase sift/checksum reads/stores on the locally-built
/// `arr` (length `n`) elide — `item 1 of arr`, `item (end+1) of arr` (guard
/// `end <= n-1`), `item i of arr` (checksum guard `i <= n`) plus the two
/// in-place swap stores. The siftDown helper's data-dependent param reads
/// (`item (root+1) of result`, indices `2*root+1` with `root := swapIdx`)
/// correctly stay CHECKED — see `heap_sort_helper_param_reads_stay_checked`.
#[test]
fn heap_sort_main_reads_and_stores_elide() {
    let src = prog_src("heap_sort");
    let (ru, _rc, su, _sc) = bce_counts(&src);
    assert!(
        ru >= 3,
        "heap_sort Main: item 1/item (end+1)/item i reads must elide (got {ru})"
    );
    assert!(
        su >= 2,
        "heap_sort Main: the two in-place swap STORES must elide (got {su})"
    );
    assert_eq!(assert_parity_args(&src, "20"), "1093 30133 319067");
}

/// graph_bfs: the CSR-style adjacency reads through the locally-built arrays of
/// length `n` (`item (i+1) of adjStarts/adjCounts`, `item front of queue`,
/// `item (v+1) of adjStarts/adjCounts/dist`, `item (u+1) of dist`) elide, as
/// does the `dist[u+1]` store. The `adj` accesses use an element-sourced base
/// (`start = item _ of adjStarts`) into an array of length `5*n` (a product the
/// linear length fact cannot hold) and correctly stay CHECKED.
#[test]
fn graph_bfs_csr_reads_elide() {
    let src = prog_src("graph_bfs");
    let (ru, _rc, su, _sc) = bce_counts(&src);
    assert!(
        ru >= 6,
        "graph_bfs: the CSR/dist reads through length-n arrays must elide (got {ru})"
    );
    assert!(su >= 1, "graph_bfs: the dist[u+1] store must elide (got {su})");
    assert_eq!(assert_parity_args(&src, "40"), "40 98");
}

// ───────────────────────── NEGATIVE / SOUNDNESS BOUNDARY ──────────────────
// The decision must REFUSE to prove what it cannot, and the kept check must
// still error EXACTLY like the tree-walker. These are the UB guard.

/// matrix_mult's hot inner accesses index by `i*n + j + 1` where BOTH `i` and
/// `n` are variables — a nonlinear (variable-stride) index a linear-integer
/// prover cannot bound, and `length(c) = n*n` is a product the linear length
/// fact cannot hold. So the inner reads/stores stay CHECKED. The program still
/// runs correctly (the indices ARE in bounds at runtime) and matches the
/// tree-walker exactly.
#[test]
fn matrix_mult_nonlinear_index_stays_checked() {
    let src = prog_src("matrix_mult");
    let (ru, rc, su, sc) = bce_counts(&src);
    assert_eq!(
        (ru, su),
        (0, 0),
        "matrix_mult's variable-stride indices must NOT be elided (got {ru} unchecked reads, {su} unchecked stores)"
    );
    assert!(rc >= 4 && sc >= 1, "the nonlinear accesses stay checked (got {rc} reads, {sc} stores)");
    assert_eq!(assert_parity_args(&src, "6"), "66780");
}

/// heap_sort's `siftDown` helper reads `arr`/`result` PARAMETERS at indices
/// `2*root+1`, `root+1`, `swapIdx+1` where `root := swapIdx` makes the index
/// non-monotone and data-dependent, and the parameter's length relative to
/// `end`/`start` is only known interprocedurally (caller passes `n-1`,
/// length `n`) — a fact the VM/JIT path does not enforce. So every helper
/// access stays CHECKED (the entry-guard precondition is AOT-only). Pinned so
/// no future change unsoundly elides a param read that could be out of bounds.
#[test]
fn heap_sort_helper_param_reads_stay_checked() {
    // The helper alone, with NO caller contract visible.
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
        "siftDown's data-dependent param accesses must NOT be elided (got {ru} reads, {su} stores)"
    );
    // Still runs and matches the tree-walker (the indices are valid here).
    let vm = vm_outcome_with_args(src, &[], None);
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!((norm(&vm.output), &vm.error), (norm(&tw.output), &tw.error));
}

/// A USER-SUPPLIED index — `item k of arr` where `k = parseInt(argv)` is opaque
/// to the optimizer — must stay CHECKED, and an OUT-OF-BOUNDS value must raise
/// the kernel's exact error on BOTH the eliding VM and the tree-walker. This is
/// the direct UB guard: were the access elided, the OOB read would be undefined
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
    // The data-dependent `item k of arr` must NOT be elided.
    let (ru, rc, _su, _sc) = bce_counts(src);
    assert_eq!(ru, 0, "a user-supplied index must never be proven in-bounds (got {ru})");
    assert!(rc >= 1, "the user-index read must stay CHECKED (got {rc})");

    // In bounds: identical value, no error.
    let in_argv = vec!["bench".to_string(), "3".to_string()];
    let vm_ok = vm_outcome_with_args(src, &in_argv, None);
    let tw_ok = tw_outcome_with_args(src, &in_argv);
    assert_eq!((norm(&vm_ok.output), &vm_ok.error), (norm(&tw_ok.output), &tw_ok.error));
    assert_eq!(vm_ok.error, None, "k=3 into a 10-element array must succeed");
    assert_eq!(norm(&vm_ok.output), "9");

    // OUT OF BOUNDS: the kept check must error IDENTICALLY on both engines.
    let oob_argv = vec!["bench".to_string(), "99".to_string()];
    let vm_err = vm_outcome_with_args(src, &oob_argv, None);
    let tw_err = tw_outcome_with_args(src, &oob_argv);
    assert_eq!(
        (norm(&vm_err.output), &vm_err.error),
        (norm(&tw_err.output), &tw_err.error),
        "an OOB user index must error identically on the VM and the tree-walker"
    );
    assert!(vm_err.error.is_some(), "index 99 of a 10-element array must error, not read OOB");
}
