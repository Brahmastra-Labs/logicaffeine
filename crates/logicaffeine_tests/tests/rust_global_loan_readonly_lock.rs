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
fn read_only_indexing_reads_the_restored_element() {
    check("## To readvalues (values: Seq of Int) -> Int:\n    Return item 1 of values + item 2 of values.\n## To change (value: mutable Int) -> Int:\n    Show xs.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [4, 9].\nShow change(item 1 of xs).\nShow readvalues(xs).\nShow xs.\n", "[nothing, 9]\n1\n16\n[7, 9]");
}

#[test]
fn read_only_iteration_reads_the_restored_elements() {
    check("## To addall (values: Seq of Int) -> Int:\n    Let total be 0.\n    Repeat for entry in values:\n        Set total to total + entry.\n    Return total.\n## To change (value: mutable Int) -> Int:\n    Show xs.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [4, 9].\nShow change(item 1 of xs).\nShow addall(xs).\nShow xs.\n", "[nothing, 9]\n1\n16\n[7, 9]");
}

#[test]
fn read_only_parameters_can_be_forwarded() {
    check("## To addall (values: Seq of Int) -> Int:\n    Let total be 0.\n    Repeat for entry in values:\n        Set total to total + entry.\n    Return total.\n## To repeatedly (values: Seq of Int) -> Int:\n    Let total be 0.\n    Repeat for i from 1 to length of values:\n        Set total to total + addall(values).\n    Return total.\n## To change (value: mutable Int) -> Int:\n    Show xs.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [4, 9].\nShow change(item 1 of xs).\nShow repeatedly(xs).\nShow xs.\n", "[nothing, 9]\n1\n32\n[7, 9]");
}

#[test]
fn a_read_only_parameter_can_be_copied_into_a_growing_local() {
    check("## To growcopy (values: Seq of Int) -> Seq of Int:\n    Let buffer be values.\n    Push 10 to buffer.\n    Return buffer.\n## To change (value: mutable Int) -> Int:\n    Show xs.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [4, 9].\nShow change(item 1 of xs).\nShow growcopy(xs).\nShow xs.\n", "[nothing, 9]\n1\n[7, 9, 10]\n[7, 9]");
}

#[test]
fn an_empty_scalar_can_be_saved_in_another_global() {
    check("## To change (value: mutable Int) -> Int:\n    Set saved to n.\n    Show saved.\n    Set n to 99.\n    Show saved.\n    Set value to 7.\n    Return 1.\n## Main\nLet n be 4.\nLet saved be 0.\nShow change(n).\nShow n.\nShow saved.\n", "nothing\nnothing\n1\n7\nnothing");
}
