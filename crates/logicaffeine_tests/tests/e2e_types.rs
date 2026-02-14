//! E2E Tests: Type Annotations
//!
//! Tests explicit type annotations and generic types.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_interpreter_output;

// === PRIMITIVE TYPE ANNOTATIONS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_let_with_int_type() {
    assert_interpreter_output(
        r#"## Main
Let x: Int be 42.
Show x.
"#,
        "42",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_let_with_text_type() {
    assert_interpreter_output(
        r#"## Main
Let s: Text be "hello".
Show s.
"#,
        "hello",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_let_with_bool_type() {
    assert_interpreter_output(
        r#"## Main
Let flag: Bool be true.
Show flag.
"#,
        "true",
    );
}

// === COLLECTION TYPE ANNOTATIONS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_let_with_seq_type() {
    assert_interpreter_output(
        r#"## Main
Let items: Seq of Int be [1, 2, 3].
Show items.
"#,
        "[1, 2, 3]",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_seq_of_text() {
    assert_interpreter_output(
        r#"## Main
Let words: Seq of Text be ["hello", "world"].
Show words.
"#,
        "[hello, world]",
    );
}

// === FUNCTION RETURN TYPES ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_function_typed_return() {
    assert_interpreter_output(
        r#"## To add (a: Int) and (b: Int) -> Int:
    Return a + b.

## Main
Show add(3, 4).
"#,
        "7",
    );
}

// === GENERIC CONSTRUCTORS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_new_seq_of_int() {
    assert_interpreter_output(
        r#"## Main
Let items be a new Seq of Int.
Push 10 to items.
Push 20 to items.
Show items.
"#,
        "[10, 20]",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_new_seq_of_text() {
    assert_interpreter_output(
        r#"## Main
Let words be a new Seq of Text.
Push "hello" to words.
Push "world" to words.
Show words.
"#,
        "[hello, world]",
    );
}
