//! M-MAP RED gate: regions speak MAP get/set/contains through pure
//! helper calls against the SAME boxed FxHashMap storage the kernel uses —
//! no representation change, so observable ITERATION ORDER is preserved
//! by construction. The int fast lane: keys and values that are Ints stay
//! unboxed in the frame; anything else (missing key, non-Int value) is a
//! helper miss that side-exits, and the replay raises the kernel's exact
//! error.

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

/// The two_sum kernel: contains + insert with int keys per iteration,
/// mixed with array reads. The loop region must tier.
#[test]
fn two_sum_kernel_region_tiers() {
    let src = "## Main\n\
               Let mutable arr be a new Seq of Int.\n\
               Let mutable seed be 42.\n\
               Let mutable i be 0.\n\
               While i is less than 30000:\n\
               \x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
               \x20   Push ((seed / 65536) % 32768) % 30000 to arr.\n\
               \x20   Set i to i + 1.\n\
               Let mutable seen be a new Map of Int to Int.\n\
               Let mutable count be 0.\n\
               Set i to 1.\n\
               While i is at most 30000:\n\
               \x20   Let x be item i of arr.\n\
               \x20   Let complement be 30000 - x.\n\
               \x20   If complement is at least 0:\n\
               \x20       If seen contains complement:\n\
               \x20           Set count to count + 1.\n\
               \x20   Set seen at x to 1.\n\
               \x20   Set i to i + 1.\n\
               Show count.\n";
    let (_, err, region_ok) = tiered(src);
    assert_eq!(err, None);
    assert!(region_ok >= 1, "the two_sum loop must tier with map traffic (got {region_ok})");
}

/// The collect kernel: insert then read-back-and-compare, int keys and
/// values throughout.
#[test]
fn collect_kernel_regions_tier() {
    let src = "## Main\n\
               Let mutable m be a new Map of Int to Int.\n\
               Let mutable i be 1.\n\
               While i is less than 20001:\n\
               \x20   Set item i of m to i * 2.\n\
               \x20   Set i to i + 1.\n\
               Let mutable found be 0.\n\
               Set i to 1.\n\
               While i is less than 20001:\n\
               \x20   If item i of m equals i * 2:\n\
               \x20       Set found to found + 1.\n\
               \x20   Set i to i + 1.\n\
               Show found.\n";
    let (out, err, region_ok) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "20000");
    assert!(region_ok >= 2, "both collect loops must tier (got {region_ok})");
}

/// A MISSING key at a data-dependent iteration: the helper miss side-exits
/// and the replay raises the kernel's exact `Key not found` error with
/// exact partial output.
#[test]
fn map_get_miss_mid_region_is_exact() {
    let src = "## Main\n\
               Show 3.\n\
               Let mutable m be a new Map of Int to Int.\n\
               Let mutable i be 0.\n\
               While i is less than 50000:\n\
               \x20   Set item i of m to i.\n\
               \x20   Set i to i + 1.\n\
               Let mutable s be 0.\n\
               Set i to 0.\n\
               While i is less than 60000:\n\
               \x20   Set s to s + item i of m.\n\
               \x20   Set i to i + 1.\n\
               Show s.\n";
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "map miss deopt diverged"
    );
    assert!(vm.error.is_some(), "key 50000 must be missing");
    assert_eq!(norm(&vm.output), "3");
}

/// A NON-INT value planted before the loop: the int fast lane must refuse
/// it (helper miss → replay), and the program's dynamic semantics hold.
#[test]
fn non_int_value_defeats_the_fast_lane_exactly() {
    let src = "## Main\n\
               Let mutable m be a new Map of Int to Int.\n\
               Let mutable i be 0.\n\
               While i is less than 5000:\n\
               \x20   Set item i of m to i * 3.\n\
               \x20   Set i to i + 1.\n\
               Let mutable s be 0.\n\
               Set i to 0.\n\
               While i is less than 5000:\n\
               \x20   Set s to s + item i of m.\n\
               \x20   Set i to i + 1.\n\
               Show s.\n";
    let (out, err, _) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "37492500", "Σ 3i over 0..4999");
}

/// Iteration order after region mutations must be IDENTICAL to the
/// tree-walker's: the fast lane shares the kernel's exact storage, so the
/// hash-order walk cannot drift.
#[test]
fn iteration_order_after_region_matches_kernel() {
    let src = "## Main\n\
               Let mutable m be a new Map of Int to Int.\n\
               Let mutable i be 0.\n\
               While i is less than 2000:\n\
               \x20   Set item (i * 7 % 1009) of m to i.\n\
               \x20   Set i to i + 1.\n\
               Let mutable acc be 0.\n\
               Repeat for pair in m:\n\
               \x20   Set acc to acc * 31 + item 1 of pair + item 2 of pair.\n\
               \x20   Set acc to acc % 1000000007.\n\
               Show acc.\n";
    let (_, err, _) = tiered(src);
    assert_eq!(err, None);
}

/// Contains over hits and misses, interleaved with inserts.
#[test]
fn contains_mix_region_tiers() {
    let src = "## Main\n\
               Let mutable m be a new Map of Int to Int.\n\
               Let mutable hits be 0.\n\
               Let mutable i be 0.\n\
               While i is less than 40000:\n\
               \x20   If m contains (i % 977):\n\
               \x20       Set hits to hits + 1.\n\
               \x20   Set item (i % 1009) of m to i.\n\
               \x20   Set i to i + 1.\n\
               Show hits.\n";
    let (_, err, region_ok) = tiered(src);
    assert_eq!(err, None);
    assert!(region_ok >= 1, "the contains/insert loop must tier (got {region_ok})");
}
