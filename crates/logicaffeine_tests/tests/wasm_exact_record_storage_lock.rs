//! Integer record fields preserve exact values, copies and bounded ownership.
#![cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod common;
const PRELUDE: &str = r#"## A Box has:
    a value, which is Int.
## No Inline
## To grow (start: Int) -> Int:
    Let n be start.
    Repeat for i from 1 to 7:
        Set n to n * n.
    Return n.
"#;
fn check(body: &str) {
    let source = format!("{PRELUDE}\n{body}\n");
    let vm = logicaffeine_compile::compile::vm_outcome(&source);
    assert_eq!(vm.error, None, "valid reference program");
    let got = common::wasm_host::run_outcome(&source).expect("valid record module");
    assert_eq!(got.failure, None, "a record field holds an unbounded Int");
    assert_eq!(
        got.output.lines().collect::<Vec<_>>(),
        vm.output.lines().collect::<Vec<_>>()
    );
}
#[test]
fn replacing_a_large_field_preserves_the_saved_record() {
    check("## Main\nLet box be a new Box with value grow(3).\nLet saved be box.\nSet box's value to grow(5).\nShow saved's value.\nShow box's value.");
}
#[test]
fn overwriting_a_large_field_with_a_small_value_keeps_the_copy() {
    check("## Main\nLet box be a new Box with value grow(3).\nLet saved be box.\nSet box's value to 7.\nShow saved's value.\nShow box's value.");
}
#[test]
fn a_record_return_and_parameter_preserve_its_integer() {
    check("## To makebox (n: Int) -> Box:\n    Return a new Box with value grow(n).\n## To readbox (box: Box) -> Int:\n    Return box's value.\n## Main\nLet box be makebox(3).\nShow readbox(box).");
}
#[test]
fn a_field_read_can_continue_exact_arithmetic() {
    check("## Main\nLet box be a new Box with value grow(3).\nShow box's value + 1.");
}
#[test]
fn small_record_fields_keep_their_values() {
    check("## Main\nLet box be a new Box with value 7.\nSet box's value to 9.\nShow box's value.");
}
#[test]
fn repeated_exact_field_replacement_reclaims_cells() {
    let mut heaps = Vec::new();
    for count in [100, 1000] {
        let source = format!("{PRELUDE}\n## No Inline\n## To churn (turns: Int) -> Int:\n    Let box be a new Box with value 0.\n    Repeat for i from 1 to turns:\n        Set box's value to grow(3).\n        Set box's value to 7.\n    Return box's value.\n## Main\nShow churn({count}).\n");
        let before = common::wasm_host::held_integers();
        let got = common::wasm_host::run_outcome(&source).expect("valid record soak");
        assert_eq!(got.failure, None);
        assert_eq!(got.output, "7");
        assert_eq!(
            common::wasm_host::held_integers(),
            before,
            "record integer cells gave every host value back"
        );
        heaps.push(got.heap_ptr.expect("record heap"));
    }
    assert_eq!(
        heaps[0], heaps[1],
        "fixed live record workload needs fixed heap space"
    );
}

#[test]
fn showing_the_whole_record_uses_its_exact_field() {
    check("## Main\nLet box be a new Box with value grow(3).\nShow box.");
}
#[test]
fn an_explicit_record_copy_owns_its_exact_field() {
    check("## Main\nLet box be a new Box with value grow(3).\nLet saved be copy of box.\nSet box's value to 7.\nShow saved's value.\nShow box's value.");
}

#[test]
fn a_list_of_records_displays_exact_fields() {
    check("## Main\nLet boxes be [a new Box with value grow(3), a new Box with value grow(5)].\nShow boxes.\nRepeat for box in boxes:\n    Show box's value.");
}
#[test]
fn formatting_a_record_displays_its_exact_field() {
    check("## Main\nLet box be a new Box with value grow(3).\nShow format(box).");
}
#[test]
fn small_and_exact_fields_coexist_in_the_same_record() {
    check("## A Pair has:\n    a first, which is Int.\n    a second, which is Int.\n## Main\nLet pair be a new Pair with first 7 and second grow(3).\nShow pair.\nShow pair's first.\nShow pair's second.");
}
#[test]
fn a_nested_record_preserves_exact_fields_after_copy() {
    check("## A Wrapper has:\n    a child, which is Box.\n## Main\nLet box be a new Box with value grow(3).\nLet wrapper be a new Wrapper with child box.\nLet saved be copy of wrapper.\nSet box's value to 7.\nLet child be saved's child.\nShow child's value.\nShow saved.");
}

#[test]
fn a_record_copy_outlives_its_original_frame() {
    check("## No Inline\n## To makecopy () -> Box:\n    Let box be a new Box with value grow(3).\n    Return copy of box.\n## Main\nLet saved be makecopy().\nLet noise be grow(5).\nShow saved's value.\nShow noise.");
}
#[test]
fn a_saved_record_copy_survives_repeated_safepoints() {
    check("## No Inline\n## To makecopy () -> Box:\n    Let box be a new Box with value grow(3).\n    Return copy of box.\n## Main\nLet saved be makecopy().\nRepeat for i from 1 to 20:\n    Let other be a new Box with value grow(5).\n    Show other's value.\nShow saved's value.");
}
