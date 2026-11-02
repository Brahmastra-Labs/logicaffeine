//! PIT-OF-SUCCESS LOCK — a `Seq of Real` compiles and runs IDENTICALLY on every tier.
//!
//! Regression guard for the AOT-Rust bug where a `Seq of Real` (scalarized to a float vector) had a
//! stale `variable_types` entry that made the copy-on-write emitter write `xs.cow()` on a type with
//! no `cow()` method → `error[E0599]: no method named `cow` found`. That blocked EVERY float-sequence
//! program (statistics, numeric pipelines) on the compiled tiers while the tree-walker/VM ran fine.
//!
//! The lock exercises the exact triggers — build a `Seq of Real`, MUTATE an element in place
//! (`Set item i of xs`), pass it to a stdlib function (`meanReal`), and read elements back — and
//! pins tree-walker ≡ VM ≡ AOT-Rust ≡ C. A tier that fails to compile OR diverges goes RED.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Build a `Seq of Real`, mutate an element in place, read it back — the `Set item of Seq of Real`
/// path that emitted the bad `cow()`.
const MUTATE: &str = "\
## Main
Let mutable xs be a new Seq of Real.
Push 1.5 to xs.
Push 2.5 to xs.
Push 3.5 to xs.
Set item 2 of xs to 10.25.
Show item 2 of xs.
";

/// A `Seq of Real` built in a loop, one element perturbed, then reduced by a stdlib function — the
/// statistics-benchmark shape verbatim.
const MEAN_LOOP: &str = "\
## Main
Let mutable xs be a new Seq of Real.
Let mutable k be 0.
While k is less than 8:
    Push toReal(k) + 0.5 to xs.
    Set k to k + 1.
Set item 3 of xs to 100.5.
Show trunc(meanReal(xs) * 100.0).
";

/// Pass a `Seq of Real` straight to a stdlib reducer (no mutation) — the plain function-argument path.
const SUM_FN: &str = "\
## Main
Let mutable xs be a new Seq of Real.
Push 1.5 to xs.
Push 2.25 to xs.
Push 3.75 to xs.
Show trunc(fsumReal(xs) * 100.0).
";

// --- tree-walker ≡ VM -------------------------------------------------------------------------

#[test]
fn seq_real_mutate_all_tiers() {
    assert_all_tiers(MUTATE, "10.25");
}
#[test]
fn seq_real_mean_loop_all_tiers() {
    // xs=[0.5,1.5,2.5,3.5,4.5,5.5,6.5,7.5], item 3 (1-indexed) → 100.5; sum 130, mean 16.25, ×100 = 1625.
    assert_all_tiers(MEAN_LOOP, "1625");
}
#[test]
fn seq_real_sum_fn_all_tiers() {
    assert_all_tiers(SUM_FN, "750");
}

// --- AOT-to-Rust ≡ interpreter (the tier that had the cow() bug) ------------------------------

#[test]
fn seq_real_mutate_compiled() {
    assert_compiled_equals_interpreted(MUTATE);
}
#[test]
fn seq_real_mean_loop_compiled() {
    assert_compiled_equals_interpreted(MEAN_LOOP);
}
#[test]
fn seq_real_sum_fn_compiled() {
    assert_compiled_equals_interpreted(SUM_FN);
}

// --- AOT-to-C --------------------------------------------------------------------------------

#[test]
fn seq_real_mutate_c() {
    assert_c_output(MUTATE, "10.25");
}
#[test]
fn seq_real_mean_loop_c() {
    assert_c_output(MEAN_LOOP, "1625");
}
