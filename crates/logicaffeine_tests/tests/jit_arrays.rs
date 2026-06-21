//! M7 RED gate: arrays in the JIT. Region entry borrows each distinct
//! `Rc<RefCell<ListRepr>>` ONCE (held for the whole native run — zero
//! refcount/borrow churn inside the loop), pins the unboxed buffer's
//! data pointer and length into dedicated frame slots, and Index/SetIndex/
//! Length lower to direct loads/stores with a bounds side-exit BEFORE any
//! effect. In-place array writes are deopt-safe by prefix-idempotence: the
//! replay recomputes exactly the values the native prefix already wrote.

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

fn tiered(src: &str) -> (String, Option<String>, u32) {
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "tiered VM diverged from tree-walker on:\n{src}"
    );
    let (_, region_ok) = tier.region_counts();
    (norm(&vm.output), vm.error, region_ok)
}

/// The sieve inner shape: Int-array reads and writes in a hot loop MUST tier
/// as a region and produce the exact count.
#[test]
fn sieve_marking_loop_tiers_with_array_stores() {
    let src = "## Main\n\
               Let n be 3000.\n\
               Let mutable flags be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is at most n:\n\
               \x20   Push 1 to flags.\n\
               \x20   Set i to i + 1.\n\
               Let mutable p be 2.\n\
               While p * p is at most n:\n\
               \x20   If item (p + 1) of flags equals 1:\n\
               \x20       Let mutable m be p * p.\n\
               \x20       While m is at most n:\n\
               \x20           Set item (m + 1) of flags to 0.\n\
               \x20           Set m to m + p.\n\
               \x20   Set p to p + 1.\n\
               Let mutable count be 0.\n\
               Set p to 2.\n\
               While p is at most n:\n\
               \x20   If item (p + 1) of flags equals 1:\n\
               \x20       Set count to count + 1.\n\
               \x20   Set p to p + 1.\n\
               Show count.\n";
    let (out, err, region_ok) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "430", "π(3000)");
    assert!(
        region_ok >= 1,
        "an array loop must tier up as a region (got {region_ok})"
    );
}

/// Float arrays (the nbody/spectral_norm substrate): read-modify-write float
/// buffers in a hot loop, bit-exact against the tree-walker.
#[test]
fn float_array_accumulation_tiers() {
    let src = "## Main\n\
               Let n be 1000.\n\
               Let mutable v be a new Seq of Float.\n\
               Let mutable i be 0.\n\
               While i is less than n:\n\
               \x20   Push 0.5 to v.\n\
               \x20   Set i to i + 1.\n\
               Let mutable pass be 0.\n\
               While pass is less than 30:\n\
               \x20   Let mutable k be 1.\n\
               \x20   While k is at most n:\n\
               \x20       Set item k of v to item k of v * 1.001 + 0.25.\n\
               \x20       Set k to k + 1.\n\
               \x20   Set pass to pass + 1.\n\
               Let mutable sum be 0.0.\n\
               Let mutable j be 1.\n\
               While j is at most n:\n\
               \x20   Set sum to sum + item j of v.\n\
               \x20   Set j to j + 1.\n\
               Show \"{sum:.6}\".\n";
    let (_, err, region_ok) = tiered(src);
    assert_eq!(err, None);
    assert!(
        region_ok >= 1,
        "the float-array loop must tier up (got {region_ok})"
    );
}

/// Aliasing: two names for the same list — native writes through one are
/// visible through the other (same buffer; the entry borrow dedups by Rc
/// identity so the RefCell is borrowed exactly once).
#[test]
fn aliased_arrays_share_native_writes() {
    let src = "## Main\n\
               Let mutable a be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is less than 500:\n\
               \x20   Push 0 to a.\n\
               \x20   Set i to i + 1.\n\
               Let b be a.\n\
               Set i to 1.\n\
               While i is at most 500:\n\
               \x20   Set item i of a to item i of b + i.\n\
               \x20   Set i to i + 1.\n\
               Show item 500 of b.\n\
               Show item 1 of a.\n";
    let (out, err, _) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "500\n1");
}

/// Out-of-bounds at a data-dependent iteration: the bounds side-exit fires
/// BEFORE the faulting access; replay produces the exact kernel error and
/// partial output, with every in-place write up to that point intact.
#[test]
fn array_oob_mid_loop_error_parity() {
    let src = "## Main\n\
               Show 5.\n\
               Let mutable xs be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is less than 300:\n\
               \x20   Push i to xs.\n\
               \x20   Set i to i + 1.\n\
               Let mutable acc be 0.\n\
               Set i to 1.\n\
               While i is at most 5000:\n\
               \x20   Set acc to acc + item i of xs.\n\
               \x20   Set i to i + 1.\n\
               Show acc.\n";
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "array OOB deopt replay diverged"
    );
    assert!(vm.error.is_some(), "index past the end must error");
    assert_eq!(norm(&vm.output), "5");
}

/// `length of` inside the hot loop reads the pinned length slot.
#[test]
fn length_in_hot_loop() {
    let src = "## Main\n\
               Let mutable xs be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is less than 400:\n\
               \x20   Push i to xs.\n\
               \x20   Set i to i + 1.\n\
               Let mutable acc be 0.\n\
               Set i to 1.\n\
               While i is at most length of xs:\n\
               \x20   Set acc to acc + item i of xs.\n\
               \x20   Set i to i + 1.\n\
               Show acc.\n";
    let (out, err, region_ok) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "79800");
    assert!(region_ok >= 1, "length-bounded array loop must tier (got {region_ok})");
}

/// A Boxed (heterogeneous) list must NOT take the native array path — the
/// guard rejects the repr and bytecode handles it, identically.
#[test]
fn boxed_lists_stay_on_bytecode_correctly() {
    let src = "## Main\n\
               Let mutable xs be [1, \"two\", 3].\n\
               Let mutable acc be 0.\n\
               Let mutable r be 0.\n\
               While r is less than 300:\n\
               \x20   Set acc to acc + item 1 of xs + item 3 of xs.\n\
               \x20   Set r to r + 1.\n\
               Show acc.\n";
    let (out, err, _) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "1200");
}
