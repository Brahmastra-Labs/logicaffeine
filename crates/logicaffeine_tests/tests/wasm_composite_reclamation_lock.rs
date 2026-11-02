//! Composite values must own their children across collection, including non-header layouts.
#![cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod common;
use common::wasm_host::run_outcome;
use logicaffeine_compile::compile::vm_outcome;

const CHURN: &str = r#"## To churn (n: Int) -> Int:
    Let mutable total be 0.
    Repeat for i from 1 to n:
        Let xs be [i, i + 1].
        Set total to total + item 1 of xs.
    Return total.

"#;

fn exact(source: &str, expected: &str) -> u32 {
    let oracle = vm_outcome(source);
    assert_eq!(oracle.error, None, "valid VM program:\n{source}");
    assert_eq!(
        oracle.output.lines().collect::<Vec<_>>(),
        expected.lines().collect::<Vec<_>>()
    );
    let got = run_outcome(source).expect("valid wasm module");
    assert_eq!(got.failure, None, "wasm must run:\n{source}");
    assert_eq!(
        got.output, expected,
        "composite ownership preserves values:\n{source}"
    );
    got.heap_ptr.expect("allocating program has a heap")
}

fn flat(declarations: &str, body: &str) {
    let mut heaps = Vec::new();
    for n in [100_u64, 1000] {
        let src = format!("{declarations}## Main\nLet mutable total be 0.\nRepeat for i from 1 to {n}:\n{body}Show total.\n");
        heaps.push(exact(&src, &(n * (n + 1) / 2).to_string()));
    }
    assert_eq!(
        heaps[0], heaps[1],
        "dead composites must return their storage"
    );
}

#[test]
fn a_heterogeneous_tuple_keeps_its_text_child_alive() {
    exact(&format!("{CHURN}## Main\nLet kept be (73, \"v\" combined with 7).\nShow churn(100).\nShow item 1 of kept.\nShow item 2 of kept.\n"), "5050\n73\nv7");
}

#[test]
fn a_dropped_heterogeneous_tuple_releases_its_header_and_children() {
    flat(
        "",
        "    Let pair be (i, \"v\" combined with 7).\n    Set total to total + item 1 of pair.\n",
    );
}

#[test]
fn an_enum_keeps_its_text_payload_alive() {
    exact(&format!("## A Parcel is one of:\n    A Packet with label Text.\n\n{CHURN}## Main\nLet kept be a new Packet with label (\"v\" combined with 7).\nShow churn(100).\nInspect kept:\n    When Packet (label): Show label.\n"), "5050\nv7");
}

#[test]
fn a_dropped_enum_releases_its_storage() {
    flat("## A Parcel is one of:\n    A Packet with value Int.\n\n", "    Let packet be a new Packet with value i.\n    Inspect packet:\n        When Packet (value): Set total to total + value.\n");
}

#[test]
fn an_enum_payload_cannot_be_mistaken_for_a_reference_count_header() {
    exact(&format!("## A Parcel is one of:\n    A Packet with value Int.\n\n{CHURN}## Main\nLet kept be [a new Packet with value 4294967296].\nShow churn(100).\nRepeat for packet in kept:\n    Inspect packet:\n        When Packet (value): Show value.\n"), "5050\n4294967296");
}

#[test]
fn a_record_field_holding_an_enum_preserves_its_payload_bits() {
    exact(&format!("## A Parcel is one of:\n    A Packet with value Int.\n\n## A Box has:\n    a parcel, which is Parcel.\n\n{CHURN}## Main\nLet kept be a new Box with parcel (a new Packet with value 4294967296).\nShow churn(100).\nInspect kept's parcel:\n    When Packet (value): Show value.\n"), "5050\n4294967296");
}

#[test]
fn a_shared_text_set_keeps_its_header_after_its_record_dies() {
    exact(&format!("## A Party is Shared and has:\n    a guests, which is a SharedSet of Text.\n\n{CHURN}## Main\nLet mutable party be a new Party.\nAdd (\"g\" combined with 7) to party's guests.\nLet kept be party's guests.\nShow churn(100).\nShow length of kept.\nIf kept contains \"g7\":\n    Show \"kept\".\n"), "5050\n1\nkept");
}

#[test]
fn a_dropped_enum_releases_its_text_payload() {
    flat("## A Parcel is one of:\n    A Packet with value Int and label Text.\n\n", "    Let packet be a new Packet with value i and label (\"v\" combined with 7).\n    Inspect packet:\n        When Packet (value, label): Set total to total + value.\n");
}

#[test]
fn a_dropped_nullary_enum_releases_its_header() {
    flat("## A Parcel is one of:\n    An Empty.\n\n", "    Let packet be a new Empty.\n    Inspect packet:\n        When Empty: Set total to total + i.\n");
}

#[test]
fn recursive_enums_release_the_entire_owned_tree() {
    let declarations = r#"## A Tree is one of:
    A Leaf.
    A Branch with left Tree and right Tree.

## To build (depth: Int) -> Tree:
    If depth is equal to 0:
        Return Leaf.
    Return a new Branch with left build(depth - 1) and right build(depth - 1).

## To count (tree: Tree) -> Int:
    Inspect tree:
        When Leaf: Return 1.
        When Branch (left, right): Return 1 + count(left) + count(right).
    Return 0.

"#;
    let mut heaps = Vec::new();
    for n in [100_u64, 1000] {
        let source = format!("{declarations}## Main\nLet mutable total be 0.\nRepeat for i from 1 to {n}:\n    Let tree be build(3).\n    Set total to total + count(tree).\nShow total.\n");
        heaps.push(exact(&source, &(n * 15).to_string()));
    }
    assert_eq!(
        heaps[0], heaps[1],
        "the parent and all descendants are reclaimed"
    );
}
