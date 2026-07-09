//! BOUNDS-CHECK ELIMINATION (V8 TurboFan / LLVM SCEV style): the Oracle's
//! range analysis (M9) proves an index lies in `[1, length]`, the compiler
//! emits `IndexUnchecked`, and the JIT drops the bounds branch.
//!
//! Safety is NOT optional here — an unchecked load that is actually out of
//! bounds reads the heap buffer past its end (the frame canary cannot see
//! it). So the gate proves two things on every shape:
//!   1. CORRECTNESS — the tiered VM matches the independent tree-walker
//!      (a wrong elision yields a wrong value → parity fails).
//!   2. DECISION SOUNDNESS — only PROVABLY in-bounds indices become
//!      `IndexUnchecked`; off-by-one, resize-in-loop, and dynamic indices
//!      stay checked (no `IndexUnchecked` emitted for them).

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::optimize::oracle_analyze_with;
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::{Compiler, NativeTier, Op};
use logicaffeine_jit::ForgeTier;

/// Run a program with `argv = ["bench", size]` (mirrors `bench_corpus`), so
/// `parseInt(item 2 of arguments)` yields a value OPAQUE to the optimizer —
/// exactly how the real benchmarks make their size symbolic.
fn assert_parity_args(src: &str, size: &str, expect: &str) {
    let argv = vec!["bench".to_string(), size.to_string()];
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &argv, Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &argv);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "tiered VM (with BCE) diverged from tree-walker on:\n{src}"
    );
    assert_eq!(norm(&vm.output), expect);
}

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// (unchecked Index ops, checked Index ops) in Main's compiled bytecode,
/// using the same Oracle path the live engine uses.
fn index_op_counts(src: &str) -> (usize, usize) {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _) = parsed.expect("parse");
        let oracle = oracle_analyze_with(stmts, interner);
        let prog = Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
            .expect("compile");
        let mut unchecked = 0;
        let mut checked = 0;
        for op in &prog.code {
            match op {
                Op::IndexUnchecked { .. } => unchecked += 1,
                Op::Index { .. } => checked += 1,
                _ => {}
            }
        }
        (unchecked, checked)
    })
}

/// (unchecked store ops, checked store ops) in Main's compiled bytecode.
fn store_op_counts(src: &str) -> (usize, usize) {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _) = parsed.expect("parse");
        let oracle = oracle_analyze_with(stmts, interner);
        let prog = Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
            .expect("compile");
        let mut unchecked = 0;
        let mut checked = 0;
        for op in &prog.code {
            match op {
                Op::SetIndexUnchecked { .. } => unchecked += 1,
                Op::SetIndex { .. } => checked += 1,
                _ => {}
            }
        }
        (unchecked, checked)
    })
}

fn assert_parity(src: &str, expect: &str) {
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "tiered VM (with BCE) diverged from tree-walker on:\n{src}"
    );
    assert_eq!(norm(&vm.output), expect);
}

/// The canonical V8 case: `while i is at most length of arr` — the index is
/// the induction variable bounded by the array's length, the array is not
/// resized. The Oracle must prove it and the compiler must emit
/// `IndexUnchecked`; the result is exact.
#[test]
fn induction_variable_loop_elides_and_is_exact() {
    let src = "## Main\n\
               Let mutable arr be a new Seq of Int.\n\
               Let mutable i be 1.\n\
               While i is at most 5000:\n\
               \x20   Push i * i to arr.\n\
               \x20   Set i to i + 1.\n\
               Let mutable acc be 0.\n\
               Set i to 1.\n\
               While i is at most length of arr:\n\
               \x20   Set acc to (acc + item i of arr) % 1000000007.\n\
               \x20   Set i to i + 1.\n\
               Show acc.\n";
    let (unchecked, _checked) = index_op_counts(src);
    assert!(
        unchecked >= 1,
        "the induction-variable read `item i of arr` must be proven in-bounds and elided (got {unchecked} unchecked)"
    );
    // Σ i² for i in 1..=5000 = 41_679_167_500; mod 1_000_000_007 = 679_167_213.
    assert_parity(src, "679167213");
}

/// OFF-BY-ONE: `while i is at most length of arr + 1` — the last index is
/// out of range, so the Oracle must REFUSE to prove it; the access stays
/// checked and the program raises the kernel's exact error.
#[test]
fn off_by_one_is_not_elided_and_errors_exactly() {
    let src = "## Main\n\
               Let mutable arr be a new Seq of Int.\n\
               Let mutable i be 1.\n\
               While i is at most 100:\n\
               \x20   Push i to arr.\n\
               \x20   Set i to i + 1.\n\
               Let mutable acc be 0.\n\
               Set i to 1.\n\
               While i is at most length of arr + 1:\n\
               \x20   Set acc to acc + item i of arr.\n\
               \x20   Set i to i + 1.\n\
               Show acc.\n";
    let (unchecked, _) = index_op_counts(src);
    assert_eq!(
        unchecked, 0,
        "an off-by-one index must NOT be proven in-bounds — {unchecked} unsound elisions"
    );
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!((norm(&vm.output), &vm.error), (norm(&tw.output), &tw.error));
    assert!(vm.error.is_some(), "index 101 of a 100-element list must error");
}

