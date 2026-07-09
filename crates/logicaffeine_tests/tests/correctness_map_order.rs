//! Part I correctness: map iteration is INSERTION-ORDERED, on every engine.
//!
//! Today the interpreter/VM back maps with `FxHashMap` and the AOT-emitted
//! Rust with `LogosMap(FxHashMap)` — iteration order is arbitrary, differs
//! between engines, and (for the audit's `Repeat for (k,v) in m:` row) can
//! diverge run-to-run. The spec: a LOGOS map remembers insertion order, like
//! Python dicts and JS objects — display, iteration, and keys all follow it.
//! (The direct-WASM backend already stores maps as a linear entry array in
//! insertion order; this brings the other engines into line with it.)

mod common;
use common::assert_compiled_equals_interpreted_eq;

#[test]
fn map_displays_in_insertion_order() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let mutable m be a new Map of Text to Int.
Set item "banana" of m to 2.
Set item "apple" of m to 1.
Set item "cherry" of m to 3.
Show m.
"#,
        r#"{banana: 2, apple: 1, cherry: 3}"#,
    );
}

#[test]
fn map_iterates_in_insertion_order() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let mutable m be a new Map of Text to Int.
Set item "banana" of m to 2.
Set item "apple" of m to 1.
Set item "cherry" of m to 3.
Repeat for (k, v) in m:
    Show k.
"#,
        "banana\napple\ncherry",
    );
}

#[test]
fn map_order_survives_overwrite() {
    // Overwriting an existing key must keep its ORIGINAL position (IndexMap
    // semantics, same as Python) — not move it to the end.
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let mutable m be a new Map of Text to Int.
Set item "banana" of m to 2.
Set item "apple" of m to 1.
Set item "banana" of m to 9.
Show m.
"#,
        r#"{banana: 9, apple: 1}"#,
    );
}

#[test]
fn int_keyed_map_is_insertion_ordered_too() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let mutable m be a new Map of Int to Text.
Set item 30 of m to "c".
Set item 10 of m to "a".
Set item 20 of m to "b".
Repeat for (k, v) in m:
    Show v.
"#,
        "c\na\nb",
    );
}
