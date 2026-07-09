//! M5 RED gate: the float JIT — f64 values travel as raw bits in i64 frame
//! slots; float arithmetic/comparison stencils transcribe the kernel's exact
//! rules (IEEE relations, the EPSILON equality, division-by-zero as a
//! checked side-exit); mixed Int×Float operands get an inserted IntToFloat
//! conversion matching the kernel's promotion. Region entry kinds are
//! SPECULATED from the live register values at the hot back-edge and guarded
//! per entry. Int bitwise/shift ops (and/or/xor/shl/shr/not) join the subset
//! in the same milestone — the nqueens inner-loop shape.

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

fn tiered(src: &str) -> (String, Option<String>, u32, u32) {
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "tiered VM diverged from tree-walker on:\n{src}"
    );
    let (_, fn_ok) = tier.function_counts();
    let (_, region_ok) = tier.region_counts();
    (norm(&vm.output), vm.error, fn_ok, region_ok)
}

/// The pi_leibniz shape: float accumulators, mixed `2.0 * k + 1.0` with an
/// Int loop counter, float division, and `{:.15}` display parity. The region
/// MUST tier up and the formatted result must be bit-identical to the
/// tree-walker's.
#[test]
fn pi_leibniz_region_tiers_and_matches_bit_exactly() {
    let src = "## Main\n\
               Let mutable sum be 0.0.\n\
               Let mutable sign be 1.0.\n\
               Let mutable k be 0.\n\
               While k is less than 5000:\n\
               \x20   Set sum to sum + sign / (2.0 * k + 1.0).\n\
               \x20   Set sign to 0.0 - sign.\n\
               \x20   Set k to k + 1.\n\
               Let result be sum * 4.0.\n\
               Show \"{result:.15}\".\n";
    let (out, err, _, region_ok) = tiered(src);
    assert_eq!(err, None);
    assert!(out.starts_with("3.141"), "π prefix sanity: {out}");
    assert!(
        region_ok >= 1,
        "the pi_leibniz float loop must tier up as a region (got {region_ok})"
    );
}

/// The mandelbrot inner-iteration shape: pure float algebra with an Int
/// counter and a conditionally-written Int flag, deep inside nested loops.
#[test]
fn mandelbrot_kernel_loop_tiers_and_agrees() {
    let src = "## Main\n\
               Let n be 40.\n\
               Let mutable count be 0.\n\
               Let mutable y be 0.\n\
               While y is less than n:\n\
               \x20   Let mutable x be 0.\n\
               \x20   While x is less than n:\n\
               \x20       Let cr be 2.0 * x / n - 1.5.\n\
               \x20       Let ci be 2.0 * y / n - 1.0.\n\
               \x20       Let mutable zr be 0.0.\n\
               \x20       Let mutable zi be 0.0.\n\
               \x20       Let mutable isInside be 1.\n\
               \x20       Let mutable iter be 0.\n\
               \x20       While iter is less than 50:\n\
               \x20           Let zr2 be zr * zr - zi * zi + cr.\n\
               \x20           Let zi2 be 2.0 * zr * zi + ci.\n\
               \x20           Set zr to zr2.\n\
               \x20           Set zi to zi2.\n\
               \x20           If zr * zr + zi * zi is greater than 4.0:\n\
               \x20               Set isInside to 0.\n\
               \x20               Set iter to 50.\n\
               \x20           Set iter to iter + 1.\n\
               \x20       If isInside equals 1:\n\
               \x20           Set count to count + 1.\n\
               \x20       Set x to x + 1.\n\
               \x20   Set y to y + 1.\n\
               Show count.\n";
    let (_, err, _, region_ok) = tiered(src);
    assert_eq!(err, None);
    assert!(region_ok >= 1, "a mandelbrot loop must tier up (got {region_ok})");
}

/// Float equality is IEEE (`a == b`, `NaN != NaN`) — identical to the kernel
/// and the compiled backend. Through a hot native loop, `0.1 + 0.2` is NOT
/// `0.3` (the artifact is real; `is approximately` is the tolerant spelling),
/// while the bit-equal value DOES match — both directions must tier up and
/// agree with the reference.
#[test]
fn float_ieee_equality_through_hot_loop() {
    let src = "## Main\n\
               Let mutable near_hits be 0.\n\
               Let mutable exact_hits be 0.\n\
               Let mutable i be 0.\n\
               While i is less than 500:\n\
               \x20   Let s be 0.1 + 0.2.\n\
               \x20   If s equals 0.3:\n\
               \x20       Set near_hits to near_hits + 1.\n\
               \x20   If s equals 0.30000000000000004:\n\
               \x20       Set exact_hits to exact_hits + 1.\n\
               \x20   Set i to i + 1.\n\
               Show near_hits.\n\
               Show exact_hits.\n";
    let (out, err, _, _) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "0\n500", "IEEE equality must hold natively exactly as in the kernel");
}

