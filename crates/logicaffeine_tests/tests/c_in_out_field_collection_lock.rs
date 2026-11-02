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
fn copying_a_record_copies_its_nullable_sequence_buffer() {
    check("## A Box has:\n    a values, which is Seq of Int.\n## To change (values: mutable Seq of Int) -> Int:\n    Show b's values.\n    Set item 1 of values to 9.\n    Push 7 to values.\n    Return length of values.\n## Main\nLet b be a new Box with values [4].\nLet saved be b.\nShow change(b's values).\nShow b's values.\nShow saved's values.\n", "nothing\n2\n[9, 7]\n[4]");
}

#[test]
fn a_record_copied_during_a_sequence_loan_keeps_nothing() {
    check("## A Box has:\n    a values, which is Seq of Int.\n## To change (values: mutable Seq of Int) -> Int:\n    Set snapshot to b.\n    Push 7 to values.\n    Return length of values.\n## Main\nLet b be a new Box with values [4].\nLet snapshot be a new Box with values [1].\nShow change(b's values).\nShow b's values.\nShow snapshot's values.\n", "2\n[4, 7]\nnothing");
}

#[test]
fn copying_a_record_copies_its_nullable_map_buffer() {
    check("## A Box has:\n    a values, which is Map of Text to Int.\n## To change (values: mutable Map of Text to Int) -> Int:\n    Show b's values.\n    Set item \"key\" of values to 9.\n    Return length of values.\n## Main\nLet values be a new Map of Text to Int.\nSet item \"key\" of values to 4.\nLet b be a new Box with values values.\nLet saved be b.\nShow change(b's values).\nShow item \"key\" of b's values.\nShow item \"key\" of saved's values.\n", "nothing\n1\n9\n4");
}
