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
fn nested_loans_preserve_a_copy_made_before_the_call() {
    check("## To change (value: mutable Int) -> Int:\n    Show xs.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [[4, 9], [2, 3]].\nLet saved be xs.\nShow change(item 1 of item 1 of xs).\nShow xs.\nShow saved.\n", "[[nothing, 9], [2, 3]]\n1\n[[7, 9], [2, 3]]\n[[4, 9], [2, 3]]");
}

#[test]
fn nested_copyback_preserves_a_copy_made_during_the_call() {
    check("## To change (value: mutable Int) -> Int:\n    Set saved to xs.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [[4, 9], [2, 3]].\nLet saved be [[0]].\nShow change(item 1 of item 1 of xs).\nShow xs.\nShow saved.\n", "1\n[[7, 9], [2, 3]]\n[[nothing, 9], [2, 3]]");
}

#[test]
fn one_list_can_lend_at_two_different_depths() {
    check("## To change (value: mutable Int) -> Int:\n    Show xs.\n    Set value to 7.\n    Return 1.\n## To row (value: mutable Seq of Int) -> Int:\n    Show xs.\n    Push 8 to value.\n    Return length of value.\n## Main\nLet xs be [[4, 9], [2, 3]].\nShow change(item 1 of item 1 of xs).\nShow row(item 1 of xs).\nShow xs.\n", "[[nothing, 9], [2, 3]]\n1\n[nothing, [2, 3]]\n3\n[[7, 9, 8], [2, 3]]");
}

#[test]
fn nested_keys_run_once_in_collection_then_element_order() {
    check("## To outer () -> Int:\n    Show \"outer\".\n    Return 1.\n## To inner () -> Int:\n    Show \"inner\".\n    Return 1.\n## To change (value: mutable Int) -> Int:\n    Show xs.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [[4, 9], [2, 3]].\nShow change(item (inner()) of item (outer()) of xs).\nShow xs.\n", "outer\ninner\n[[nothing, 9], [2, 3]]\n1\n[[7, 9], [2, 3]]");
}

#[test]
fn a_captured_generic_field_retains_its_empty_type() {
    check("## A Box of [T] has:\n    value: T.\n## To change (value: mutable Int) -> Int:\n    Let captured be b's value.\n    Show captured is equal to nothing.\n    Set value to 7.\n    Return 1.\n## Main\nLet b be a new Box of Int with value 4.\nShow change(b's value).\nShow b's value.\n", "true\n1\n7");
}

#[test]
fn nullable_machine_integer_equality_does_not_round_at_two_to_the_53() {
    check("## To change (value: mutable Int) -> Int:\n    Show n.\n    Set value to 9007199254740993.\n    Return 1.\n## Main\nLet n be 0.\nShow change(n).\nShow n is equal to 9007199254740992.0.\n", "nothing\n1\nfalse");
}

#[test]
fn a_looping_function_uses_one_sequence_representation_for_all_callers() {
    check("## To whole (value: mutable Seq of Int) -> Int:\n    Show xs.\n    Let size be length of value.\n    Repeat for i from 1 to size:\n        Push 10 to value.\n    Return length of value.\n## To element (value: mutable Int) -> Int:\n    Show xs.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [4, 9].\nLet ys be [2].\nShow element(item 1 of xs).\nShow whole(xs).\nShow whole(ys).\nShow ys.\nShow xs.\n", "[nothing, 9]\n1\nnothing\n4\n[7, 9, 10, 10]\n2\n[2, 10]\n[7, 9, 10, 10]");
}
#[test]
fn a_nullable_sequence_parameter_accepts_an_inline_temporary() {
    check("## To whole (value: mutable Seq of Int) -> Int:\n    Show xs.\n    Let size be length of value.\n    Repeat for i from 1 to size:\n        Push 10 to value.\n    Return length of value.\n## To element (value: mutable Int) -> Int:\n    Show xs.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [4, 9].\nShow element(item 1 of xs).\nShow whole(xs).\nShow whole([2]).\nShow xs.\n", "[nothing, 9]\n1\nnothing\n4\n[7, 9, 10, 10]\n2\n[7, 9, 10, 10]");
}