/// RESIZE-IN-LOOP: the array grows inside the loop, so `length` is not
/// loop-invariant. The Oracle must not prove a fixed bound; behavior stays
/// exact regardless.
#[test]
fn resize_in_loop_stays_exact() {
    let src = "## Main\n\
               Let mutable arr be a new Seq of Int.\n\
               Push 1 to arr.\n\
               Let mutable i be 1.\n\
               While i is at most length of arr:\n\
               \x20   If i is less than 2000:\n\
               \x20       Push item i of arr + 1 to arr.\n\
               \x20   Set i to i + 1.\n\
               Show length of arr.\n";
    // i walks 1..=2000 as the array grows to 2000 elements.
    assert_parity(src, "2000");
}

// ───────────────────────── SYMBOLIC LENGTH (V8/LLVM allocation-size) ──────
// The real benchmarks never guard on `length of arr` directly. They build an
// array to a symbolic size `n` with a counted push-loop, then read it back
// with `i <= n` (and affine offsets `i ± k`). Proving these elidable needs
// the Oracle to track `length(arr) >= n` from the build loop — the standard
// allocation-size analysis — and then relate the read guard's bound variable
// to that length symbolically.

/// array_fill's shape: build `arr` to length `n` (counted `while i < n: push`,
/// `n` opaque from args), then `while i <= n: ... item i of arr ...`. The bare
/// induction read must be proven and elided.
#[test]
fn built_array_bare_read_elides_and_is_exact() {
    let src = "## To native args () -> Seq of Text\n\
               ## To native parseInt (s: Text) -> Int\n\
               ## Main\n\
               Let arguments be args().\n\
               Let n be parseInt(item 2 of arguments).\n\
               Let mutable arr be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is less than n:\n\
               \x20   Push (i * 7 + 3) % 1000000 to arr.\n\
               \x20   Set i to i + 1.\n\
               Let mutable sum be 0.\n\
               Set i to 1.\n\
               While i is at most n:\n\
               \x20   Set sum to (sum + item i of arr) % 1000000007.\n\
               \x20   Set i to i + 1.\n\
               Show sum.\n";
    let (unchecked, _) = index_op_counts(src);
    assert!(
        unchecked >= 1,
        "`item i of arr` with `i <= n` and `length(arr) >= n` from the build loop must elide (got {unchecked})"
    );
    // n = 10: values (i*7+3)%1e6 for i in 0..9, summed for i in 1..=10 → arr is
    // 0-based built, read 1-based so sum of arr[1..10] = build values i=1..9
    // plus i=10 → out of build range? arr has 10 elements (i=0..9); read i=1..10
    // reads arr[1..10] = build indices 0..9 → values for i=0..9: (0,10,17,24,
    // 31,38,45,52,59,66)? recompute below — rely on the tree-walker parity.
    let argv = vec!["bench".to_string(), "10".to_string()];
    let tw = tw_outcome_with_args(src, &argv);
    assert_parity_args(src, "10", &norm(&tw.output));
    assert_eq!(tw.error, None, "the proven program must not error");
}

/// prefix_sum's shape: build to `n`, then `while i is at most n` reading BOTH
/// `item i of arr` and the affine `item (i - 1) of arr` (i starts at 2, so
/// `i - 1 >= 1`). Both reads must be proven.
#[test]
fn built_array_affine_minus_one_elides_and_is_exact() {
    let src = "## To native args () -> Seq of Text\n\
               ## To native parseInt (s: Text) -> Int\n\
               ## Main\n\
               Let arguments be args().\n\
               Let n be parseInt(item 2 of arguments).\n\
               Let mutable arr be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is less than n:\n\
               \x20   Push i to arr.\n\
               \x20   Set i to i + 1.\n\
               Set i to 2.\n\
               While i is at most n:\n\
               \x20   Set item i of arr to (item i of arr + item (i - 1) of arr) % 1000000007.\n\
               \x20   Set i to i + 1.\n\
               Show item n of arr.\n";
    let (unchecked, _) = index_op_counts(src);
    assert!(
        unchecked >= 2,
        "both `item i of arr` and `item (i - 1) of arr` must be proven (got {unchecked})"
    );
    let argv = vec!["bench".to_string(), "50".to_string()];
    let tw = tw_outcome_with_args(src, &argv);
    assert_parity_args(src, "50", &norm(&tw.output));
    assert_eq!(tw.error, None);
}

