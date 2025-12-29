//! E2E Tests: Comparison Operators
//!
//! Tests all comparison operators at runtime:
//! - equals, is not (!=)
//! - is less than, is greater than
//! - is at least (>=), is at most (<=)
//! - Symbolic: <, >, <=, >=

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_output;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_runs;

// === EQUALS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_equals_true() {
    assert_output(
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
fn e2e_equals_false() {
    assert_output(
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
fn e2e_not_equals() {
    assert_output(
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
fn e2e_less_than_true() {
    assert_output(
        r#"## Main
If 3 is less than 5:
    Show "yes".
"#,
        "yes",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_less_than_false() {
    // When condition is false, nothing should be printed
    assert_runs(
        r#"## Main
If 5 is less than 3:
    Show "wrong".
"#,
    );
}

// === GREATER THAN ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_greater_than_true() {
    assert_output(
        r#"## Main
If 5 is greater than 3:
    Show "yes".
"#,
        "yes",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_greater_than_false() {
    assert_runs(
        r#"## Main
If 3 is greater than 5:
    Show "wrong".
"#,
    );
}

// === AT LEAST (>=) ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_at_least_boundary() {
    // Boundary case: 5 is at least 5 should be true
    assert_output(
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
fn e2e_at_most_boundary() {
    // Boundary case: 5 is at most 5 should be true
    assert_output(
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
fn e2e_symbolic_lt() {
    assert_output(
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
fn e2e_symbolic_gt() {
    assert_output(
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
fn e2e_symbolic_lteq_and_gteq() {
    assert_output(
        r#"## Main
Let x be 5.
If x <= 5 and x >= 5:
    Show "equal".
"#,
        "equal",
    );
}
