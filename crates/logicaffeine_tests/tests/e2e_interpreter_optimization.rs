//! E2E Interpreter Tests: Optimization Correctness
//!
//! Mirrors e2e_codegen_optimization.rs through the interpreter pipeline.
//! Verifies that these programs produce correct output regardless of backend.
//! TCO deep recursion (n=10000) is omitted — interpreter has stack limits.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_interpreter_output;

// =============================================================================
// TCO (Tail Call Optimization)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_opt_tco_factorial() {
    assert_interpreter_output(
        r#"## To factorial (n: Int) and (acc: Int) -> Int:
    If n is at most 1:
        Return acc.
    Return factorial(n - 1, acc * n).

## Main
Show factorial(5, 1).
"#,
        "120",
    );
}

// =============================================================================
// Constant Propagation
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_opt_const_prop_simple() {
    assert_interpreter_output(
        r#"## Main
Let x be 10.
Let y be x + 5.
Show y.
"#,
        "15",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_opt_const_prop_chain() {
    assert_interpreter_output(
        r#"## Main
Let a be 1.
Let b be a + 1.
Let c be b + 1.
Let d be c + 1.
Show d.
"#,
        "4",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_opt_const_prop_loop_safe() {
    assert_interpreter_output(
        r#"## Main
Let x be 0.
Repeat for i from 1 to 5:
    Set x to x + i.
Show x.
"#,
        "15",
    );
}

// =============================================================================
// Dead Code Elimination
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_opt_dce_unused_var() {
    assert_interpreter_output(
        r#"## Main
Let unused be 999.
Let result be 42.
Show result.
"#,
        "42",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_opt_dce_after_return() {
    assert_interpreter_output(
        r#"## To getValue -> Int:
    Return 42.

## Main
Show getValue().
"#,
        "42",
    );
}

// =============================================================================
// Vec Fill Pattern
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_opt_vec_fill_bool() {
    assert_interpreter_output(
        r#"## Main
Let flags be a new Seq of Bool.
Repeat for i from 1 to 5:
    Push true to flags.
Show length of flags.
"#,
        "5",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_opt_vec_fill_int() {
    assert_interpreter_output(
        r#"## Main
Let nums be a new Seq of Int.
Repeat for i from 1 to 10:
    Push 0 to nums.
Show length of nums.
"#,
        "10",
    );
}

// =============================================================================
// Swap Pattern
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_opt_swap_correct() {
    assert_interpreter_output(
        r#"## Main
Let items be [3, 1, 2].
Let temp be item 1 of items.
Set item 1 of items to item 2 of items.
Set item 2 of items to temp.
Show items.
"#,
        "[1, 3, 2]",
    );
}

// =============================================================================
// Constant Folding
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_opt_fold_expression() {
    assert_interpreter_output("## Main\nShow 2 + 3 * 4.", "14");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_opt_fold_subtraction() {
    assert_interpreter_output("## Main\nShow 10 - 3 - 2.", "5");
}

// =============================================================================
// Index Simplification
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_opt_index_simplification() {
    assert_interpreter_output(
        r#"## Main
Let items be [10, 20, 30].
Let i be 2.
Show item (i + 1 - 1) of items.
"#,
        "20",
    );
}

// =============================================================================
// WithCapacity Runtime
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_opt_with_capacity_runtime() {
    assert_interpreter_output(
        r#"## Main
Let items be a new Seq of Int.
Repeat for i from 1 to 100:
    Push i to items.
Show length of items.
"#,
        "100",
    );
}

// =============================================================================
// String Append in Loop
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_opt_string_append_large() {
    assert_interpreter_output(
        r#"## Main
Let s be "".
Repeat for i from 1 to 100:
    Set s to s + "x".
Show length of s.
"#,
        "100",
    );
}
