//! A mutable loan makes the global place absent, independently of its payload kind.
#![cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod common;

fn check(ty: &str, declaration: &str, initialization: &str) {
    let source = format!(
        r#"## No Inline
{declaration}
## To touch (value: mutable {ty}) -> {ty}:
    Let observed be saved.
    Show observed.
    Show observed is equal to nothing.
    Return value.

## Main
{initialization}
Show saved is equal to nothing.
Show touch(saved).
Show saved.
"#
    );
    let expected = logicaffeine_compile::compile::vm_outcome(&source);
    assert_eq!(expected.error, None, "reference fixture must run");
    let reference: Vec<_> = expected.output.lines().collect();
    assert_eq!(
        &reference[..3],
        &["false", "nothing", "true"],
        "the source must distinguish a present payload from its absent global place"
    );
    let got = common::wasm_host::run_outcome(&source).expect("valid global loan module");
    assert_eq!(
        got.failure, None,
        "reading an absent place is a value, not a trap"
    );
    assert_eq!(
        got.output,
        expected
            .output
            .strip_suffix('\n')
            .unwrap_or(&expected.output)
    );
}

#[test]
fn zero_int_is_present_before_and_after_its_loan() {
    check("Int", "", "Let saved be 0.");
}

#[test]
fn empty_text_is_present_before_and_after_its_loan() {
    check("Text", "", "Let saved be \"\".");
}

#[test]
fn nonempty_text_is_absent_during_its_loan() {
    check("Text", "", "Let saved be \"hello\".");
}

#[test]
fn empty_list_is_present_before_and_after_its_loan() {
    check("Seq of Int", "", "Let saved be a new Seq of Int.");
}

#[test]
fn nonempty_list_is_absent_during_its_loan() {
    check("Seq of Int", "", "Let saved be [41, 42].");
}

#[test]
fn record_is_absent_during_its_loan() {
    check(
        "Box",
        "## A Box has:\n    a value, which is Int.\n",
        "Let saved be a new Box with value 42.",
    );
}

#[test]
fn map_is_absent_during_its_loan() {
    check(
        "Map of Int to Int",
        "",
        "Let saved be a new Map of Int to Int.\nSet saved at 1 to 42.",
    );
}