/// ADVERSARIAL — affine `+1` against an EXACT-`n` length: `i <= n` with
/// `length(arr) >= n` means `item (i + 1) of arr` reaches `n + 1` at `i = n`
/// — out of bounds. Must NOT be elided.
#[test]
fn built_array_affine_plus_one_overflow_not_elided() {
    let src = "## To native args () -> Seq of Text\n\
               ## To native parseInt (s: Text) -> Int\n\
               ## Main\n\
               Let arguments be args().\n\
               Let n be parseInt(item 2 of arguments).\n\
               Let mutable arr be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is less than n:\n\
               \x20   Push i to arr.\n\
               \x20   Set i to i + 1.\n\
               Let mutable acc be 0.\n\
               Set i to 1.\n\
               While i is at most n:\n\
               \x20   Set acc to acc + item (i + 1) of arr.\n\
               \x20   Set i to i + 1.\n\
               Show acc.\n";
    let (unchecked, _) = index_op_counts(src);
    assert_eq!(unchecked, 0, "`item (i + 1) of arr` reaches n+1 — must stay checked");
    // It errors (index n+1 of an n-element list); both engines agree.
    let argv = vec!["bench".to_string(), "50".to_string()];
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &argv, Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &argv);
    assert_eq!((norm(&vm.output), &vm.error), (norm(&tw.output), &tw.error));
    assert!(vm.error.is_some());
}

/// ADVERSARIAL — the size variable is REASSIGNED after the build, so the
/// length fact `length(arr) >= n` no longer holds for the new `n`. Must NOT
/// be elided.
#[test]
fn built_array_bound_reassigned_not_elided() {
    let src = "## To native args () -> Seq of Text\n\
               ## To native parseInt (s: Text) -> Int\n\
               ## Main\n\
               Let arguments be args().\n\
               Let mutable n be parseInt(item 2 of arguments).\n\
               Let mutable arr be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is less than n:\n\
               \x20   Push i to arr.\n\
               \x20   Set i to i + 1.\n\
               Set n to n + 10.\n\
               Let mutable acc be 0.\n\
               Set i to 1.\n\
               While i is at most n:\n\
               \x20   Set acc to acc + item i of arr.\n\
               \x20   Set i to i + 1.\n\
               Show acc.\n";
    let (unchecked, _) = index_op_counts(src);
    assert_eq!(unchecked, 0, "n was grown after the build — the length fact is stale");
    let argv = vec!["bench".to_string(), "50".to_string()];
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &argv, Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &argv);
    assert_eq!((norm(&vm.output), &vm.error), (norm(&tw.output), &tw.error));
    assert!(vm.error.is_some());
}

/// ADVERSARIAL — a CONDITIONAL push in the build loop means `length(arr)` is
/// NOT provably `>= n`; reading `i <= n` could exceed it. Must NOT be elided.
#[test]
fn built_array_conditional_push_not_elided() {
    let src = "## To native args () -> Seq of Text\n\
               ## To native parseInt (s: Text) -> Int\n\
               ## Main\n\
               Let arguments be args().\n\
               Let n be parseInt(item 2 of arguments).\n\
               Let mutable arr be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is less than n:\n\
               \x20   If i % 2 equals 0:\n\
               \x20       Push i to arr.\n\
               \x20   Set i to i + 1.\n\
               Let mutable acc be 0.\n\
               Set i to 1.\n\
               While i is at most n:\n\
               \x20   Set acc to acc + item i of arr.\n\
               \x20   Set i to i + 1.\n\
               Show acc.\n";
    let (unchecked, _) = index_op_counts(src);
    assert_eq!(unchecked, 0, "conditional push → length < n possible → must stay checked");
    let argv = vec!["bench".to_string(), "50".to_string()];
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &argv, Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &argv);
    assert_eq!((norm(&vm.output), &vm.error), (norm(&tw.output), &tw.error));
    assert!(vm.error.is_some());
}

/// Count `RegionBoundsGuard` ops in the whole program (Main + functions share
/// one code vector).
fn region_guard_count(src: &str) -> usize {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _) = parsed.expect("parse");
        let oracle = oracle_analyze_with(stmts, interner);
        let prog = Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
            .expect("compile");
        prog.code
            .iter()
            .filter(|op| matches!(op, Op::RegionBoundsGuard { .. }))
            .count()
    })
}

// ───────────────────────── RUNTIME HOISTING (passed/parameter arrays) ──────
// When an array's length is not statically provable (e.g. a function
// parameter), the loop's accesses are speculatively in bounds, justified by a
// SINGLE region-entry runtime check (`RegionBoundsGuard`). The VM verifies it
// once before entering native code and declines the region (replays on
// bytecode) on failure — V8 TurboFan loop bound-check elimination.

const SCAN_FN: &str = "## To native args () -> Seq of Text\n\
                       ## To native parseInt (s: Text) -> Int\n\
                       ## To scan (arr: Seq of Int, hi: Int) -> Int:\n\
                       \x20   Let mutable acc be 0.\n\
                       \x20   Let mutable j be 1.\n\
                       \x20   While j is at most hi:\n\
                       \x20       Set acc to (acc + item j of arr) % 1000000007.\n\
                       \x20       Set j to j + 1.\n\
                       \x20   Return acc.\n";

