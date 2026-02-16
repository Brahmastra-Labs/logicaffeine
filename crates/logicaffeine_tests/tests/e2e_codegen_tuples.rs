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
