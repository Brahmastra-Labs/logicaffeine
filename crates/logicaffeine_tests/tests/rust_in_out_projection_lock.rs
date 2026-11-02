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
const BUMP: &str = "## To bump (value: mutable Int) -> Int:\n    Set value to value + 1.\n    Return value.\n";

#[test]
fn a_key_can_read_the_sequence_it_indexes() {
    check(&format!("{BUMP}## Main\nLet xs be [4, 7].\nLet saved be xs.\nShow bump(item (length of xs) of xs).\nShow xs.\nShow saved.\n"), "8\n[4, 8]\n[4, 7]");
}
#[test]
fn an_effectful_key_is_evaluated_once() {
    check(&format!("{BUMP}## To position () -> Int:\n    Set calls to calls + 1.\n    Return 2.\n## Main\nLet calls be 0.\nLet xs be [4, 7].\nLet saved be xs.\nShow bump(item (position()) of xs).\nShow calls.\nShow xs.\nShow saved.\n"), "8\n1\n[4, 8]\n[4, 7]");
}
#[test]
fn a_negative_key_updates_the_last_element() {
    check(&format!("{BUMP}## Main\nLet xs be [4, 7].\nLet saved be xs.\nShow bump(item -1 of xs).\nShow xs.\nShow saved.\n"), "8\n[4, 8]\n[4, 7]");
}
#[test]
fn a_text_element_retains_its_new_buffer() {
    check("## To append (s: mutable Text) -> Int:\n    Set s to s + \"!\".\n    Return length of s.\n## Main\nLet xs be [\"ok\"].\nLet saved be xs.\nShow append(item 1 of xs).\nShow xs.\nShow saved.\n", "3\n[ok!]\n[ok]");
}
#[test]
fn a_nested_record_field_updates_only_its_owner() {
    check(&format!("{BUMP}## A Box has:\n    a value, which is Int.\n## A Outer has:\n    an inner, which is Box.\n## Main\nLet b be a new Box with value 4.\nLet o be a new Outer with inner b.\nLet saved be o.\nShow bump(o's inner's value).\nShow o's inner's value.\nShow saved's inner's value.\n"), "5\n5\n4");
}
#[test]
fn a_lent_inner_sequence_preserves_a_copied_outer_sequence() {
    check("## To append (xs: mutable Seq of Int) -> Int:\n    Push 9 to xs.\n    Return length of xs.\n## Main\nLet xs be [[4]].\nLet saved be xs.\nShow append(item 1 of xs).\nShow xs.\nShow saved.\n", "2\n[[4, 9]]\n[[4]]");
}
