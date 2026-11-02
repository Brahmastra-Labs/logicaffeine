//! Mutable arguments retain dynamic empty-slot and exact-number semantics.
#![cfg(not(target_arch = "wasm32"))]
mod common;
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

fn check(source: &str, expected: &str) {
    let tw = tw_outcome(source);
    let vm = vm_outcome(source);
    assert_eq!(tw.error, None, "{tw:?}\n{source}");
    assert_eq!(vm.error, None, "{vm:?}\n{source}");
    assert_eq!(tw.output.trim_end_matches('\n'), expected, "tree-walker");
    assert_eq!(vm.output.trim_end_matches('\n'), expected, "VM");
    let optimized = logicaffeine_compile::compile::vm_outcome_tiered(
        source, &[], logicaffeine_compile::optimization::Tier::T3, None,
    );
    assert_eq!(optimized.error, None, "optimized VM: {optimized:?}\n{source}");
    assert_eq!(optimized.output.trim_end_matches('\n'), expected, "optimized VM");
    common::assert_c_output(source, expected);
    common::assert_exact_output(source, expected);
}

#[test]
fn a_lent_global_record_field_is_nothing() {
    check("## A Box has:\n    a value, which is Int.\n## To change (value: mutable Int) -> Int:\n    Show b's value.\n    Set value to 7.\n    Return 1.\n## Main\nLet b be a new Box with value 4.\nShow change(b's value).\nShow b's value.\n", "nothing\n1\n7");
}

#[test]
fn a_lent_global_sequence_element_is_nothing() {
    check("## To change (value: mutable Int) -> Int:\n    Show item 1 of xs.\n    Show xs.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [4].\nShow change(item 1 of xs).\nShow xs.\n", "nothing\n[nothing]\n1\n[7]");
}

#[test]
fn a_lent_nested_global_element_preserves_its_parent() {
    check("## To change (value: mutable Int) -> Int:\n    Show xs.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [[4, 5]].\nShow change(item 1 of (item 1 of xs)).\nShow xs.\n", "[[nothing, 5]]\n1\n[[7, 5]]");
}

#[test]
fn a_lent_global_map_entry_is_nothing() {
    check("## To change (value: mutable Int) -> Int:\n    Show item \"key\" of values.\n    Set value to 7.\n    Return 1.\n## Main\nLet values be a new Map of Text to Int.\nSet item \"key\" of values to 4.\nShow change(item \"key\" of values).\nShow item \"key\" of values.\n", "nothing\n1\n7");
}
