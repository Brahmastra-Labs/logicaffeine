//! E2E Codegen Tests: Enums and Pattern Matching
//!
//! Mirrors e2e_enums.rs but compiles through the Rust codegen pipeline.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_exact_output;

// === UNIT VARIANTS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_enum_unit_variant() {
    assert_exact_output(
        r#"## A Color is one of:
    A Red.
    A Green.
    A Blue.

## Main
Let c be a new Red.
Inspect c:
    When Red: Show "red".
    When Green: Show "green".
    When Blue: Show "blue".
"#,
        "red",
    );
}

// === PAYLOAD VARIANTS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_enum_payload_variant() {
    assert_exact_output(
        r#"## A Shape is one of:
    A Circle with radius Int.
    A Rectangle with width Int and height Int.

## Main
Let s be a new Circle with radius 10.
Inspect s:
    When Circle (r): Show r.
    When Rectangle (w, h): Show w.
"#,
        "10",
    );
}

// === SIMPLE PATTERN MATCHING ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_inspect_simple() {
    assert_exact_output(
        r#"## A Color is one of:
    A Red.
    A Green.
    A Blue.

## Main
Let c be a new Green.
Inspect c:
    When Red: Show "red".
    When Green: Show "green".
    When Blue: Show "blue".
"#,
        "green",
    );
}

// === PATTERN MATCHING WITH BINDING ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_inspect_with_binding() {
    assert_exact_output(
        r#"## A Shape is one of:
    A Circle with radius Int.
    A Rectangle with width Int and height Int.

## Main
Let s be a new Circle with radius 42.
Inspect s:
    When Circle (r): Show r.
    When Rectangle (w, h): Show w.
"#,
        "42",
    );
}

// === OTHERWISE CLAUSE ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_inspect_otherwise() {
    assert_exact_output(
        r#"## A Color is one of:
    A Red.
    A Green.
    A Blue.

## Main
Let c be a new Blue.
Inspect c:
    When Red: Show "red".
    Otherwise: Show "other".
"#,
        "other",
    );
}

// === ENUM IN FUNCTION ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_enum_in_function() {
    assert_exact_output(
        r#"## A Color is one of:
    A Red.
    A Green.
    A Blue.

## To isRed (c: Color) -> Bool:
    Inspect c:
        When Red: Return true.
        Otherwise: Return false.

## Main
Let c be a new Red.
Show isRed(c).
"#,
        "true",
    );
}

// === ENUM RETURN ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_enum_return() {
    assert_exact_output(
        r#"## A Color is one of:
    A Red.
    A Green.
    A Blue.

## To primary -> Color:
    Return a new Red.

## Main
Let c be primary().
Inspect c:
    When Red: Show "red".
    Otherwise: Show "other".
"#,
        "red",
    );
}

// === NESTED INSPECT ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_nested_inspect() {
    assert_exact_output(
        r#"## A Color is one of:
    A Red.
    A Green.
    A Blue.

## Main
Let c1 be a new Red.
Let c2 be a new Green.
Inspect c1:
    When Red:
        Inspect c2:
            When Green: Show "red-green".
            Otherwise: Show "red-other".
    Otherwise: Show "not-red".
"#,
        "red-green",
    );
}

// === RECTANGLE PAYLOAD ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_enum_rectangle_area() {
    assert_exact_output(
        r#"## A Shape is one of:
    A Circle with radius Int.
    A Rectangle with width Int and height Int.

## To area (s: Shape) -> Int:
    Inspect s:
        When Circle (r): Return r * r.
        When Rectangle (w, h): Return w * h.

## Main
Let rect be a new Rectangle with width 5 and height 3.
Show area(rect).
"#,
        "15",
    );
}
