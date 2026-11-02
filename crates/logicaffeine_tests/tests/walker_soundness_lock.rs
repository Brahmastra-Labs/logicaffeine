//! ★ LOCK — an optimization is sound for statements its recognizer does not name. ★
//!
//! Each row is a program an optimizer once got wrong because a statement walker, meeting a statement kind it had no
//! case for, concluded "this does not touch the variable": a multi-push after an array's build, a swap, a reversible
//! block, an assignment inside a branch, foreign code. Every row runs on the tree-walker (the reference), the VM, the
//! optimized run path `largo run --interpret` executes, the Rust backend and the C backend, and must print the same
//! bytes. The census that finds such walkers is `statement_walker_census_lock`.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test walker_soundness_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::{assert_c_output, assert_exact_output};
use logicaffeine_compile::compile::{
    optimizations_fired, tw_outcome, vm_outcome, vm_outcome_tiered,
};
use logicaffeine_compile::optimization::Tier;

/// `src` prints exactly `expected` on the tree-walker, the VM, the optimized run path, and the Rust and C backends.
fn every_tier(src: &str, expected: &str) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker refused:\n{src}");
    assert_eq!(
        tw.output.trim_end(),
        expected,
        "the tree-walker's answer:\n{src}"
    );
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM refused:\n{src}");
    assert_eq!(vm.output.trim_end(), expected, "the VM's answer:\n{src}");
    let run = vm_outcome_tiered(src, &[], Tier::T3, None);
    assert_eq!(run.error, None, "the optimized run path refused:\n{src}");
    assert_eq!(
        run.output.trim_end(),
        expected,
        "the optimized run path's answer:\n{src}"
    );
    assert_exact_output(src, expected);
    assert_c_output(src, expected);
}

#[test]
fn an_affine_array_extended_by_a_multi_push_after_its_build_keeps_every_element() {
    every_tier(
        "## Main
Let mutable a be a new Seq of Int.
Let mutable i be 0.
While i is less than 4:
    Push i * 5 to a.
    Set i to i + 1.
Push 100, 200 to a.
Show item 2 of a.
Show item 6 of a.
Show length of a.
",
        "5\n200\n6",
    );
}

#[test]
fn an_affine_array_swapped_after_its_build_trades_values() {
    every_tier(
        "## Main
Let mutable a be a new Seq of Int.
Let mutable i be 0.
While i is less than 4:
    Push i * 5 to a.
    Set i to i + 1.
Let mutable b be a new Seq of Int.
Push 7 to b.
Swap a and b.
Show item 1 of a.
Show item 3 of b.
",
        "7\n10",
    );
}

#[test]
fn an_affine_build_loop_whose_counter_also_steps_inside_a_reversible_block_is_not_affine() {
    every_tier(
        "## Main
Let mutable a be a new Seq of Int.
Let mutable i be 0.
While i is less than 5:
    Push i to a.
    Set i to i + 1.
    Reversibly:
        Set i to i + 1.
Show item 2 of a.
Show length of a.
",
        "2\n3",
    );
}

#[test]
fn an_affine_build_loop_whose_counter_is_set_in_a_branch_first_starts_there() {
    every_tier(
        "## To build (n: Int) -> Int:
    Let mutable a be a new Seq of Int.
    Let mutable i be 0.
    If n is greater than 2:
        Set i to 3.
    While i is less than 6:
        Push i * 5 to a.
        Set i to i + 1.
    Return item 1 of a.

## Main
Let mutable k be 0.
Repeat for x from 1 to 3:
    Set k to k + x.
Show build(k).
Show build(1).
",
        "15\n0",
    );
}

/// The run path's own affine pass, which rewrites `## Main`'s top-level build loops: the counter set in a branch before
/// the loop is not known to start at 0.
#[test]
fn a_main_build_loop_whose_counter_is_set_in_a_branch_first_starts_there() {
    every_tier(
        "## Main
Let mutable k be 0.
Repeat for x from 1 to 3:
    Set k to k + x.
Let mutable a be a new Seq of Int.
Let mutable i be 0.
If k is greater than 2:
    Set i to 3.
While i is less than 6:
    Push i * 5 to a.
    Set i to i + 1.
Show item 1 of a.
Show item 3 of a.
",
        "15\n25",
    );
}

#[test]
fn an_affine_build_loop_whose_counter_also_steps_inside_a_branch_is_not_affine() {
    every_tier(
        "## Main
Let mutable a be a new Seq of Int.
Let mutable i be 0.
While i is less than 6:
    Push i * 5 to a.
    Set i to i + 1.
    If i equals 2:
        Set i to i + 1.
Show item 3 of a.
",
        "15",
    );
}

