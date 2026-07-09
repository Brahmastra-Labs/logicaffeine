//! Regression net for the EXODIA JIT float-pin / mem-form-stencil bug.
//!
//! The forge keeps hot float VM-registers in caller-saved XMM registers across
//! the stencil chain (the "float cluster" keystone). The REGISTER-form float
//! ops (AddF/SubF/MulF) thread those pins correctly, but the MEM-form float
//! stencils — DivF, the float compares/branch, IntToFloat, SqrtF — use XMM
//! scratch and only spill/reload the slot THEY touch, clobbering the OTHER live
//! float pins. In a JIT region that mixed a mem-form op with a pinned float
//! accumulator, that clobber corrupted memory: `spectral_norm` at the tier-up
//! size SIGSEGV'd the instant its inner loop (`1.0/denom * v[j]`, carrying DivF
//! + IntToFloat next to the pinned `sum`) went native.
//!
//! Every program here runs hot enough to cross the region tier-up threshold
//! (100 back-edges) and carries a mem-form float op beside a pinned float, so
//! each one drove the crash before `select_pins` learned to keep floats
//! frame-resident in a region that contains a mem-form float op. They run on
//! BOTH the bytecode VM with the JIT installed AND the tree-walker oracle; the
//! outputs must match (a native miscompile would diverge — or, as it did,
//! crash the process). The `*_still_pins` cases use only register-form float
//! arithmetic and must stay correct too, proving the fix did not over-disable
//! the cluster.

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

/// Run `src` on the bytecode VM (with the forge JIT installed) AND the
/// tree-walker; assert identical output + no error + the expected value. A
/// native miscompile diverges here; the float-pin clobber crashed the process.
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

/// The exact `spectral_norm` shape that crashed: an inner loop whose body
/// inlines `1.0 / denom` (DivF + IntToFloat) and multiplies it by a pinned
/// array element into a pinned float accumulator, the result stored to a float
/// array — driven hot by an outer repeat so the region tiers up.
#[test]
fn float_div_times_arrayread_into_accumulator() {
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
         Let n be 8.\n\
         Let mutable u be a new Seq of Float.\n\
         Let mutable w be a new Seq of Float.\n\
         Let mutable k be 0.\n\
         While k is less than n:\n\
         \x20   Push 1.0 to u.\n\
         \x20   Push 0.0 to w.\n\
         \x20   Set k to k + 1.\n\
         Let mutable r be 0.\n\
         While r is less than 60:\n\
         \x20   Set w to mulAv(n, u, w).\n\
         \x20   Set r to r + 1.\n\
         Show \"{item 1 of w:.6}\".\n",
        "2.126204",
    );
}

/// DivF beside a pinned float accumulator: `s += 1.0 / i`.
#[test]
fn float_division_in_hot_loop() {
    assert_jit_matches(
        "## Main\n\
         Let mutable s be 0.0.\n\
         Let mutable i be 1.\n\
         While i is less than 5000:\n\
         \x20   Set s to s + 1.0 / i.\n\
         \x20   Set i to i + 1.\n\
         Show \"{s:.4}\".\n",
        "9.0943",
    );
}

/// SqrtF beside a pinned float accumulator: `s += sqrt(i)`.
#[test]
fn float_sqrt_in_hot_loop() {
    assert_jit_matches(
        "## Main\n\
         Let mutable s be 0.0.\n\
         Let mutable i be 1.\n\
         While i is less than 5000:\n\
         \x20   Set s to s + sqrt(i).\n\
         \x20   Set i to i + 1.\n\
         Show \"{s:.2}\".\n",
        "235666.70",
    );
}

/// A float comparison (mem-form BranchF) inside a loop carrying a pinned float
/// accumulator.
#[test]
fn float_compare_in_hot_loop() {
    assert_jit_matches(
        "## Main\n\
         Let mutable c be 0.\n\
         Let mutable x be 0.0.\n\
         Let mutable i be 0.\n\
         While i is less than 5000:\n\
         \x20   Set x to x + 0.5.\n\
         \x20   If x is greater than 1000.0:\n\
         \x20       Set c to c + 1.\n\
         \x20   Set i to i + 1.\n\
         Show c.\n",
        "3000",
    );
}

/// IntToFloat (mem-form) producing a value that feeds a pinned-float multiply.
#[test]
fn int_to_float_feeding_pinned_mul() {
    assert_jit_matches(
        "## Main\n\
         Let mutable s be 0.0.\n\
         Let mutable i be 0.\n\
         While i is less than 5000:\n\
         \x20   Set s to s + 1.5 * i.\n\
         \x20   Set i to i + 1.\n\
         Show \"{s:.1}\".\n",
        "18746250.0",
    );
}

