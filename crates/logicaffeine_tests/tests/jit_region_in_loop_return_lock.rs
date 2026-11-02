//! A loop with a `Return` inside it, compiled by the JIT into a native region, leaves the region one
//! of two ways: through the `Return`, which raises the region's return flag and stores the value, or
//! by finishing the loop, after which the function's own code runs on. The flag's write sits inside
//! the loop's span but only on the leaving path, and the VM reads the flag after the region exits, so
//! a run that finishes the loop must exit with the flag down whatever any earlier run did.
//!
//! Each program calls its function enough times that the region compiles part-way through, then
//! alternates calls that return from inside the loop with calls that finish it. Found through the
//! supertask engine's search residual (`If proved: Return v. Otherwise: Return provisional(v, …)`).

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome, vm_outcome_with_args};
use logicaffeine_jit::ForgeTier;

fn assert_jit_matches_the_engines(source: &str) {
    let tier = ForgeTier::new();
    let jit = vm_outcome_with_args(source, &[], Some(&tier as &dyn logicaffeine_compile::vm::NativeTier));
    let vm = vm_outcome_with_args(source, &[], None);
    let tw = tw_outcome(source);
    assert_eq!(
        (vm.output.trim(), vm.error.as_deref()),
        (tw.output.trim(), tw.error.as_deref()),
        "the VM and the tree-walker disagree on:\n{source}"
    );
    assert_eq!(
        (jit.output.trim(), jit.error.as_deref()),
        (vm.output.trim(), vm.error.as_deref()),
        "the JIT disagrees with the VM on:\n{source}"
    );
}

/// A search that sets its flag and returns from inside the loop, then branches on the flag after it.
const FLAG_THEN_BRANCH: &str = "## To find (m: Int) -> Bool:
    Let mutable found be false.
    Repeat for n from 0 to 99:
        If n * n = m:
            Set found to true.
            Return true.
    If found:
        Return false.
    Return true.

";

#[test]
fn a_flag_set_on_the_way_out_of_one_call_is_not_the_next_call_s_flag() {
    assert_jit_matches_the_engines(&format!(
        "{FLAG_THEN_BRANCH}## Main\nShow find(49).\nShow find(50).\nShow find(50).\n"
    ));
}

#[test]
fn the_answer_does_not_depend_on_which_call_came_first() {
    assert_jit_matches_the_engines(&format!(
        "{FLAG_THEN_BRANCH}## Main\nShow find(50).\nShow find(49).\nShow find(50).\nShow find(64).\nShow find(65).\n"
    ));
}

#[test]
fn an_integer_flag_behaves_as_a_boolean_one_does() {
    assert_jit_matches_the_engines(
        "## To find (m: Int) -> Int:
    Let mutable hits be 0.
    Repeat for n from 0 to 99:
        If n * n = m:
            Set hits to hits + 1.
            Return n.
    If hits > 0:
        Return 0 - 1.
    Return 0 - 2.

## Main
Show find(49).
Show find(50).
Show find(81).
Show find(82).
",
    );
}

#[test]
fn a_counter_that_survives_the_loop_starts_each_call_afresh() {
    assert_jit_matches_the_engines(
        "## To scan (m: Int) -> Int:
    Let mutable seen be 0.
    Repeat for n from 1 to 200:
        Set seen to seen + 1.
        If n = m:
            Return seen.
    If seen = 200:
        Return 1000.
    Return seen.

## Main
Show scan(10).
Show scan(500).
Show scan(150).
Show scan(500).
",
    );
}

#[test]
fn a_return_from_an_inner_loop_leaves_the_outer_search_s_answer_alone() {
    assert_jit_matches_the_engines(
        "## To first (m: Int) -> Int:
    Repeat for i from 1 to 30:
        Repeat for j from 1 to 30:
            If i * j = m:
                Return i * 100 + j.
    Return 0.

## Main
Show first(12).
Show first(997).
Show first(36).
Show first(1000).
Show first(899).
Show first(1000).
",
    );
}

#[test]
fn a_while_loop_that_runs_out_answers_from_after_the_loop() {
    assert_jit_matches_the_engines(
        "## To seek (m: Int) -> Int:
    Let mutable n be 0.
    While n < 500:
        If n * 3 = m:
            Return n.
        Set n to n + 1.
    Return 0 - n.

## Main
Show seek(30).
Show seek(31).
Show seek(300).
Show seek(301).
Show seek(0).
Show seek(1499).
",
    );
}

#[test]
fn a_real_valued_search_returns_from_the_loop_or_after_it() {
    assert_jit_matches_the_engines(
        "## To grow (m: Int) -> Real:
    Let mutable x be 0.5.
    Repeat for n from 1 to 200:
        Set x to x + 0.25.
        If n = m:
            Return x.
    Return x * 2.0.

## Main
Show grow(4).
Show grow(500).
Show grow(10).
Show grow(500).
Show grow(200).
",
    );
}

#[test]
fn every_call_after_the_region_compiles_finishes_its_loop() {
    assert_jit_matches_the_engines(&format!(
        "{FLAG_THEN_BRANCH}## Main\nShow find(50).\nShow find(51).\nShow find(52).\nShow find(53).\n"
    ));
}

#[test]
fn every_call_after_the_region_compiles_returns_from_inside_it() {
    assert_jit_matches_the_engines(&format!(
        "{FLAG_THEN_BRANCH}## Main\nShow find(9801).\nShow find(1).\nShow find(4).\nShow find(9).\nShow find(0).\n"
    ));
}