/// The parameter array `arr` (no static length proof) under `j <= hi` (hi a
/// parameter, loop-invariant) must hoist: a `RegionBoundsGuard` plus the
/// speculative `IndexUnchecked` read.
#[test]
fn passed_array_hoists() {
    let src = format!(
        "{SCAN_FN}\
         ## Main\n\
         Let arguments be args().\n\
         Let n be parseInt(item 2 of arguments).\n\
         Let mutable arr be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push i to arr.\n\
         \x20   Set i to i + 1.\n\
         Show scan(arr, n).\n"
    );
    let guards = region_guard_count(&src);
    let (unchecked, _) = index_op_counts(&src);
    assert!(guards >= 1, "scan's loop must emit a region-entry bounds guard (got {guards})");
    assert!(unchecked >= 1, "the parameter-array read must elide speculatively (got {unchecked})");
}

/// The offsets `(add_max, add_min)` of the first `RegionBoundsGuard` emitted.
fn first_region_guard(src: &str) -> Option<(i32, i32)> {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _) = parsed.expect("parse");
        let oracle = oracle_analyze_with(stmts, interner);
        let prog = Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
            .expect("compile");
        prog.code.iter().find_map(|op| match op {
            Op::RegionBoundsGuard { add_max, add_min, .. } => Some((*add_max, *add_min)),
            _ => None,
        })
    })
}

/// The guard MATH must be exact. `scan`'s `item j of arr` under `j <= hi`
/// (offset 0, non-strict) needs `len(arr) >= hi` and `j >= 1` — i.e.
/// `add_max == 0`, `add_min == 0`. A too-large `add_max` makes the guard
/// always fail (no win); a too-SMALL one makes it too weak (an OOB read).
#[test]
fn hoist_guard_offsets_are_exact() {
    let src = format!(
        "{SCAN_FN}\
         ## Main\n\
         Let arguments be args().\n\
         Let n be parseInt(item 2 of arguments).\n\
         Let mutable arr be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push i to arr.\n\
         \x20   Set i to i + 1.\n\
         Show scan(arr, n).\n"
    );
    assert_eq!(
        first_region_guard(&src),
        Some((0, 0)),
        "scan's `item j of arr` under `j <= hi` needs exactly len >= hi and j >= 1"
    );
}

/// SAFETY (tight boundary) — the array is EXACTLY one element short. A guard
/// that's even one too weak would run unchecked and read out of bounds; it
/// must decline and the result must match the tree-walker.
#[test]
fn hoist_guard_rejects_off_by_one_short() {
    let src = format!(
        "{SCAN_FN}\
         ## Main\n\
         Let arguments be args().\n\
         Let n be parseInt(item 2 of arguments).\n\
         Let mutable arr be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push i to arr.\n\
         \x20   Set i to i + 1.\n\
         Let mutable total be 0.\n\
         Let mutable r be 0.\n\
         While r is less than 50:\n\
         \x20   Set total to (total + scan(arr, n + 1)) % 1000000007.\n\
         \x20   Set r to r + 1.\n\
         Show total.\n"
    );
    // hi = n + 1 but the array has exactly n elements → reading item n+1 is
    // out of bounds by one. The entry guard `len >= hi` must reject it.
    assert!(vm_tw_agree(&src, "300"), "one element short must still error on both engines");
}

/// Diagnostic: how many region guards + unchecked accesses each passed-array
/// program emits. Printed with `--nocapture`.
#[test]
fn passed_array_hoist_breadth_report() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../benchmarks/programs");
    for name in [
        "quicksort",
        "mergesort",
        "heap_sort",
        "nbody",
        "matrix_mult",
        "graph_bfs",
        "counting_sort",
        "two_sum",
    ] {
        let path = format!("{dir}/{name}/main.lg");
        let Ok(src) = std::fs::read_to_string(&path) else { continue };
        let guards = region_guard_count(&src);
        let (un_r, _) = index_op_counts(&src);
        let (un_s, _) = store_op_counts(&src);
        println!("HOIST {name}: guards={guards} unchecked_reads={un_r} unchecked_stores={un_s}");
    }
    // Ratchet: quicksort sorts a PASSED array (`result` aliases the `arr`
    // parameter), so its `item j of result` accesses must hoist behind a
    // region-entry guard — the canonical runtime-hoist win.
    let qs = std::fs::read_to_string(format!("{dir}/quicksort/main.lg")).unwrap();
    assert!(
        region_guard_count(&qs) >= 1,
        "quicksort's passed-array loop must emit a region-entry bounds guard"
    );
    // mergesort builds `left`/`right` from the `arr` parameter — the
    // "build B from A" loops hoist the parameter reads.
    let ms = std::fs::read_to_string(format!("{dir}/mergesort/main.lg")).unwrap();
    assert!(
        region_guard_count(&ms) >= 1,
        "mergesort's build-from-parameter loops must emit region-entry guards"
    );
}

/// Correct when the array is long enough (the common path — guard holds).
#[test]
fn passed_array_hoist_is_exact() {
    let src = format!(
        "{SCAN_FN}\
         ## Main\n\
         Let arguments be args().\n\
         Let n be parseInt(item 2 of arguments).\n\
         Let mutable arr be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push (i * 3 + 1) % 1000 to arr.\n\
         \x20   Set i to i + 1.\n\
         Let mutable total be 0.\n\
         Let mutable r be 0.\n\
         While r is less than 50:\n\
         \x20   Set total to (total + scan(arr, n)) % 1000000007.\n\
         \x20   Set r to r + 1.\n\
         Show total.\n"
    );
    assert!(!vm_tw_agree(&src, "300"), "in-bounds — no error");
}

