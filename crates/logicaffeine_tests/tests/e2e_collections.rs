//! E2E Tests: Collections
//!
//! Tests that collection operations (Push, Pop, length, index, slice, copy)
//! work correctly at runtime.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_exact_output;

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_list_literal() {
    assert_exact_output("## Main\nLet items be [1, 2, 3].\nShow items.", "[1, 2, 3]");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_push_to_list() {
    assert_exact_output(
        r#"## Main
Let items be [1, 2].
Push 3 to items.
Show items.
"#,
        "[1, 2, 3]",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_pop_from_list() {
    assert_exact_output(
        r#"## Main
Let items be [1, 2, 3].
Pop from items into last.
Show last.
"#,
        "3",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_length_of_list() {
    assert_exact_output(
        r#"## Main
Let items be [1, 2, 3, 4, 5].
Let n be length of items.
Show n.
"#,
        "5",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_index_1based() {
    assert_exact_output(
        r#"## Main
Let items be [10, 20, 30].
Let first be item 1 of items.
Show first.
"#,
        "10",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_slice_inclusive() {
    assert_exact_output(
        r#"## Main
Let items be [1, 2, 3, 4, 5].
Let middle be items 2 through 4.
Show middle.
"#,
        "[2, 3, 4]",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_copy_creates_independent() {
    assert_exact_output(
        r#"## Main
Let original be [1, 2, 3].
Let cloned be copy of original.
Push 4 to original.
Show cloned.
"#,
        "[1, 2, 3]",
    );
}

// === NEW TESTS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_empty_list() {
    assert_exact_output(
        r#"## Main
Let items be a new Seq of Int.
Show length of items.
"#,
        "0",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_push_to_empty() {
    assert_exact_output(
        r#"## Main
Let items be a new Seq of Int.
Push 42 to items.
Show items.
"#,
        "[42]",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_pop_after_push() {
    assert_exact_output(
        r#"## Main
Let items be [1, 2, 3].
Pop from items into x.
Pop from items into y.
Show x + y.
"#,
        "5",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_index_last_element() {
    assert_exact_output(
        r#"## Main
Let items be [10, 20, 30].
Show item 3 of items.
"#,
        "30",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_index_with_variable() {
    assert_exact_output(
        r#"## Main
Let items be [100, 200, 300].
Let i be 2.
Show item i of items.
"#,
        "200",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_slice_full_list() {
    assert_exact_output(
        r#"## Main
Let items be [1, 2, 3, 4, 5].
Let result be items 1 through 5.
Show result.
"#,
        "[1, 2, 3, 4, 5]",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_slice_single_element() {
    assert_exact_output(
        r#"## Main
Let items be [1, 2, 3, 4, 5].
Let single be items 3 through 3.
Show single.
"#,
        "[3]",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_length_after_push() {
    assert_exact_output(
        r#"## Main
Let items be [1, 2].
Push 3 to items.
Push 4 to items.
Show length of items.
"#,
        "4",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_length_after_pop() {
    assert_exact_output(
        r#"## Main
Let items be [1, 2, 3, 4, 5].
Pop from items into x.
Pop from items into y.
Show length of items.
"#,
        "3",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_set_item_of_list() {
    assert_exact_output(
        r#"## Main
Let items be [1, 2, 3].
Set item 2 of items to 99.
Show items.
"#,
        "[1, 99, 3]",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_list_in_function() {
    assert_exact_output(
        r#"## To sumList (items: Seq of Int) -> Int:
    Let sum be 0.
    Repeat for x in items:
        Set sum to sum + x.
    Return sum.

## Main
Show sumList([10, 20, 30]).
"#,
        "60",
    );
}
