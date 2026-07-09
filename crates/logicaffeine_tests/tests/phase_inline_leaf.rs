//! Statement-body leaf-function inlining (the gcd shape) on the RUN PATH.
//!
//! The optimizer folds a pure iterative helper into its call sites so the
//! calling loop tiers as one call-free region. These tests are the soundness
//! gate: every one runs the program through the OPTIMIZED interpret path
//! (`interpret_for_ui_sync`, which runs `optimize_for_run` — inlining included)
//! and asserts it matches the UNOPTIMIZED VM (`vm_outcome`, raw parse, no
//! optimizer). If inlining ever changes a result, the differential fails.

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::ui_bridge::interpret_for_ui_sync;

/// Optimized (inlined) output must equal the raw VM output, with no error.
fn assert_inlining_sound(src: &str) -> String {
    let raw = vm_outcome(src);
    assert_eq!(raw.error, None, "raw VM errored:\n{src}");
    let opt = interpret_for_ui_sync(src);
    assert_eq!(opt.error, None, "optimized (inlined) path errored:\n{src}");
    let opt_out = opt.lines.join("\n");
    let raw_out = raw.output.trim_end().to_string();
    assert_eq!(
        opt_out.trim_end(),
        raw_out,
        "inlined output diverged from the raw VM on:\n{src}"
    );
    opt_out
}

/// The gcd benchmark kernel: an iterative `gcd` called O(n²) times from a
/// nested loop. Inlining folds Euclid into the inner loop.
#[test]
fn gcd_kernel_inlines_and_matches_raw() {
    let src = "\
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
While i is at most 5:
    Let mutable j be i.
    While j is at most 5:
        Set sum to sum + gcd(i, j).
        Set j to j + 1.
    Set i to i + 1.
Show sum.
";
    // gcd sums to 26 over 1<=i<=j<=5.
    let out = assert_inlining_sound(src);
    assert_eq!(out.trim(), "26");
}

/// A helper that MUTATES its parameter copy (gcd does `Set x`) — alpha-renaming
/// must keep each inline instance's locals private; the caller's `x`/`y` are
/// untouched.
#[test]
fn param_mutating_helper_does_not_clobber_caller() {
    let src = "\
## To shrink (x: Int) -> Int:
    Let mutable v be x.
    While v is greater than 10:
        Set v to v - 10.
    Return v.

## Main
Let mutable v be 7.
Let mutable total be 0.
Let mutable i be 23.
While i is at most 25:
    Set total to total + shrink(i).
    Set i to i + 1.
Show \"\" + v + \" \" + total.
";
    // caller v stays 7; shrink(23)=3, shrink(24)=4, shrink(25)=5 → total 12.
    let out = assert_inlining_sound(src);
    assert_eq!(out.trim(), "7 12");
}

/// One loop-free helper inlined at several distinct call sites — each instance
/// gets its own fresh names, no cross-site capture.
#[test]
fn multi_site_inlining_is_independent() {
    let src = "\
## To sq (n: Int) -> Int:
    Let r be n * n.
    Return r.

## Main
Let a be sq(3).
Let b be sq(4).
Let c be sq(a).
Show \"\" + a + \" \" + b + \" \" + c.
";
    let out = assert_inlining_sound(src);
    assert_eq!(out.trim(), "9 16 81");
}

/// SHORT-CIRCUIT SAFETY: a candidate call in the right operand of `or` must NOT
/// be hoisted — otherwise it would always run. Here the right operand divides
/// by zero; with the LHS true, short-circuit must skip it. The optimized path
/// must behave exactly like the raw VM (both skip it).
#[test]
fn call_under_or_short_circuit_is_not_hoisted() {
    let src = "\
## To risky (d: Int) -> Int:
    Let mutable acc be 0.
    Let mutable k be 0.
    While k is less than 1:
        Set acc to 100 / d.
        Set k to k + 1.
    Return acc.

## Main
Let mutable guard be 5.
Let mutable hits be 0.
Let mutable i be 0.
While i is less than 3:
    If guard is greater than 0 or risky(0) is greater than 0:
        Set hits to hits + 1.
    Set i to i + 1.
Show hits.
";
    let out = assert_inlining_sound(src);
    assert_eq!(out.trim(), "3");
}

/// A call in an `If` condition (evaluated once on arrival) is a safe lift.
#[test]
fn call_in_if_condition_inlines_soundly() {
    let src = "\
## To dbl (n: Int) -> Int:
    Let mutable r be 0.
    Let mutable k be 0.
    While k is less than 2:
        Set r to r + n.
        Set k to k + 1.
    Return r.

## Main
Let mutable hits be 0.
Let mutable i be 1.
While i is at most 6:
    If dbl(i) is greater than 6:
        Set hits to hits + 1.
    Set i to i + 1.
Show hits.
";
    // dbl(i) = 2i > 6 for i in {4,5,6} → 3 hits.
    let out = assert_inlining_sound(src);
    assert_eq!(out.trim(), "3");
}

/// A non-candidate (recursive) helper must be left as a call — the program
/// still runs correctly through the optimized path.
#[test]
fn recursive_helper_left_as_call() {
    let src = "\
## To fact (n: Int) -> Int:
    If n is at most 1:
        Return 1.
    Return n * fact(n - 1).

## Main
Show fact(5).
";
    let out = assert_inlining_sound(src);
    assert_eq!(out.trim(), "120");
}
