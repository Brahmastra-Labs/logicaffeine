//! A mutable collection parameter lends the caller's value, including replacement.
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
const CHANGE: &str = "## To change (xs: mutable Seq of Int) -> Int:\n    Set xs to [7].\n    Return length of xs.\n";
#[test]
fn a_sequence_parameter_can_replace_the_callers_value() {
    check(&format!("{CHANGE}## Main\nLet xs be [4].\nLet saved be xs.\nShow change(xs).\nShow xs.\nShow saved.\n"), "1\n[7]\n[4]");
}
#[test]
fn a_forwarded_sequence_parameter_returns_its_replacement() {
    check(&format!("{CHANGE}## To relay (xs: mutable Seq of Int) -> Int:\n    Return change(xs).\n## Main\nLet xs be [4].\nLet saved be xs.\nShow relay(xs).\nShow xs.\nShow saved.\n"), "1\n[7]\n[4]");
}
#[test]
fn a_sequence_copy_made_inside_the_callee_keeps_its_value() {
    check("## To change (xs: mutable Seq of Int) -> Int:\n    Let saved be xs.\n    Push 9 to xs.\n    Show saved.\n    Return length of xs.\n## Main\nLet xs be [4].\nLet saved be xs.\nShow change(xs).\nShow xs.\nShow saved.\n", "[4]\n2\n[4, 9]\n[4]");
}
#[test]
fn a_map_parameter_can_replace_the_callers_value() {
    check("## To change (m: mutable Map of Text to Int) -> Int:\n    Set m to a new Map of Text to Int.\n    Set item \"x\" of m to 7.\n    Return length of m.\n## Main\nLet m be a new Map of Text to Int.\nSet item \"x\" of m to 4.\nLet saved be m.\nShow change(m).\nShow item \"x\" of m.\nShow item \"x\" of saved.\n", "1\n7\n4");
}
#[test]
fn a_map_copy_made_inside_the_callee_keeps_its_value() {
    check("## To change (m: mutable Map of Text to Int) -> Int:\n    Let saved be m.\n    Set item \"x\" of m to 9.\n    Show item \"x\" of saved.\n    Return length of m.\n## Main\nLet m be a new Map of Text to Int.\nSet item \"x\" of m to 4.\nLet saved be m.\nShow change(m).\nShow item \"x\" of m.\nShow item \"x\" of saved.\n", "4\n1\n9\n4");
}
