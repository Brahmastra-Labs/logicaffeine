//! ★ SPEED LOCK — specializing a function on its divisor pays. ★
//!
//! The same hot function, a digit sum in base `k` with `k` read from the command line and passed on every call, runs
//! twice through the JIT: specialized (its divisions by `k` are a multiply-high and shifts) and with specialization off
//! (`LOGOS_VALUE_SPEC=0`, the generic function's `idiv`). Both print the same answer, and the specialized run is
//! faster by at least the floor below, best of three each. The `_perf_lock` name puts it in the run's `cpu-timing`
//! group (`.config/nextest.toml`), so nextest runs it with nothing else of the run beside it. This binary holds one
//! test: it sets the environment, and no other test may run in its process while it does.

#![cfg(all(target_arch = "x86_64", not(target_os = "windows")))]

use logicaffeine_jit::{optimization_trace_jit, JitOptimization};
use std::time::{Duration, Instant};

const HOT_DIGIT_SUM: &str = "\
## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## To digitSum (k: Int, x: Int) -> Int:
    If x is less than 1:
        Return 0.
    Let mutable n be x.
    Let mutable s be 0.
    While n is greater than 0:
        Set s to s + n % k.
        Set n to n / k.
    Return s.

## Main
Let k be parseInt(item 2 of args()).
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 20000000:
    Set acc to acc + digitSum(k, i * 7919 + 1000003).
    Set i to i + 1.
Show acc.
";

/// The specialized function runs at least this many times as fast as the generic one. Measured 2.79–2.87× over six
/// runs on an i9-14900K (2026-11-20, test-opt); the floor leaves room for a CPU with a faster divider, and a function
/// that fell back to `idiv` runs at 1.0×.
const FLOOR: f64 = 1.5;

fn best_of_three(specialize: bool) -> (Duration, String, u32) {
    if specialize {
        std::env::remove_var("LOGOS_VALUE_SPEC");
    } else {
        std::env::set_var("LOGOS_VALUE_SPEC", "0");
    }
    let args = ["logos".to_string(), "7".to_string()];
    let mut best = Duration::MAX;
    let mut output = String::new();
    let mut specialized = 0;
    for _ in 0..3 {
        let start = Instant::now();
        let traced = optimization_trace_jit(HOT_DIGIT_SUM, &args);
        best = best.min(start.elapsed());
        assert_eq!(traced.outcome.error, None);
        assert!(traced.function_successes >= 1, "`digitSum` compiles: {traced:?}");
        output = traced.outcome.output;
        specialized = traced.native.count(JitOptimization::ValueSpecializedFunction);
    }
    std::env::remove_var("LOGOS_VALUE_SPEC");
    (best, output, specialized)
}

#[test]
fn a_function_specialized_on_its_divisor_beats_the_generic_function() {
    let (generic, generic_output, generic_specialized) = best_of_three(false);
    let (fast, fast_output, fast_specialized) = best_of_three(true);
    assert_eq!(generic_specialized, 0, "LOGOS_VALUE_SPEC=0 compiles the generic function");
    assert_eq!(fast_specialized, 1, "the function is specialized on k");
    assert_eq!(fast_output, generic_output, "both functions print the same answer");
    let ratio = generic.as_secs_f64() / fast.as_secs_f64();
    eprintln!("generic {generic:?}, specialized {fast:?}: {ratio:.2}x");
    assert!(ratio >= FLOOR, "the specialized function runs {ratio:.2}x the generic one; the floor is {FLOOR}x");
}
