//! ★ SPEED LOCK — specializing a loop on its divisor pays. ★
//!
//! The same hot loop, `% k` with `k` read from the command line, runs twice through the JIT: specialized (the
//! division is a multiply-high and shifts) and with specialization off (`LOGOS_VALUE_SPEC=0`, the generic region's
//! `idiv`). Both print the same answer, and the specialized run is faster by at least the floor below, best of three
//! each. The `_perf_lock` name puts it in the run's `cpu-timing` group (`.config/nextest.toml`), so nextest runs it
//! with nothing else of the run beside it. This binary holds one test: it sets the environment, and no other test may
//! run in its process while it does.

#![cfg(all(target_arch = "x86_64", not(target_os = "windows")))]

use logicaffeine_jit::optimization_trace_jit;
use std::time::{Duration, Instant};

const HOT_REMAINDER: &str = "\
## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let k be parseInt(item 2 of args()).
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 40000000:
    Set acc to acc + (i * 7919 - 20000000) % k.
    Set i to i + 1.
Show acc.
";

/// The specialized loop runs at least this many times as fast as the generic one. Measured 1.61–1.63× over six runs
/// on an i9-14900K (2026-11-18); the floor leaves room for a CPU with a faster divider, and a loop that fell back to
/// `idiv` runs at 1.0×.
const FLOOR: f64 = 1.25;

fn best_of_three(specialize: bool) -> (Duration, String, bool) {
    if specialize {
        std::env::remove_var("LOGOS_VALUE_SPEC");
    } else {
        std::env::set_var("LOGOS_VALUE_SPEC", "0");
    }
    let args = ["logos".to_string(), "7".to_string()];
    let mut best = Duration::MAX;
    let mut output = String::new();
    let mut specialized = false;
    for _ in 0..3 {
        let start = Instant::now();
        let traced = optimization_trace_jit(HOT_REMAINDER, &args);
        best = best.min(start.elapsed());
        assert_eq!(traced.outcome.error, None);
        output = traced.outcome.output;
        specialized = traced.native.keywords().contains(&"value_specialized_region");
    }
    std::env::remove_var("LOGOS_VALUE_SPEC");
    (best, output, specialized)
}

#[test]
fn a_loop_specialized_on_its_divisor_beats_the_generic_loop() {
    let (generic, generic_output, generic_specialized) = best_of_three(false);
    let (fast, fast_output, fast_specialized) = best_of_three(true);
    assert!(!generic_specialized, "LOGOS_VALUE_SPEC=0 compiles the generic region");
    assert!(fast_specialized, "the loop is specialized on k");
    assert_eq!(fast_output, generic_output, "both regions print the same answer");
    let ratio = generic.as_secs_f64() / fast.as_secs_f64();
    eprintln!("generic {generic:?}, specialized {fast:?}: {ratio:.2}x");
    assert!(ratio >= FLOOR, "the specialized loop runs {ratio:.2}x the generic one; the floor is {FLOOR}x");
}