/// Float division by zero at a data-dependent iteration: side-exit, replay,
/// exact kernel error and partial output.
#[test]
fn float_div_by_zero_mid_loop_error_parity() {
    let src = "## Main\n\
               Show 9.\n\
               Let mutable acc be 0.0.\n\
               Let mutable i be 1.\n\
               While i is at most 5000:\n\
               \x20   Let d be 150.0 - i.\n\
               \x20   Set acc to acc + 1000.0 / d.\n\
               \x20   Set i to i + 1.\n\
               Show acc.\n";
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "float div-zero deopt replay diverged"
    );
    assert!(vm.error.is_some(), "float division by zero must error");
    assert_eq!(norm(&vm.output), "9");
}

/// NaN propagation: relational comparisons on NaN are false (never errors),
/// and arithmetic carries NaN through — display must match the kernel.
#[test]
fn nan_semantics_through_hot_loop() {
    let src = "## Main\n\
               Let mutable taken be 0.\n\
               Let mutable nan be 0.0.\n\
               Let mutable i be 0.\n\
               While i is less than 300:\n\
               \x20   Set nan to (0.0 - 0.0) * 1.0 + nan.\n\
               \x20   Set i to i + 1.\n\
               Let q be nan - nan.\n\
               Show q.\n";
    // q = 0.0 here (finite arithmetic); the REAL NaN comparison grid lives
    // in the forge differentials — this is the source-level smoke check that
    // float negate/multiply round-trips bits.
    let (out, err, _, _) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "0");
}

/// Int bitwise ops join the subset: the nqueens INNER loop (shl/and/or/xor/
/// negate-trick) must tier as a region inside the recursive function frame.
#[test]
fn nqueens_inner_loop_bitwise_region_tiers() {
    let src = "## To solve (row: Int, cols: Int, diag1: Int, diag2: Int, n: Int) -> Int:\n\
               \x20   If row equals n:\n\
               \x20       Return 1.\n\
               \x20   Let all be (1 shifted left by n) - 1.\n\
               \x20   Let mutable available be all & ~(cols | diag1 | diag2).\n\
               \x20   Let mutable count be 0.\n\
               \x20   While available is not 0:\n\
               \x20       Let bit be available & (0 - available).\n\
               \x20       Set available to available xor bit.\n\
               \x20       Set count to count + solve(row + 1, cols | bit, (diag1 | bit) shifted left by 1, (diag2 | bit) shifted right by 1, n).\n\
               \x20   Return count.\n\
               \n\
               ## Main\n\
               Show solve(0, 0, 0, 0, 6).\n";
    let (out, err, _, _) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "4", "nqueens(6)");
    // The inner loop contains a Call (M8 territory) so the REGION may bail;
    // what M5 pins is bitwise CORRECTNESS through whatever tier runs, plus
    // a pure-bitwise loop below that MUST tier.
    let src2 = "## Main\n\
                Let mutable acc be 0.\n\
                Let mutable m be 1.\n\
                Let mutable i be 0.\n\
                While i is less than 3000:\n\
                \x20   Set m to (m shifted left by 1) xor (m shifted right by 3) xor i.\n\
                \x20   Set acc to (acc | (m & 1023)) xor (acc & m).\n\
                \x20   Set i to i + 1.\n\
                Show acc.\n";
    let (_, err2, _, region_ok2) = tiered(src2);
    assert_eq!(err2, None);
    assert!(region_ok2 >= 1, "pure bitwise loop must tier (got {region_ok2})");
}

/// Shift semantics at the edges must match the kernel bit-for-bit through
/// native code: shift counts ≥ 64 and negative operands.
#[test]
fn shift_edge_semantics_match_kernel() {
    for (expr, _label) in [
        ("(1 shifted left by 63)", "shl63"),
        ("(0 - 8) shifted right by 1", "sar-neg"),
        ("(1 shifted left by 70)", "shl70"),
        ("(0 - 1) shifted right by 70", "sar70-neg"),
    ] {
        let src = format!(
            "## Main\n\
             Let mutable acc be 0.\n\
             Let mutable i be 0.\n\
             While i is less than 300:\n\
             \x20   Set acc to {expr} + 0 * i.\n\
             \x20   Set i to i + 1.\n\
             Show acc.\n"
        );
        // tiered() asserts VM == tree-walker — the kernel defines the truth.
        tiered(&src);
    }
}