/// Two pinned float accumulators alongside a DivF — the case where the clobber
/// of the SECOND pin (not the divisor's slot) was the corruption.
#[test]
fn two_float_pins_with_division() {
    assert_jit_matches(
        "## Main\n\
         Let mutable a be 0.0.\n\
         Let mutable b be 0.0.\n\
         Let mutable i be 1.\n\
         While i is less than 5000:\n\
         \x20   Set a to a + 1.0 / i.\n\
         \x20   Set b to b + 2.0 / i.\n\
         \x20   Set i to i + 1.\n\
         Show \"{a:.4} {b:.4}\".\n",
        "9.0943 18.1886",
    );
}

/// Register-form float arithmetic ONLY (`v[i]*v[i]` accumulated): no mem-form
/// op, so the float pins MUST still engage and stay correct — proves the fix
/// did not over-disable the cluster.
#[test]
fn pure_float_arith_still_pins_and_is_correct() {
    assert_jit_matches(
        "## Main\n\
         Let n be 30.\n\
         Let mutable v be a new Seq of Float.\n\
         Let mutable k be 0.\n\
         While k is less than n:\n\
         \x20   Push 2.0 to v.\n\
         \x20   Set k to k + 1.\n\
         Let mutable s be 0.0.\n\
         Let mutable r be 0.\n\
         While r is less than 5000:\n\
         \x20   Let mutable i be 0.\n\
         \x20   While i is less than n:\n\
         \x20       Set s to s + item (i + 1) of v * item (i + 1) of v.\n\
         \x20       Set i to i + 1.\n\
         \x20   Set r to r + 1.\n\
         Show \"{s:.1}\".\n",
        "600000.0",
    );
}

/// Two register-form float accumulators reading two arrays (the `spectral_norm`
/// final-norm loop) — register-form only, must keep pinning and stay correct.
#[test]
fn two_array_float_reductions_still_pin() {
    assert_jit_matches(
        "## Main\n\
         Let n be 30.\n\
         Let mutable u be a new Seq of Float.\n\
         Let mutable v be a new Seq of Float.\n\
         Let mutable k be 0.\n\
         While k is less than n:\n\
         \x20   Push 1.5 to u.\n\
         \x20   Push 2.5 to v.\n\
         \x20   Set k to k + 1.\n\
         Let mutable vbv be 0.0.\n\
         Let mutable vv be 0.0.\n\
         Let mutable r be 0.\n\
         While r is less than 5000:\n\
         \x20   Let mutable i be 0.\n\
         \x20   While i is less than n:\n\
         \x20       Set vbv to vbv + item (i + 1) of u * item (i + 1) of v.\n\
         \x20       Set vv to vv + item (i + 1) of v * item (i + 1) of v.\n\
         \x20       Set i to i + 1.\n\
         \x20   Set r to r + 1.\n\
         Show \"{vbv:.1}\".\n",
        "562500.0",
    );
}

/// A `Map of Int to Float` read (`MapGet`) beside a pinned float accumulator.
/// `MapGet` lowers to a HELPER CALL, which wipes xmm0–15 exactly like a normal
/// call — so the pinned `s` would be clobbered. The original blocklist named
/// `Call`/`CallSelf` but NOT `MapGet`, so it missed this; the whitelist (only
/// register-form arith + array/int/control ops are pin-safe) closes it.
#[test]
fn map_of_float_get_beside_pinned_accumulator() {
    assert_jit_matches(
        "## Main\n\
         Let mutable m be a new Map of Int to Float.\n\
         Let mutable k be 0.\n\
         While k is less than 30:\n\
         \x20   Set item k of m to k * 1.5.\n\
         \x20   Set k to k + 1.\n\
         Let mutable s be 0.0.\n\
         Let mutable r be 0.\n\
         While r is less than 5000:\n\
         \x20   Let mutable i be 0.\n\
         \x20   While i is less than 30:\n\
         \x20       Set s to s + item i of m.\n\
         \x20       Set i to i + 1.\n\
         \x20   Set r to r + 1.\n\
         Show \"{s:.1}\".\n",
        "3262500.0",
    );
}

