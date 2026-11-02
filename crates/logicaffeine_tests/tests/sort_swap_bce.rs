//! The compare-swap bounds-check elision — the comparison-sort codegen win.
//!
//! A comparison sort's inner loop is dominated by a compare-swap of two array
//! elements. The peephole that recognizes it (`try_emit_swap_pattern`) used to
//! hardcode CHECKED indexing (`arr[(j-1) as usize]`), so bubble_sort — whose only
//! array accesses ARE the swap — carried a per-iteration bounds branch that C/Zig
//! do not, the whole of its ~1.2× gap. When the bounds-elision oracle proves BOTH
//! swapped indices in range (the loop guard `j <= n-1-i` bounds them, exactly as
//! the VM's `runpath_bce::bubble_sort_reads_and_stores_elide` already relied on),
//! the swap now lowers to `get_unchecked`/`get_unchecked_mut` on the real `largo`
//! build path — no bounds branch, and LLVM can then unroll the loop.
//!
//! These lock that the elision fires through the oracle-populated `compile_project`
//! path (`compile_to_rust` has no oracle, so the checked `__swap_tmp` form there is
//! locked separately by `e2e_codegen_optimization`). Soundness is oracle-gated: an
//! unproven index keeps the checked form, so this can never emit an out-of-bounds
//! `get_unchecked`.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::fs;

use tempfile::tempdir;

use logicaffeine_compile::compile::compile_project;

const BUBBLE_SORT: &str = include_str!("../../../benchmarks/programs/bubble_sort/main.lg");
const HEAP_SORT: &str = include_str!("../../../benchmarks/programs/heap_sort/main.lg");
const QUICKSORT: &str = include_str!("../../../benchmarks/programs/quicksort/main.lg");

fn emit_via_largo_path(src: &str) -> String {
    let dir = tempdir().unwrap();
    let main = dir.path().join("main.lg");
    fs::write(&main, src).unwrap();
    compile_project(&main)
        .expect("bubble_sort should compile through the project (largo) path")
        .rust_code
}

/// On the oracle-populated build path, bubble_sort's compare-swap must lower to
/// `get_unchecked` — the reads AND the swap stores — with NO checked `__swap_tmp`
/// form surviving (the whole swap is elided, so the hot loop has no bounds branch).
///
/// With `sortnest` on, bubble sort's nest runs as a heap sort and the nest survives only as the guarded fallback for
/// a window that does not fit, whose swap rightly keeps its checks; the nest is compiled with `## No sortnest`, so
/// this locks the swap mechanism on the nest itself (`sort_nest_lock` covers the heap sort).
#[test]
fn bubble_sort_swap_elides_bounds_checks_on_the_build_path() {
    let rust = emit_via_largo_path(&logicaffeine_compile::optimization::decorate_source(
        BUBBLE_SORT,
        &["sortnest"],
    ));
    assert!(
        rust.contains("get_unchecked"),
        "bubble_sort's compare-swap must lower to `get_unchecked` when the oracle \
         proves the indices in range:\n{rust}"
    );
    assert!(
        !rust.contains("__swap_tmp"),
        "the fully-elided swap must NOT keep the checked `__swap_tmp` form \
         (bubble_sort's only swap is the oracle-proven compare-swap):\n{rust}"
    );
}

/// heap_sort's `siftDown` swaps elide their bounds checks — the dominant
/// (`O(n log n)`) cost of the sort, now bounds-branch-free on the `largo` build path.
///
/// The swap-BCE MECHANISM (the peephole defers to `get_unchecked` once the oracle
/// proves BOTH swapped indices) was already in place; what was missing was the ORACLE
/// proving `siftDown`'s conditionally-assigned heap indices. That is now supplied by:
///  - `detect_heap_sift_guard`: the affine-index descent shape `while <affine(root)>
///    <= end` over a `Seq` parameter, emitting the entry precondition
///    `start >= 0 && end < arr.len()` and seeding `end < len` + a verified
///    counter lower bound;
///  - `try_prove_affine`'s derived-bound step: `s >= 0` (co-induction) and
///    `s <= end` (path-sensitive `sym_le_sym`) for a conditionally-assigned index
///    (`swapIdx`);
///  - the closing fix: `collect_local_affine_defs` feeds `child = 2*root+1` (a
///    single-`Let` iteration-invariant that `augment_body_scalar_defs` drops because
///    `root` is later reassigned) into `prove_le_bound` — SOUND because it only ever
///    combines with the loop guard, itself dropped once `root` is reassigned.
///
/// The `siftDown` body now has ZERO checked accesses, so the whole-program emit keeps
/// no `__swap_tmp` (the checked swap form). Soundness is oracle-gated + byte-exact:
/// heap_sort matches `expected_*.txt` at every size. (The MAIN-phase `arr[end]` swap
/// keeps 2 checked direct accesses — its monotone-DECREASING `end` bound is a distinct,
/// secondary case that does not affect this assertion; scoped in memory.)
#[test]
fn heap_sort_swaps_elide_bounds_checks_on_the_build_path() {
    let rust = emit_via_largo_path(HEAP_SORT);
    assert!(
        rust.contains("get_unchecked") && !rust.contains("__swap_tmp"),
        "heap_sort's swaps must lower to `get_unchecked` (no `__swap_tmp`):\n{rust}"
    );
}

/// quicksort's Lomuto partition swap elides its bounds checks — BOTH the in-loop
/// compare-swap AND the POST-LOOP pivot swap (`item i of arr` / `item hi of arr`
/// after the `while j < hi` scan). The in-loop swap was already de-guarded by the
/// entry-guard precondition + the `i <= j` 2-IV invariant; the pivot swap was the
/// residual — the affine index proof runs only for LOOP bodies, so a post-loop
/// access was never proven and kept its bounds branch (measured: ~16 extra
/// instructions LLVM could not drop, since SCEV cannot recover `i <= hi`).
///
/// That is now closed by two sound additions:
///  - `rich_walk_loop` records a loop-EXIT induction bound: for `while j < hi` with
///    `j` a top-level unconditional `+1` counter and `i` a monotone-inc var sharing
///    `j`'s entry def, the strict guard lands `j` on exactly `hi` and `i <= j` gives
///    `lo <= i <= hi` AFTER the loop (as symbolic `scalar_lower`/`scalar_upper`);
///  - the entry-guarded partition's function body gets a top-level affine pass
///    (`prove_toplevel_accesses`) — the same proof `affine_walk` runs over a loop
///    body, applied to the post-loop statements at their pre-state — so the pivot
///    swap discharges `1 <= i <= hi <= len` and lowers to `get_unchecked`.
///
/// Both are gated to the entry-guarded partition functions, so no other program's
/// oracle output changes. Soundness is oracle-gated + byte-exact (quicksort matches
/// `expected_*.txt` at every size).
#[test]
fn quicksort_partition_swap_elides_bounds_checks_on_the_build_path() {
    let rust = emit_via_largo_path(QUICKSORT);
    assert!(
        rust.contains("get_unchecked") && !rust.contains("__swap_tmp"),
        "quicksort's partition swap must lower to `get_unchecked` (no `__swap_tmp`):\n{rust}"
    );
}
