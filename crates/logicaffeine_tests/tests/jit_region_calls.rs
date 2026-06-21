//! M10d RED gate: loop REGIONS may call already-tiered FUNCTIONS through
//! the FnTable — the spectral_norm shape (a hot nested loop in Main
//! invoking a tiny scalar helper per element) goes fully native.
//!
//! Soundness boundary: region deopt is DISCARD-AND-REPLAY (re-run the
//! remaining iterations on bytecode from the last back-edge state), which
//! is only sound when everything the region did is replay-idempotent.
//! A scalar-pure callee (mode A: declared scalar params and return, no
//! list pins) recomputes identically; a list-MUTATING callee (mode B)
//! would double-apply its writes — so regions must refuse to inline calls
//! to mode-B functions, and the program still runs exactly (bytecode
//! call boundary) when they appear.

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

/// (output, error, fn_ok, region_ok) with full tree-walker parity.
fn tiered(src: &str) -> (String, Option<String>, u32, u32) {
    let src = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "tiered VM diverged from tree-walker on:\n{src}"
        );
        let (_, fn_ok) = tier.function_counts();
        let (_, region_ok) = tier.region_counts();
        (norm(&vm.output), vm.error, fn_ok, region_ok)
    })
}

/// The spectral_norm kernel: a float helper called per inner-loop element.
/// Both the helper AND the loop region must tier, with bit-exact output.
#[test]
fn region_calling_float_helper_tiers() {
    let src = "## To aVal (i: Int, j: Int) -> Float:\n\
               \x20   Return 1.0 / ((i + j) * (i + j + 1) / 2 + i + 1).\n\
               \n\
               ## Main\n\
               Let mutable acc be 0.0.\n\
               Let mutable i be 0.\n\
               While i is less than 300:\n\
               \x20   Let mutable j be 0.\n\
               \x20   While j is less than 300:\n\
               \x20       Set acc to acc + aVal(i, j).\n\
               \x20       Set j to j + 1.\n\
               \x20   Set i to i + 1.\n\
               Show \"{acc:.9}\".\n";
    let (_, err, fn_ok, region_ok) = tiered(src);
    assert_eq!(err, None);
    assert!(fn_ok >= 1, "aVal must JIT (got {fn_ok})");
    assert!(
        region_ok >= 1,
        "the loop region containing the call must tier (got {region_ok})"
    );
}

/// An Int helper through a single hot loop — the simplest region-call.
#[test]
fn region_calling_int_helper_tiers() {
    let src = "## To mix (a: Int, b: Int) -> Int:\n\
               \x20   Return (a * 31 + b) % 1000003.\n\
               \n\
               ## Main\n\
               Let mutable h be 7.\n\
               Let mutable i be 0.\n\
               While i is less than 200000:\n\
               \x20   Set h to mix(h, i).\n\
               \x20   Set i to i + 1.\n\
               Show h.\n";
    let (_, err, fn_ok, region_ok) = tiered(src);
    assert_eq!(err, None);
    assert!(fn_ok >= 1, "mix must JIT (got {fn_ok})");
    assert!(region_ok >= 1, "the mixing loop must tier with its call (got {region_ok})");
}

/// A SELF-RECURSIVE callee inside a region call: the region's call enters
/// the callee, which recurses natively through its own table entry.
#[test]
fn region_calling_recursive_helper_stays_exact() {
    // `n + tri(n-1)` is accumulator-shaped (single linear `+`) and is now
    // strength-reduced to a constant-stack loop — so `tri` would no longer
    // recurse natively (nothing to JIT). Binding the call (`Let r be tri(...)`)
    // then returning `r + n` keeps it genuine recursion the native tier JITs,
    // and preserves the triangular value.
    let src = "## To tri (n: Int) -> Int:\n\
               \x20   If n is at most 0:\n\
               \x20       Return 0.\n\
               \x20   Let r be tri(n - 1).\n\
               \x20   Return r + n.\n\
               \n\
               ## Main\n\
               Let mutable total be 0.\n\
               Let mutable i be 0.\n\
               While i is less than 5000:\n\
               \x20   Set total to total + tri(i % 50).\n\
               \x20   Set i to i + 1.\n\
               Show total.\n";
    let (_, err, fn_ok, _) = tiered(src);
    assert_eq!(err, None);
    assert!(fn_ok >= 1, "tri must JIT (got {fn_ok})");
}