/// SAFETY GATE — the array is SHORTER than the bound. Whatever tier runs (the
/// interpreter's checked access, or a compiled region whose entry guard fails
/// and deopts), the result must match the tree-walker EXACTLY: a wrong elision
/// would read out of bounds and DIVERGE. Both engines raise the same error.
#[test]
fn passed_array_too_short_is_exact_and_errors() {
    // Build `arr` to length n, then ask scan to read up to n + 50 — out of
    // bounds. The region (if it forms) must decline via the entry guard.
    let src = format!(
        "{SCAN_FN}\
         ## Main\n\
         Let arguments be args().\n\
         Let n be parseInt(item 2 of arguments).\n\
         Let mutable arr be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push i to arr.\n\
         \x20   Set i to i + 1.\n\
         Let mutable total be 0.\n\
         Let mutable r be 0.\n\
         While r is less than 50:\n\
         \x20   Set total to (total + scan(arr, n + 50)) % 1000000007.\n\
         \x20   Set r to r + 1.\n\
         Show total.\n"
    );
    assert!(vm_tw_agree(&src, "300"), "reading past the array must error on BOTH engines");
}

/// Run on both engines with a symbolic size; assert exact agreement and
/// return whether the run errored. The single safety invariant: the tiered
/// VM (with BCE) must behave EXACTLY like the independent tree-walker.
fn vm_tw_agree(src: &str, size: &str) -> bool {
    let argv = vec!["bench".to_string(), size.to_string()];
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &argv, Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &argv);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "tiered VM (with BCE) diverged from tree-walker on:\n{src}"
    );
    vm.error.is_some()
}

const BUILD_HDR: &str = "## To native args () -> Seq of Text\n\
                         ## To native parseInt (s: Text) -> Int\n\
                         ## Main\n\
                         Let arguments be args().\n\
                         Let n be parseInt(item 2 of arguments).\n";

/// `Let b be arr` is a VALUE COPY, so popping `b` leaves `arr` untouched — its
/// length really is still `n`, and reading all `n` elements succeeds (no error).
/// The optimizer stays conservative around the copy+pop and does NOT elide
/// (`unchecked == 0`), but correctness no longer hinges on that: the original is
/// isolated from the copy's mutation. (Under the old reference semantics this
/// same program shrank `arr` through the alias and read out of bounds.)
#[test]
fn pop_through_value_copy_leaves_original_intact() {
    let src = format!(
        "{BUILD_HDR}\
         Let mutable arr be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push i to arr.\n\
         \x20   Set i to i + 1.\n\
         Let mutable b be arr.\n\
         Pop from b.\n\
         Let mutable acc be 0.\n\
         Set i to 1.\n\
         While i is at most n:\n\
         \x20   Set acc to acc + item i of arr.\n\
         \x20   Set i to i + 1.\n\
         Show acc.\n"
    );
    let (unchecked, _) = index_op_counts(&src);
    assert_eq!(unchecked, 0, "the optimizer stays conservative around a copy+pop");
    // No error, and acc = 0 + 1 + … + (n-1); for n = 50 that is 1225.
    assert!(!vm_tw_agree(&src, "50"), "reading all n elements of the untouched original must succeed");
    let tier = ForgeTier::new();
    let out = vm_outcome_with_args(&src, &["bench".to_string(), "50".to_string()], Some(&tier as &dyn NativeTier));
    assert_eq!(norm(&out.output), "1225", "arr is untouched by the copy's pop");
}

/// An aliased READ stays correct (a missed elision is fine; a wrong one is
/// not). No assertion on the decision — only that the two engines agree.
#[test]
fn aliased_read_stays_correct() {
    let src = format!(
        "{BUILD_HDR}\
         Let mutable arr be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push i * 2 to arr.\n\
         \x20   Set i to i + 1.\n\
         Let mutable b be arr.\n\
         Let mutable acc be 0.\n\
         Set i to 1.\n\
         While i is at most n:\n\
         \x20   Set acc to (acc + item i of b) % 1000000007.\n\
         \x20   Set i to i + 1.\n\
         Show acc.\n"
    );
    assert!(!vm_tw_agree(&src, "200"), "the aliased read is in bounds — no error");
}

/// ADVERSARIAL — the induction variable is mutated (by more than the step)
/// BEFORE the read, so the guard's bound no longer holds at the access. Must
/// NOT be elided.
#[test]
fn induction_mutated_before_read_not_elided() {
    let src = format!(
        "{BUILD_HDR}\
         Let mutable arr be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push i to arr.\n\
         \x20   Set i to i + 1.\n\
         Let mutable acc be 0.\n\
         Set i to 1.\n\
         While i is at most n:\n\
         \x20   Set i to i + 3.\n\
         \x20   Set acc to acc + item i of arr.\n\
         Show acc.\n"
    );
    let (unchecked, _) = index_op_counts(&src);
    assert_eq!(unchecked, 0, "index mutated before the read — guard bound is stale");
    assert!(vm_tw_agree(&src, "50"), "i overshoots n → out of bounds → error");
}

