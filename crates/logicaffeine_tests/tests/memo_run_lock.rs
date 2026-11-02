//! Run-path memoization lock.
//!
//! The run-path memoizer caches a pure single-arg self-recursive function in a
//! module-level `Map` global, bringing the AOT memoizer's win to the
//! interpreter/VM tier. These locks pin:
//!
//!   1. FIRING — it fires on `fib` on the run-path pipeline and gates off under
//!      `## No memo`.
//!   2. CORRECTNESS — the memoized (optimized) run equals the raw run for
//!      moderate `n`.
//!   3. COMPLEXITY — `fib(40)` COMPLETES under the optimizer. Without
//!      memoization that is ~3.3 billion calls (intractable); its completion is
//!      the proof that exponential recursion became linear. (The raw baseline is
//!      NOT run at that size, for the same reason.)

use logicaffeine_compile::compile::{
    compile_to_rust, interpret_program, optimizations_fired_run, vm_run_source,
};
use logicaffeine_compile::optimization::decorate_source;

fn fib_src(n: i64) -> String {
    format!(
        "## To fib (n: Int) -> Int:\n    If n is at most 1:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib({n}).\n"
    )
}

fn opt(src: &str) -> String {
    interpret_program(src)
        .unwrap_or_else(|e| panic!("optimized run failed: {e:?}"))
        .trim_end()
        .to_string()
}
fn raw(src: &str) -> String {
    vm_run_source(src)
        .unwrap_or_else(|e| panic!("raw run failed: {e}"))
        .trim_end()
        .to_string()
}

#[test]
fn memo_fires_and_gates() {
    let src = fib_src(20);
    let run = optimizations_fired_run(&src);
    assert!(
        run.contains(&"memo"),
        "run-path must memoize fib; got {run:?}"
    );
    let off = optimizations_fired_run(&decorate_source(&src, &["memo"]));
    assert!(
        !off.contains(&"memo"),
        "`## No memo` must remove it; got {off:?}"
    );
}

/// The memoized function returns the same value as the un-memoized one, for
/// sizes small enough that the raw exponential run is still feasible.
#[test]
fn memo_preserves_fib() {
    let fibs = [
        (5, "5"),
        (10, "55"),
        (15, "610"),
        (20, "6765"),
        (25, "75025"),
    ];
    for (n, expected) in fibs {
        let src = fib_src(n);
        assert_eq!(opt(&src), expected, "n={n}: memoized value");
        assert_eq!(
            opt(&src),
            raw(&src),
            "n={n}: memoization changed the value (unsound!)"
        );
        assert_eq!(
            opt(&decorate_source(&src, &["memo"])),
            expected,
            "n={n}: memo-off value"
        );
    }
}

/// COMPLEXITY: `fib(40)` completes under the optimizer only because memoization
/// turned the exponential recursion into O(n). If memoization stopped firing,
/// this would run ~3.3 billion calls and hit the test timeout.
#[test]
fn memo_makes_exponential_recursion_tractable() {
    assert_eq!(opt(&fib_src(40)), "102334155", "memoized fib(40)");
}

/// `## Require memo` compiles on `fib` (memoization is no longer an AOT-only
/// construct on the run path) and errors where there is no memoizable function.
#[test]
fn memo_require_gate() {
    let required = fib_src(20).replace("## Main", "## Require memo\n## Main");
    assert!(
        compile_to_rust(&required).is_ok(),
        "`## Require memo` must compile on fib"
    );
    let no_site = "## Require memo\n## Main\nShow 42.\n";
    assert!(
        compile_to_rust(no_site).is_err(),
        "`## Require memo` must error with no memoizable function"
    );
}
