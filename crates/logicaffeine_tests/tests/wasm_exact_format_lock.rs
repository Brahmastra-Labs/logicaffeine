//! Exact integer display preserves values across format, interpolation and failure.
#![cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod common;

const GROW: &str = "## NoInline\n## To grow (n: Int) -> Int:\n    Repeat for i from 1 to 7:\n        Set n to n * n.\n    Return n.\n";
fn check(body: &str) {
    let source = format!("{GROW}\n## Main\n{body}\n");
    let vm = logicaffeine_compile::compile::vm_outcome(&source);
    let got = common::wasm_host::run_outcome(&source)
        .expect("the exact display program compiles and runs");
    assert_eq!(
        got.output.lines().collect::<Vec<_>>(),
        vm.output.lines().collect::<Vec<_>>()
    );
    assert_eq!(got.failure, vm.error);
}
#[test]
fn format_preserves_a_held_integer() {
    check("Let n be grow(3).\nShow format(n).");
}
#[test]
fn a_failure_can_format_a_held_integer() {
    check("Let n be grow(3).\nFail with \"stopped at \" + format(n).");
}
#[test]
fn a_failure_can_display_a_held_integer_directly() {
    check("Let n be grow(3).\nFail with n.");
}
#[test]
fn formatting_a_loop_counter_preserves_its_value() {
    check("Let n be 0.\nWhile n is less than 3:\n    Show format(n).\n    Set n to n + 1.");
}