/// A strict `<` guard with a bare index starting at 1: `i ∈ [1, n-1]`,
/// `length(arr) >= n` → in bounds. Must elide.
#[test]
fn strict_guard_bare_from_one_elides() {
    let src = format!(
        "{BUILD_HDR}\
         Let mutable arr be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push i * 3 to arr.\n\
         \x20   Set i to i + 1.\n\
         Let mutable acc be 0.\n\
         Set i to 1.\n\
         While i is less than n:\n\
         \x20   Set acc to (acc + item i of arr) % 1000000007.\n\
         \x20   Set i to i + 1.\n\
         Show acc.\n"
    );
    let (unchecked, _) = index_op_counts(&src);
    assert!(unchecked >= 1, "i<n from 1 with length>=n must elide (got {unchecked})");
    assert!(!vm_tw_agree(&src, "200"), "in bounds — no error");
}

/// ADVERSARIAL — a bare index starting at 0 reads `item 0` (invalid 1-based),
/// so the lower-bound check `iv_lo + k >= 1` must REFUSE it.
#[test]
fn bare_index_from_zero_not_elided() {
    let src = format!(
        "{BUILD_HDR}\
         Let mutable arr be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push i to arr.\n\
         \x20   Set i to i + 1.\n\
         Let mutable acc be 0.\n\
         Set i to 0.\n\
         While i is less than n:\n\
         \x20   Set acc to acc + item i of arr.\n\
         \x20   Set i to i + 1.\n\
         Show acc.\n"
    );
    let (unchecked, _) = index_op_counts(&src);
    assert_eq!(unchecked, 0, "`item 0 of arr` is out of bounds (1-based) — must stay checked");
    assert!(vm_tw_agree(&src, "50"), "item 0 errors on both engines");
}

/// A FLIPPED guard `n is at least i` (i.e. `i <= n`) proves the same bound.
#[test]
fn flipped_ge_guard_elides() {
    let src = format!(
        "{BUILD_HDR}\
         Let mutable arr be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push i + 1 to arr.\n\
         \x20   Set i to i + 1.\n\
         Let mutable acc be 0.\n\
         Set i to 1.\n\
         While n is at least i:\n\
         \x20   Set acc to (acc + item i of arr) % 1000000007.\n\
         \x20   Set i to i + 1.\n\
         Show acc.\n"
    );
    let (unchecked, _) = index_op_counts(&src);
    assert!(unchecked >= 1, "`n >= i` is the same bound as `i <= n` (got {unchecked})");
    assert!(!vm_tw_agree(&src, "200"));
}

/// TWO arrays, both built to `n`, both read under the same guard — each read
/// resolves against its own length fact.
#[test]
fn two_arrays_both_elide() {
    let src = format!(
        "{BUILD_HDR}\
         Let mutable a be a new Seq of Int.\n\
         Let mutable b be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push i to a.\n\
         \x20   Set i to i + 1.\n\
         Set i to 0.\n\
         While i is less than n:\n\
         \x20   Push i * 2 to b.\n\
         \x20   Set i to i + 1.\n\
         Let mutable acc be 0.\n\
         Set i to 1.\n\
         While i is at most n:\n\
         \x20   Set acc to (acc + item i of a + item i of b) % 1000000007.\n\
         \x20   Set i to i + 1.\n\
         Show acc.\n"
    );
    let (unchecked, _) = index_op_counts(&src);
    assert!(unchecked >= 2, "both `item i of a` and `item i of b` must elide (got {unchecked})");
    assert!(!vm_tw_agree(&src, "200"));
}

/// A CONJOINED guard `i <= n and i <= m` over two differently-sized arrays —
/// each access proven by the conjunct that names its size variable.
#[test]
fn conjunction_guards_elide() {
    let src = format!(
        "{BUILD_HDR}\
         Let m be n - 5.\n\
         Let mutable a be a new Seq of Int.\n\
         Let mutable b be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push i to a.\n\
         \x20   Set i to i + 1.\n\
         Set i to 0.\n\
         While i is less than m:\n\
         \x20   Push i to b.\n\
         \x20   Set i to i + 1.\n\
         Let mutable acc be 0.\n\
         Set i to 1.\n\
         While i is at most n and i is at most m:\n\
         \x20   Set acc to (acc + item i of a + item i of b) % 1000000007.\n\
         \x20   Set i to i + 1.\n\
         Show acc.\n"
    );
    let (unchecked, _) = index_op_counts(&src);
    assert!(unchecked >= 2, "each conjunct proves its own array (got {unchecked})");
    assert!(!vm_tw_agree(&src, "200"));
}

// ───────────────────────────────────── STORE-BCE (the sorts/scatter side) ──