/// The N-Queens counting search the reflection-symmetry pass recognizes, and an entry that enumerates the first
/// row's columns — `{body}` is the rest of the entry loop.
fn queens(body: &str) -> String {
    format!(
        "## To solve (row: Int, cols: Int, d1: Int, d2: Int, n: Int) -> Int:
    If row equals n:
        Return 1.
    Let all be (1 shifted left by n) - 1.
    Let mutable available be all & ~(cols | d1 | d2).
    Let mutable count be 0.
    While available is not 0:
        Let bit be available & (0 - available).
        Set available to available xor bit.
        Set count to count + solve(row + 1, cols | bit, (d1 | bit) shifted left by 1, (d2 | bit) shifted right by 1, n).
    Return count.

## To total (n: Int) -> Int:
    Let mutable acc be 0.
    Let mutable col be 0.
    While col is less than n:
        Let bit be 1 shifted left by col.
{body}        Set acc to acc + solve(1, bit, bit shifted left by 1, bit shifted right by 1, n).
        Set col to col + 1.
    Return acc.

## Main
Show total(5).
Show total(6).
"
    )
}

#[test]
fn the_reflection_symmetry_of_a_queens_count_still_halves_the_search() {
    let src = queens("");
    every_tier(&src, "10\n4");
    assert!(
        optimizations_fired(&src).contains(&"symmetry"),
        "the plain entry loop is the recognized shape"
    );
}

#[test]
fn a_queens_entry_loop_that_also_shows_its_column_shows_every_column() {
    every_tier(
        &queens("        Show col.\n"),
        "0\n1\n2\n3\n4\n10\n0\n1\n2\n3\n4\n5\n4",
    );
}

#[test]
fn a_queens_search_that_shows_its_row_shows_it_for_every_column() {
    let src = queens("").replace(
        "    Let all be (1 shifted left by n) - 1.\n",
        "    Show row.\n    Let all be (1 shifted left by n) - 1.\n",
    );
    let reference = tw_outcome(&src);
    assert_eq!(reference.error, None, "the tree-walker refused:\n{src}");
    every_tier(&src, reference.output.trim_end());
}

/// A search that also counts its own calls in a program variable has an effect the mirror would halve: it runs whole.
#[test]
fn a_queens_search_that_counts_its_calls_in_a_program_variable_counts_them_all() {
    let src = queens("")
        .replace(
            "    If row equals n:\n        Return 1.\n",
            "    If row equals n:\n        Return 1.\n    Set visits to visits + 1.\n",
        )
        .replace("## Main\n", "## Main\nLet mutable visits be 0.\n")
        + "Show visits.\n";
    let reference = tw_outcome(&src);
    assert_eq!(reference.error, None, "the tree-walker refused:\n{src}");
    every_tier(&src, reference.output.trim_end());
}

/// The same count kept by a function the search calls: an effect one call away is still an effect.
#[test]
fn a_queens_search_whose_callee_counts_its_calls_counts_them_all() {
    let src = format!(
        "## To tick (by: Int) -> Int:
    Set visits to visits + by.
    Return 0.

{}",
        queens("")
            .replace(
                "    If row equals n:\n        Return 1.\n",
                "    If row equals n:\n        Return 1.\n    Let z be tick(1).\n",
            )
            .replace("## Main\n", "## Main\nLet mutable visits be 0.\n")
    ) + "Show visits.\n";
    let reference = tw_outcome(&src);
    assert_eq!(reference.error, None, "the tree-walker refused:\n{src}");
    every_tier(&src, reference.output.trim_end());
}

/// Recursion unrolled into its caller: an in-out (`mutable`) parameter is the caller's own variable at every level, so
/// what an unrolled level writes through it is what the next level and the caller read.
#[test]
fn an_unrolled_search_writes_through_its_in_out_parameter_at_every_level() {
    let src = "## To walk (seen: mutable Int, row: Int, n: Int) -> Int:
    If row equals n:
        Return 1.
    Set seen to seen + 1.
    Let mutable count be 0.
    Let mutable k be 0.
    While k is less than 3:
        Set count to count + walk(seen, row + 1, n).
        Set k to k + 1.
    Return count.

## Main
Let mutable seen be 0.
Show walk(seen, 0, 5).
Show seen.
";
    let reference = tw_outcome(src);
    assert_eq!(reference.error, None, "the tree-walker refused:\n{src}");
    every_tier(src, reference.output.trim_end());
}

#[test]
fn a_constant_swapped_with_a_parameter_in_a_branch_takes_the_parameter() {
    every_tier(
        "## To pick (p: Int, c: Bool) -> Int:
    Let mutable x be 5.
    If c:
        Swap x and p.
    Return x + 1.

## Main
Show pick(9, true).
Show pick(9, false).
",
        "10\n6",
    );
}

#[test]
fn a_constant_foreign_code_changes_in_a_branch_is_not_folded() {
    let src = "## To pick (c: Bool) -> Int:
    Let mutable x be 5.
    If c:
        Escape to Rust:
            x = 7;
    Let y be x + 1.
    Return y.

## Main
Show pick(true).
Show pick(false).
";
    assert_exact_output(src, "8\n6");
}
