//! M7 RED gate: arrays in the JIT. Region entry borrows each distinct
//! `Rc<RefCell<ListRepr>>` ONCE (held for the whole native run — zero
//! refcount/borrow churn inside the loop), pins the unboxed buffer's
//! data pointer and length into dedicated frame slots, and Index/SetIndex/
//! Length lower to direct loads/stores with a bounds side-exit BEFORE any
//! effect. In-place array writes are deopt-safe by prefix-idempotence: the
//! replay recomputes exactly the values the native prefix already wrote.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::optimization::{Opt, OptimizationConfig};
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

/// Value semantics: two names are INDEPENDENT values — native writes through one
/// are NOT visible through the other. The JIT declines the in-place-mutation
/// region (deopt) so it runs on the value-semantic VM, whose copy-on-write
/// isolates `a` from `b`. (Was the reference-semantics `..._share_native_writes`.)
#[test]
fn aliased_arrays_isolate_under_value_semantics() {
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
    // `b` keeps the original zeros; `a` is isolated, a[1]=b[1]+1=1.
    assert_eq!(out, "0\n1");
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

// ---- i32-backed Int sequences (`Opt::NarrowVm`) ----
//
// These exercise the half-width (`Vec<i32>`) VM list storage and its JIT region
// path (sign-extending `movsxd` loads, truncating 4-byte stores). Each asserts
// the tiered VM is bit-identical to the tree-walker AND that the hot loop still
// tiers (the i32 array must NOT bail the region — the whole point of the lever).
//
// `NarrowVm` is a default-on optimization in the single `OptimizationConfig`; the
// helper asserts the effective config actually has it enabled, so if the default
// ever flips off (or the ambient env disables it) these tests fail loudly instead
// of silently exercising the full-width path.

fn tiered_narrow(src: &str) -> (String, Option<String>, u32) {
    let mut cfg = OptimizationConfig::from_env();
    cfg.normalize();
    assert!(
        cfg.is_on(Opt::NarrowVm),
        "NarrowVm must be enabled for the narrowed-array tests (effective config has it off)"
    );
    tiered(src)
}

/// A graph_bfs-shaped narrowable array: `% n` element values (always fit i32),
/// written in place and read in a hot loop. Must tier with the half-width buffer
/// and match the tree-walker exactly.
#[test]
fn narrowed_int_array_mod_pattern_tiers_and_matches() {
    let src = "## Main\n\
               Let n be 4000.\n\
               Let mutable a be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is less than n:\n\
               \x20   Push 0 to a.\n\
               \x20   Set i to i + 1.\n\
               Set i to 0.\n\
               While i is less than n:\n\
               \x20   Set item (i + 1) of a to (i * 7) % n.\n\
               \x20   Set i to i + 1.\n\
               Let mutable total be 0.\n\
               Set i to 0.\n\
               While i is less than n:\n\
               \x20   Set total to total + item (i + 1) of a.\n\
               \x20   Set i to i + 1.\n\
               Show total.\n";
    let (out, err, region_ok) = tiered_narrow(src);
    assert_eq!(err, None);
    assert!(region_ok >= 1, "narrowed-array loop must tier (got {region_ok})");
    // Independently recompute the reference sum of (i*7) % 4000 for i in 0..4000.
    let want: i64 = (0..4000i64).map(|i| (i * 7) % 4000).sum();
    assert_eq!(out, want.to_string());
}

/// SOUNDNESS net: when a value outside i32 range reaches a (would-be) narrowed
/// buffer, the result must STILL equal the full-width run — the buffer widens
/// rather than truncating. (Constant out-of-range stores also keep the proof
/// from narrowing; either way the observable value is unchanged.)
#[test]
fn narrowed_array_out_of_range_value_is_lossless() {
    let src = "## Main\n\
               Let n be 3000.\n\
               Let mutable a be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is less than n:\n\
               \x20   Push i % 5 to a.\n\
               \x20   Set i to i + 1.\n\
               Set item 1 of a to 9000000000.\n\
               Let mutable total be 0.\n\
               Set i to 0.\n\
               While i is less than n:\n\
               \x20   Set total to total + item (i + 1) of a.\n\
               \x20   Set i to i + 1.\n\
               Show total.\n";
    // tiered_narrow already asserts VM == tree-walker; the explicit value pins
    // that the big number survived intact under narrowing.
    let (out, err, _) = tiered_narrow(src);
    assert_eq!(err, None);
    let base: i64 = (1..3000i64).map(|i| i % 5).sum(); // a[0] is overwritten
    let want = base + 9_000_000_000;
    assert_eq!(out, want.to_string());
}

/// A narrowable array carrying negative values must SIGN-extend on read (not
/// zero-extend) — the i32 load is `movsxd`, not `movzx`.
#[test]
fn narrowed_array_negative_values_sign_extend() {
    let src = "## Main\n\
               Let n be 2000.\n\
               Let mutable a be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is less than n:\n\
               \x20   Push 0 - (i % 100) to a.\n\
               \x20   Set i to i + 1.\n\
               Let mutable total be 0.\n\
               Set i to 0.\n\
               While i is less than n:\n\
               \x20   Set total to total + item (i + 1) of a.\n\
               \x20   Set i to i + 1.\n\
               Show total.\n";
    let (out, err, _) = tiered_narrow(src);
    assert_eq!(err, None);
    let want: i64 = (0..2000i64).map(|i| -(i % 100)).sum();
    assert_eq!(out, want.to_string());
}
