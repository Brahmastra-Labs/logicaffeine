//! M10b RED gate: DECLARED parameter/return kinds flow into the function
//! JIT. `## To f (x: Float) -> Float` seeds the adapter's kind dataflow
//! (no more params-are-Int assumption), `try_native` guards each argument
//! against its declared kind (floats travel as bits in i64 slots, re-boxed
//! at the return boundary), and the `sqrt` builtin compiles as a pure
//! helper-call — together these unlock the nbody/spectral_norm cluster.

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

fn tiered(src: &str) -> (String, Option<String>, u32, u32) {
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
        let (_, fn_ok) = tier.function_counts();
        let (_, region_ok) = tier.region_counts();
        (norm(&vm.output), vm.error, fn_ok, region_ok)
    })
}

/// A declared-Float function must JIT and produce bit-exact results
/// (dyadic arithmetic — the sum is exact, no display slack).
#[test]
fn float_function_tiers_and_matches() {
    let src = "## To smooth (x: Float) -> Float:\n\
               \x20   Return x * 0.5 + 0.25.\n\
               \n\
               ## Main\n\
               Let mutable acc be 0.0.\n\
               Let mutable i be 0.\n\
               While i is less than 2000:\n\
               \x20   Set acc to acc + smooth(0.5 * i).\n\
               \x20   Set i to i + 1.\n\
               Show \"{acc:.1}\".\n";
    let (out, err, fn_ok, _) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "500250.0");
    assert!(fn_ok >= 1, "declared-Float function must JIT (got {fn_ok})");
}

/// Mixed declared kinds (Float, Int) through SELF-RECURSION: the declared
/// return kind seeds the self-call without the two-pass inference.
#[test]
fn mixed_float_int_recursion_tiers() {
    let src = "## To geo (x: Float, n: Int) -> Float:\n\
               \x20   If n equals 0:\n\
               \x20       Return x.\n\
               \x20   Return geo(x * 0.5, n - 1).\n\
               \n\
               ## Main\n\
               Let mutable acc be 0.0.\n\
               Let mutable i be 0.\n\
               While i is less than 500:\n\
               \x20   Set acc to acc + geo(1024.0, 10).\n\
               \x20   Set i to i + 1.\n\
               Show \"{acc:.1}\".\n";
    let (out, err, fn_ok, _) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "500.0", "500 × geo(1024, 10) = 500 × 1.0");
    assert!(fn_ok >= 1, "mixed-kind recursive function must JIT (got {fn_ok})");
}

/// A declared-Bool parameter: the entry guard admits Bool values and the
/// body's branch-on-param compiles.
#[test]
fn bool_param_function_tiers() {
    let src = "## To pick (flag: Bool, a: Int, b: Int) -> Int:\n\
               \x20   If flag:\n\
               \x20       Return a.\n\
               \x20   Return b.\n\
               \n\
               ## Main\n\
               Let mutable t be 0.\n\
               Let mutable i be 0.\n\
               While i is less than 1000:\n\
               \x20   Set t to t + pick(i % 2 equals 0, i, 0 - i).\n\
               \x20   Set i to i + 1.\n\
               Show t.\n";
    let (out, err, fn_ok, _) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "-500", "Σ even − Σ odd over 0..999");
    assert!(fn_ok >= 1, "Bool-param function must JIT (got {fn_ok})");
}

/// Regression floor: declared-Int functions keep tiering exactly as before.
#[test]
fn int_function_floor_still_tiers() {
    let src = "## To fib (n: Int) -> Int:\n\
               \x20   If n is less than 2:\n\
               \x20       Return n.\n\
               \x20   Return fib(n - 1) + fib(n - 2).\n\
               \n\
               ## Main\n\
               Show fib(20).\n";
    let (out, err, fn_ok, _) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "6765");
    assert!(fn_ok >= 1, "Int functions must keep tiering (got {fn_ok})");
}

/// `sqrt` in a hot loop compiles as a pure helper-call inside the REGION
/// (the spectral_norm/nbody shape). Perfect-square dyadics keep the sum
/// exact — bit-for-bit display parity, no slack.
#[test]
fn sqrt_region_tiers_and_matches() {
    let src = "## Main\n\
               Let mutable acc be 0.0.\n\
               Let mutable i be 0.\n\
               While i is less than 5000:\n\
               \x20   Set acc to acc + sqrt(0.25 * i * i).\n\
               \x20   Set i to i + 1.\n\
               Show \"{acc:.1}\".\n";
    let (out, err, _, region_ok) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "6248750.0", "Σ 0.5·i over 0..4999");
    assert!(region_ok >= 1, "the sqrt loop must tier as a region (got {region_ok})");
}

/// `sqrt` inside a declared-Float FUNCTION (the nbody `dist` kernel).
#[test]
fn sqrt_inside_function_tiers() {
    let src = "## To dist (a: Float, b: Float) -> Float:\n\
               \x20   Return sqrt(a * a + b * b).\n\
               \n\
               ## Main\n\
               Let mutable acc be 0.0.\n\
               Let mutable i be 0.\n\
               While i is less than 1000:\n\
               \x20   Set acc to acc + dist(i * 3.0, i * 4.0).\n\
               \x20   Set i to i + 1.\n\
               Show \"{acc:.1}\".\n";
    let (out, err, fn_ok, _) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "2497500.0", "Σ 5·i over 0..999 (3-4-5 triangles)");
    assert!(fn_ok >= 1, "the dist kernel must JIT (got {fn_ok})");
}

/// Deopt parity through a float-param function: an int division by zero at
/// the recursion base must unwind the native stack, replay on bytecode and
/// surface the exact kernel error with exact partial output.
#[test]
fn float_function_deopt_replays_with_exact_error() {
    let src = "## To risky (x: Float, n: Int) -> Float:\n\
               \x20   If n equals 0:\n\
               \x20       Return x + 100 / n * 1.0.\n\
               \x20   Return risky(x * 0.5, n - 1).\n\
               \n\
               ## Main\n\
               Show 7.\n\
               Show risky(64.0, 300).\n";
    let src_owned = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src_owned, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src_owned, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "float deopt replay diverged"
        );
        assert!(vm.error.is_some(), "division by zero at the base case must error");
        assert_eq!(norm(&vm.output), "7");
    });
}
