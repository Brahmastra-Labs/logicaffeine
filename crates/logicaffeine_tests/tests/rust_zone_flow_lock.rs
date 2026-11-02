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
fn break_in_a_zone_leaves_the_zone() {
    check("## Main\nInside a zone called \"Z\":\n    Show 1.\n    Break.\n    Show 2.\nShow 3.\n", "1\n3");
}

#[test]
fn break_in_a_loop_in_a_zone_leaves_the_loop() {
    check("## Main\nInside a zone called \"Z\":\n    Repeat for i from 1 to 3:\n        If i is 2:\n            Break.\n        Show i.\n    Show \"rest of zone\".\nShow \"after\".\n", "1\nrest of zone\nafter");
}

#[test]
fn return_in_a_zone_leaves_the_zone_not_the_function() {
    check("## To f -> Int:\n    Inside a zone called \"Z\":\n        Show \"in\".\n        Return 5.\n    Return 7.\n\n## Main\nShow f().\n", "in\n7");
}

#[test]
fn return_from_a_loop_in_a_zone_leaves_the_zone() {
    check("## Main\nLet mutable n be 0.\nInside a zone called \"Z\":\n    Repeat for i from 1 to 5:\n        If i is 3:\n            Return.\n        Set n to n + i.\nShow n.\n", "3");
}

#[test]
fn a_value_used_after_a_zone_break_remains_live() {
    check("## A Box has:\n    a value, which is Text.\n## No Inline\n## To display (text: Text) -> Int:\n    Repeat for i from 1 to 1:\n        Inside a zone called \"Scratch\":\n            Let stored be a new Box with value text.\n            Show stored's value.\n            Break.\n        Show text.\n    Return 1.\n## Main\nShow display(\"kept\").\n", "kept\nkept\n1");
}
