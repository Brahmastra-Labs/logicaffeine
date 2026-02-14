//! E2E Tests: Iteration
//!
//! Tests that Repeat loops work correctly.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_interpreter_output;

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_repeat_for_in_list() {
    assert_interpreter_output(
        r#"## Main
Let sum be 0.
Repeat for x in [1, 2, 3]:
    Set sum to sum + x.
Show sum.
"#,
        "6",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_repeat_for_range() {
    assert_interpreter_output(
        r#"## Main
Let sum be 0.
Repeat for i from 1 to 5:
    Set sum to sum + i.
Show sum.
"#,
        "15",
    );
}

// === NEW TESTS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_repeat_empty_list() {
    // Iterating over empty list should produce no iterations
    assert_interpreter_output(
        r#"## Main
Let sum be 0.
Repeat for x in a new Seq of Int:
    Set sum to sum + 1.
Show sum.
"#,
        "0",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_repeat_single_element() {
    assert_interpreter_output(
        r#"## Main
Let sum be 0.
Repeat for x in [42]:
    Set sum to sum + x.
Show sum.
"#,
        "42",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_repeat_modifies_external() {
    assert_interpreter_output(
        r#"## Main
Let count be 0.
Let items be [1, 2, 3, 4, 5].
Repeat for x in items:
    Set count to count + 1.
Show count.
"#,
        "5",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_repeat_nested_loops() {
    // 2 * 3 = 6 iterations total
    assert_interpreter_output(
        r#"## Main
Let count be 0.
Repeat for i from 1 to 2:
    Repeat for j from 1 to 3:
        Set count to count + 1.
Show count.
"#,
        "6",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_range_single_value() {
    // Range from 5 to 5 should iterate once
    assert_interpreter_output(
        r#"## Main
Let sum be 0.
Repeat for i from 5 to 5:
    Set sum to sum + i.
Show sum.
"#,
        "5",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_repeat_builds_list() {
    assert_interpreter_output(
        r#"## Main
Let result be a new Seq of Int.
Repeat for i from 1 to 3:
    Push i * 10 to result.
Show result.
"#,
        "[10, 20, 30]",
    );
}
