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
fn a_nullable_field_copies_a_live_text_builder() {
    check("## A Box has:\n    a value, which is Text.\n## To source (n: Int) -> Text:\n    Return \"x\" + n.\n## To change (value: mutable Text) -> Int:\n    Show b's value.\n    Set value to value + \"!\".\n    Return length of value.\n## Main\nLet text be source(7).\nLet b be a new Box with value text.\nSet text to text + \"?\".\nShow text.\nShow b's value.\nShow change(b's value).\nShow b's value.\nShow text.\n", "x7?\nx7\nnothing\n3\nx7!\nx7?");
}
