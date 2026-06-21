//! Regression pins for Bug Report #1 — closed-form recursion (BUG-027),
//! parallel-reduction lowering (BUG-028), Futamura parallel encoding (BUG-031).

#![cfg(not(target_arch = "wasm32"))]
mod common;
use common::{assert_exact_output, compile_to_rust};
use logicaffeine_compile::compile::encode_program_source;

/// BUG-027: the double-recursion closed form must use WRAPPING arithmetic with a
/// bit-width guard — matching the recurrence's wrapping doubling — not a raw
/// `(base << d) - k` that is UB / masks the shift for d >= 64 (`1 << 64` -> 1
/// instead of the recurrence's wrapping 0).
#[test]
fn closed_form_double_recursion_uses_wrapping_arithmetic() {
    // `equals` (Eq), not `is`, is numeric equality in LOGOS.
    let src = "## To pow2 (d: Int) -> Int:\n    If d equals 0:\n        Return 1.\n    Return pow2(d - 1) + pow2(d - 1).\n\n## Main\nShow pow2(3).\n";
    let rust = compile_to_rust(src).expect("Int double-recursion compiles");
    assert!(rust.contains("wrapping_mul"), "closed form must use wrapping_mul:\n{}", rust);
    assert!(rust.contains(">= 64"), "closed form must guard the shift at the i64 bit width:\n{}", rust);
}

/// BUG-027 value check (small d, so the COMPILED closed form runs; the
/// interpreter is not invoked, avoiding the exponential 2^d recursion).
#[test]
fn closed_form_double_recursion_small_value_correct() {
    assert_exact_output(
        "## To pow2 (d: Int) -> Int:\n    If d equals 0:\n        Return 1.\n    Return pow2(d - 1) + pow2(d - 1).\n\n## Main\nShow pow2(10).\n",
        "1024",
    );
}

/// BUG-028: a float Repeat-reduction must not be lowered with a hardcoded
/// `.sum::<i64>()` (which fails to type-check for an f64 accumulator).
#[test]
fn par_reduction_float_sum_not_pinned_to_i64() {
    let source = r#"## To total (nums: Seq of Float) -> Float:
    Let mutable acc be 0.0.
    Repeat for x in nums:
        Set acc to acc + x.
    Return acc.

## Main
Let mutable xs be a new Seq of Float.
Push 1.5 to xs.
Show total(xs).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("sum::<i64>()"),
        "float Repeat-sum was lowered with a hardcoded i64 reduction:\n{}",
        rust
    );
}

/// BUG-028 behavioral: the float reduction must compile AND run.
#[test]
fn par_reduction_float_sum_runs() {
    assert_exact_output(
        r#"## To total (nums: Seq of Float) -> Float:
    Let mutable acc be 0.0.
    Repeat for x in nums:
        Set acc to acc + x.
    Return acc.

## Main
Let mutable xs be a new Seq of Float.
Push 1.5 to xs.
Push 2.5 to xs.
Show total(xs).
"#,
        "4",
    );
}

/// BUG-031: the Futamura encoder must emit ONE inner branch per task of a
/// `Simultaneously` block, not collapse them into a single branch.
#[test]
fn encode_parallel_preserves_one_inner_branch_per_task() {
    let src = "## Main\nSimultaneously:\n    Show \"a\".\n    Show \"b\".\n    Show \"c\".\n";
    let encoded = encode_program_source(src).unwrap();
    assert!(
        encoded.contains("a new CParallel with branches"),
        "expected a CParallel in encoded output:\n{}",
        encoded
    );
    // Find the branches list (the Seq-of-Seq) and count how many per-task inner
    // branches are pushed into it: one per task (3), not a single collapsed one.
    let branches_var = encoded
        .lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("Let ")
                .and_then(|r| r.strip_suffix(" be a new Seq of Seq of CStmt."))
        })
        .expect("branches list var");
    let pushes = encoded.matches(&format!(" to {}.", branches_var)).count();
    assert_eq!(
        pushes, 3,
        "all three Simultaneously tasks must each get their own branch (expected 3 pushes into \
         the branches list, found {}):\n{}",
        pushes, encoded
    );
}
