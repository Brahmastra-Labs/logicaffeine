//! A loan empties observable storage; copies of the empty value remain empty.
#![cfg(not(target_arch = "wasm32"))]
mod common;
use logicaffeine_compile::{compile::{tw_outcome, vm_outcome, vm_outcome_tiered}, optimization::Tier};
fn check(source: &str, expected: &str) {
    for (name, run) in [("tree walker", tw_outcome(source)), ("VM", vm_outcome(source)), ("optimized VM", vm_outcome_tiered(source, &[], Tier::T3, None))] {
        assert_eq!(run.error, None, "{name}: {run:?}\n{source}");
        assert_eq!(run.output.trim_end_matches('\n'), expected, "{name}");
    }
    common::assert_exact_output(source, expected);
}
#[test]
fn a_saved_sequence_does_not_observe_the_later_element_loan() {
    check("## To change (value: mutable Int) -> Int:\n    Show xs.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [4, 9].\nLet saved be xs.\nShow change(item 1 of xs).\nShow xs.\nShow saved.\n", "[nothing, 9]\n1\n[7, 9]\n[4, 9]");
}

#[test]
fn a_copy_saved_during_an_element_loan_keeps_its_empty_slot() {
    check("## To change (value: mutable Int) -> Int:\n    Set saved to xs.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [4, 9].\nLet saved be [0].\nShow change(item 1 of xs).\nShow xs.\nShow saved.\n", "1\n[7, 9]\n[nothing, 9]");
}

#[test]
fn an_empty_global_compares_unequal_to_an_integer() {
    check("## To change (value: mutable Int) -> Int:\n    Show n is equal to 4.\n    Show 4 is equal to n.\n    Set value to 7.\n    Return 1.\n## Main\nLet n be 4.\nShow change(n).\nShow n.\n", "false\nfalse\n1\n7");
}

#[test]
fn a_nullable_sequence_survives_a_forwarded_mutable_call() {
    check("## To relay (value: mutable Seq of Int) -> Int:\n    Push 10 to value.\n    Return length of value.\n## To whole (value: mutable Seq of Int) -> Int:\n    Show xs.\n    Return relay(value).\n## To element (value: mutable Int) -> Int:\n    Show xs.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [4, 9].\nShow element(item 1 of xs).\nShow whole(xs).\nShow xs.\n", "[nothing, 9]\n1\nnothing\n3\n[7, 9, 10]");
}

#[test]
fn a_nullable_sequence_can_be_replaced_as_a_whole() {
    check("## To whole (value: mutable Seq of Int) -> Int:\n    Show xs.\n    Set value to [2, 3].\n    Return length of value.\n## To element (value: mutable Int) -> Int:\n    Show xs.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [4, 9].\nShow element(item 1 of xs).\nShow whole(xs).\nShow xs.\n", "[nothing, 9]\n1\nnothing\n2\n[2, 3]");
}

#[test]
fn another_caller_adopts_the_nullable_sequence_parameter_representation() {
    check("## To whole (value: mutable Seq of Int) -> Int:\n    Show xs.\n    Push 10 to value.\n    Return length of value.\n## To element (value: mutable Int) -> Int:\n    Show xs.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [4, 9].\nLet ys be [2].\nShow element(item 1 of xs).\nShow whole(ys).\nShow ys.\nShow xs.\n", "[nothing, 9]\n1\n[7, 9]\n2\n[2, 10]\n[7, 9]");
}

#[test]
fn a_copied_record_keeps_its_empty_field_after_copyback() {
    check("## A Box has:\n    a value, which is Int.\n## To change (value: mutable Int) -> Int:\n    Set saved to b.\n    Set value to 7.\n    Return 1.\n## Main\nLet b be a new Box with value 4.\nLet saved be a new Box with value 0.\nShow change(b's value).\nShow b's value.\nShow saved's value.\n", "1\n7\nnothing");
}