/// A proven STORE `Set item i of arr to v` (i bounded by `n`, `length>=n`)
/// must lower to `SetIndexUnchecked`.
#[test]
fn proven_store_elides_and_is_exact() {
    let src = format!(
        "{BUILD_HDR}\
         Let mutable arr be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push 0 to arr.\n\
         \x20   Set i to i + 1.\n\
         Set i to 1.\n\
         While i is at most n:\n\
         \x20   Set item i of arr to (i * i) % 1000000007.\n\
         \x20   Set i to i + 1.\n\
         Show item n of arr.\n"
    );
    let (unchecked, _) = store_op_counts(&src);
    assert!(unchecked >= 1, "proven store must elide its bounds check (got {unchecked})");
    assert!(!vm_tw_agree(&src, "200"), "in-bounds store — no error");
}

/// ADVERSARIAL — a store at `i + 1` under `i <= n` (length `n`) overruns at
/// `i = n`. Must stay checked.
#[test]
fn store_affine_overflow_not_elided() {
    let src = format!(
        "{BUILD_HDR}\
         Let mutable arr be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push 0 to arr.\n\
         \x20   Set i to i + 1.\n\
         Set i to 1.\n\
         While i is at most n:\n\
         \x20   Set item (i + 1) of arr to i.\n\
         \x20   Set i to i + 1.\n\
         Show item n of arr.\n"
    );
    let (unchecked, _) = store_op_counts(&src);
    assert_eq!(unchecked, 0, "store at n+1 must stay checked");
    assert!(vm_tw_agree(&src, "50"), "the overrun store errors on both engines");
}

/// A data-dependent store index stays checked (still in bounds, just not
/// provably so) — the two engines must agree.
#[test]
fn store_dynamic_index_not_elided() {
    let src = format!(
        "{BUILD_HDR}\
         Let mutable arr be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push 0 to arr.\n\
         \x20   Set i to i + 1.\n\
         Let mutable seed be 7.\n\
         Set i to 1.\n\
         While i is at most n:\n\
         \x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
         \x20   Set item ((seed % n) + 1) of arr to i.\n\
         \x20   Set i to i + 1.\n\
         Show item 1 of arr.\n"
    );
    let (unchecked, _) = store_op_counts(&src);
    assert_eq!(unchecked, 0, "a data-dependent store index must not be elided");
    assert!(!vm_tw_agree(&src, "200"), "the dynamic store stays in bounds");
}

/// ADVERSARIAL — a guard `j <= n - e` where the subtracted term `e` can be
/// NEGATIVE makes `n - e > n`, so `j` can overrun `length(arr) = n`. The
/// headroom proof must REQUIRE the subtracted term to be provably `>= 0`.
#[test]
fn negative_subtrahend_bound_not_elided() {
    let src = format!(
        "{BUILD_HDR}\
         Let e be 0 - 5.\n\
         Let mutable arr be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push i to arr.\n\
         \x20   Set i to i + 1.\n\
         Let mutable acc be 0.\n\
         Let mutable j be 1.\n\
         While j is at most n - e:\n\
         \x20   Set acc to acc + item j of arr.\n\
         \x20   Set j to j + 1.\n\
         Show acc.\n"
    );
    let (unchecked, _) = index_op_counts(&src);
    assert_eq!(unchecked, 0, "`n - e` with e possibly negative exceeds n — must stay checked");
    assert!(vm_tw_agree(&src, "50"), "j reaches n+5 > length → out of bounds → error");
}

/// STORE BREADTH RATCHET — prefix_sum's `Set item i of arr` and sieve's inner
/// `Set item (j + 1) of flags` are now provable stores.
#[test]
fn real_benchmarks_eliminate_store_bounds_checks() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../benchmarks/programs");
    let expect: &[(&str, usize)] = &[("prefix_sum", 1), ("sieve", 1), ("bubble_sort", 2)];
    for (name, min_unchecked) in expect {
        let path = format!("{dir}/{name}/main.lg");
        let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        let (unchecked, _) = store_op_counts(&src);
        assert!(
            unchecked >= *min_unchecked,
            "{name}: expected >= {min_unchecked} eliminated store checks, got {unchecked}"
        );
    }
}

/// BREADTH RATCHET — the real benchmark programs. Each array-cluster program
/// builds its array(s) with a counted loop and reads them back with a guard
/// the analysis must now prove. This pins the actual win (and fails loudly if
/// a future change silently returns these hot loops to checked indexing).
/// Correctness for these exact programs is covered by `bench_corpus`
/// (vm ≡ tree-walker on all 30); here we assert the elision FIRES.
#[test]
fn real_benchmarks_eliminate_bounds_checks() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../benchmarks/programs");
    // (program, minimum proven `IndexUnchecked` reads expected).
    let expect: &[(&str, usize)] = &[
        ("array_fill", 1),
        ("prefix_sum", 2),
        ("sieve", 1),
        ("histogram", 1),
        // Expression-bound guard `j <= n - 1 - i`: both `item j` and
        // `item (j + 1)` reads are proven.
        ("bubble_sort", 2),
    ];
    for (name, min_unchecked) in expect {
        let path = format!("{dir}/{name}/main.lg");
        let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        let (unchecked, _) = index_op_counts(&src);
        assert!(
            unchecked >= *min_unchecked,
            "{name}: expected >= {min_unchecked} eliminated bounds checks, got {unchecked}"
        );
    }
}

