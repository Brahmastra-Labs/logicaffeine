//! Explicit copies preserve the collection's declared key and value layouts.
#![cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod common;

fn agrees(source: &str, expected: &[&str]) {
    let reference = logicaffeine_compile::compile::vm_outcome(source);
    assert_eq!(reference.error, None, "valid reference program");
    assert_eq!(reference.output.lines().collect::<Vec<_>>(), expected);
    let actual =
        common::wasm_host::run_outcome(source).expect("copied collection retains its layout");
    assert_eq!(actual.failure, None);
    assert_eq!(actual.output.lines().collect::<Vec<_>>(), expected);
}

#[test]
fn copying_an_integer_map_preserves_its_value_kind() {
    agrees(
        r#"## Main
Let values be a new Map of Int to Int.
Set values at 1 to 41.
Let earlier be copy of values.
Set values at 1 to 99.
Show item 1 of earlier.
Show item 1 of values.
"#,
        &["41", "99"],
    );
}

#[test]
fn copying_a_text_map_preserves_its_key_and_value_kinds() {
    agrees(
        r#"## Main
Let values be a new Map of Text to Text.
Set values at "key" to "old".
Let earlier be copy of values.
Set values at "key" to "new".
Show item "key" of earlier.
Show item "key" of values.
"#,
        &["old", "new"],
    );
}

#[test]
fn assigning_a_map_preserves_layout_and_value_semantics() {
    agrees(
        r#"## Main
Let values be a new Map of Int to Int.
Set values at 1 to 41.
Let earlier be values.
Set values at 1 to 99.
Show item 1 of earlier.
Show item 1 of values.
"#,
        &["41", "99"],
    );
}
