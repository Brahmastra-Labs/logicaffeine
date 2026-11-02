//! A value-only leaf prelude cannot replace a call that lends mutable places.
use logicaffeine_compile::compile::{tw_outcome, vm_outcome, vm_outcome_tiered};
use logicaffeine_compile::optimization::Tier;

fn check(source: &str, expected: &str) {
    for (name, run) in [
        ("tree walker", tw_outcome(source)),
        ("VM", vm_outcome(source)),
        (
            "optimized VM",
            vm_outcome_tiered(source, &[], Tier::T3, None),
        ),
    ] {
        assert_eq!(run.error, None, "{name}: {run:?}");
        assert_eq!(run.output.trim_end_matches('\n'), expected, "{name}");
    }
}

#[test]
fn a_mutable_scalar_retains_its_caller_write() {
    check("## To bump (value: mutable Int) -> Int:\n    Set value to value + 1.\n    Return value.\n## Main\nLet n be 4.\nShow bump(n).\nShow n.\n", "5\n5");
}

#[test]
fn a_forwarded_mutable_scalar_retains_both_writes() {
    check("## To bump (value: mutable Int) -> Int:\n    Set value to value + 1.\n    Return value.\n## To twice (value: mutable Int) -> Int:\n    Let first be bump(value).\n    Return bump(value).\n## Main\nLet n be 4.\nShow twice(n).\nShow n.\n", "6\n6");
}

#[test]
fn two_mutable_parameters_return_both_values() {
    check("## To change (left: mutable Int, right: mutable Int) -> Int:\n    Set left to left + 2.\n    Set right to right + 3.\n    Return left + right.\n## Main\nLet a be 1.\nLet b be 2.\nShow change(a, b).\nShow a.\nShow b.\n", "8\n3\n5");
}

#[test]
fn a_mutable_text_returns_its_new_value_and_preserves_an_alias() {
    check("## To append (value: mutable Text) -> Int:\n    Set value to value + \"!\".\n    Return length of value.\n## Main\nLet s be \"hello\".\nLet saved be s.\nShow append(s).\nShow s.\nShow saved.\n", "6\nhello!\nhello");
}

#[test]
fn a_value_parameter_keeps_its_callers_value() {
    check("## To changed (value: Int) -> Int:\n    Set value to value + 1.\n    Return value.\n## Main\nLet n be 4.\nShow changed(n).\nShow n.\n", "5\n4");
}
