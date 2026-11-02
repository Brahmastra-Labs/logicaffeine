//! A `Seq` parameter the function writes element by element, copied into a new `Seq` by a push loop.
//!
//! A function that only sets items of its `Seq` parameter and returns it takes the parameter as `&mut [T]`, and a
//! push loop copying a run of it into a fresh `Seq` lowers to one slice copy. That copy must index the slice directly:
//! reading it through `.borrow()`, the `LogosSeq` form, is Rust that does not compile (E0599), which is what
//! benchmarks/programs/mergesort — shaped like its C twin, copying both halves and merging back into the parameter —
//! hit through Rust. A copy of the whole run, a copy of part of it, and a copy that reads past the end all behave the
//! same on the compiled tier as on the VM and the tree-walker; the last fails with the loop's own out-of-bounds error.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test mut_borrowed_seq_copy_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::{assert_all_tiers, assert_exact_output, run_all_tiers, run_logos};

/// `split` copies items `1..=upto` of its parameter into a new `Seq`, then writes their sum back into the
/// parameter's first item and returns it — the element-only writes that make the parameter `&mut [Int]`.
fn program(upto: &str, xs: &str) -> String {
    format!(
        "## To split (arr: Seq of Int) -> Seq of Int:
    Let mutable part be a new Seq of Int.
    Let mutable i be 1.
    While i is at most {upto}:
        Push item i of arr to part.
        Set i to i + 1.
    Let mutable total be 0.
    Repeat for v in part:
        Set total to total + v.
    Let mutable result be arr.
    Set item 1 of result to total.
    Return result.

## Main
Let mutable xs be {xs}.
Set xs to split(xs).
Show xs.
"
    )
}

#[test]
fn a_whole_copy_of_a_mutably_borrowed_seq() {
    let src = program("length of arr", "[3, 1, 4, 1, 5]");
    assert_all_tiers(&src, "[14, 1, 4, 1, 5]");
    assert_exact_output(&src, "[14, 1, 4, 1, 5]");
}

#[test]
fn a_partial_copy_of_a_mutably_borrowed_seq() {
    let src = program("(length of arr) / 2", "[3, 1, 4, 1, 5, 9]");
    assert_all_tiers(&src, "[8, 1, 4, 1, 5, 9]");
    assert_exact_output(&src, "[8, 1, 4, 1, 5, 9]");
}

#[test]
fn the_benchmark_mergesort_sorts_through_every_tier() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../benchmarks/programs/mergesort/main.lg"
    ))
    .expect("mergesort/main.lg")
    .replace(
        "Let arguments be args().\nLet n be parseInt(item 2 of arguments).",
        "Let n be 1000.",
    );
    let expected = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../benchmarks/programs/mergesort/expected_1000.txt"
    ))
    .expect("expected_1000.txt");
    assert_all_tiers(&src, expected.trim());
    assert_exact_output(&src, expected.trim());
}

#[test]
fn a_copy_past_the_end_fails_with_the_loops_own_error() {
    let src = program("(length of arr) + 1", "[3, 1, 4]");
    let agreed = run_all_tiers(&src);
    let error = agreed
        .error
        .expect("reading item 4 of a 3-item Seq is an error on the tree-walker and the VM");
    assert!(
        error.contains("out of bounds") && error.contains('4'),
        "the loop's own out-of-bounds error: {error}"
    );
    let compiled = run_logos(&src);
    assert!(
        !compiled.success,
        "the compiled program must fail too:\n{}",
        compiled.rust_code
    );
    assert!(
        compiled.stderr.contains("out of bounds") && compiled.stderr.contains('4'),
        "the compiled program fails with the same error the VM reports ({error}): {}",
        compiled.stderr
    );
}
