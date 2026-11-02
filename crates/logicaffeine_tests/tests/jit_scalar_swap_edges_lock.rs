//! Scalar register exchanges preserve values across native execution and fallback.
#![cfg(not(target_arch = "wasm32"))]
use logicaffeine_compile::compile::{tw_outcome, vm_outcome, vm_outcome_tiered};
use logicaffeine_compile::optimization::Tier;
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

fn holds(source: &str, expected: &str, must_tier: bool) {
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
    let tier = ForgeTier::new();
    let native_before = (
        logicaffeine_jit::REGION_RUNS.load(std::sync::atomic::Ordering::Relaxed),
        logicaffeine_jit::REGION_DEOPTS.load(std::sync::atomic::Ordering::Relaxed),
    );
    let run = vm_outcome_tiered(source, &[], Tier::T3, Some(&tier as &dyn NativeTier));
    assert_eq!(run.error, None, "{source}");
    assert_eq!(run.output.trim_end_matches('\n'), expected, "{source}");
    if cfg!(target_arch = "x86_64") && must_tier {
        assert!(
            tier.region_counts().1 > 0,
            "swap loop did not compile natively: {source}"
        );
        let native_after = (
            logicaffeine_jit::REGION_RUNS.load(std::sync::atomic::Ordering::Relaxed),
            logicaffeine_jit::REGION_DEOPTS.load(std::sync::atomic::Ordering::Relaxed),
        );
        assert!(
            native_after.0 > native_before.0 || native_after.1 > native_before.1,
            "compiled swap loop never entered native code: {source}"
        );
    }
}

#[test]
fn self_swap_preserves_the_live_register() {
    holds(
        "## Main
Let mutable a be 41.
Let mutable total be 0.
Let mutable i be 0.
While i is less than 4001:
    Swap a and a.
    Set total to total + a.
    Set i to i + 1.
Show total.
Show a.
",
        "164041\n41",
        true,
    );
}

#[test]
fn boolean_swap_preserves_branch_choices() {
    holds(
        "## Main
Let mutable a be true.
Let mutable b be false.
Let mutable total be 0.
Let mutable i be 0.
While i is less than 4001:
    If a:
        Set total to total + 1.
    Swap a and b.
    Set i to i + 1.
Show total.
",
        "2001",
        true,
    );
}

#[test]
fn word32_swap_preserves_high_bits() {
    holds(
        "## Main
Let mutable a be word32(4294967295).
Let mutable b be word32(17).
Let mutable total be 0.
Let mutable i be 0.
While i is less than 4001:
    Set total to total + intOfWord32(a).
    Swap a and b.
    Set i to i + 1.
Show total.
Show intOfWord32(a).
Show intOfWord32(b).
",
        &format!("{}\n17\n4294967295", 2001i64 * 4294967295 + 2000 * 17),
        true,
    );
}

#[test]
fn word64_swap_preserves_high_bits() {
    holds(
        "## Main
Let mutable a be word64(0 - 3).
Let mutable b be word64(17).
Let mutable total be 0.
Let mutable i be 0.
While i is less than 4001:
    Set total to total + intOfWord64(a).
    Swap a and b.
    Set i to i + 1.
Show total.
Show intOfWord64(a).
Show intOfWord64(b).
",
        &format!("{}\n17\n-3", 2001i64 * -3 + 2000 * 17),
        true,
    );
}

#[test]
fn overflow_after_swap_preserves_both_values_at_deoptimization() {
    holds(
        "## Main
Let mutable a be 1.
Let mutable b be 2.
Let mutable i be 0.
While i is less than 2001:
    Swap a and b.
    If i is equal to 2000:
        Set a to a + 9223372036854775807.
    Set i to i + 1.
Show a.
Show b.
",
        "9223372036854775809\n1",
        true,
    );
}

#[test]
fn collection_swap_fallback_preserves_copy_on_write() {
    holds(
        "## Main
Let mutable xs be [1].
Let mutable ys be [2].
Let saved be copy of xs.
Let mutable i be 0.
While i is less than 1001:
    Swap xs and ys.
    Set item 1 of xs to (item 1 of xs) + 1.
    Set i to i + 1.
Show xs.
Show ys.
Show saved.
",
        "[503]\n[501]\n[1]",
        false,
    );
}

#[test]
fn mixed_scalar_swap_roundtrip_preserves_intermediate_float_bits() {
    holds(
        "## Main
Let mutable a be 3.
Let mutable b be 1.5.
Let mutable total be 0.0.
Let mutable i be 0.
While i is less than 4000:
    Swap a and b.
    Set total to total + a.
    Swap a and b.
    Set i to i + 1.
Show total.
Show a.
Show b.
",
        "6000\n3\n1.5",
        true,
    );
}

#[test]
fn mixed_scalar_swap_before_break_preserves_exit_types() {
    holds(
        "## Main
Let mutable a be 3.
Let mutable b be 1.5.
Let mutable i be 0.
While i is less than 4000:
    Swap a and b.
    If i is equal to 3999:
        Break.
    Swap a and b.
    Set i to i + 1.
Show a.
Show b.
Show i.
",
        "1.5\n3\n3999",
        true,
    );
}

#[test]
fn mixed_scalar_swap_before_overflow_preserves_deoptimized_types() {
    holds(
        "## Main
Let mutable a be 3.
Let mutable b be 1.5.
Let mutable i be 0.
While i is less than 4000:
    Swap a and b.
    If i is equal to 3999:
        Set b to b + 9223372036854775807.
        Break.
    Swap a and b.
    Set i to i + 1.
Show a.
Show b.
",
        "1.5\n9223372036854775810",
        true,
    );
}

#[test]
fn boolean_word_swap_preserves_native_scalar_representations() {
    holds(
        "## Main
Let mutable a be true.
Let mutable b be word64(0 - 3).
Let mutable total be 0.
Let mutable i be 0.
While i is less than 4000:
    Swap a and b.
    Set total to total + intOfWord64(a).
    Swap a and b.
    If a:
        Set total to total + 1.
    Set i to i + 1.
Show total.
Show a.
Show intOfWord64(b).
",
        "-8000\ntrue\n-3",
        true,
    );
}
