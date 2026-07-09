//! Part II: any place expression is an l-value.
//!
//! Reads already compose (`item j of (item i of grid)` is an ordinary
//! expression) but writes demanded a bare identifier: the interpreter rejects
//! `Set item … of (expr)` with "SetIndex collection must be an identifier",
//! and the VM compiler only accepts an identifier or identifier-field target.
//! This is why matrix_mult flattens grids into three 1-D seqs with manual
//! `i*n+j` indexing. The spec: `Set`/`Push` targets are evaluated like any
//! other expression — collections are shared handles, so mutating through the
//! evaluated handle is exactly the aliasing model the engines already agree on
//! (`diff_cross_index_double_buffer_alias`).
//!
//! Also here: multi-push `Push a, b, c to xs.` — one statement per element is
//! the audit's graph_bfs complaint (5 consecutive Pushes to fill a row).

mod common;
use common::{assert_compiled_equals_interpreted_eq, assert_identical_lowering};

// =====================================================================
// Nested writes through place expressions
// =====================================================================

#[test]
fn set_through_nested_index_writes_the_inner_seq() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let mutable grid be [[1, 2], [3, 4]].
Set item 2 of (item 1 of grid) to 9.
Show item 2 of (item 1 of grid).
"#,
        "9",
    );
}

#[test]
fn nested_write_is_visible_in_the_whole_grid() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let mutable grid be [[1, 2], [3, 4]].
Set item 1 of (item 2 of grid) to 7.
Show grid.
"#,
        "[[1, 2], [7, 4]]",
    );
}

#[test]
fn push_through_place_expression_appends_to_inner_seq() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let mutable grid be [[1, 2]].
Push 5 to item 1 of grid.
Show item 1 of grid.
"#,
        "[1, 2, 5]",
    );
}

// =====================================================================
// Multi-push
// =====================================================================

#[test]
fn multi_push_appends_in_order() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let mutable xs be a new Seq of Int.
Push 1, 2, 3 to xs.
Show xs.
"#,
        "[1, 2, 3]",
    );
}

#[test]
fn multi_push_lowers_identically_to_consecutive_pushes() {
    assert_identical_lowering(
        r#"## Main
Let mutable xs be a new Seq of Int.
Push 1 to xs.
Push 2 to xs.
Push 3 to xs.
Show xs.
"#,
        r#"## Main
Let mutable xs be a new Seq of Int.
Push 1, 2, 3 to xs.
Show xs.
"#,
    );
}
