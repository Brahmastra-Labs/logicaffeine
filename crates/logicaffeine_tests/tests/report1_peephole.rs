//! Regression pins for Bug Report #1 — peephole swap/drain optimizations
//! (BUG-020, BUG-021, BUG-022).

#![cfg(not(target_arch = "wasm32"))]
mod common;
use common::assert_exact_output;

/// BUG-020: the conditional-swap peephole consumes the `Let a`/`Let b` element
/// captures; if they are read after the swap, the optimizer must re-bind them to
/// the PRE-swap values (it previously dropped them, producing undefined-variable
/// Rust). a=items[0]=3, b=items[1]=1; a>b true => items become [1,3,2], but the
/// captured `a` must still be 3.
#[test]
fn e2e_opt_conditional_swap_keeps_captured_value() {
    assert_exact_output(
        r#"## Main
Let mutable items be [3, 1, 2].
Let a be item 1 of items.
Let b be item 2 of items.
If a is greater than b:
    Set item 1 of items to b.
    Set item 2 of items to a.
Show a.
"#,
        "3",
    );
}

/// BUG-021: when the guard is written `b OP a` (higher-index element first), the
/// emitted comparison must keep source operand order. a=arr[1]=1, b=arr[2]=3,
/// `b > a` = `3 > 1` = true => swap => [3, 1, 2].
#[test]
fn e2e_opt_conditional_swap_reversed_operands() {
    assert_exact_output(
        r#"## Main
Let mutable arr be [1, 3, 2].
Let j be 1.
Let a be item j of arr.
Let b be item (j + 1) of arr.
If b is greater than a:
    Set item j of arr to b.
    Set item (j + 1) of arr to a.
Show arr.
"#,
        "[3, 1, 2]",
    );
}

/// Companion: the canonical `a OP b` orientation must still be correct.
#[test]
fn e2e_opt_conditional_swap_forward_operands_still_correct() {
    assert_exact_output(
        r#"## Main
Let mutable arr be [3, 1, 2].
Let j be 1.
Let a be item j of arr.
Let b be item (j + 1) of arr.
If a is greater than b:
    Set item j of arr to b.
    Set item (j + 1) of arr to a.
Show arr.
"#,
        "[1, 3, 2]",
    );
}

/// BUG-022: the drain-tail peephole must copy only up to the loop bound, not to
/// the end of the source. bound (2) < length of src (4) => result = [10, 20].
#[test]
fn e2e_opt_drain_tail_respects_loop_bound() {
    assert_exact_output(
        r#"## Main
Let src be [10, 20, 30, 40].
Let mutable result be a new Seq of Int.
Let mutable i be 1.
While i is at most 2:
    If 1 is greater than 0:
        Push item i of src to result.
        Set i to i + 1.
    Otherwise:
        Set i to i + 1.
Show result.
"#,
        "[10, 20]",
    );
}
