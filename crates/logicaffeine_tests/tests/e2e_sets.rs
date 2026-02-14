//! E2E Tests: Set Collection Type
//!
//! Tests Set type at runtime.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_interpreter_output;

// === BASIC SET OPERATIONS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_set_creation_and_add() {
    assert_interpreter_output(
        r#"## Main
Let names be a new Set of Text.
Add "Alice" to names.
Add "Bob" to names.
Show length of names.
"#,
        "2",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_set_deduplication() {
    assert_interpreter_output(
        r#"## Main
Let s be a new Set of Int.
Add 1 to s.
Add 2 to s.
Add 1 to s.
Add 1 to s.
Show length of s.
"#,
        "2",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_set_remove() {
    assert_interpreter_output(
        r#"## Main
Let s be a new Set of Int.
Add 1 to s.
Add 2 to s.
Add 3 to s.
Remove 2 from s.
Show length of s.
"#,
        "2",
    );
}

// === CONTAINS CHECK ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_set_contains_natural() {
    assert_interpreter_output(
        r#"## Main
Let s be a new Set of Text.
Add "hello" to s.
If s contains "hello":
    Show "found".
"#,
        "found",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_set_contains_int() {
    assert_interpreter_output(
        r#"## Main
Let s be a new Set of Int.
Add 42 to s.
If s contains 42:
    Show "yes".
"#,
        "yes",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_set_not_contains() {
    assert_interpreter_output(
        r#"## Main
Let s be a new Set of Int.
Add 1 to s.
If s contains 99:
    Show "found".
Otherwise:
    Show "missing".
"#,
        "missing",
    );
}

// === SET ALGEBRA ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_set_union() {
    assert_interpreter_output(
        r#"## Main
Let a be a new Set of Int.
Let b be a new Set of Int.
Add 1 to a.
Add 2 to a.
Add 2 to b.
Add 3 to b.
Let c be a union b.
Show length of c.
"#,
        "3",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_set_intersection() {
    assert_interpreter_output(
        r#"## Main
Let a be a new Set of Int.
Let b be a new Set of Int.
Add 1 to a.
Add 2 to a.
Add 3 to a.
Add 2 to b.
Add 3 to b.
Add 4 to b.
Let c be a intersection b.
Show length of c.
"#,
        "2",
    );
}

// === SET ITERATION ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_set_iteration() {
    assert_interpreter_output(
        r#"## Main
Let s be a new Set of Int.
Add 10 to s.
Add 20 to s.
Let sum be 0.
Repeat for x in s:
    Set sum to sum + x.
Show sum.
"#,
        "30",
    );
}
