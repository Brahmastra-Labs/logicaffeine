//! ★ LOCK — `largo verify` is sound on the statements it reads. ★
//!
//! The static verifier turns a program's statements into facts for Z3 and asks whether each `Assert` follows. A fact
//! that is not true of the run makes every later assertion "follow": `Let x be 1.` then `Set x to 2.` must not leave
//! both `x = 1` and `x = 2` standing, or `Assert that x equals 7.` is proved from a contradiction. So:
//!
//! - an assignment makes a new version of the name, and earlier facts speak of the earlier version;
//! - a branch's assignments hold only under the branch's condition, and the two sides join after it;
//! - a loop forgets what its body may change, and after it only the loop's exit condition is known;
//! - an assertion inside a branch is checked under the branch's condition.
//!
//! Each case states a program and whether its last assertion follows. A refused case must be refused and an accepted
//! case accepted: a verifier that accepts everything fails the first half, one that refuses everything the second.

#![cfg(feature = "verification")]

use logicaffeine_compile::compile::compile_to_rust_verified;

fn accepted(source: &str) {
    if let Err(e) = compile_to_rust_verified(source) {
        panic!("the assertion follows, so the program verifies:\n{source}\n--- refused with ---\n{e:?}");
    }
}

fn refused(source: &str) {
    assert!(
        compile_to_rust_verified(source).is_err(),
        "the assertion does not follow, so the program must not verify:\n{source}"
    );
}

#[test]
fn a_false_assertion_after_a_reassignment_is_refused() {
    refused("## Main\nLet mutable x be 1.\nSet x to 2.\nAssert that x equals 7.\n");
}

#[test]
fn the_value_before_a_reassignment_does_not_hold_after_it() {
    refused("## Main\nLet mutable x be 1.\nSet x to 2.\nAssert that x equals 1.\n");
}

#[test]
fn the_value_after_a_reassignment_holds() {
    accepted("## Main\nLet mutable x be 1.\nSet x to 2.\nAssert that x equals 2.\n");
}

#[test]
fn a_reassignment_from_the_old_value_reads_the_old_value() {
    accepted("## Main\nLet mutable x be 1.\nSet x to x + 5.\nAssert that x equals 6.\n");
    refused("## Main\nLet mutable x be 1.\nSet x to x + 5.\nAssert that x equals 5.\n");
}

#[test]
fn a_branch_not_taken_leaves_the_value_alone() {
    let program = "## Main\nLet mutable x be 1.\nLet c be 3.\nIf c is greater than 5:\n    Set x to 9.\n";
    accepted(&format!("{program}Assert that x equals 1.\n"));
    refused(&format!("{program}Assert that x equals 9.\n"));
}

#[test]
fn a_branch_taken_changes_the_value() {
    let program = "## Main\nLet mutable x be 1.\nLet c be 8.\nIf c is greater than 5:\n    Set x to 9.\n";
    accepted(&format!("{program}Assert that x equals 9.\n"));
    refused(&format!("{program}Assert that x equals 1.\n"));
}

#[test]
fn both_sides_of_a_branch_join_after_it() {
    let program = "## Main\nLet mutable x be 0.\nLet c be 2.\nIf c is greater than 5:\n    Set x to 9.\nOtherwise:\n    Set x to 4.\n";
    accepted(&format!("{program}Assert that x equals 4.\n"));
    refused(&format!("{program}Assert that x equals 9.\n"));
}

#[test]
fn an_assertion_inside_a_branch_holds_under_its_condition() {
    accepted("## Main\nLet x be 4.\nIf x is greater than 3:\n    Assert that x is greater than 3.\n");
    refused("## Main\nLet x be 4.\nIf x is greater than 3:\n    Assert that x is greater than 10.\n");
}

#[test]
fn a_loop_forgets_what_its_body_changes() {
    let program = "## Main\nLet mutable i be 0.\nWhile i is less than 10:\n    Set i to i + 1.\n";
    refused(&format!("{program}Assert that i equals 0.\n"));
    refused(&format!("{program}Assert that i equals 3.\n"));
}

#[test]
fn after_a_loop_its_exit_condition_holds() {
    accepted("## Main\nLet mutable i be 0.\nWhile i is less than 10:\n    Set i to i + 1.\nAssert that i is at least 10.\n");
}

#[test]
fn a_loop_keeps_what_its_body_does_not_change() {
    accepted("## Main\nLet k be 7.\nLet mutable i be 0.\nWhile i is less than 10:\n    Set i to i + 1.\nAssert that k equals 7.\n");
}

#[test]
fn termination_holds_on_every_iteration_not_only_the_first() {
    // 10, 8, 6, … never equals 3 and runs below zero: the variant is bounded below only on the first iteration.
    refused("## Main\nLet mutable x be 10.\nWhile x is not 3 (decreasing x):\n    Set x to x - 2.\n");
    accepted("## Main\nLet mutable x be 10.\nWhile x is greater than 0 (decreasing x):\n    Set x to x - 2.\n");
}

#[test]
fn a_function_bodys_facts_stay_in_the_function() {
    let program = "## To f (n: Int) -> Int:\n    Let y be 5.\n    Return y.\n\n## Main\nLet y be 6.\n";
    accepted(&format!("{program}Assert that y equals 6.\n"));
    refused(&format!("{program}Assert that y equals 5.\n"));
}

#[test]
fn two_readings_of_the_clock_are_not_assumed_equal() {
    refused("## Main\nLet a be now().\nLet b be now().\nAssert that a equals b.\n");
}

#[test]
fn two_calls_of_a_pure_function_with_equal_arguments_are_equal() {
    accepted(
        "## To double (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nLet a be double(3).\nLet b be double(3).\nAssert that a equals b.\n",
    );
}

#[test]
fn a_loop_that_can_break_does_not_promise_its_exit_condition() {
    refused(
        "## Main\nLet mutable i be 0.\nWhile i is less than 10:\n    If i equals 5:\n        Break.\n    Set i to i + 1.\nAssert that i is at least 10.\n",
    );
}

#[test]
fn an_early_return_narrows_the_rest_of_the_function() {
    let head = "## To f (n: Int) -> Int:\n    If n is less than 0:\n        Return 0.\n";
    let tail = "    Return n.\n\n## Main\nShow f(3).\n";
    accepted(&format!("{head}    Assert that n is at least 0.\n{tail}"));
    refused(&format!("{head}    Assert that n is at least 1.\n{tail}"));
}

#[test]
fn a_declaration_in_a_branch_shadows_only_inside_it() {
    let program = "## Main\nLet x be 1.\nLet c be 9.\nIf c is greater than 5:\n    Let x be 50.\n";
    accepted(&format!("{program}Assert that x equals 1.\n"));
    refused(&format!("{program}Assert that x equals 50.\n"));
}

#[test]
fn a_swap_exchanges_the_two_values() {
    let program = "## Main\nLet mutable a be 1.\nLet mutable b be 2.\nSwap a and b.\n";
    accepted(&format!("{program}Assert that a equals 2.\n"));
    refused(&format!("{program}Assert that a equals 1.\n"));
}

#[test]
fn an_assertion_the_verifier_cannot_model_is_not_reported_proved() {
    refused("## Main\nLet xs be [1, 2, 3].\nAssert that length of xs equals 3.\n");
}

#[test]
fn a_requirement_is_enforced_at_run_time_so_what_follows_may_assume_it() {
    accepted(
        "## To withdraw (balance: Int, amount: Int) -> Int:\n    Require that amount is at most balance.\n    Assert that balance - amount is at least 0.\n    Return balance - amount.\n\n## Main\nShow withdraw(10, 3).\n",
    );
}
