//! Nothing has its own value identity, independently of zero or an empty payload.
#![cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod common;

fn check(source: &str, expected: &[&str]) {
    let vm = logicaffeine_compile::compile::vm_outcome(source);
    assert_eq!(vm.error, None, "the reference fixture must run");
    assert_eq!(vm.output.lines().collect::<Vec<_>>(), expected);
    let wasm = common::wasm_host::run_outcome(source).expect("valid Nothing-value module");
    assert_eq!(wasm.failure, None);
    assert_eq!(wasm.output.lines().collect::<Vec<_>>(), expected);
}

#[test]
fn nothing_displays_as_nothing_beside_zero_and_false() {
    check(
        "## Main\nShow nothing.\nShow 0.\nShow false.\nShow nothing.\n",
        &["nothing", "0", "false", "nothing"],
    );
}

#[test]
fn nothing_and_integer_zero_compare_by_value_kind_in_both_orders() {
    check(
        "## Main\nShow 0 is equal to nothing.\nShow nothing is equal to 0.\nShow 0 is not equal to nothing.\nShow nothing is not equal to 0.\nShow nothing is equal to nothing.\nShow nothing is not equal to nothing.\n",
        &["false", "false", "true", "true", "true", "false"],
    );
}

#[test]
fn nothing_and_false_compare_by_value_kind_in_both_orders() {
    check(
        "## Main\nShow false is equal to nothing.\nShow nothing is equal to false.\nShow false is not equal to nothing.\nShow nothing is not equal to false.\n",
        &["false", "false", "true", "true"],
    );
}

#[test]
fn empty_text_and_empty_list_are_present_values() {
    check(
        "## Main\nLet text be \"\".\nLet items be a new Seq of Int.\nShow text is equal to nothing.\nShow nothing is equal to text.\nShow items is equal to nothing.\nShow nothing is equal to items.\n",
        &["false", "false", "false", "false"],
    );
}

fn compare_exact_integer_with_nothing(start: i64) {
    let source = format!(
        r#"## No Inline
## To grow (start: Int) -> Int:
    Let value be start.
    Repeat for i from 1 to 7:
        Set value to value * value.
    Return value.

## Main
Let value be grow({start}).
Show value is equal to nothing.
Show nothing is equal to value.
Show value is not equal to nothing.
Show nothing is not equal to value.
"#
    );
    check(&source, &["false", "false", "true", "true"]);
}

#[test]
fn an_exact_integer_zero_remains_a_present_value() {
    compare_exact_integer_with_nothing(0);
}

#[test]
fn a_large_exact_integer_can_be_compared_with_nothing_without_narrowing() {
    compare_exact_integer_with_nothing(3);
}

#[test]
fn nested_mutable_calls_restore_integer_float_and_heap_parameters() {
    for (ty, initial, first, inner, last, expected) in [
        ("Int", "4", "1", "3", "2", "17"),
        ("Float", "4.5", "1.0", "3.0", "2.0", "18.5"),
    ] {
        let source = format!(
            r#"## To inner (y: mutable {ty}):
    Set y to y * {inner}.

## To outer (x: mutable {ty}):
    Set x to x + {first}.
    inner(x).
    Set x to x + {last}.

## Main
Let mutable value be {initial}.
outer(value).
Show value.
"#
        );
        check(&source, &[expected]);
    }
    check(
        r#"## To inner (y: mutable Text):
    Set y to y + "B".

## To outer (x: mutable Text):
    Set x to x + "A".
    inner(x).
    Set x to x + "C".

## Main
Let mutable value be "".
outer(value).
Show value.
"#,
        &["ABC"],
    );
}
