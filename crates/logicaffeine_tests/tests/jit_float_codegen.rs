//! WS-D float codegen-quality gate (wave 5): the JIT float cluster — pinned-
//! accumulator FMA fusion (Lever 1a), register-form float ordering compares
//! (Lever 2a), and fusion-aware XMM pin selection (Lever 3).
//!
//! Every program here drives a hot float loop past the region tier-up threshold
//! (100 back-edges) and exercises a pattern the float levers reshape: a pinned
//! float accumulator next to a `MulF`-feeding-`AddF` chain (the FmaF case), a
//! distance-sum (`dx*dx + dy*dy + dz*dz`, nbody's hot kernel), a float `>`/`>=`
//! guard, and the spectral/mandelbrot reduction shapes. Each runs on BOTH the
//! bytecode VM with the forge JIT installed AND the tree-walker oracle; the
//! outputs MUST be bit-identical.
//!
//! THE SACRED CONSTRAINT: the `FmaF` micro-op is `(a*b)+c` with TWO SEPARATE
//! IEEE roundings — NOT a hardware fused-multiply-add — so it is bit-identical
//! to the `MulF; AddF` it replaces. Float ORDERING (`<,<=,>,>=`) is exact IEEE
//! and safe register-form; float EQUALITY is epsilon-fuzzy in the tree-walker
//! and is NOT touched here. Any reassociation or a real FMA would diverge on
//! the formatted `{:.N}` outputs below — that is exactly what these tests catch.

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

/// Run `src` on the bytecode VM (forge JIT installed) AND the tree-walker;
/// assert identical output + no error + the expected value. A float miscompile
/// (FMA, reassociation, a wrong spill/reload around a pinned accumulator)
/// diverges on the formatted output here.
fn assert_jit_matches(src: &str, expected: &str) {
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "VM+JIT diverged from the tree-walker on:\n{src}"
    );
    assert_eq!(vm.error, None, "errored on:\n{src}");
    assert_eq!(norm(&vm.output), expected, "wrong output for:\n{src}");
}

/// Run `src` and return (output, fn_tier_successes, region_tier_successes).
fn tiered(src: &str) -> (String, u32, u32) {
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "tiered VM diverged from the tree-walker on:\n{src}"
    );
    let (_, fn_ok) = tier.function_counts();
    let (_, region_ok) = tier.region_counts();
    (norm(&vm.output), fn_ok, region_ok)
}

/// LEVER 1a — the nbody distance-sum (`dx*dx + dy*dy + dz*dz`) accumulated into a
/// PINNED float `sum`, fed to `sqrt`. The middle product (`dy*dy`) already fuses
/// (its `dst` is an unpinned temp); the last product+add (`dz*dz + partial`)
/// writes the pinned distance-sum and previously refused to fuse because its
/// `dst` was pinned. The result drives `sqrt` and a division, then accumulates a
/// loop-carried float — bit-identical to the tree-walker, or every later digit
/// diverges.
#[test]
fn nbody_distance_sum_with_pinned_partial() {
    assert_jit_matches(
        "## Main\n\
         Let mutable bx be a new Seq of Float.\n\
         Let mutable by be a new Seq of Float.\n\
         Let mutable bz be a new Seq of Float.\n\
         Let mutable k be 0.\n\
         While k is less than 5:\n\
         \x20   Push k * 1.3 + 0.7 to bx.\n\
         \x20   Push k * 0.9 - 1.1 to by.\n\
         \x20   Push k * 2.1 + 0.3 to bz.\n\
         \x20   Set k to k + 1.\n\
         Let mutable e be 0.0.\n\
         Let mutable r be 0.\n\
         While r is less than 4000:\n\
         \x20   Let mutable i be 1.\n\
         \x20   While i is at most 5:\n\
         \x20       Let mutable j be i + 1.\n\
         \x20       While j is at most 5:\n\
         \x20           Let dx be item i of bx - item j of bx.\n\
         \x20           Let dy be item i of by - item j of by.\n\
         \x20           Let dz be item i of bz - item j of bz.\n\
         \x20           Let dist be sqrt(dx * dx + dy * dy + dz * dz).\n\
         \x20           Set e to e - 1.0 / dist.\n\
         \x20           Set j to j + 1.\n\
         \x20       Set i to i + 1.\n\
         \x20   Set r to r + 1.\n\
         Show \"{e:.9}\".\n",
        "-9764.060163659",
    );
}

