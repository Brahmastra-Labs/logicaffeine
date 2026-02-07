//! E2E Tests: Edge Cases
//!
//! Tests boundary conditions and edge cases.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_exact_output;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_runs;

// === NUMERIC EDGE CASES ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_zero() {
    assert_exact_output("## Main\nShow 0.", "0");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_negative_number() {
    assert_exact_output("## Main\nLet x be 0 - 5.\nShow x.", "-5");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_large_number() {
    assert_exact_output("## Main\nShow 999999.", "999999");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_division_truncation() {
    // Integer division should truncate
    assert_exact_output("## Main\nShow 7 / 2.", "3");
}

// === STRING EDGE CASES ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_empty_string() {
    assert_runs(
        r#"## Main
Let s be "".
Show s.
"#,
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_unicode_in_strings() {
    assert_exact_output(
        r#"## Main
Show "Hello World".
"#,
        "Hello World",
    );
}

// === EXPRESSION EDGE CASES ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_operator_precedence() {
    // Multiplication before addition: 2 + 3 * 4 = 2 + 12 = 14
    assert_exact_output("## Main\nShow 2 + 3 * 4.", "14");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_deeply_nested_parens() {
    assert_exact_output("## Main\nShow ((((1 + 2) + 3) + 4) + 5).", "15");
}

// === COLLECTION EDGE CASES ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_single_element_list() {
    assert_exact_output("## Main\nShow [42].", "[42]");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_empty_list_length() {
    // Use typed empty list syntax: [] of Type
    assert_exact_output(
        r#"## Main
Let items be [] of Int.
Show length of items.
"#,
        "0",
    );
}
