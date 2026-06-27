//! P8 — background forge compilation (HOTSWAP §6): the native JIT compile that today
//! stalls the interpreter (`try_native` → `tier.compile_function`) moves to a worker
//! thread. The interpreter keeps running bytecode while the worker compiles, then
//! drains the result at its profiling points and publishes it (sole `FnTable` writer).
//!
//! This test lives in its own binary so nextest runs it in an isolated PROCESS — the
//! process-wide `install_native_tier` here cannot leak into other tests. It installs a
//! forge tier, runs a hot program through the background path, and asserts (a) output
//! is byte-identical to the tree-walker oracle (background compile is the SAME
//! `compile_function`, just off-thread) and (b) the worker actually engaged (≥1 native
//! function compiled).

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_bg};
use logicaffeine_compile::vm::install_native_tier;
use logicaffeine_jit::ForgeTier;

/// Iterative gcd called 40 000× — well past the 100-call tier threshold, so it is
/// submitted to the background compiler and its native form is reached and used
/// mid-run. Self-contained (no argv). Syntax verified against `benchmarks/programs/gcd`.
const HOT: &str = "\
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
While i is at most 200:
    Let mutable j be 1.
    While j is at most 200:
        Set sum to sum + gcd(i, j).
        Set j to j + 1.
    Set i to i + 1.
Show sum.
";

#[test]
fn background_compile_matches_treewalker_and_engages() {
    // Process-wide install (isolated by nextest's process-per-test).
    let tier: &'static ForgeTier = Box::leak(Box::new(ForgeTier::new()));
    install_native_tier(tier);

    let bg = vm_outcome_bg(HOT, &[]);
    let tw = tw_outcome_with_args(HOT, &[]);
    assert_eq!(
        (bg.output.trim(), &bg.error),
        (tw.output.trim(), &tw.error),
        "background-compiled run diverged from the tree-walker oracle"
    );
    assert!(bg.error.is_none(), "background run errored: {:?}", bg.error);

    let (_, fn_ok) = tier.function_counts();
    assert!(
        fn_ok >= 1,
        "the background worker should have compiled >=1 native function, got {fn_ok}"
    );
}
