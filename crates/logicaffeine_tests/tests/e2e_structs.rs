//! E2E Tests: Structs
//!
//! Tests struct construction, field access, and mutation.
//! Note: These tests will reveal if struct features are fully implemented.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::{assert_interpreter_output, assert_interpreter_output_lines};

// === STRUCT CONSTRUCTOR ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_struct_constructor() {
    assert_interpreter_output(
        r#"## A Point has:
    An x: Int.
    A y: Int.

## Main
Let p be a new Point.
Show p's x.
"#,
        "0",
    );
}

// === FIELD ACCESS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_struct_field_access() {
    assert_interpreter_output(
        r#"## A Point has:
    An x: Int.
    A y: Int.

## Main
Let p be a new Point with x 10 and y 20.
Show p's x.
"#,
        "10",
    );
}

// === FIELD MUTATION ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_struct_field_mutation() {
    assert_interpreter_output(
        r#"## A Point has:
    An x: Int.
    A y: Int.

## Main
Let mutable p be a new Point with x 10 and y 20.
Set p's x to 100.
Show p's x.
"#,
        "100",
    );
}

// === STRUCT WITH INITIALIZATION ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_struct_with_init() {
    assert_interpreter_output_lines(
        r#"## A Point has:
    An x: Int.
    A y: Int.

## Main
Let p be a new Point with x 5 and y 10.
Show p's x.
Show p's y.
"#,
        &["5", "10"],
    );
}

// === STRUCT IN FUNCTION ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_struct_in_function() {
    assert_interpreter_output(
        r#"## A Point has:
    An x: Int.
    A y: Int.

## To getX (p: Point) -> Int:
    Return p's x.

## Main
Let p be a new Point with x 42 and y 0.
Show getX(p).
"#,
        "42",
    );
}

// === STRUCT RETURN ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_struct_return() {
    assert_interpreter_output(
        r#"## A Point has:
    An x: Int.
    A y: Int.

## To origin -> Point:
    Return a new Point with x 0 and y 0.

## Main
Let p be origin().
Show p's x.
"#,
        "0",
    );
}

// === NESTED FIELD ACCESS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_nested_field_access() {
    assert_interpreter_output(
        r#"## A Point has:
    An x: Int.
    A y: Int.

## A Box has:
    A location: Point.

## Main
Let p be a new Point with x 10 and y 20.
Let b be a new Box with location p.
Show b's location's x.
"#,
        "10",
    );
}

// === STRUCT IN COLLECTION ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_struct_in_collection() {
    assert_interpreter_output(
        r#"## A Point has:
    An x: Int.
    A y: Int.

## Main
Let points be a new Seq of Point.
Let p be a new Point with x 1 and y 2.
Push p to points.
Show length of points.
"#,
        "1",
    );
}
