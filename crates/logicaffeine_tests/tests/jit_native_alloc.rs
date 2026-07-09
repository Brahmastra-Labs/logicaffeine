//! M-ALLOC RED gate: native code ALLOCATES fresh lists (the mergesort
//! shape — `a new Seq of Int` locals, pushes, recursion, returning the
//! built list).
//!
//! Ownership protocol: every native allocation lives in a thread-local
//! REGISTRY until the boundary. On success, the returned list detaches
//! and re-boxes as a real kernel value; everything else (temps) drops.
//! On ANY deopt the whole registry drops and the replay re-runs
//! interpreted — fresh lists never escaped, so nothing leaks and nothing
//! is double-built.
//!
//! Pushing into PARAMETER lists stays excluded (a callee push would leave
//! the caller's pinned ptr/len stale); site-allocated lists have no other
//! holder, so their pushes are free.

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

fn tiered(src: &str) -> (String, Option<String>, u32) {
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
        assert_eq!(
            logicaffeine_jit::native_alloc_registry_len(),
            0,
            "the allocation registry must drain at every boundary"
        );
        let (_, fn_ok) = tier.function_counts();
        (norm(&vm.output), vm.error, fn_ok)
    })
}

/// A function that BUILDS a fresh list and returns it: the simplest
/// native-allocation round trip.
#[test]
fn fresh_list_builder_tiers() {
    let src = "## To build (n: Int) -> Seq of Int:\n\
               \x20   Let mutable out be a new Seq of Int.\n\
               \x20   Let mutable i be 1.\n\
               \x20   While i is at most n:\n\
               \x20       Push i * i to out.\n\
               \x20       Set i to i + 1.\n\
               \x20   Return out.\n\
               \n\
               ## Main\n\
               Let mutable total be 0.\n\
               Let mutable k be 0.\n\
               While k is less than 300:\n\
               \x20   Let xs be build(20).\n\
               \x20   Set total to total + item 20 of xs + length of xs.\n\
               \x20   Set k to k + 1.\n\
               Show total.\n";
    let (out, err, fn_ok) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "126000", "300 × (400 + 20)");
    assert!(fn_ok >= 1, "the builder must JIT with native allocation (got {fn_ok})");
}

/// The REAL mergesort kernel: fresh left/right/result locals, read-only
/// list params, self-recursion passing fresh lists, list return.
#[test]
fn mergesort_with_native_allocation_tiers() {
    let src = "## To mergeSort (arr: Seq of Int) -> Seq of Int:\n\
               \x20   If length of arr is at most 1:\n\
               \x20       Return arr.\n\
               \x20   Let mid be length of arr / 2.\n\
               \x20   Let mutable left be a new Seq of Int.\n\
               \x20   Let mutable right be a new Seq of Int.\n\
               \x20   Let mutable i be 1.\n\
               \x20   While i is at most mid:\n\
               \x20       Push item i of arr to left.\n\
               \x20       Set i to i + 1.\n\
               \x20   While i is at most length of arr:\n\
               \x20       Push item i of arr to right.\n\
               \x20       Set i to i + 1.\n\
               \x20   Let sortedLeft be mergeSort(left).\n\
               \x20   Let sortedRight be mergeSort(right).\n\
               \x20   Let mutable result be a new Seq of Int.\n\
               \x20   Let mutable li be 1.\n\
               \x20   Let mutable ri be 1.\n\
               \x20   While li is at most length of sortedLeft and ri is at most length of sortedRight:\n\
               \x20       If item li of sortedLeft is at most item ri of sortedRight:\n\
               \x20           Push item li of sortedLeft to result.\n\
               \x20           Set li to li + 1.\n\
               \x20       Otherwise:\n\
               \x20           Push item ri of sortedRight to result.\n\
               \x20           Set ri to ri + 1.\n\
               \x20   While li is at most length of sortedLeft:\n\
               \x20       Push item li of sortedLeft to result.\n\
               \x20       Set li to li + 1.\n\
               \x20   While ri is at most length of sortedRight:\n\
               \x20       Push item ri of sortedRight to result.\n\
               \x20       Set ri to ri + 1.\n\
               \x20   Return result.\n\
               \n\
               ## Main\n\
               Let mutable arr be a new Seq of Int.\n\
               Let mutable seed be 42.\n\
               Let mutable i be 0.\n\
               While i is less than 400:\n\
               \x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
               \x20   Push (seed / 65536) % 32768 to arr.\n\
               \x20   Set i to i + 1.\n\
               Let sorted be mergeSort(arr).\n\
               Let mutable checksum be 0.\n\
               Set i to 1.\n\
               While i is at most 400:\n\
               \x20   Set checksum to (checksum + item i of sorted * i) % 1000000007.\n\
               \x20   Set i to i + 1.\n\
               Show checksum.\n";
    let (_, err, fn_ok) = tiered(src);
    assert_eq!(err, None);
    assert!(fn_ok >= 1, "mergeSort must JIT with native allocation (got {fn_ok})");
}

/// A deopt AFTER allocations and pushes (division by zero at a
/// data-dependent point): the registry drains, the replay is exact.
#[test]
fn deopt_after_allocation_drains_and_replays() {
    let src = "## To risky (n: Int) -> Int:\n\
               \x20   Let mutable tmp be a new Seq of Int.\n\
               \x20   Let mutable i be 0.\n\
               \x20   While i is less than 10:\n\
               \x20       Push i to tmp.\n\
               \x20       Set i to i + 1.\n\
               \x20   Return item 5 of tmp + 100 / n.\n\
               \n\
               ## Main\n\
               Show 9.\n\
               Let mutable s be 0.\n\
               Let mutable k be 200.\n\
               While k is at least 0 - 1:\n\
               \x20   Set s to s + risky(k).\n\
               \x20   Set k to k - 1.\n\
               Show s.\n";
    let src_owned = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src_owned, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src_owned, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "post-allocation deopt diverged"
        );
        assert!(vm.error.is_some(), "k reaches 0: division by zero");
        assert_eq!(
            logicaffeine_jit::native_alloc_registry_len(),
            0,
            "deopt must drain the allocation registry"
        );
    });
}

/// Pushing into a `mutable` PARAMETER is by-reference: the callee grows the
/// caller's list, so after 200 calls its length is exactly 200. Exactness under
/// the tier is the contract (it may decline rather than miscompile the shared
/// mutation).
#[test]
fn param_push_stays_exact() {
    let src = "## To grow (xs: mutable Seq of Int, n: Int) -> Int:\n\
               \x20   Push n to xs.\n\
               \x20   Return length of xs.\n\
               \n\
               ## Main\n\
               Let mutable xs be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is less than 200:\n\
               \x20   Let l be grow(xs, i).\n\
               \x20   Set i to i + 1.\n\
               Show length of xs.\n";
    let (out, err, _) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "200");
}
