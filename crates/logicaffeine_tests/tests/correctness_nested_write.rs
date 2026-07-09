//! Task #10: nested-write through-write fast path. `Set item i of (item k of grid) to v`
//! writes the inner element in place via a value-semantic copy-on-write (cow the row
//! only if it is shared) instead of cloning the whole row. Correctness — including the
//! value-semantics ALIASING guarantee — must be identical to the interpreter.

mod common;
use common::assert_compiled_equals_interpreted_eq as eq;

#[test]
fn nested_write_basic() {
    eq(
        "## Main\nLet mutable grid be [[1, 2], [3, 4]].\nSet item 1 of (item 1 of grid) to 9.\nShow grid.\n",
        "[[9, 2], [3, 4]]",
    );
}

#[test]
fn nested_write_second_row() {
    eq(
        "## Main\nLet mutable grid be [[1, 2], [3, 4]].\nSet item 2 of (item 2 of grid) to 9.\nShow grid.\n",
        "[[1, 2], [3, 9]]",
    );
}

#[test]
fn nested_write_in_a_loop() {
    // Fill the diagonal.
    eq(
        "## Main\nLet mutable grid be [[0, 0, 0], [0, 0, 0], [0, 0, 0]].\nRepeat for i from 1 to 3:\n    Set item i of (item i of grid) to i.\nShow grid.\n",
        "[[1, 0, 0], [0, 2, 0], [0, 0, 3]]",
    );
}

#[test]
fn nested_write_preserves_value_semantics_for_an_alias() {
    // THE soundness gate: `row` is an independent value copy of grid's first row (value
    // semantics). Mutating grid's first row must NOT change `row` — the through-write must
    // cow the shared buffer before writing.
    eq(
        "## Main\nLet mutable grid be [[1, 2], [3, 4]].\nLet row be item 1 of grid.\nSet item 1 of (item 1 of grid) to 9.\nShow row.\nShow grid.\n",
        "[1, 2]\n[[9, 2], [3, 4]]",
    );
}

#[test]
fn nested_write_float_grid() {
    eq(
        "## Main\nLet mutable grid be [[1.5, 2.5], [3.5, 4.5]].\nSet item 2 of (item 1 of grid) to 9.5.\nShow grid.\n",
        "[[1.5, 9.5], [3.5, 4.5]]",
    );
}
