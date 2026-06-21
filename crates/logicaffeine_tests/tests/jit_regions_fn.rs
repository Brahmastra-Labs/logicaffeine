//! M4 RED gate: OSR everywhere — loop regions tier up inside FUNCTION frames,
//! not just at Main depth, with the per-function named-register maps the
//! write-back contract needs. Plus the Shannon-entropy primitives (EXODIA 3.3)
//! that drive Tier-2 nomination later.

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

fn tiered(src: &str) -> (String, Option<String>, u32, u32) {
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "tiered VM diverged from tree-walker on:\n{src}"
    );
    let (_, fn_ok) = tier.function_counts();
    let (_, region_ok) = tier.region_counts();
    (norm(&vm.output), vm.error, fn_ok, region_ok)
}

/// A hot loop inside a function called ONCE: the function-call threshold
/// never trips, so only in-function OSR can take it native.
#[test]
fn hot_loop_inside_function_tiers_as_region() {
    // The function also takes a Text param, putting the WHOLE body outside
    // the function adapter's subset — the loop region is the only native
    // path.
    let src = "## To label (tag: Text, n: Int) -> Int:\n\
               \x20   Let mutable sum be 0.\n\
               \x20   Let mutable i be 1.\n\
               \x20   While i is at most n:\n\
               \x20       Set sum to (sum + i) % 1000000007.\n\
               \x20       Set i to i + 1.\n\
               \x20   Return sum.\n\
               \n\
               ## Main\n\
               Show label(\"x\", 5000).\n";
    let (out, err, _, region_ok) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "12502500");
    assert!(
        region_ok >= 1,
        "the in-function hot loop must tier up as a region (got {region_ok})"
    );
}

/// The primes `isPrime`-flag shape INSIDE a function frame: the conditional
/// write-back preservation must consult the FUNCTION's named-register map,
/// not Main's (their indices are unrelated).
#[test]
fn conditional_write_preservation_uses_function_frame_names() {
    let src = "## To countPrimes (tag: Text, n: Int) -> Int:\n\
               \x20   Let mutable count be 0.\n\
               \x20   Let mutable i be 2.\n\
               \x20   While i is at most n:\n\
               \x20       Let mutable isPrime be 1.\n\
               \x20       Let mutable d be 2.\n\
               \x20       While d * d is at most i:\n\
               \x20           If i % d equals 0:\n\
               \x20               Set isPrime to 0.\n\
               \x20               Break.\n\
               \x20           Set d to d + 1.\n\
               \x20       If isPrime equals 1:\n\
               \x20           Set count to count + 1.\n\
               \x20       Set i to i + 1.\n\
               \x20   Return count.\n\
               \n\
               ## Main\n\
               Show countPrimes(\"x\", 500).\n";
    let (out, err, _, _) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "95", "π(500) through a function-frame region");
}

/// Deopt inside a function-frame region: the div-by-zero replay must produce
/// the exact kernel error and partial output.
#[test]
fn function_frame_region_deopt_error_parity() {
    let src = "## To risky (tag: Text, n: Int) -> Int:\n\
               \x20   Let mutable acc be 0.\n\
               \x20   Let mutable i be 1.\n\
               \x20   While i is at most n:\n\
               \x20       Let d be 150 - i.\n\
               \x20       Set acc to acc + 1000 / d.\n\
               \x20       Set i to i + 1.\n\
               \x20   Return acc.\n\
               \n\
               ## Main\n\
               Show 7.\n\
               Show risky(\"x\", 5000).\n";
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "function-frame deopt replay diverged"
    );
    assert!(vm.error.is_some());
    assert_eq!(norm(&vm.output), "7");
}

/// Shannon entropy of branch profiles (EXODIA 3.3): 50/50 is maximal (1.0),
/// always-taken is 0.0, and the function is symmetric.
#[test]
fn branch_entropy_primitives() {
    use logicaffeine_compile::vm::branch_entropy;
    assert!((branch_entropy(500, 500) - 1.0).abs() < 1e-9);
    assert_eq!(branch_entropy(1000, 0), 0.0);
    assert_eq!(branch_entropy(0, 1000), 0.0);
    assert_eq!(branch_entropy(0, 0), 0.0);
    let h1 = branch_entropy(900, 100);
    let h2 = branch_entropy(100, 900);
    assert!((h1 - h2).abs() < 1e-12, "entropy must be symmetric");
    assert!(h1 > 0.0 && h1 < 1.0);
    assert!(branch_entropy(990, 10) < branch_entropy(700, 300));
}
