//! P6 — `## Tier <opt> <eager|t1|t2|t3|never>` decorators pin an optimization to a
//! hotness tier (HOTSWAP §8). End-to-end: the decorator lexes (`BlockType::Tier`),
//! the parser collects it (`program_tier_pins`), the run-path engine overlays it onto
//! the env `HotswapConfig`, and `optimize_for_run_tiered` honors it via `admits_pinned`.
//! `decorate_tiers` injects these exactly as `decorate_source` injects `## No <X>`, so
//! the benchmarks UI can pin an opt the same way it disables one.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{
    optimizations_fired_run, tw_outcome_with_args, vm_outcome_tiered,
};
use logicaffeine_compile::optimization::{decorate_tiers, Tier};

/// Self-contained, no-args program whose iterative-`gcd` helper the run-path optimizer
/// rewrites (inline-leaf folds it, etc.). Syntax verified against `benchmarks/programs/gcd`.
const PROG: &str = "\
## To gcd (a: Int, b: Int) -> Int:
    Let mutable x be a.
    Let mutable y be b.
    While y is greater than 0:
        Let temp be y.
        Set y to x % y.
        Set x to temp.
    Return x.

## Main
Let mutable sum be 0.
Let mutable i be 1.
While i is at most 50:
    Set sum to sum + gcd(i, 50).
    Set i to i + 1.
Show sum.
";

/// The optimizations `optimize_for_run_tiered` actually gates — the ones a `## Tier`
/// pin can suppress (the rest fire on the AOT/VM-compile paths, not the run optimizer).
const GATED: &[&str] = &[
    "inline", "specialize", "comptime", "loopcse", "floatstrength", "cse", "affine",
    "unroll", "scalarize", "loophoist", "closedform", "fuse", "oracle", "deadcode", "unfold",
];

/// The first run-path-gated optimization that fires on `PROG` — the victim to pin.
fn a_fired_gated_opt() -> &'static str {
    let fired = optimizations_fired_run(PROG);
    GATED
        .iter()
        .copied()
        .find(|o| fired.contains(o))
        .unwrap_or_else(|| panic!("no tier-gated opt fired on the run path, got {fired:?}"))
}

#[test]
fn tier_never_pin_suppresses_an_opt_on_the_run_path() {
    let victim = a_fired_gated_opt();
    let fired_before = optimizations_fired_run(PROG);
    assert!(fired_before.contains(&victim));
    // `## Tier <victim> never` pins it off the ladder — it no longer fires.
    let pinned = decorate_tiers(PROG, &[(victim, "never")]);
    let fired_after = optimizations_fired_run(&pinned);
    assert!(
        !fired_after.contains(&victim),
        "`## Tier {victim} never` must suppress it; before={fired_before:?} after={fired_after:?}"
    );
}

#[test]
fn tier_pin_preserves_output() {
    // A pin changes WHICH opts run, never the result: the pinned program, optimized at
    // T3, still matches the tree-walker oracle byte-for-byte.
    let victim = a_fired_gated_opt();
    let pinned = decorate_tiers(PROG, &[(victim, "never")]);
    let vm = vm_outcome_tiered(&pinned, &[], Tier::T3, None);
    let tw = tw_outcome_with_args(&pinned, &[]);
    assert_eq!(
        (vm.output.trim(), &vm.error),
        (tw.output.trim(), &tw.error),
        "pinned program diverged from the oracle"
    );
    assert!(!vm.output.trim().is_empty(), "pinned program produced no output");
}
