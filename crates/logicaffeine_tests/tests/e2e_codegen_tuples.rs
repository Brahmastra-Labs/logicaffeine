//! E2E Codegen Tests: Tuples
//!
//! Mirrors e2e_tuples.rs but compiles through the Rust codegen pipeline.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_exact_output;

// === TUPLE CREATION ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_tuple_creation() {
    assert_exact_output(
        r#"## Main
Let t be (1, 2, 3).
Show length of t.
"#,
        "3",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_tuple_mixed_types() {
    assert_exact_output(
        r#"## Main
Let t be (42, "hello", true).
Show length of t.
"#,
        "3",
    );
}

// === BRACKET ACCESS (1-indexed) ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_tuple_bracket_access_first() {
    assert_exact_output(
        r#"## Main
Let t be (10, 20, 30).
Show t[1].
"#,
        "10",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_tuple_bracket_access_middle() {
    assert_exact_output(
        r#"## Main
Let t be (10, 20, 30).
Show t[2].
"#,
        "20",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_tuple_bracket_access_last() {
    assert_exact_output(
        r#"## Main
Let t be (10, 20, 30).
Show t[3].
"#,
        "30",
    );
}

// === NATURAL LANGUAGE ACCESS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_tuple_item_of_first() {
    assert_exact_output(
        r#"## Main
Let t be (100, 200, 300).
Show item 1 of t.
"#,
        "100",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_tuple_item_of_middle() {
    assert_exact_output(
        r#"## Main
Let t be (100, 200, 300).
Show item 2 of t.
"#,
        "200",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_tuple_item_of_last() {
    assert_exact_output(
        r#"## Main
Let t be (100, 200, 300).
Show item 3 of t.
"#,
        "300",
    );
}

// === MIXED ACCESS IN EXPRESSIONS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_tuple_access_in_arithmetic() {
    assert_exact_output(
        r#"## Main
Let t be (5, 10, 15).
Let sum be t[1] + t[2] + t[3].
Show sum.
"#,
        "30",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_tuple_natural_access_in_arithmetic() {
    assert_exact_output(
        r#"## Main
Let t be (5, 10, 15).
Let sum be item 1 of t + item 2 of t + item 3 of t.
Show sum.
"#,
        "30",
    );
}

// === PAIR (2-TUPLE) ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_pair_creation() {
    assert_exact_output(
        r#"## Main
Let p be (42, "answer").
Show p[1].
"#,
        "42",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_pair_second_element() {
    assert_exact_output(
        r#"## Main
Let p be (42, "answer").
Show p[2].
"#,
        "answer",
    );
}

// === FLOAT SUPPORT ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_tuple_with_float() {
    assert_exact_output(
        r#"## Main
Let person be ("Bob", 30, 5.9).
Show item 3 of person.
"#,
        "5.9",
    );
}

// === A LOOP OVER TUPLES ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_tuple_loop_destructures_the_tuples_of_a_variable() {
    assert_exact_output(
        r#"## Main
Let pairs be [("left", 1), ("right", 2)].
Repeat for (side, n) in pairs:
    Show "{side} {n}".
"#,
        "left 1\nright 2",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_tuple_loop_destructures_the_tuples_of_a_list_written_in_place() {
    assert_exact_output(
        r#"## Main
Repeat for (side, n) in [("left", 1), ("right", 2)]:
    Show "{side} {n}".
"#,
        "left 1\nright 2",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_tuple_loop_destructures_the_tuples_of_an_item_of_a_list() {
    assert_exact_output(
        r#"## Main
Let groups be [[("left", 1)], [("right", 2), ("mid", 3)]].
Repeat for (side, n) in item 2 of groups:
    Show "{side} {n}".
"#,
        "right 2\nmid 3",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_tuple_loop_destructures_the_tuples_of_a_slice() {
    assert_exact_output(
        r#"## Main
Let pairs be [("left", 1), ("right", 2), ("mid", 3)].
Repeat for (side, n) in items 2 through 3 of pairs:
    Show "{side} {n}".
"#,
        "right 2\nmid 3",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_tuple_loop_destructures_the_tuples_of_a_call() {
    assert_exact_output(
        r#"## To pairs () -> Seq of (Text, Int):
    Return [("left", 1), ("right", 2)].

## Main
Repeat for (side, n) in pairs():
    Show "{side} {n}".
"#,
        "left 1\nright 2",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_tuple_loop_destructures_the_tuples_of_a_parameter() {
    assert_exact_output(
        r#"## To total (pairs: Seq of (Text, Int)) -> Int:
    Let mutable t be 0.
    Repeat for (side, n) in pairs:
        Set t to t + n.
    Return t.

## Main
Show total([("left", 1), ("right", 2)]).
"#,
        "3",
    );
}
