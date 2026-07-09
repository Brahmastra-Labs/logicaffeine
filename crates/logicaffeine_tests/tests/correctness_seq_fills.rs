//! Part II: build collections in place — fills, repeat, and concatenation.
//!
//! The audit's Lists table, rows 1-2: every benchmark that needs `[0] * n`
//! writes a `new Seq` + `While`/`Push` fill loop (histogram, graph_bfs), and a
//! grid needs three flat 1-D seqs with manual `i*n+j` indexing (matrix_mult).
//! The spec adds:
//!
//!   - `xs * n` (and `n * xs`) — repeat a list; `[0] * 5` is five zeros.
//!   - `n copies of x` — the English spelling of the same fill.
//!   - `xs + ys` — concatenation, exactly `xs followed by ys`.
//!
//! Value-semantics guard: a fill of a LIST element deep-copies per slot —
//! `3 copies of (2 copies of 0)` is three INDEPENDENT rows, not three aliases
//! of one row (`diff_let_binding_isolates` is the binding-level analogue).

mod common;
use common::{assert_compiled_equals_interpreted_eq, assert_same_meaning};

// =====================================================================
// Repeat / fill
// =====================================================================

#[test]
fn star_repeat_fills_a_list() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show [0] * 5.
"#,
        "[0, 0, 0, 0, 0]",
    );
}

#[test]
fn star_repeat_preserves_element_order() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show [1, 2] * 3.
"#,
        "[1, 2, 1, 2, 1, 2]",
    );
}

#[test]
fn copies_of_is_the_english_fill() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let xs be 5 copies of 0.
Show xs.
"#,
        "[0, 0, 0, 0, 0]",
    );
}

#[test]
fn star_repeat_means_the_same_as_the_push_loop() {
    assert_same_meaning(
        r#"## Main
Let mutable xs be a new Seq of Int.
Let mutable i be 0.
While i is less than 4:
    Push 7 to xs.
    Set i to i + 1.
Show xs.
"#,
        r#"## Main
Let xs be [7] * 4.
Show xs.
"#,
    );
}

#[test]
fn nested_fill_rows_are_independent() {
    // THE value-semantics lock for fills: writing one row must not write
    // its siblings (each slot gets its own copy, not a shared handle).
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let mutable grid be 3 copies of (2 copies of 0).
Set item 1 of (item 2 of grid) to 9.
Show grid.
"#,
        "[[0, 0], [9, 0], [0, 0]]",
    );
}

// =====================================================================
// Concatenation
// =====================================================================

#[test]
fn plus_concatenates_lists() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show [1, 2] + [3, 4].
"#,
        "[1, 2, 3, 4]",
    );
}

#[test]
fn plus_means_exactly_followed_by() {
    assert_same_meaning(
        r#"## Main
Let a be [1, 2].
Let b be [3, 4].
Show a followed by b.
"#,
        r#"## Main
Let a be [1, 2].
Let b be [3, 4].
Show a + b.
"#,
    );
}

#[test]
fn concat_does_not_mutate_its_operands() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let a be [1, 2].
Let b be [3, 4].
Let c be a + b.
Show length of a.
Show length of b.
Show length of c.
"#,
        "2\n2\n4",
    );
}