/// A divide-by-zero INSIDE the callee at a data-dependent iteration: the
/// region replays on bytecode and the kernel error + partial output are
/// exact.
#[test]
fn callee_error_mid_region_is_exact() {
    let src = "## To risky (n: Int) -> Int:\n\
               \x20   Return 1000 / (n - 77777).\n\
               \n\
               ## Main\n\
               Show 11.\n\
               Let mutable s be 0.\n\
               Let mutable i be 0.\n\
               While i is less than 200000:\n\
               \x20   Set s to s + risky(i).\n\
               \x20   Set i to i + 1.\n\
               Show s.\n";
    let src_owned = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src_owned, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src_owned, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "callee error through a region call diverged"
        );
        assert!(vm.error.is_some(), "i == 77777 must divide by zero");
        assert_eq!(norm(&vm.output), "11");
    });
}

/// SOUNDNESS BOUNDARY: a region whose loop calls a LIST-MUTATING (mode B)
/// function must not inline that call into the region — replay would
/// double-apply the callee's writes. Exactness is the assertion; the
/// region may simply not tier.
#[test]
fn region_never_inlines_list_mutating_callee() {
    let src = "## To bump (xs: Seq of Int, k: Int) -> Int:\n\
               \x20   Set item 1 of xs to item 1 of xs + k.\n\
               \x20   Return item 1 of xs.\n\
               \n\
               ## Main\n\
               Let mutable xs be [0].\n\
               Let mutable s be 0.\n\
               Let mutable i be 0.\n\
               While i is less than 150000:\n\
               \x20   Set s to s + bump(xs, 1).\n\
               \x20   Set i to i + 1.\n\
               Show s.\n\
               Show item 1 of xs.\n";
    let (out, err, _, _) = tiered(src);
    assert_eq!(err, None);
    // Σ k for k in 1..150000, and the final accumulator value.
    assert_eq!(out, "11250075000\n150000");
}

/// Calls mixed with array traffic in the same region body (the mulAv
/// shape: indexed reads, a float call, an indexed write per iteration).
#[test]
fn region_with_call_and_array_traffic_tiers() {
    let src = "## To weight (i: Int) -> Float:\n\
               \x20   Return 1.0 / (i * i + 1).\n\
               \n\
               ## Main\n\
               Let mutable vs be a new Seq of Float.\n\
               Let mutable i be 0.\n\
               While i is less than 2000:\n\
               \x20   Push i * 0.5 to vs.\n\
               \x20   Set i to i + 1.\n\
               Let mutable acc be 0.0.\n\
               Set i to 1.\n\
               While i is at most 2000:\n\
               \x20   Set acc to acc + weight(i) * item i of vs.\n\
               \x20   Set i to i + 1.\n\
               Show \"{acc:.9}\".\n";
    let (_, err, fn_ok, region_ok) = tiered(src);
    assert_eq!(err, None);
    assert!(fn_ok >= 1, "weight must JIT (got {fn_ok})");
    assert!(
        region_ok >= 1,
        "the call+array loop must tier as a region (got {region_ok})"
    );
}

/// LEVER B: a region that CALLS a LIST-PARAMETER function — the heap_sort /
/// worklist shape `arr = step(arr)` where `step` mutates the list IN PLACE and
/// returns its own argument. The region must tier (it passes the pinned buffer
/// to the callee, which mutates it through the same handle) AND stay bit-exact
/// with the tree-walker. Before Lever B such a region refused to tier.
#[test]
fn region_calling_list_mutating_helper_tiers_and_is_exact() {
    let src = "## To bump (xs: Seq of Int) -> Seq of Int:\n\
               \x20   Let mutable r be xs.\n\
               \x20   Set item 1 of r to item 1 of r + 1.\n\
               \x20   Return r.\n\
               ## To run () -> Int:\n\
               \x20   Let mutable arr be a new Seq of Int.\n\
               \x20   Let mutable k be 0.\n\
               \x20   While k is less than 8:\n\
               \x20       Push 0 to arr.\n\
               \x20       Set k to k + 1.\n\
               \x20   Let mutable i be 0.\n\
               \x20   While i is less than 5000:\n\
               \x20       Set arr to bump(arr).\n\
               \x20       Set i to i + 1.\n\
               \x20   Return item 1 of arr.\n\
               ## Main\n\
               Show run().\n";
    let (out, err, _fn_ok, region_ok) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "5000");
    assert!(
        region_ok >= 1,
        "the loop region calling the list-param helper must tier (got {region_ok})"
    );
}

