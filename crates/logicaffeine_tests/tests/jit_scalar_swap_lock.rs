//! Scalar swaps execute natively and preserve the reference result.
#![cfg(not(target_arch = "wasm32"))]
use logicaffeine_compile::compile::{tw_outcome, vm_outcome, vm_outcome_tiered};
use logicaffeine_compile::optimization::Tier;
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;
fn reference(source: &str, expected: &str) {
    for (name, run) in [
        ("tree walker", tw_outcome(source)),
        ("VM", vm_outcome(source)),
    ] {
        assert_eq!(run.error, None, "{name}: {source}");
        assert_eq!(
            run.output.trim_end_matches('\n'),
            expected,
            "{name}: {source}"
        );
    }
}
fn native_regions(src: &str) -> (String, u64) {
    let tier = ForgeTier::new();
    let run = vm_outcome_tiered(src, &[], Tier::T3, Some(&tier as &dyn NativeTier));
    assert_eq!(run.error, None, "the native tier refused:\n{src}");
    let (_, regions) = tier.region_counts();
    (run.output.trim_end().to_string(), regions as u64)
}

#[test]
fn a_loop_that_swaps_two_integers_runs_as_native_code() {
    let src = "## Main
Let mutable a be 1.
Let mutable b be 2.
Let mutable total be 0.
Let mutable i be 0.
While i is less than 5000:
    Set total to (total * 31 + a) % 1000000007.
    Swap a and b.
    Set i to i + 1.
Show total.
";
    let expected = {
        let (mut a, mut b, mut total) = (1i64, 2i64, 0i64);
        for _ in 0..5000 {
            total = (total * 31 + a) % 1_000_000_007;
            std::mem::swap(&mut a, &mut b);
        }
        total.to_string()
    };
    reference(src, &expected);
    let (out, regions) = native_regions(src);
    assert_eq!(out, expected);
    #[cfg(target_arch = "x86_64")]
    assert!(
        regions >= 1,
        "the swapping loop must compile to native code"
    );
}

#[test]
fn a_loop_that_swaps_two_reals_runs_as_native_code() {
    let src = "## Main
Let mutable a be 1.5.
Let mutable b be 0.25.
Let mutable total be 0.0.
Let mutable i be 0.
While i is less than 4000:
    Set total to total + a * 2.0.
    Swap a and b.
    Set i to i + 1.
Show total.
";
    let (out, regions) = native_regions(src);
    reference(src, &out);
    assert_eq!(out, "7000");
    #[cfg(target_arch = "x86_64")]
    assert!(
        regions >= 1,
        "the swapping loop must compile to native code"
    );
}
