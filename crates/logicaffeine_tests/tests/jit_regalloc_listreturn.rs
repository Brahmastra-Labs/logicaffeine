//! WS-G WAVE 22: the LIST-RETURN recursion shape (mergesort) tiers through the
//! CONTIGUOUS register-allocating FUNCTION backend.
//!
//! A recursive function that ALLOCATES fresh `left`/`right`/`result` Seqs each
//! level (`NewList` + reallocating `ArrPush`) and RETURNS a new list previously
//! tiered only through the per-piece precise stencil tier
//! (`regalloc_function_count() == 0`). The function-precise regalloc backend
//! (`compile_function_regalloc_precise`) excluded `NewList`/`ArrPush`/`ListClear`;
//! this wave admits them — the registry-owned fresh-list detach + precise resume
//! over a reallocating push is sound because the regalloc body shares the SAME
//! `adapt_function` micro stream, deopt-code table, runtime helpers, and
//! `ChainFn` boundary as the stencil tier; only the body codegen changes.
//!
//! Every test proves three things:
//!   1. bit-identical to the tree-walker (output AND error), incl. deopt,
//!   2. the function tiered through the regalloc backend
//!      (`regalloc_function_count() >= 1`),
//!   3. the native allocation registry drained to zero at the boundary
//!      (`native_alloc_registry_len() == 0`) — the key correctness guard.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("test thread panicked")
}

/// Run `src` (no args) on the JIT-tiered VM and the tree-walker; assert
/// bit-identical and a drained registry; return `(output, error, regalloc_fn)`.
fn tiered_listreturn(src: &str) -> (String, Option<String>, u32) {
    let src = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "list-return regalloc VM diverged from tree-walker on:\n{src}"
        );
        assert_eq!(
            logicaffeine_jit::native_alloc_registry_len(),
            0,
            "the allocation registry must drain at every boundary"
        );
        (norm(&vm.output), vm.error, tier.regalloc_function_count())
    })
}

/// The mergeSort kernel string at a chosen warm-up size `n`. Fresh
/// `left`/`right`/`result` locals, read-only `arr` param, self-recursion
/// passing fresh lists, list return — the exact benchmark shape.
fn mergesort_program(n: usize) -> String {
    format!(
        "## To mergeSort (arr: Seq of Int) -> Seq of Int:\n\
         \x20   Let m be length of arr.\n\
         \x20   If m is at most 1:\n\
         \x20       Return arr.\n\
         \x20   Let mid be m / 2.\n\
         \x20   Let mutable left be a new Seq of Int.\n\
         \x20   Let mutable right be a new Seq of Int.\n\
         \x20   Let mutable i be 1.\n\
         \x20   While i is at most mid:\n\
         \x20       Push item i of arr to left.\n\
         \x20       Set i to i + 1.\n\
         \x20   While i is at most m:\n\
         \x20       Push item i of arr to right.\n\
         \x20       Set i to i + 1.\n\
         \x20   Set left to mergeSort(left).\n\
         \x20   Set right to mergeSort(right).\n\
         \x20   Let mutable result be a new Seq of Int.\n\
         \x20   Let mutable li be 1.\n\
         \x20   Let mutable ri be 1.\n\
         \x20   While li is at most length of left:\n\
         \x20       If ri is greater than length of right:\n\
         \x20           Push item li of left to result.\n\
         \x20           Set li to li + 1.\n\
         \x20       Otherwise:\n\
         \x20           If item li of left is at most item ri of right:\n\
         \x20               Push item li of left to result.\n\
         \x20               Set li to li + 1.\n\
         \x20           Otherwise:\n\
         \x20               Push item ri of right to result.\n\
         \x20               Set ri to ri + 1.\n\
         \x20   While ri is at most length of right:\n\
         \x20       Push item ri of right to result.\n\
         \x20       Set ri to ri + 1.\n\
         \x20   Return result.\n\
         \n\
         ## Main\n\
         Let mutable arr be a new Seq of Int.\n\
         Let mutable seed be 42.\n\
         Let mutable i be 0.\n\
         While i is less than {n}:\n\
         \x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
         \x20   Push (seed / 65536) % 32768 to arr.\n\
         \x20   Set i to i + 1.\n\
         Set arr to mergeSort(arr).\n\
         Let mutable checksum be 0.\n\
         Set i to 1.\n\
         While i is at most {n}:\n\
         \x20   Set checksum to (checksum + item i of arr) % 1000000007.\n\
         \x20   Set i to i + 1.\n\
         Show \"\" + item 1 of arr + \" \" + item {n} of arr + \" \" + checksum.\n"
    )
}

/// RED: the simplest fresh-list-RETURN recursion — a function that allocates a
/// fresh list, fills it, and returns it, called from a recursive splitter —
/// must tier through the CONTIGUOUS regalloc FUNCTION backend, bit-identical and
/// with a drained registry.
#[test]
#[cfg(target_arch = "x86_64")]
fn fresh_list_return_recursion_tiers_via_regalloc() {
    // `dup` builds a fresh list [v, v, ..n times] and recurses on the tail; the
    // returned list is a fresh registry-owned allocation at every level.
    let src = "## To dup (v: Int, n: Int) -> Seq of Int:\n\
               \x20   Let mutable out be a new Seq of Int.\n\
               \x20   If n is at most 0:\n\
               \x20       Return out.\n\
               \x20   Push v to out.\n\
               \x20   Let rest be dup(v, n - 1).\n\
               \x20   Let mutable j be 1.\n\
               \x20   While j is at most length of rest:\n\
               \x20       Push item j of rest to out.\n\
               \x20       Set j to j + 1.\n\
               \x20   Return out.\n\
               \n\
               ## Main\n\
               Let mutable total be 0.\n\
               Let mutable k be 0.\n\
               While k is less than 400:\n\
               \x20   Let xs be dup(7, 20).\n\
               \x20   Set total to total + length of xs + item 20 of xs.\n\
               \x20   Set k to k + 1.\n\
               Show total.\n";
    let (out, err, ra_fn) = tiered_listreturn(src);
    assert_eq!(err, None);
    // each call returns 20 sevens: length 20 + item 20 (=7) = 27; ×400 = 10800.
    assert_eq!(out, "10800");
    assert!(
        ra_fn >= 1,
        "fresh-list-return recursion must tier through the CONTIGUOUS regalloc \
         FUNCTION backend (got {ra_fn})"
    );
}

