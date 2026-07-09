//! Regression pins for Bug Report #1 — C backend codegen (BUG-010, BUG-029).

#![cfg(not(target_arch = "wasm32"))]
mod common;
use common::{assert_c_output, assert_interpreter_output};

/// BUG-010: integer `pow` must be exact, not lowered to floating-point `pow`.
/// 3^34 = 16677181699666569 fits in i64 but needs 54 bits, so it is NOT exactly
/// representable as f64.
#[test]
fn c_pow_large_integer_is_exact() {
    let src = "## Main\nShow pow(3, 34).\n";
    assert_interpreter_output(src, "16677181699666569");
    assert_c_output(src, "16677181699666569");
}

/// BUG-029: `min`/`max` must evaluate each argument exactly once. `tick` prints
/// its argument then returns it; each arg must print once.
#[test]
fn c_min_evaluates_each_argument_once() {
    assert_c_output(
        "## To tick (n: Int) -> Int:\n    Show n.\n    Return n.\n\n## Main\nLet r be min(tick(3), tick(9)).\nShow r.\n",
        "3\n9\n3",
    );
}

#[test]
fn c_max_evaluates_each_argument_once() {
    assert_c_output(
        "## To tick (n: Int) -> Int:\n    Show n.\n    Return n.\n\n## Main\nLet r be max(tick(3), tick(9)).\nShow r.\n",
        "3\n9\n9",
    );
}
