//! E2E Codegen Tests: Comparison Operators
//!
//! Mirrors e2e_comparisons.rs but compiles through the Rust codegen pipeline.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_exact_output;

// === EQUALS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_equals_true() {
    assert_exact_output(
        r#"## Main
Let x be 5.
If x equals 5:
    Show "yes".
"#,
        "yes",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_equals_false() {
    assert_exact_output(
        r#"## Main
Let x be 5.
If x equals 10:
    Show "wrong".
Otherwise:
    Show "no".
"#,
        "no",
    );
}

// === NOT EQUALS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_not_equals() {
    assert_exact_output(
        r#"## Main
Let x be 5.
If x is not 10:
    Show "different".
"#,
        "different",
    );
}

// === LESS THAN ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_less_than_true() {
    assert_exact_output(
        r#"## Main
If 3 is less than 5:
    Show "yes".
"#,
        "yes",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_less_than_false() {
    assert_exact_output(
        r#"## Main
If 5 is less than 3:
    Show "wrong".
"#,
        "",
    );
}

// === GREATER THAN ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_greater_than_true() {
    assert_exact_output(
        r#"## Main
If 5 is greater than 3:
    Show "yes".
"#,
        "yes",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_greater_than_false() {
    assert_exact_output(
        r#"## Main
If 3 is greater than 5:
    Show "wrong".
"#,
        "",
    );
}

// === AT LEAST (>=) ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_at_least_boundary() {
    assert_exact_output(
        r#"## Main
If 5 is at least 5:
    Show "yes".
"#,
        "yes",
    );
}

// === AT MOST (<=) ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_at_most_boundary() {
    assert_exact_output(
        r#"## Main
If 5 is at most 5:
    Show "yes".
"#,
        "yes",
    );
}

// === SYMBOLIC OPERATORS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_symbolic_lt() {
    assert_exact_output(
        r#"## Main
Let x be 3.
If x < 5:
    Show "less".
"#,
        "less",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_symbolic_gt() {
    assert_exact_output(
        r#"## Main
Let x be 10.
If x > 5:
    Show "greater".
"#,
        "greater",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_symbolic_lteq_and_gteq() {
    assert_exact_output(
        r#"## Main
Let x be 5.
If x <= 5 and x >= 5:
    Show "equal".
"#,
        "equal",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_is_equal_to() {
    assert_exact_output(
        r#"## Main
Let x be 7.
If x is equal to 7:
    Show "match".
"#,
        "match",
    );
}