/// LEVER 1a — a clean `partial = a*a; sum = partial + b*b` chain where `sum` is
/// the PINNED accumulator written by an FmaF whose `dst` is pinned but whose
/// product `b*b` and addend `partial` are frame-resident temps. The full nbody
/// inner loop with the velocity update (the actual `Set item i of bvx to ... -
/// dx * item j of bm * mag` FmaF stores) stays bit-exact.
#[test]
fn nbody_velocity_update_fma_stores() {
    assert_jit_matches(
        "## Main\n\
         Let mutable bx be a new Seq of Float.\n\
         Let mutable bvx be a new Seq of Float.\n\
         Let mutable bm be a new Seq of Float.\n\
         Let mutable k be 0.\n\
         While k is less than 5:\n\
         \x20   Push k * 1.3 + 0.7 to bx.\n\
         \x20   Push 0.0 to bvx.\n\
         \x20   Push k * 0.4 + 1.0 to bm.\n\
         \x20   Set k to k + 1.\n\
         Let dt be 0.01.\n\
         Let mutable step be 0.\n\
         While step is less than 3000:\n\
         \x20   Let mutable i be 1.\n\
         \x20   While i is at most 5:\n\
         \x20       Let mutable j be i + 1.\n\
         \x20       While j is at most 5:\n\
         \x20           Let dx be item i of bx - item j of bx.\n\
         \x20           Let dist be sqrt(dx * dx + 1.0).\n\
         \x20           Let mag be dt / (dist * dist * dist).\n\
         \x20           Set item i of bvx to item i of bvx - dx * item j of bm * mag.\n\
         \x20           Set item j of bvx to item j of bvx + dx * item i of bm * mag.\n\
         \x20           Set j to j + 1.\n\
         \x20       Set i to i + 1.\n\
         \x20   Set step to step + 1.\n\
         Show \"{item 1 of bvx:.9}\".\n",
        "25.546133372",
    );
}

/// LEVER 3 — a long `MulF`-feeding-`AddF` reduction whose intermediate partial
/// sums are SINGLE-USE float temps. With fusion-aware pin selection the temps
/// stay frame-resident so the products fuse into FmaFs (fewer pieces); the
/// result must stay bit-identical.
#[test]
fn float_dot_product_reduction_fuses() {
    assert_jit_matches(
        "## Main\n\
         Let mutable v be a new Seq of Float.\n\
         Let mutable k be 0.\n\
         While k is less than 8:\n\
         \x20   Push k * 0.5 + 0.25 to v.\n\
         \x20   Set k to k + 1.\n\
         Let mutable acc be 0.0.\n\
         Let mutable r be 0.\n\
         While r is less than 5000:\n\
         \x20   Let s be item 1 of v * item 1 of v + item 2 of v * item 2 of v + item 3 of v * item 3 of v + item 4 of v * item 4 of v.\n\
         \x20   Set acc to acc + s.\n\
         \x20   Set r to r + 1.\n\
         Show \"{acc:.6}\".\n",
        "26250.000000",
    );
}

/// LEVER 2a constraint check — a float `>` guard (ORDERING) inside a hot loop
/// carrying a pinned float accumulator. The ordering compare is exact IEEE; the
/// branch count must match the tree-walker exactly. (In nbody/mandelbrot shape
/// the compare fuses into a register-form BranchF; this asserts the guard never
/// changes the result.)
#[test]
fn float_ordering_guard_in_hot_loop() {
    assert_jit_matches(
        "## Main\n\
         Let mutable c be 0.\n\
         Let mutable x be 0.0.\n\
         Let mutable acc be 0.0.\n\
         Let mutable i be 0.\n\
         While i is less than 5000:\n\
         \x20   Set x to x + 0.3.\n\
         \x20   Set acc to acc + x * x.\n\
         \x20   If x is greater than 500.0:\n\
         \x20       Set c to c + 1.\n\
         \x20   If x is at least 750.0:\n\
         \x20       Set c to c + 1.\n\
         \x20   Set i to i + 1.\n\
         Show c.\n",
        "5834",
    );
}