/// A data-dependent index `(seed % 1000) + 1` into an array of EXACTLY 1000
/// elements IS provably in bounds: the LITERAL-divisor modulo gives
/// `seed % 1000 ∈ [0, 999]` (A1 interval analysis), so the 1-based index sits
/// in `[1, 1000]` = the array's valid range. The check elides — value-range
/// analysis reaching a "dynamic" index a counter/relational proof cannot.
#[test]
fn modulo_index_into_exact_length_elides() {
    let src = "## Main\n\
               Let mutable arr be a new Seq of Int.\n\
               Let mutable i be 1.\n\
               While i is at most 1000:\n\
               \x20   Push i to arr.\n\
               \x20   Set i to i + 1.\n\
               Let mutable seed be 7.\n\
               Let mutable acc be 0.\n\
               Set i to 1.\n\
               While i is at most 1000:\n\
               \x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
               \x20   Set acc to (acc + item ((seed % 1000) + 1) of arr) % 1000000007.\n\
               \x20   Set i to i + 1.\n\
               Show acc.\n";
    let (unchecked, _) = index_op_counts(src);
    assert!(unchecked >= 1, "`seed % 1000` ∈ [0,999] into a 1000-element array is provable");
    // The elision must be CORRECT: VM (eliding) ≡ tree-walker (checked).
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!((norm(&vm.output), &vm.error), (norm(&tw.output), &tw.error));
    assert_eq!(vm.error, None);
}

/// The genuinely-UNPROVABLE companion: a VARIABLE-divisor modulo `seed % n`.
/// At runtime `seed % n ∈ [0, n-1]` so `arr[(seed%n)+1]` is in bounds (`arr`
/// has `n` elements) and the program runs correctly — but the analysis cannot
/// bound a modulo by a non-literal divisor, so the index stays CHECKED. This
/// pins that A1 does not over-reach to variable divisors (which would be
/// unsound for a negative or zero `n`).
#[test]
fn variable_divisor_index_stays_checked() {
    let src = "## To native args () -> Seq of Text\n\
               ## To native parseInt (s: Text) -> Int\n\
               ## Main\n\
               Let arguments be args().\n\
               Let n be parseInt(item 2 of arguments).\n\
               Let mutable arr be a new Seq of Int.\n\
               Let mutable i be 1.\n\
               While i is at most n:\n\
               \x20   Push i to arr.\n\
               \x20   Set i to i + 1.\n\
               Let mutable seed be 7.\n\
               Let mutable acc be 0.\n\
               Set i to 1.\n\
               While i is at most n:\n\
               \x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
               \x20   Set acc to (acc + item ((seed % n) + 1) of arr) % 1000000007.\n\
               \x20   Set i to i + 1.\n\
               Show acc.\n";
    let (unchecked, _) = index_op_counts(src);
    assert_eq!(unchecked, 0, "a modulo by a VARIABLE divisor must not be elided");
    let argv = vec!["bench".to_string(), "1000".to_string()];
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &argv, Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &argv);
    assert_eq!((norm(&vm.output), &vm.error), (norm(&tw.output), &tw.error));
    assert_eq!(vm.error, None);
}

/// RELATIONAL two-induction-variable bound (the quicksort/Lomuto partition
/// shape): `i` and `j` both start at the same value, `j` increments every
/// iteration, `i` increments only on a guarded branch — so `i <= j` is a loop
/// invariant. Under the guard `j < 64` (and `length of arr = 64`) the access
/// `item i of arr` is in bounds, but only RELATIONAL reasoning (i <= j) proves
/// it; the interval domain widens `i` to an unknown upper bound. The Oracle must
/// derive `i <= j` and elide the `item i` check, exactly as it elides `item j`.
#[test]
fn relational_i_le_j_partition_elides_and_is_exact() {
    let src = "## Main\n\
               Let mutable arr be a new Seq of Int.\n\
               Let mutable b be 1.\n\
               While b is at most 64:\n\
               \x20   Push b to arr.\n\
               \x20   Set b to b + 1.\n\
               Let lo be 1.\n\
               Let mutable i be lo.\n\
               Let mutable j be lo.\n\
               Let mutable acc be 0.\n\
               While j is less than 64:\n\
               \x20   If item j of arr is at most 50:\n\
               \x20       Let x be item i of arr.\n\
               \x20       Set acc to acc + x.\n\
               \x20       Set i to i + 1.\n\
               \x20   Set j to j + 1.\n\
               Show acc.\n";
    let (unchecked, checked) = index_op_counts(src);
    // `item j` (j < length) AND `item i` (i <= j < length) must both elide.
    assert_eq!(
        (unchecked, checked),
        (2, 0),
        "both the j-read and the relational i-read must be proven in-bounds"
    );
    // arr = [1..64]; for each j in 1..63 with arr[j] <= 50 (j in 1..50), acc +=
    // arr[i] where i walks 1,2,3,... So acc = sum of arr[1..50] = 1+2+...+50 =
    // 1275 (i increments exactly when the guard holds; the i-th hit reads arr[i]).
    assert_parity(src, "1275");
}
