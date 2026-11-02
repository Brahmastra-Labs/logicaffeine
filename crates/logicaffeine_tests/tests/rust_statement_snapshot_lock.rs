//! Statement operands and place keys keep source evaluation order.
mod common;
use logicaffeine_compile::compile::{tw_outcome, vm_outcome, vm_outcome_tiered};
use logicaffeine_compile::optimization::Tier;

fn holds(source: &str, expected: &str) {
    let reference = tw_outcome(source);
    assert_eq!(reference.error, None, "tree-walker: {source}");
    assert_eq!(reference.output.lines().collect::<Vec<_>>(), expected.lines().collect::<Vec<_>>(), "tree-walker: {source}");
    let vm = vm_outcome(source);
    assert_eq!(vm.error, None, "VM: {source}");
    assert_eq!(vm.output.lines().collect::<Vec<_>>(), expected.lines().collect::<Vec<_>>(), "VM: {source}");
    let optimized = vm_outcome_tiered(source, &[], Tier::T3, None);
    assert_eq!(optimized.error, None, "optimized VM: {source}");
    assert_eq!(optimized.output.lines().collect::<Vec<_>>(), expected.lines().collect::<Vec<_>>(), "optimized VM: {source}");
    common::assert_exact_output(source, expected.trim_end_matches('\n'));
}

#[test]
fn set_index_before_mutating_value() {
    holds("## To replace (first: mutable Int) -> Int:\n    Set first to 2.\n    Return 9.\n## Main\nLet mutable first be 1.\nLet values be [7, 8].\nSet item first of values to replace(first).\nShow values.\nShow first.\n", "[9, 8]\n2\n");
}

#[test]
fn push_value_before_mutating_place_key() {
    holds("## To choose (value: mutable Int) -> Int:\n    Set value to 9.\n    Return 1.\n## Main\nLet mutable value be 7.\nLet mutable grid be [[1]].\nPush value to item (choose(value)) of grid.\nShow grid.\nShow value.\n", "[[1, 7]]\n9\n");
}

#[test]
fn set_value_before_mutating_place_key() {
    holds("## To choose (value: mutable Int) -> Int:\n    Set value to 9.\n    Return 1.\n## Main\nLet mutable value be 7.\nLet mutable grid be [[3, 4]].\nSet item 1 of (item (choose(value)) of grid) to value.\nShow grid.\nShow value.\n", "[[7, 4]]\n9\n");
}

#[test]
fn set_index_before_mutating_place_key() {
    holds("## To choose (key: mutable Int) -> Int:\n    Set key to 2.\n    Return 1.\n## Main\nLet mutable key be 1.\nLet mutable grid be [[3, 4]].\nSet item key of (item (choose(key)) of grid) to 7.\nShow grid.\nShow key.\n", "[[7, 4]]\n2\n");
}

#[test]
fn nested_place_keys_follow_root_to_leaf_order() {
    holds("## To rootKey () -> Int:\n    Show \"root\".\n    Return 1.\n## To leafKey () -> Int:\n    Show \"leaf\".\n    Return 1.\n## Main\nLet mutable grid be [[[3]]].\nPush 7 to item (leafKey()) of (item (rootKey()) of grid).\nShow grid.\n", "root\nleaf\n[[[3, 7]]]\n");
}

#[test]
fn interpolation_before_mutating_hole() {
    holds("## To replace (first: mutable Int) -> Int:\n    Set first to 9.\n    Return 1.\n## Main\nLet mutable first be 7.\nShow \"{first}:{replace(first)}\".\nShow first.\n", "7:1\n9\n");
}

#[test]
fn pure_statement_control() {
    holds("## Main\nLet mutable grid be [[3, 4]].\nSet item 1 of (item 1 of grid) to 7.\nPush 8 to item 1 of grid.\nShow grid.\n", "[[7, 4, 8]]\n");
}

#[test]
fn a_call_in_a_binding_invalidates_later_reads() {
    holds("## To choose (value: mutable Int) -> Int:\n    Set value to 9.\n    Return 1.\n## Main\nLet mutable value be 7.\nLet chosen be choose(value).\nLet after be value.\nShow chosen.\nShow after.\nShow value.\n", "1\n9\n9\n");
}

#[test]
fn a_call_in_a_condition_invalidates_later_reads() {
    holds("## To choose (value: mutable Int) -> Int:\n    Set value to 9.\n    Return 1.\n## Main\nLet mutable value be 7.\nIf choose(value) is equal to 1:\n    Let inside be value.\n    Show inside.\nLet after be value.\nShow after.\n", "9\n9\n");
}

#[test]
fn a_call_preserves_its_place_and_later_operand() {
    holds("## To choose (value: mutable Int) -> Int:\n    Set value to 9.\n    Return 1.\n## Main\nLet mutable value be 7.\nLet result be choose(value) + value.\nShow result.\nShow value.\n", "10\n9\n");
}

#[test]
fn a_call_in_assignment_invalidates_later_reads() {
    holds("## To choose (value: mutable Int) -> Int:\n    Set value to 9.\n    Return 1.\n## Main\nLet mutable value be 7.\nLet mutable result be 0.\nSet result to choose(value).\nLet after be value.\nShow result.\nShow after.\n", "1\n9\n");
}

#[test]
fn a_call_in_a_while_condition_invalidates_body_and_exit_reads() {
    holds("## To choose (value: mutable Int) -> Int:\n    Set value to 9.\n    Return 1.\n## Main\nLet mutable value be 7.\nWhile choose(value) is equal to 1:\n    Let inside be value.\n    Show inside.\n    Break.\nLet after be value.\nShow after.\n", "9\n9\n");
}

#[test]
fn a_call_in_a_repeat_iterable_invalidates_body_and_exit_reads() {
    holds("## To choose (value: mutable Int) -> Seq of Int:\n    Set value to 9.\n    Return [1].\n## Main\nLet mutable value be 7.\nRepeat for n in choose(value):\n    Let inside be value.\n    Show inside.\nLet after be value.\nShow after.\n", "9\n9\n");
}

#[test]
fn nested_set_keys_are_evaluated_once_from_the_root() {
    holds("## To rootKey () -> Int:\n    Show \"root\".\n    Return 1.\n## To leafKey () -> Int:\n    Show \"leaf\".\n    Return 1.\n## Main\nLet mutable grid be [[[3]]].\nSet item 1 of (item (leafKey()) of (item (rootKey()) of grid)) to 7.\nShow grid.\n", "root\nleaf\n[[[7]]]\n");
}
