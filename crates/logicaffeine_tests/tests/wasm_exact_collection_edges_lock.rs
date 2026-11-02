//! Exact integer collection storage survives copies, replacement and calls.
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

fn agrees(body: &str) {
    let source = format!("{GROW}{body}");
    let expected = logicaffeine_compile::compile::vm_outcome(&source);
    assert_eq!(expected.error, None, "reference program must run");
    let got = common::wasm_host::run_outcome(&source).expect("valid exact-value module");
    assert_eq!(got.failure, None, "collection storage must preserve unbounded Int values");
    assert_eq!(
        got.output.lines().collect::<Vec<_>>(),
        expected.output.lines().collect::<Vec<_>>(),
    );
}

#[test]
fn replacing_an_exact_list_element_preserves_its_copy() {
    agrees(r#"## Main
Let values be [grow(3)].
Let earlier be copy of values.
Set item 1 of values to grow(5).
Show item 1 of earlier.
Show item 1 of values.
"#);
}

#[test]
fn pushing_an_exact_value_preserves_existing_small_elements() {
    agrees(r#"## Main
Let values be [41].
Push grow(3) to values.
Show item 1 of values.
Show item 2 of values.
"#);
}

#[test]
fn iteration_reads_exact_integer_elements() {
    agrees(r#"## Main
Let values be [grow(3), grow(5)].
Repeat for value in values:
    Show value.
"#);
}

#[test]
fn a_returned_integer_sequence_keeps_its_exact_elements() {
    agrees(r#"## To collect (value: Int) -> Seq of Int:
    Return [value].

## Main
Let values be collect(grow(3)).
Show item 1 of values.
"#);
}

#[test]
fn showing_an_integer_sequence_formats_values_instead_of_handles() {
    agrees(r#"## Main
Let values be [41, grow(3)].
Show values.
"#);
}

#[test]
fn replacing_an_exact_map_value_preserves_its_copy() {
    agrees(r#"## Main
Let values be a new Map of Int to Int.
Set values at 1 to grow(3).
Let earlier be copy of values.
Set values at 1 to grow(5).
Show item 1 of earlier.
Show item 1 of values.
"#);
}

#[test]
fn equal_large_integer_map_keys_name_the_same_entry() {
    agrees(r#"## Main
Let values be a new Map of Int to Int.
Let key be grow(3).
Set values at key to 41.
Set values at grow(3) to 42.
Show length of values.
Show item key of values.
"#);
}
