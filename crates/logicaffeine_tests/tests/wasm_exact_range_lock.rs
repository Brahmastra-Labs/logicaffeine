//! Range frames capture numeric bounds before temporary exact-to-word conversion is restored.
#![cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod common;
fn check(body: &str) {
    let source = format!(
        "## No Inline\n## To twice (n: Int) -> Int:\n    Return n + n.\n\n## Main\n{body}\n"
    );
    let program =
        logicaffeine_compile::compile::compile_to_program(&source).expect("valid range program");
    assert!(
        program
            .code
            .iter()
            .any(|op| matches!(op, logicaffeine_compile::vm::Op::NewRange { .. })),
        "exercise the range-frame lowering, not a counted loop"
    );
    let expected = logicaffeine_compile::compile::vm_outcome(&source);
    assert_eq!(expected.error, None);
    let got = common::wasm_host::run_outcome(&source).expect("valid range module");
    assert_eq!(got.failure, None);
    assert_eq!(
        got.output.lines().collect::<Vec<_>>(),
        expected.output.lines().collect::<Vec<_>>()
    );
}
#[test]
fn a_call_result_is_the_numeric_upper_bound() {
    check("Let total be 0.\nRepeat for i from 1 to twice(3):\n    Set total to total + i.\nShow total.");
}
#[test]
fn both_bounds_can_be_call_results() {
    check("Repeat for i from twice(1) to twice(3):\n    Show i.");
}
#[test]
fn an_empty_range_keeps_its_bounds_in_numeric_order() {
    check("Let total be 0.\nRepeat for i from twice(4) to twice(2):\n    Set total to total + 1.\nShow total.");
}
#[test]
fn negative_bounds_keep_their_signed_values() {
    check("Repeat for i from twice(0 - 2) to twice(0 - 1):\n    Show i.");
}
#[test]
fn a_held_integer_bound_is_captured_as_its_value() {
    check(
        "Repeat for i from twice(4611686018427387903) to twice(4611686018427387903):\n    Show i.",
    );
}
#[test]
fn the_body_cannot_change_the_captured_call_bound() {
    check("Let n be 3.\nRepeat for i from 1 to twice(n):\n    Show i.\n    Set n to 0.");
}
