//! E2E Tests: Refinement Types
//!
//! Tests that refinement type constraints execute correctly at runtime.
//! Includes both valid value tests (should pass) and constraint violation tests (should panic).

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::{assert_exact_output, assert_panics};

// ============================================================================
// Part 1: Valid Values (should run successfully)
// ============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_refinement_valid_positive() {
    assert_exact_output(
        r#"## Main
Let x: Int where x > 0 be 5.
Set x to 10.
Show x.
"#,
        "10",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_refinement_compound_and_valid() {
    assert_exact_output(
        r#"## Main
Let x: Int where x > 0 and x < 100 be 50.
Set x to 75.
Show x.
"#,
        "75",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_refinement_boundary_value() {
    assert_exact_output(
        r#"## Main
Let x: Int where x >= 0 be 0.
Show x.
"#,
        "0",
    );
}

// ============================================================================
// Part 2: Constraint Violations (should panic)
// ============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_refinement_violation_at_let() {
    assert_panics(
        r#"## Main
Let x: Int where x > 0 be 0.
Show x.
"#,
        "assertion",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_refinement_violation_at_set() {
    assert_panics(
        r#"## Main
Let x: Int where x > 0 be 5.
Set x to 0.
Show x.
"#,
        "assertion",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_refinement_compound_and_violation() {
    assert_panics(
        r#"## Main
Let x: Int where x > 0 and x < 100 be 50.
Set x to 200.
Show x.
"#,
        "assertion",
    );
}
