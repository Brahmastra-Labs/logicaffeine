//! C calls must return each mutable argument to the place that lent it.
#![cfg(not(target_arch = "wasm32"))]
mod common;
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

fn same_result(source: &str, expected: &str) {
    let tw = tw_outcome(source);
    let vm = vm_outcome(source);
    assert_eq!(tw.error, None, "tree-walker fixture:\n{source}");
    assert_eq!(vm.error, None, "VM fixture:\n{source}");
    assert_eq!(tw.output.trim_end_matches('\n'), expected);
    assert_eq!(vm.output.trim_end_matches('\n'), expected);
    let optimized = logicaffeine_compile::compile::vm_outcome_tiered(
        source, &[], logicaffeine_compile::optimization::Tier::T3, None,
    );
    assert_eq!(optimized.error, None, "optimized VM: {optimized:?}\n{source}");
    assert_eq!(optimized.output.trim_end_matches('\n'), expected, "optimized VM");
    common::assert_c_output(source, expected);
    common::assert_exact_output(source, expected);
}

const BUMP: &str = "## To bump (value: mutable Int) -> Int:
    Set value to value + 1.
    Return value.
";

#[test]
fn a_mutable_scalar_updates_its_caller() {
    same_result(&format!("{BUMP}## Main\nLet mutable n be 4.\nShow bump(n).\nShow n.\n"), "5\n5");
}

#[test]
fn a_mutable_scalar_is_returned_on_an_early_return() {
    same_result("## To change (value: mutable Int) -> Int:
    Set value to value + 1.
    If value is greater than 0:
        Return value.
    Set value to 99.
    Return value.
## Main
Let mutable n be 4.
Show change(n).
Show n.
", "5\n5");
}

#[test]
fn a_forwarded_mutable_scalar_reaches_the_outer_caller() {
    same_result(&format!("{BUMP}
## To twice (value: mutable Int) -> Int:
    Let first be bump(value).
    Return bump(value).
## Main
Let mutable n be 4.
Show twice(n).
Show n.
"), "6\n6");
}

#[test]
fn two_distinct_mutable_scalars_are_both_returned() {
    same_result("## To change (left: mutable Int, right: mutable Int) -> Int:
    Set left to left + 2.
    Set right to right + 3.
    Return left + right.
## Main
Let mutable a be 1.
Let mutable b be 2.
Show change(a, b).
Show a.
Show b.
", "8\n3\n5");
}

#[test]
fn a_mutable_text_returns_its_new_buffer() {
    same_result("## To append (s: mutable Text) -> Int:
    Set s to s + \"!\".
    Return length of s.
## Main
Let mutable s be \"hello\".
Let saved be s.
Show append(s).
Show s.
Show saved.
", "6\nhello!\nhello");
}

#[test]
fn a_mutable_sequence_returns_changes_without_changing_an_alias() {
    same_result("## To change (xs: mutable Seq of Int) -> Int:
    Push 9 to xs.
    Set item 1 of xs to 7.
    Return length of xs.
## Main
Let mutable xs be [1].
Let saved be xs.
Show change(xs).
Show xs.
Show saved.
", "2\n[7, 9]\n[1]");
}

#[test]
fn a_mutable_record_field_returns_to_its_record() {
    same_result(&format!("{BUMP}
## A Box has:
    a value, which is Int.
## Main
Let mutable b be a new Box with value 4.
Let saved be b.
Show bump(b's value).
Show b's value.
Show saved's value.
"), "5\n5\n4");
}

#[test]
fn a_mutable_sequence_item_returns_to_its_sequence() {
    same_result(&format!("{BUMP}## Main\nLet mutable xs be [4].\nLet saved be xs.\nShow bump(item 1 of xs).\nShow item 1 of xs.\nShow item 1 of saved.\n"), "5\n5\n4");
}

#[test]
fn a_value_expression_can_be_passed_without_a_place_to_return_to() {
    same_result(&format!("{BUMP}## Main\nLet n be 4.\nShow bump(n + 1).\nShow n.\n"), "6\n4");
}