/// LEVER B soundness: a list-param call beside a RECOVERABLE deopt (a
/// `Map of Int to Float` read that misses the Int fast lane). The callee mutates
/// the list IN PLACE; on the deopt the region replays from the head, so the
/// callee's mutation MUST roll back (the pinned buffer is snapshotted on entry) —
/// otherwise `arr[1]` double-counts. Disabling the snapshot makes this crash/
/// diverge; with it, bit-exact with the tree-walker.
#[test]
fn region_list_call_with_recoverable_deopt_rolls_back() {
    let src = "## To bump (xs: Seq of Int) -> Seq of Int:\n\
               \x20   Let mutable r be xs.\n\
               \x20   Set item 1 of r to item 1 of r + 1.\n\
               \x20   Return r.\n\
               ## To run () -> Int:\n\
               \x20   Let mutable m be a new Map of Int to Float.\n\
               \x20   Let mutable k be 0.\n\
               \x20   While k is less than 30:\n\
               \x20       Set item k of m to k * 1.5.\n\
               \x20       Set k to k + 1.\n\
               \x20   Let mutable arr be a new Seq of Int.\n\
               \x20   Set k to 0.\n\
               \x20   While k is less than 30:\n\
               \x20       Push 0 to arr.\n\
               \x20       Set k to k + 1.\n\
               \x20   Let mutable s be 0.0.\n\
               \x20   Let mutable i be 0.\n\
               \x20   While i is less than 5000:\n\
               \x20       Set arr to bump(arr).\n\
               \x20       Set s to s + item 1 of m.\n\
               \x20       Set i to i + 1.\n\
               \x20   Return item 1 of arr.\n\
               ## Main\n\
               Show run().\n";
    let (out, err, _fn_ok, region_ok) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "5000");
    assert!(
        region_ok >= 1,
        "the list-param-call region must tier (got {region_ok})"
    );
}

/// The fannkuch idiom: an outer loop body that allocates a FRESH list each
/// iteration (`Let mutable scratch be a new Seq` → `Op::NewEmptyList`), fills it
/// via `Push`, then mutates it IN PLACE via `Set item ... of scratch` swaps, and
/// consumes the result. Before in-region `NewEmptyList` lowering (the `ListClear`
/// stencil) the outer region bailed to bytecode at `translate_op` because it had
/// no lowering for the in-region allocation — running fannkuch's whole outer loop
/// interpreted. With the lowering, the allocation clears+reuses the pinned buffer
/// and the outer loop tiers. Asserts the tree-walker differential (the soundness
/// gate — a wrong clear silently corrupts the permutation) AND that it tiers.
#[test]
fn region_with_fresh_inner_list_alloc_tiers() {
    let src = "## Main\n\
               Let mutable src be a new Seq of Int.\n\
               Let mutable j be 0.\n\
               While j is less than 6:\n\
               \x20   Push j * 3 to src.\n\
               \x20   Set j to j + 1.\n\
               Let mutable total be 0.\n\
               Let mutable iter be 0.\n\
               While iter is less than 4000:\n\
               \x20   Let mutable scratch be a new Seq of Int.\n\
               \x20   Set j to 1.\n\
               \x20   While j is at most 6:\n\
               \x20       Push item j of src to scratch.\n\
               \x20       Set j to j + 1.\n\
               \x20   Let mutable lo be 1.\n\
               \x20   Let mutable hi be 6.\n\
               \x20   While lo is less than hi:\n\
               \x20       Let tmp be item lo of scratch.\n\
               \x20       Set item lo of scratch to item hi of scratch.\n\
               \x20       Set item hi of scratch to tmp.\n\
               \x20       Set lo to lo + 1.\n\
               \x20       Set hi to hi - 1.\n\
               \x20   Set total to total + item 1 of scratch.\n\
               \x20   Set iter to iter + 1.\n\
               Show total.\n";
    let (out, err, _fn_ok, region_ok) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "60000");
    assert!(
        region_ok >= 1,
        "the fresh-inner-list-alloc region must tier (got {region_ok})"
    );
}