/// LEVER 2a — a float ORDERING comparison whose result is STORED AS A VALUE
/// (not directly branched), so a `GtF`/`GtEqF` micro-op survives into the
/// region. This is the one shape the register-form ordering stencil targets:
/// the value-form float `>` must stay bit-identical and (after the lever) not
/// force the whole region unpinned.
#[test]
fn float_ordering_value_form_survives() {
    assert_jit_matches(
        "## Main\n\
         Let mutable trues be 0.\n\
         Let mutable s be 0.0.\n\
         Let mutable x be 0.0.\n\
         Let mutable i be 0.\n\
         While i is less than 5000:\n\
         \x20   Set x to x + 0.7.\n\
         \x20   Set s to s + x.\n\
         \x20   Let above be x is greater than 1000.0.\n\
         \x20   Let atLeast be x is at least 2000.0.\n\
         \x20   If above:\n\
         \x20       Set trues to trues + 1.\n\
         \x20   If atLeast:\n\
         \x20       Set trues to trues + 1.\n\
         \x20   Set i to i + 1.\n\
         Show trues.\n",
        "5715",
    );
}

/// LEVER 2a — the value-form float ordering region MUST still tier (with the
/// mem-form `GtF`/`GtEqF` lowering it is no longer forced unpinned). The output
/// is checked bit-identically by the differential helper; here we additionally
/// require the region to tier up, proving the pinnable path is exercised.
#[test]
fn float_ordering_value_form_region_tiers() {
    let src = "## Main\n\
               Let mutable trues be 0.\n\
               Let mutable s be 0.0.\n\
               Let mutable x be 0.0.\n\
               Let mutable i be 0.\n\
               While i is less than 5000:\n\
               \x20   Set x to x + 0.7.\n\
               \x20   Set s to s + x.\n\
               \x20   Let above be x is greater than 1000.0.\n\
               \x20   Let atLeast be x is at least 2000.0.\n\
               \x20   If above:\n\
               \x20       Set trues to trues + 1.\n\
               \x20   If atLeast:\n\
               \x20       Set trues to trues + 1.\n\
               \x20   Set i to i + 1.\n\
               Show trues.\n";
    let (out, _fn_ok, region_ok) = tiered(src);
    assert_eq!(out, "5715");
    assert!(
        region_ok >= 1,
        "the value-form float ordering loop must tier as a region (Lever 2a; got {region_ok})"
    );
}

/// spectral_norm shape: a function `mulAv` with an inner reduction over a float
/// array into a pinned `sum`, alongside an inlined `1.0/denom` (DivF + IntToFloat
/// mem-form ops). Driven hot so the region tiers; the formatted norm must be
/// bit-identical.
#[test]
fn spectral_norm_reduction_shape() {
    assert_jit_matches(
        "## To aVal (i: Int, j: Int) -> Float:\n\
         \x20   Return 1.0 / ((i + j) * (i + j + 1) / 2 + i + 1).\n\
         ## To mulAv (n: Int, v: Seq of Float, out: Seq of Float) -> Seq of Float:\n\
         \x20   Let mutable result be out.\n\
         \x20   Let mutable i be 0.\n\
         \x20   While i is less than n:\n\
         \x20       Let mutable sum be 0.0.\n\
         \x20       Let mutable j be 0.\n\
         \x20       While j is less than n:\n\
         \x20           Set sum to sum + aVal(i, j) * item (j + 1) of v.\n\
         \x20           Set j to j + 1.\n\
         \x20       Set item (i + 1) of result to sum.\n\
         \x20       Set i to i + 1.\n\
         \x20   Return result.\n\
         ## Main\n\
         Let n be 10.\n\
         Let mutable u be a new Seq of Float.\n\
         Let mutable w be a new Seq of Float.\n\
         Let mutable k be 0.\n\
         While k is less than n:\n\
         \x20   Push 1.0 to u.\n\
         \x20   Push 0.0 to w.\n\
         \x20   Set k to k + 1.\n\
         Let mutable r be 0.\n\
         While r is less than 80:\n\
         \x20   Set w to mulAv(n, u, w).\n\
         \x20   Set r to r + 1.\n\
         Show \"{item 1 of w:.6}\".\n",
        "2.174970",
    );
}

