//! Exact map keys compare by value and each map owns the integers it keeps.
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

fn agrees(body: &str) -> u32 {
    let source = format!("{GROW}{body}");
    let expected = logicaffeine_compile::compile::vm_outcome(&source);
    assert_eq!(expected.error, None, "{source}");
    let held = common::wasm_host::held_integers();
    let got = common::wasm_host::run_outcome(&source).expect("exact map module");
    assert_eq!(got.failure, None, "{source}");
    assert_eq!(
        got.output.lines().collect::<Vec<_>>(),
        expected.output.lines().collect::<Vec<_>>(),
        "{source}"
    );
    assert_eq!(
        common::wasm_host::held_integers(),
        held,
        "map cells release every host integer"
    );
    got.heap_ptr.unwrap_or(0)
}

#[test]
fn exact_keys_compare_by_value_for_contains_and_removal() {
    agrees(
        r#"## Main
Let largeKey3 be grow(3).
Let largeKey7 be grow(7).
Let values be a new Map of Int to Int.
Set item largeKey3 of values to grow(5).
Set item largeKey7 of values to 42.
Show values contains grow(3).
Remove grow(3) from values.
Show values contains grow(3).
Show item largeKey7 of values.
Show length of values.
"#,
    );
}

#[test]
fn a_copy_keeps_its_cells_after_removal_and_replacement() {
    agrees(
        r#"## Main
Let largeKey3 be grow(3).
Let values be a new Map of Int to Int.
Set item largeKey3 of values to grow(5).
Let saved be copy of values.
Remove grow(3) from values.
Set item largeKey3 of values to grow(7).
Show item largeKey3 of saved.
Show item largeKey3 of values.
"#,
    );
}

#[test]
fn exact_values_coexist_with_text_keys() {
    agrees(
        r#"## Main
Let values be a new Map of Text to Int.
Set item "first" of values to 41.
Set item "second" of values to grow(3).
Show item "first" of values.
Show item "second" of values.
Show values.
"#,
    );
}

#[test]
fn exact_keys_coexist_with_owned_sequence_values() {
    agrees(
        r#"## Main
Let largeKey3 be grow(3).
Let values be a new Map of Int to (Seq of Int).
Set item largeKey3 of values to [41, 42].
Let saved be copy of values.
Set item largeKey3 of values to [51, 52].
Show item largeKey3 of saved.
Show item largeKey3 of values.
Remove grow(3) from values.
Show length of values.
"#,
    );
}

#[test]
fn map_arguments_and_results_preserve_exact_values() {
    agrees(
        r#"## No Inline
## To collect (value: Int) -> Map of Int to Int:
    Let values be a new Map of Int to Int.
    Set item value of values to value + 1.
    Return values.

## No Inline
## To read (values: Map of Int to Int) and (key: Int) -> Int:
    Return item key of values.

## Main
Let key be grow(3).
Let values be collect(key).
Show read(values, key).
Show values.
"#,
    );
}

#[test]
fn small_map_functions_work_beside_an_exact_map() {
    agrees(
        r#"## No Inline
## To small (value: Int) -> Int:
    Let values be a new Map of Int to Int.
    Set item 1 of values to value.
    Return item 1 of values.

## Main
Let largeKey3 be grow(3).
Let values be a new Map of Int to Int.
Set item largeKey3 of values to grow(5).
Show small(41).
Show item largeKey3 of values.
"#,
    );
}

#[test]
fn repeated_map_copy_replace_and_remove_keep_a_flat_heap() {
    let mut heaps = Vec::new();
    for turns in [100, 1000] {
        heaps.push(agrees(&format!(
            r#"## Main
Let key be grow(3).
Let value be grow(5).
Repeat for i from 1 to {turns}:
    Let values be a new Map of Int to Int.
    Set item key of values to value.
    Let saved be copy of values.
    Set item key of values to value + 1.
    Remove key from values.
    Let observed be item key of saved.
Show "done".
"#
        )));
    }
    assert_eq!(
        heaps[0], heaps[1],
        "ten times the map work keeps the same heap bound"
    );
}

#[test]
fn shared_text_key_maps_keep_sorted_entries_beside_exact_maps() {
    agrees(
        r#"## Definition
An Inventory is Shared and has:
    an items, which is a SharedMap from Text to Int.

## Main
Let largeKey3 be grow(3).
Let values be a new Map of Int to Int.
Set item largeKey3 of values to grow(5).
Let inv be a new Inventory.
Set inv's items["wood"] to 50.
Set inv's items["stone"] to 30.
Show inv's items["wood"].
Show inv's items["stone"].
Show inv's items.
"#,
    );
}

#[test]
fn shared_maps_preserve_the_reference_scalar_domain() {
    for assignment in [
        "Set inv's items[grow(3)] to 42.",
        "Set inv's items[42] to grow(3).",
    ] {
        let source = format!("{GROW}## Definition\nAn Inventory is Shared and has:\n    an items, which is a SharedMap from Int to Int.\n\n## Main\nLet inv be a new Inventory.\n{assignment}\n");
        let expected = logicaffeine_compile::compile::vm_outcome(&source);
        assert!(
            expected.error.is_some(),
            "the reference rejects a wide shared scalar"
        );
        let got = common::wasm_host::run_outcome(&source).expect("shared scalar guard module");
        assert!(
            got.failure.is_some(),
            "WASM must preserve the same shared scalar domain"
        );
    }
}
