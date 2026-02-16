//! E2E Codegen Tests: Edge Cases
//!
//! Mirrors e2e_edge_cases.rs but compiles through the Rust codegen pipeline.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_exact_output;

// === NUMERIC EDGE CASES ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_zero() {
    assert_exact_output("## Main\nShow 0.", "0");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_negative_number() {
    assert_exact_output("## Main\nLet x be 0 - 5.\nShow x.", "-5");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_large_number() {
    assert_exact_output("## Main\nShow 999999.", "999999");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_division_truncation() {
    assert_exact_output("## Main\nShow 7 / 2.", "3");
}

// === STRING EDGE CASES ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_empty_string() {
    assert_exact_output(
        r#"## Main
Let s be "".
Show s.
"#,
        "",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_unicode_in_strings() {
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
fn e2e_codegen_operator_precedence() {
    assert_exact_output("## Main\nShow 2 + 3 * 4.", "14");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_deeply_nested_parens() {
    assert_exact_output("## Main\nShow ((((1 + 2) + 3) + 4) + 5).", "15");
}

// === COLLECTION EDGE CASES ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_single_element_list() {
    assert_exact_output("## Main\nShow [42].", "[42]");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_empty_list_length() {
    assert_exact_output(
        r#"## Main
Let items be [] of Int.
Show length of items.
"#,
        "0",
    );
}

// === VARIABLE SHADOWING ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_variable_reuse_in_loop() {
    assert_exact_output(
        r#"## Main
Let total be 0.
Repeat for i from 1 to 3:
    Let temp be i * 10.
    Set total to total + temp.
Show total.
"#,
        "60",
    );
}
