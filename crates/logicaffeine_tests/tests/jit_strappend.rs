//! WAVE 17 RED gate: a STRING-APPEND op for the contiguous regalloc backend so
//! the haystack-BUILD loop (`Set text to text + ch`) tiers instead of falling
//! back to bytecode.
//!
//! The string_search benchmark spends ~85% of its time in a ~15M-iteration loop
//! that grows an accumulator string one char at a time (`Set text to text + ch`,
//! lowered to `Op::AddAssign`/`Op::Concat` of a `Text` and a 1-char `Text`). A
//! prior wave made the VM append in place when the accumulator is the sole owner
//! of its `Rc<String>` (amortized O(n)), but the loop still runs on BYTECODE: the
//! regalloc backend has no string-append op, so the `AddAssign` on a non-numeric
//! operand makes the whole build region ineligible and it never tiers.
//!
//! SOUNDNESS — the differential gate is sacred (the tiered VM must be
//! BIT-IDENTICAL to the tree-walker). The append must reproduce the VM's
//! `add_assign`/`concat` semantics EXACTLY:
//!   * sole-owned `Text` → append in place (`Rc::get_mut(rc).push_str(...)`);
//!   * NOT sole-owned (aliased) → copy-on-write: a FRESH `Rc::new(format!(...))`
//!     replaces the accumulator's register, leaving the alias untouched;
//!   * a non-`Text` accumulator falls to the kernel `add`.
//! A region that cannot guarantee it matches the VM for every alias case MUST
//! fall back (decline to tier) — correctness over coverage.
//!
//! These tests assert two things per shape: the output is bit-identical to the
//! tree-walker (the spec), and the build loop now tiers through the contiguous
//! regalloc REGION backend (`regalloc_region_count() >= 1`). Correctness never
//! depends on tiering; the tiering assertion is a SEPARATE, secondary check.

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

/// Run through the tiered VM + the tree-walker, assert they agree bit for bit,
/// and return `(normalized output, error, regalloc REGION count)`.
fn tiered_region(src: &str) -> (String, Option<String>, u32) {
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
        (norm(&vm.output), vm.error, tier.regalloc_region_count())
    })
}

/// Same, with positional CLI args (string_search reads `item 2 of arguments`).
fn tiered_region_args(src: &str, args: &[String]) -> (String, Option<String>, u32) {
    let src = src.to_string();
    let args = args.to_vec();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src, &args, Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src, &args);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "tiered VM diverged from tree-walker on:\n{src}"
        );
        (norm(&vm.output), vm.error, tier.regalloc_region_count())
    })
}

/// RED: a pure sole-owned string-BUILD loop (`Set text to text + ch`) must tier
/// through the contiguous regalloc REGION backend, bit-identical to the
/// tree-walker. The accumulator is grown in place (it is the sole owner of its
/// `Rc<String>`), so the append helper takes the in-place push path.
#[test]
#[cfg(target_arch = "x86_64")]
fn str_build_loop_tiers_via_regalloc_region() {
    let src = "## Main\n\
               Let mutable text be \"\".\n\
               Let mutable i be 0.\n\
               While i is less than 50000:\n\
               \x20   Let mutable ch be \"a\".\n\
               \x20   If i % 5 equals 1:\n\
               \x20       Set ch to \"b\".\n\
               \x20   If i % 5 equals 2:\n\
               \x20       Set ch to \"c\".\n\
               \x20   Set text to text + ch.\n\
               \x20   Set i to i + 1.\n\
               Show length of text.\n";
    let (out, err, ra) = tiered_region(src);
    assert_eq!(err, None);
    assert_eq!(out, "50000");
    assert!(
        ra >= 1,
        "the sole-owned string-build loop must tier through the regalloc REGION backend (got {ra})"
    );
}

