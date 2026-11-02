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
fn a_copied_empty_global_stays_empty_after_the_global_is_assigned() {
    check("## To change (value: mutable Int) -> Int:\n    Let captured be n.\n    Show captured.\n    Set n to 99.\n    Show captured.\n    Show n.\n    Set value to 7.\n    Return 1.\n## Main\nLet n be 4.\nShow change(n).\nShow n.\n", "nothing\nnothing\n99\n1\n7");
}
#[test]
fn an_empty_global_can_be_stored_in_a_sequence() {
    check("## To change (value: mutable Int) -> Int:\n    Let captured be [n].\n    Set value to 7.\n    Show captured.\n    Return 1.\n## Main\nLet n be 4.\nShow change(n).\nShow n.\n", "[nothing]\n1\n7");
}
#[test]
fn an_empty_global_compares_equal_to_nothing() {
    check("## To change (value: mutable Int) -> Int:\n    Show n is equal to nothing.\n    Set value to 7.\n    Return 1.\n## Main\nLet n be 4.\nShow change(n).\nShow n.\n", "true\n1\n7");
}
#[test]
fn a_lent_global_record_field_is_empty_until_copyback() {
    check("## A Box has:\n    a value, which is Int.\n## To change (value: mutable Int) -> Int:\n    Show b's value.\n    Set value to 7.\n    Return 1.\n## Main\nLet b be a new Box with value 4.\nShow change(b's value).\nShow b's value.\n", "nothing\n1\n7");
}
#[test]
fn a_lent_global_sequence_element_is_empty_until_copyback() {
    check("## To change (value: mutable Int) -> Int:\n    Show item 1 of xs.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [4, 9].\nShow change(item 1 of xs).\nShow xs.\n", "nothing\n1\n[7, 9]");
}
#[test]
fn a_later_argument_observes_an_already_empty_global() {
    check("## To inspect () -> Int:\n    Show n.\n    Return 2.\n## To change (value: mutable Int, amount: Int) -> Int:\n    Set value to value + amount.\n    Return 1.\n## Main\nLet n be 4.\nShow change(n, inspect()).\nShow n.\n", "nothing\n1\n6");
}