/// mandelbrot inner loop: loop-carried float accumulators `zr`/`zi` reassigned
/// each iteration, with the `zr*zr + zi*zi > 4.0` ORDERING escape test. The
/// pinned carried accumulators plus the FmaF chain in `zr*zr - zi*zi + cr` must
/// stay bit-identical (the escape count is integer, so any float drift flips it).
#[test]
fn mandelbrot_inner_loop_shape() {
    assert_jit_matches(
        "## Main\n\
         Let mutable count be 0.\n\
         Let mutable y be 0.\n\
         While y is less than 50:\n\
         \x20   Let mutable x be 0.\n\
         \x20   While x is less than 50:\n\
         \x20       Let cr be 2.0 * x / 50 - 1.5.\n\
         \x20       Let ci be 2.0 * y / 50 - 1.0.\n\
         \x20       Let mutable zr be 0.0.\n\
         \x20       Let mutable zi be 0.0.\n\
         \x20       Let mutable isInside be 1.\n\
         \x20       Let mutable iter be 0.\n\
         \x20       While iter is less than 100:\n\
         \x20           Let zr2 be zr * zr - zi * zi + cr.\n\
         \x20           Let zi2 be 2.0 * zr * zi + ci.\n\
         \x20           Set zr to zr2.\n\
         \x20           Set zi to zi2.\n\
         \x20           If zr * zr + zi * zi is greater than 4.0:\n\
         \x20               Set isInside to 0.\n\
         \x20               Set iter to 100.\n\
         \x20           Set iter to iter + 1.\n\
         \x20       If isInside equals 1:\n\
         \x20           Set count to count + 1.\n\
         \x20       Set x to x + 1.\n\
         \x20   Set y to y + 1.\n\
         Show count.\n",
        "962",
    );
}

/// The float region MUST still tier up after the levers (no over-disabling): the
/// distance-sum loop tiers as a region.
#[test]
fn nbody_distance_sum_still_tiers() {
    let src = "## Main\n\
               Let mutable bx be a new Seq of Float.\n\
               Let mutable by be a new Seq of Float.\n\
               Let mutable bz be a new Seq of Float.\n\
               Let mutable k be 0.\n\
               While k is less than 5:\n\
               \x20   Push k * 1.3 + 0.7 to bx.\n\
               \x20   Push k * 0.9 - 1.1 to by.\n\
               \x20   Push k * 2.1 + 0.3 to bz.\n\
               \x20   Set k to k + 1.\n\
               Let mutable e be 0.0.\n\
               Let mutable r be 0.\n\
               While r is less than 4000:\n\
               \x20   Let mutable i be 1.\n\
               \x20   While i is at most 5:\n\
               \x20       Let mutable j be i + 1.\n\
               \x20       While j is at most 5:\n\
               \x20           Let dx be item i of bx - item j of bx.\n\
               \x20           Let dy be item i of by - item j of by.\n\
               \x20           Let dz be item i of bz - item j of bz.\n\
               \x20           Let dist be sqrt(dx * dx + dy * dy + dz * dz).\n\
               \x20           Set e to e - 1.0 / dist.\n\
               \x20           Set j to j + 1.\n\
               \x20       Set i to i + 1.\n\
               \x20   Set r to r + 1.\n\
               Show \"{e:.9}\".\n";
    let (_out, _fn_ok, region_ok) = tiered(src);
    assert!(region_ok >= 1, "the nbody distance-sum loop must tier as a region (got {region_ok})");
}
