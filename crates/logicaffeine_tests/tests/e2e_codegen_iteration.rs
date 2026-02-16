//! E2E Codegen Tests: Iteration
//!
//! Mirrors e2e_iteration.rs but compiles through the Rust codegen pipeline.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_exact_output;

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_repeat_for_in_list() {
    assert_exact_output(
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
fn e2e_codegen_repeat_for_range() {
    assert_exact_output(
        r#"## Main
Let sum be 0.
Repeat for i from 1 to 5:
    Set sum to sum + i.
Show sum.
"#,
        "15",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_repeat_empty_list() {
    assert_exact_output(
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
fn e2e_codegen_repeat_single_element() {
    assert_exact_output(
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
fn e2e_codegen_repeat_modifies_external() {
    assert_exact_output(
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
fn e2e_codegen_repeat_nested_loops() {
    assert_exact_output(
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
fn e2e_codegen_range_single_value() {
    assert_exact_output(
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
fn e2e_codegen_repeat_builds_list() {
    assert_exact_output(
        r#"## Main
Let result be a new Seq of Int.
Repeat for i from 1 to 3:
    Push i * 10 to result.
Show result.
"#,
        "[10, 20, 30]",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_for_in_with_accumulator() {
    assert_exact_output(
        r#"## Main
Let items be [10, 20, 30, 40].
Let total be 0.
Repeat for x in items:
    Set total to total + x.
Show total.
"#,
        "100",
    );
}