/// RED: the REAL mergesort kernel (the benchmark shape) tiers through the
/// regalloc FUNCTION backend, bit-identical to the tree-walker, registry drained.
#[test]
#[cfg(target_arch = "x86_64")]
fn mergesort_tiers_via_regalloc_function() {
    let src = mergesort_program(400);
    let (_, err, ra_fn) = tiered_listreturn(&src);
    assert_eq!(err, None);
    assert!(
        ra_fn >= 1,
        "mergeSort must tier through the CONTIGUOUS regalloc FUNCTION backend \
         (got {ra_fn})"
    );
}

/// A LARGE mergesort run — deep recursion, thousands of reallocating pushes and
/// fresh-list allocations per level — must stay bit-identical to the tree-walker
/// through the regalloc backend, with the registry drained. The stress soundness
/// gate for the realloc-coherence + registry-detach machinery at scale.
#[test]
#[cfg(target_arch = "x86_64")]
fn mergesort_large_is_bit_identical_via_regalloc() {
    let src = mergesort_program(20_000);
    let (_, err, ra_fn) = tiered_listreturn(&src);
    assert_eq!(err, None);
    assert!(
        ra_fn >= 1,
        "mergeSort (large) must tier through the regalloc FUNCTION backend (got {ra_fn})"
    );
}

/// RED: a DEOPT deep inside fresh-list-return recursion — after allocations and
/// reallocating pushes, a division by zero at the recursion base. The native
/// call stack must unwind, the registry must drain, and the bytecode replay must
/// raise the EXACT error with the EXACT partial output as the tree-walker —
/// proving the registry-owned fresh-list materialization on a precise side exit
/// through the regalloc body.
#[test]
#[cfg(target_arch = "x86_64")]
fn list_return_recursion_deopt_drains_and_replays() {
    // `build(n, d)` allocates a fresh list, pushes into it, recurses, and at the
    // base divides by `d`. Warmed on `d=5` so it tiers; then `d=0` faults at the
    // base of a real recursion — the fresh lists at every level must drain.
    let src = "## To build (n: Int, d: Int) -> Seq of Int:\n\
               \x20   Let mutable out be a new Seq of Int.\n\
               \x20   Let mutable i be 0.\n\
               \x20   While i is less than 8:\n\
               \x20       Push i to out.\n\
               \x20       Set i to i + 1.\n\
               \x20   If n is at most 0:\n\
               \x20       Push 100 / d to out.\n\
               \x20       Return out.\n\
               \x20   Let rest be build(n - 1, d).\n\
               \x20   Push length of rest to out.\n\
               \x20   Return out.\n\
               \n\
               ## Main\n\
               Let mutable acc be 0.\n\
               Let mutable k be 0.\n\
               While k is less than 600:\n\
               \x20   Let xs be build(30, 5).\n\
               \x20   Set acc to acc + length of xs.\n\
               \x20   Set k to k + 1.\n\
               Show acc.\n\
               Let bad be build(40, 0).\n\
               Show length of bad.\n";
    let src_owned = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src_owned, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src_owned, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "list-return recursive deopt replay diverged"
        );
        assert!(vm.error.is_some(), "build(_, 0) divides by zero at the base");
        assert_eq!(
            logicaffeine_jit::native_alloc_registry_len(),
            0,
            "deopt must drain the allocation registry"
        );
        assert!(
            tier.regalloc_function_count() >= 1,
            "build must tier through the regalloc FUNCTION backend (got {})",
            tier.regalloc_function_count()
        );
    });
}

/// RED: a DEPTH-LIMIT crossing inside fresh-list-return recursion. The function
/// allocates+returns a fresh list and recurses non-tail deep enough to exceed
/// MAX_CALL_DEPTH; the side exit (status=5) must unwind, the registry drain, and
/// the bytecode replay raise the IDENTICAL kernel error at the same depth as the
/// tree-walker.
#[test]
#[cfg(target_arch = "x86_64")]
fn list_return_recursion_depth_limit_parity() {
    let src = "## To chain (n: Int) -> Seq of Int:\n\
               \x20   Let mutable out be a new Seq of Int.\n\
               \x20   Push n to out.\n\
               \x20   If n is at most 0:\n\
               \x20       Return out.\n\
               \x20   Let rest be chain(n - 1).\n\
               \x20   Push length of rest to out.\n\
               \x20   Return out.\n\
               \n\
               ## Main\n\
               Show 3.\n\
               Let deep be chain(5000).\n\
               Show length of deep.\n";
    let src_owned = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src_owned, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src_owned, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "list-return depth-limit replay diverged"
        );
        assert!(vm.error.is_some(), "depth 5000 must exceed the cap");
        assert_eq!(norm(&vm.output), "3");
        assert_eq!(
            logicaffeine_jit::native_alloc_registry_len(),
            0,
            "depth-limit side exit must drain the allocation registry"
        );
    });
}