/// The mandelbrot inner loop: loop-carried float accumulators `zr`/`zi`
/// reassigned every iteration via `Set zr to zr2` (a float `Move`) and tested by
/// a mem-form `BranchF` (`zr*zr + zi*zi > 4.0`). The `Move` now threads its XMM
/// pins (V_FMOV f→f) instead of round-tripping a stale frame cell, so `zr`/`zi`
/// stay register-resident across the chain; the escape count must still match
/// the tree-walker exactly. Before the fix the pinned-float `Move` corrupted the
/// carried accumulator (it copied a never-spilled frame slot).
#[test]
fn float_move_carried_accumulator_mandelbrot_shape() {
    assert_jit_matches(
        "## Main\n\
         Let mutable count be 0.\n\
         Let mutable y be 0.\n\
         While y is less than 40:\n\
         \x20   Let mutable x be 0.\n\
         \x20   While x is less than 40:\n\
         \x20       Let cr be 2.0 * x / 40 - 1.5.\n\
         \x20       Let ci be 2.0 * y / 40 - 1.0.\n\
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
         Show count.\n",
        "633",
    );
}

/// A float `Push` (`ArrPush`, which can call the allocator on a realloc) inside
/// a loop that also carries a pinned float accumulator. Another helper-call op
/// the blocklist missed and the whitelist blocks.
#[test]
fn float_push_beside_pinned_accumulator() {
    assert_jit_matches(
        "## Main\n\
         Let mutable total be 0.0.\n\
         Let mutable r be 0.\n\
         While r is less than 3000:\n\
         \x20   Let mutable v be a new Seq of Float.\n\
         \x20   Let mutable i be 0.\n\
         \x20   While i is less than 30:\n\
         \x20       Push i * 0.5 to v.\n\
         \x20       Set total to total + i * 0.5.\n\
         \x20       Set i to i + 1.\n\
         \x20   Set r to r + 1.\n\
         Show \"{total:.1}\".\n",
        "652500.0",
    );
}

// =====================================================================
// WS-G wave 11: the float cluster goes through the CONTIGUOUS regalloc
// backend. Each program below is bit-identical to the tree-walker AND its
// hot float region was register-allocated (`regalloc_region_count() >= 1`),
// not the per-piece stencil tier. These are the nbody / mandelbrot /
// spectral_norm / pi_leibniz shapes the float XMM register class unlocks.
// =====================================================================

fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("test thread panicked")
}

/// Assert `src` is bit-identical to the tree-walker AND its hot float loop
/// went through the contiguous regalloc backend.
fn assert_jit_regalloc(src: &str, expected: &str) {
    let src = src.to_string();
    let expected = expected.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "VM+JIT diverged from the tree-walker on:\n{src}"
        );
        assert_eq!(vm.error, None, "errored on:\n{src}");
        assert_eq!(norm(&vm.output), expected, "wrong output for:\n{src}");
        assert!(
            tier.regalloc_region_count() >= 1,
            "float hot loop must use the contiguous regalloc backend (got {}) for:\n{src}",
            tier.regalloc_region_count()
        );
    });
}

/// pi_leibniz: a pure float accumulate with an alternating sign and a float
/// divide — the simplest float-cluster shape, should regalloc. (Bit-identical
/// to the tree-walker is the gate; the printed digits just pin the value.)
#[test]
fn pi_leibniz_shape_regallocs() {
    assert_jit_regalloc(
        "## Main\n\
         Let mutable acc be 0.0.\n\
         Let mutable sign be 1.0.\n\
         Let mutable k be 0.\n\
         While k is less than 200000:\n\
         \x20   Set acc to acc + sign / (2.0 * k + 1.0).\n\
         \x20   Set sign to 0.0 - sign.\n\
         \x20   Set k to k + 1.\n\
         Let pi be 4.0 * acc.\n\
         Show \"{pi:.4}\".\n",
        "3.1416",
    );
}

/// mandelbrot escape iteration: `z = z*z + c` carried floats + a float
/// ordering escape test. The complex-multiply float chain regallocs.
#[test]
fn mandelbrot_step_regallocs() {
    assert_jit_regalloc(
        "## Main\n\
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
        "621",
    );
}

/// nbody / spectral_norm distance shape: `sqrt(dx*dx + dy*dy + dz*dz)` and a
/// reciprocal, accumulated in a hot loop. The divide+sqrt float chain
/// regallocs.
#[test]
fn nbody_distance_shape_regallocs() {
    assert_jit_regalloc(
        "## Main\n\
         Let mutable energy be 0.0.\n\
         Let mutable i be 1.\n\
         While i is less than 100000:\n\
         \x20   Let dx be 0.001 * i.\n\
         \x20   Let dy be 0.002 * i.\n\
         \x20   Let dz be 0.003 * i.\n\
         \x20   Let dist be sqrt(dx * dx + dy * dy + dz * dz).\n\
         \x20   Set energy to energy + 1.0 / dist.\n\
         \x20   Set i to i + 1.\n\
         Show \"{energy:.2}\".\n",
        "3231.22",
    );
}
