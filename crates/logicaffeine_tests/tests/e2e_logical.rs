//! E2E Tests: Logical Operators
//!
//! Tests logical operators (and, or) at runtime.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_exact_output;

// === AND OPERATOR ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_and_both_true() {
    assert_exact_output(
        r#"## Main
Let a be 3.
Let b be 4.
If a is less than 5 and b is less than 5:
    Show "both".
"#,
        "both",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_and_first_false() {
    // First condition false, should not print
    assert_exact_output(
        r#"## Main
Let a be 10.
Let b be 3.
If a is less than 5 and b is less than 5:
    Show "wrong".
"#,
        "",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_and_second_false() {
    // Second condition false, should not print
    assert_exact_output(
        r#"## Main
Let a be 3.
Let b be 10.
If a is less than 5 and b is less than 5:
    Show "wrong".
"#,
        "",
    );
}

// === OR OPERATOR ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_or_first_true() {
    assert_exact_output(
        r#"## Main
Let a be 3.
Let b be 10.
If a is less than 5 or b is less than 5:
    Show "one".
"#,
        "one",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_or_second_true() {
    assert_exact_output(
        r#"## Main
Let a be 10.
Let b be 3.
If a is less than 5 or b is less than 5:
    Show "one".
"#,
        "one",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_or_both_false() {
    // Both conditions false, should not print
    assert_exact_output(
        r#"## Main
Let a be 10.
Let b be 20.
If a is less than 5 or b is less than 5:
    Show "wrong".
"#,
        "",
    );
}

// === CHAINED CONDITIONS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_chained_and() {
    assert_exact_output(
        r#"## Main
Let a be 1.
Let b be 2.
Let c be 3.
If a is less than 5 and b is less than 5 and c is less than 5:
    Show "all".
"#,
        "all",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_chained_or() {
    // All false except we test that at least one true triggers it
    assert_exact_output(
        r#"## Main
Let a be 10.
Let b be 20.
Let c be 3.
If a is less than 5 or b is less than 5 or c is less than 5:
    Show "one".
"#,
        "one",
    );
}
