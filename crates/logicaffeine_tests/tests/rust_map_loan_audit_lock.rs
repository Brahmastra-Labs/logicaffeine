//! Native mutable places preserve keys, caller updates, and copied aliases.
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
fn a_map_copy_before_the_loan_retains_present_values() {
    check("## To change (value: mutable Int) -> Int:\n    Show item \"first\" of saved.\n    Show item \"first\" of values.\n    Set value to 7.\n    Return 1.\n## Main\nLet values be a new Map of Text to Int.\nSet item \"first\" of values to 4.\nSet item \"second\" of values to 5.\nLet saved be values.\nShow change(item \"first\" of values).\nShow item \"first\" of saved.\nShow item \"first\" of values.\n", "4\nnothing\n1\n4\n7");
}

#[test]
fn a_map_copy_during_the_loan_retains_absence() {
    check("## To change (value: mutable Int) -> Int:\n    Set saved to values.\n    Set value to 7.\n    Return 1.\n## Main\nLet values be a new Map of Text to Int.\nSet item \"first\" of values to 4.\nSet item \"second\" of values to 5.\nLet saved be a new Map of Text to Int.\nShow change(item \"first\" of values).\nShow item \"first\" of saved.\nShow item \"second\" of saved.\nShow length of saved.\nShow item \"first\" of values.\n", "1\nnothing\n5\n2\n7");
}

#[test]
fn a_map_loan_captures_an_effectful_text_key_once() {
    check("## To key () -> Text:\n    Show \"key\".\n    Return \"first\".\n## To change (value: mutable Int) -> Int:\n    Show item \"first\" of values.\n    Set value to 7.\n    Return 1.\n## Main\nLet values be a new Map of Text to Int.\nSet item \"first\" of values to 4.\nSet item \"second\" of values to 5.\nShow change(item (key()) of values).\nShow item \"first\" of values.\n", "key\nnothing\n1\n7");
}

#[test]
fn a_map_loan_rebuilds_into_the_replaced_map() {
    check("## To change (value: mutable Int) -> Int:\n    Set values to replacement.\n    Set value to 7.\n    Return 1.\n## Main\nLet values be a new Map of Text to Int.\nSet item \"first\" of values to 4.\nSet item \"second\" of values to 5.\nLet replacement be a new Map of Text to Int.\nSet item \"first\" of replacement to 8.\nSet item \"second\" of replacement to 9.\nShow change(item \"first\" of values).\nShow item \"first\" of values.\nShow item \"second\" of values.\nShow item \"first\" of replacement.\n", "1\n7\n9\n8");
}

#[test]
fn a_nullable_map_can_be_returned_and_passed_read_only() {
    check("## No Inline\n## To save () -> Map of Text to Int:\n    Return values.\n## No Inline\n## To display (items: Map of Text to Int) -> Int:\n    Show item \"first\" of items.\n    Return length of items.\n## To change (value: mutable Int) -> Int:\n    Let copied be save().\n    Show display(copied).\n    Set value to 7.\n    Return 1.\n## Main\nLet values be a new Map of Text to Int.\nSet item \"first\" of values to 4.\nSet item \"second\" of values to 5.\nShow change(item \"first\" of values).\nShow item \"first\" of values.\n", "nothing\n2\n1\n7");
}

#[test]
fn a_map_can_lend_a_nested_sequence_element() {
    check("## To change (value: mutable Int) -> Int:\n    Show item \"first\" of values.\n    Set value to 7.\n    Return 1.\n## Main\nLet values be a new Map of Text to Seq of Int.\nSet item \"first\" of values to [4, 5].\nLet saved be values.\nShow change(item 1 of (item \"first\" of values)).\nShow item \"first\" of values.\nShow item \"first\" of saved.\n", "[nothing, 5]\n1\n[7, 5]\n[4, 5]");
}

#[test]
fn a_map_text_loan_retains_text_storage() {
    check("## To change (value: mutable Text) -> Int:\n    Show item \"first\" of values.\n    Set value to value + \"!\".\n    Return 1.\n## Main\nLet values be a new Map of Text to Text.\nSet item \"first\" of values to \"kept\".\nLet saved be values.\nShow change(item \"first\" of values).\nShow item \"first\" of values.\nShow item \"first\" of saved.\n", "nothing\n1\nkept!\nkept");
}