/// RED-soundness: a build loop where the accumulator is ALSO aliased (a snapshot
/// `Set saved to text` is taken at a fixed point and shown afterward). The append
/// at the alias point is NOT sole-owned, so the VM takes the copy-on-write path
/// (a fresh `Rc` for the accumulator, leaving `saved` pointing at the old value).
/// The tiered VM must reproduce this EXACTLY — the alias must keep its captured
/// prefix while the accumulator keeps growing. Output is the load-bearing spec.
#[test]
#[cfg(target_arch = "x86_64")]
fn str_build_loop_with_alias_is_cow_correct() {
    let src = "## Main\n\
               Let mutable text be \"\".\n\
               Let mutable saved be \"\".\n\
               Let mutable i be 0.\n\
               While i is less than 2000:\n\
               \x20   If i equals 1000:\n\
               \x20       Set saved to text.\n\
               \x20   Set text to text + \"x\".\n\
               \x20   Set i to i + 1.\n\
               Show length of text.\n\
               Show length of saved.\n";
    // The differential gate inside `tiered_region` is the real assertion: the
    // tiered VM (whatever it does for the aliased append) must match the
    // tree-walker, which takes the COW path at i == 1000.
    let (out, err, _ra) = tiered_region(src);
    assert_eq!(err, None);
    assert_eq!(out, "2000\n1000");
}

/// RED: the REAL string_search build shape — the `% 5` cascade selecting `ch`,
/// the periodic 5-char `"XXXXX"` append, and the 1-char `Set text to text + ch`
/// hot append — then the naive count over the built haystack. The build region
/// must tier through the regalloc backend AND the count must be bit-identical to
/// the tree-walker (the benchmark's exact output).
#[test]
#[cfg(target_arch = "x86_64")]
fn string_search_build_shape_tiers_and_counts() {
    let src = "## Main\n\
               Let arguments be args().\n\
               Let n be parseInt(item 2 of arguments).\n\
               Let mutable text be \"\".\n\
               Let mutable pos be 0.\n\
               While pos is less than n:\n\
               \x20   If pos is greater than 0:\n\
               \x20       If pos % 1000 equals 0:\n\
               \x20           If pos + 5 is at most n:\n\
               \x20               Set text to text + \"XXXXX\".\n\
               \x20               Set pos to pos + 5.\n\
               \x20   If pos is less than n:\n\
               \x20       Let mutable ch be \"a\".\n\
               \x20       If pos % 5 equals 1:\n\
               \x20           Set ch to \"b\".\n\
               \x20       If pos % 5 equals 2:\n\
               \x20           Set ch to \"c\".\n\
               \x20       If pos % 5 equals 3:\n\
               \x20           Set ch to \"d\".\n\
               \x20       If pos % 5 equals 4:\n\
               \x20           Set ch to \"e\".\n\
               \x20       Set text to text + ch.\n\
               \x20       Set pos to pos + 1.\n\
               Let needle be \"XXXXX\".\n\
               Let needleLen be 5.\n\
               Let textLen be length of text.\n\
               Let mutable count be 0.\n\
               Let mutable i be 1.\n\
               While i is at most textLen - needleLen + 1:\n\
               \x20   Let mutable match be 1.\n\
               \x20   Let mutable j be 0.\n\
               \x20   While j is less than needleLen:\n\
               \x20       If item (i + j) of text is not item (j + 1) of needle:\n\
               \x20           Set match to 0.\n\
               \x20           Set j to needleLen.\n\
               \x20       Set j to j + 1.\n\
               \x20   If match equals 1:\n\
               \x20       Set count to count + 1.\n\
               \x20   Set i to i + 1.\n\
               Show count.\n";
    let args = vec!["string_search".to_string(), "200000".to_string()];
    let (out, err, ra) = tiered_region_args(src, &args);
    assert_eq!(err, None);
    // The tree-walker is the oracle; the differential inside `tiered_region_args`
    // already pins VM == tree-walker. We assert the count is the benchmark's
    // expected value (one "XXXXX" injected per 1000 positions over 200000).
    assert_eq!(out, "199");
    assert!(
        ra >= 1,
        "the string_search build loop must tier through the regalloc REGION backend (got {ra})"
    );
}
