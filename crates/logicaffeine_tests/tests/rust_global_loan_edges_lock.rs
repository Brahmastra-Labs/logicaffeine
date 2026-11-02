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
fn ordinary_arithmetic_reads_the_present_global() {
    check("## To change (value: mutable Int) -> Int:\n    Show n.\n    Set value to 7.\n    Return 1.\n## Main\nLet n be 4.\nShow n + 1.\nShow change(n).\nShow n + 1.\n", "5\nnothing\n1\n8");
}

#[test]
fn a_typed_return_reads_the_present_global() {
    check("## To read () -> Int:\n    Return n.\n## To change (value: mutable Int) -> Int:\n    Show n.\n    Set value to 7.\n    Return 1.\n## Main\nLet n be 4.\nShow read().\nShow change(n).\nShow read().\n", "4\nnothing\n1\n7");
}

#[test]
fn a_mixed_sequence_keeps_its_empty_element() {
    check("## To change (value: mutable Int) -> Int:\n    Let captured be [n, 9].\n    Set n to 99.\n    Show captured.\n    Set value to 7.\n    Return 1.\n## Main\nLet n be 4.\nShow change(n).\nShow n.\n", "[nothing, 9]\n1\n7");
}

#[test]
fn assigning_a_copy_of_an_empty_slot_does_not_fill_the_original() {
    check("## To change (value: mutable Int) -> Int:\n    Let mutable captured be n.\n    Set captured to 8.\n    Show captured.\n    Show n.\n    Set value to 7.\n    Return 1.\n## Main\nLet n be 4.\nShow change(n).\nShow n.\n", "8\nnothing\n1\n7");
}

#[test]
fn a_negative_element_key_is_evaluated_once_before_the_loan() {
    check("## To position () -> Int:\n    Show \"key\".\n    Return -1.\n## To change (value: mutable Int) -> Int:\n    Show xs.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [4, 9].\nShow change(item (position()) of xs).\nShow xs.\n", "key\n[4, nothing]\n1\n[4, 7]");
}

#[test]
fn a_global_sequence_supports_whole_and_element_loans() {
    check("## To element (value: mutable Int) -> Int:\n    Show xs.\n    Set value to 7.\n    Return 1.\n## To whole (value: mutable Seq of Int) -> Int:\n    Show xs.\n    Push 10 to value.\n    Return length of value.\n## Main\nLet xs be [4, 9].\nShow element(item 1 of xs).\nShow whole(xs).\nShow xs.\n", "[nothing, 9]\n1\nnothing\n3\n[7, 9, 10]");
}
