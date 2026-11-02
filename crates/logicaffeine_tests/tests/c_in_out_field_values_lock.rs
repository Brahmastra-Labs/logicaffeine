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
fn default_record_fields_start_present_before_their_loan() {
    check("## A Box has:\n    a value, which is Int.\n## To change (value: mutable Int) -> Int:\n    Show b's value.\n    Set value to 7.\n    Return 1.\n## Main\nLet b be a new Box.\nShow b's value.\nShow change(b's value).\nShow b's value.\n", "0\nnothing\n1\n7");
}

#[test]
fn a_saved_empty_field_stays_empty_after_the_field_is_written() {
    check("## A Box has:\n    a value, which is Int.\n## To change (value: mutable Int) -> Int:\n    Let saved be b's value.\n    Set b's value to 99.\n    Show saved.\n    Show b's value.\n    Show saved is equal to nothing.\n    Set value to 7.\n    Return 1.\n## Main\nLet b be a new Box with value 4.\nShow change(b's value).\nShow b's value.\n", "nothing\n99\ntrue\n1\n7");
}

#[test]
fn a_lent_text_field_returns_its_new_value() {
    check("## A Box has:\n    a value, which is Text.\n## To change (value: mutable Text) -> Int:\n    Show b's value.\n    Set value to value + \"!\".\n    Return length of value.\n## Main\nLet b be a new Box with value \"hi\".\nLet saved be b.\nShow change(b's value).\nShow b's value.\nShow saved's value.\n", "nothing\n3\nhi!\nhi");
}
