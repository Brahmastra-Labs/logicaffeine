//! Unbounded Int values preserve their value when moved into language-level places.
#![cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod common;

const GROW: &str = r#"## No Inline
## To grow (start: Int) -> Int:
    Let mutable value be start.
    Repeat for i from 1 to 7:
        Set value to value * value.
    Return value.

"#;

fn agrees(source: &str) {
    let expected = logicaffeine_compile::compile::vm_outcome(source);
    assert_eq!(expected.error, None, "reference program must run");
    let got = common::wasm_host::run_outcome(source).expect("valid exact-value module");
    assert_eq!(
        got.failure, None,
        "storing an Int must preserve the language's unbounded value"
    );
    assert_eq!(
        got.output.lines().collect::<Vec<_>>(),
        expected.output.lines().collect::<Vec<_>>()
    );
}

#[test]
fn an_exact_integer_survives_a_list_element() {
    agrees(&format!(
        "{GROW}## Main\nLet value be grow(3).\nLet values be [value].\nShow item 1 of values.\n"
    ));
}

#[test]
fn an_exact_integer_survives_a_record_field() {
    agrees(&format!("{GROW}## A Box has:\n    a value, which is Int.\n\n## Main\nLet value be grow(3).\nLet box be a new Box with value value.\nShow box's value.\n"));
}

#[test]
fn an_exact_integer_survives_a_map_value() {
    agrees(&format!("{GROW}## Main\nLet value be grow(3).\nLet values be a new Map of Int to Int.\nSet values at 1 to value.\nShow item 1 of values.\n"));
}

#[test]
fn a_mutable_global_is_absent_while_its_value_is_lent() {
    agrees(
        r#"## No Inline
## To grow (value: mutable Int) -> Int:
    Show n.
    Repeat for i from 1 to 7:
        Set value to value * value.
    Return value.

## Main
Let n be 3.
Show grow(n).
Show n.
"#,
    );
}

#[test]
fn small_values_keep_the_same_place_semantics() {
    agrees(
        r#"## A Box has:
    a value, which is Int.

## Main
Let values be [41].
Let box be a new Box with value 42.
Let table be a new Map of Int to Int.
Set table at 1 to 43.
Show item 1 of values.
Show box's value.
Show item 1 of table.
"#,
    );
}
