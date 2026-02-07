//! E2E Tests for Zone System (Phase 8.5)
//!
//! These tests compile LOGOS to Rust and actually run the generated code,
//! verifying that zones work correctly at runtime.

mod common;
use common::{assert_exact_output, assert_output_lines};

// =============================================================================
// Basic Zone Functionality
// =============================================================================

#[test]
fn e2e_zone_basic_allocation() {
    let source = r#"## Main
Inside a zone called "Work":
    Let x be 42.
    Show x."#;
    assert_exact_output(source,"42");
}

#[test]
fn e2e_zone_with_size_mb() {
    let source = r#"## Main
Inside a zone called "Buffer" of size 1 MB:
    Let value be 100.
    Show value."#;
    assert_exact_output(source,"100");
}

#[test]
fn e2e_zone_with_size_kb() {
    let source = r#"## Main
Inside a zone called "Small" of size 64 KB:
    Let n be 7.
    Show n."#;
    assert_exact_output(source,"7");
}

// =============================================================================
// Zones in Functions
// =============================================================================

#[test]
fn e2e_zone_in_function_body() {
    let source = r#"## To process:
    Inside a zone called "Temp":
        Let result be 99.
        Show result.
    Return.

## Main
Call process."#;
    assert_exact_output(source,"99");
}

#[test]
fn e2e_function_with_zone_called_twice() {
    // Tests that function names can be verbs like "work"
    let source = r#"## To work:
    Inside a zone called "Local":
        Let x be 5.
        Show x.
    Return.

## Main
Call work.
Call work."#;
    assert_output_lines(source, &["5", "5"]);
}

// =============================================================================
// Nested Zones
// =============================================================================

#[test]
fn e2e_nested_zones() {
    let source = r#"## Main
Inside a zone called "Outer":
    Let a be 1.
    Inside a zone called "Inner":
        Let b be 2.
        Show b.
    Show a."#;
    assert_output_lines(source, &["2", "1"]);
}

// =============================================================================
// Zone with Control Flow
// =============================================================================

#[test]
fn e2e_zone_with_conditional() {
    let source = r#"## Main
Inside a zone called "Logic":
    Let x be 10.
    If x > 5:
        Show x."#;
    assert_exact_output(source,"10");
}

// =============================================================================
// Zone Scope Isolation
// =============================================================================

#[test]
fn e2e_zone_scope_isolation() {
    let source = r#"## Main
Let outer be 1.
Inside a zone called "Isolated":
    Let inner be 2.
Show outer."#;
    assert_exact_output(source,"1");
}

#[test]
fn e2e_multiple_sequential_zones() {
    let source = r#"## Main
Inside a zone called "First":
    Let a be 1.
Inside a zone called "Second":
    Let b be 2.
    Show b."#;
    assert_exact_output(source,"2");
}

#[test]
fn e2e_zone_default_capacity() {
    let source = r#"## Main
Inside a zone called "Default":
    Let x be 123.
    Show x."#;
    assert_exact_output(source,"123");
}
