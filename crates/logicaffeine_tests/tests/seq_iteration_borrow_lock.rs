//! `Repeat for v in xs` over a local `Seq` the loop never writes reads the sequence where it is.
//!
//! Every tier iterates a snapshot of `xs`. For a body that never writes `xs`, the buffer itself is that snapshot:
//! under value semantics any other handle copies before it writes, so the loop borrows it —
//! `xs.borrow().iter().copied()` — instead of copying all of it first (`xs.to_vec()`), which was a full copy of a
//! 400 MB sequence per loop in benchmarks/programs/prefix_sum. A body that writes `xs` — directly, or by lending it to
//! a `mutable` parameter — still iterates a copy taken on entry, and a write to another handle of the same buffer
//! copies that handle and leaves the loop's buffer alone. Every program prints on the compiled tier what it prints on
//! the VM and the tree-walker; a borrow held while the buffer was written would panic instead.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test seq_iteration_borrow_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::{assert_all_tiers, assert_exact_output};
use logicaffeine_compile::compile::compile_to_rust;

const HEADER: &str = "## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int
";

/// A runtime-sized sequence (its length comes from a conditional push, so no pass fixes it as an array).
fn program(body: &str) -> String {
    format!(
        "{HEADER}
## Main
Let mutable xs be a new Seq of Int.
Let mutable i be 1.
While i is at most 20:
    If i % 3 is not 0:
        Push i * i to xs.
    Set i to i + 1.
{body}"
    )
}

fn loops(rust: &str) -> Vec<&str> {
    rust.lines()
        .map(str::trim)
        .filter(|l| l.starts_with("for v in xs"))
        .collect()
}

#[test]
fn a_read_only_loop_borrows_the_sequence() {
    let src = program("Let mutable h be 0.\nRepeat for v in xs:\n    Set h to (h * 31 + v) & 268435455.\nShow h.\n");
    let expected = common::run_interpreter(&src).output;
    assert_all_tiers(&src, expected.trim());
    assert_exact_output(&src, expected.trim());
    let rust = compile_to_rust(&src).expect("compiles");
    assert_eq!(
        loops(&rust),
        ["for v in xs.borrow().iter().copied() {"],
        "{rust}"
    );
}

#[test]
fn a_loop_that_pushes_to_the_sequence_iterates_a_copy() {
    let src = program("Repeat for v in xs:\n    If v is greater than 300:\n        Push v to xs.\nShow length of xs.\n");
    assert_all_tiers(&src, "16");
    assert_exact_output(&src, "16");
    let rust = compile_to_rust(&src).expect("compiles");
    assert_eq!(loops(&rust), ["for v in xs.to_vec() {"], "{rust}");
}

#[test]
fn a_loop_that_lends_the_sequence_to_a_mutable_parameter_iterates_a_copy() {
    let src = format!(
        "{HEADER}
## To bump (ys: mutable Seq of Int):
    Push 0 to ys.

## Main
Let mutable xs be [1, 2, 3].
Let mutable n be 0.
Repeat for v in xs:
    Set n to n + v.
    bump(xs).
Show n.
Show length of xs.
"
    );
    assert_all_tiers(&src, "6\n6");
    assert_exact_output(&src, "6\n6");
    let rust = compile_to_rust(&src).expect("compiles");
    assert!(
        !rust.contains("xs.borrow().iter()"),
        "a lent sequence is not borrowed across the loop:\n{rust}"
    );
}

#[test]
fn a_write_to_another_handle_leaves_the_borrowed_buffer_alone() {
    let src = program(
        "Let mutable ys be xs.\nLet mutable h be 0.\nRepeat for v in xs:\n    Push v to ys.\n    Set h to h + v.\nShow h.\nShow length of xs.\nShow length of ys.\n",
    );
    let expected = common::run_interpreter(&src).output;
    assert_all_tiers(&src, expected.trim());
    assert_exact_output(&src, expected.trim());
    let rust = compile_to_rust(&src).expect("compiles");
    assert_eq!(
        loops(&rust),
        ["for v in xs.borrow().iter().copied() {"],
        "{rust}"
    );
}

#[test]
fn a_text_sequence_is_read_element_by_element() {
    let src = format!(
        "{HEADER}
## Main
Let mutable words be a new Seq of Text.
Let mutable i be 1.
While i is at most 4:
    If i is not 2:
        Push \"w\" + i to words.
    Set i to i + 1.
Let mutable joined be \"\".
Repeat for v in words:
    Set joined to joined + v + \";\".
Show joined.
"
    );
    assert_all_tiers(&src, "w1;w3;w4;");
    assert_exact_output(&src, "w1;w3;w4;");
    let rust = compile_to_rust(&src).expect("compiles");
    assert!(
        rust.contains("for v in words.borrow().iter().cloned() {"),
        "{rust}"
    );
}

#[test]
fn no_borrow_turns_it_off() {
    let src = program("Let mutable h be 0.\nRepeat for v in xs:\n    Set h to (h * 31 + v) & 268435455.\nShow h.\n")
        .replacen("## Main", "## No borrow\n## Main", 1);
    let rust = compile_to_rust(&src).expect("compiles");
    assert!(
        !rust.contains("xs.borrow().iter()"),
        "`## No borrow` keeps the copy:\n{rust}"
    );
}
