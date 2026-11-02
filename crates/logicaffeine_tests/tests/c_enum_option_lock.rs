//! The C tier's enums and `Option`, byte-identical to the interpreter.
//!
//! C has no built-in `Option`: `some x` / `none` / `Option of T` lower (through monomorphization) to
//! instances of core's `An Option of [T] is either: A Some (value: T). A None.`, and core's generic
//! `Result` to one concrete enum per application. Every enum value shows the interpreter's way — a
//! unit variant as its name, a payload variant as `Variant(a, b)`, a recursive field through its
//! box — and an `Option` shows its payload or `nothing`. Each program is pinned to the oracle string
//! on the C tier and to the interpreter on the tree-walker and VM.

mod common;
use common::{assert_all_tiers, assert_c_output};

fn on_c_and_interpreters(src: &str, expected: &str) {
    assert_all_tiers(src, expected);
    assert_c_output(src, expected);
}

#[test]
fn enum_and_option_values_show_like_the_interpreter() {
    on_c_and_interpreters(
        "## A Maybe of [T] is either:\n    A Nothing.\n    A Just with value T.\n\n## A Tree is either:\n    A Leaf.\n    A Node with value Int and left Tree and right Tree.\n\n## Main\nShow some 3.\nLet n be none.\nShow n.\nLet m be a new Just with value 42.\nShow m.\nShow a new Just with value \"s\".\nLet l be a new Leaf.\nShow a new Node with value 1 and left l and right (a new Node with value 2 and left l and right l).\nLet r be a new Ok with value 5.\nShow r.\nShow a new Err with error \"bad\".\n",
        "3\nnothing\nJust(42)\nJust(s)\nNode(1, Leaf, Node(2, Leaf, Leaf))\nOk(5)\nErr(bad)",
    );
}

#[test]
fn an_option_parameter_is_inspected_on_c() {
    on_c_and_interpreters(
        "## To orElse (o: Option of Int, d: Int) -> Int:\n    Inspect o:\n        When Some (v): Return v.\n        When None: Return d.\n\n## Main\nShow orElse(some 3, 0).\nShow orElse(none, 7).\n",
        "3\n7",
    );
}

#[test]
fn a_result_parameter_is_inspected_on_c() {
    on_c_and_interpreters(
        "## To valueOr (r: Result of Int and Text, d: Int) -> Int:\n    Inspect r:\n        When Ok (v): Return v.\n        When Err (e): Return d.\n\n## Main\nShow valueOr(a new Ok with value 8, 0).\nShow valueOr(a new Err with error \"bad\", 0).\n",
        "8\n0",
    );
}

#[test]
fn repeat_walks_a_sequence_of_records_built_with_a_new_seq() {
    on_c_and_interpreters(
        "## A Box has:\n    a value, which is Int.\n\n## Main\nLet bs be a new Seq of Box.\nPush a new Box with value 1 to bs.\nPush a new Box with value 2 to bs.\nLet mutable total be 0.\nRepeat for b in bs:\n    Set total to total + b's value.\nShow total.\n",
        "3",
    );
}

#[test]
fn repeat_walks_a_sequence_of_records_built_from_a_literal() {
    on_c_and_interpreters(
        "## A Box has:\n    a value, which is Int.\n\n## Main\nLet a be a new Box with value 1.\nLet bs be [a, a].\nLet mutable total be 0.\nRepeat for b in bs:\n    Set total to total + b's value.\nShow total.\n",
        "2",
    );
}

#[test]
fn an_empty_literal_takes_the_element_type_of_the_field_it_fills() {
    on_c_and_interpreters(
        "## A Point has:\n    An x: Int.\n\n## A Grid has:\n    A pts: Seq of Point.\n\n## Main\nLet g be a new Grid with pts [].\nShow g.\n",
        "Grid { pts: [] }",
    );
}

// ── Sequences of enums, of Options, and of sequences, as values and as record fields ──────────────
// C types every `Seq of X` through ONE mapping (`seq_of`): a record or an enum gets its `Seq_<Name>`
// family, a sequence a nested `Seq_Seq_…` wrapper — which a record field may now hold, since every
// container is defined before any record body (`codegen_c_aggregates`).

#[test]
fn a_sequence_of_options_shows_payloads_and_nothing() {
    on_c_and_interpreters(
        "## A Box of [T] has:\n    a value, which is T.\n\n## Main\nShow some 3.\nLet n be none.\nShow n.\nShow [some 1, none].\nLet r be a new Ok with value 5.\nShow r.\nLet e be a new Err with error \"bad\".\nShow e.\nLet b be a new Box of (Option of Int) with value some 2.\nShow b.\n",
        "3\nnothing\n[1, nothing]\nOk(5)\nErr(bad)\nBox { value: 2 }",
    );
}

#[test]
fn a_record_holds_a_sequence_of_sequences() {
    on_c_and_interpreters(
        "## A Grid has:\n    A rows: Seq of (Seq of Int).\n\n## Main\nLet g be a new Grid with rows [[1, 2], [3]].\nShow g.\n",
        "Grid { rows: [[1, 2], [3]] }",
    );
}

#[test]
fn a_record_holds_a_sequence_of_an_enum() {
    on_c_and_interpreters(
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Square with side Int.\n\n## A Canvas has:\n    A shapes: Seq of Shape.\n\n## Main\nLet c be a new Canvas with shapes [a new Circle with radius 1].\nShow c.\n",
        "Canvas { shapes: [Circle(1)] }",
    );
}

#[test]
fn a_sequence_of_enum_values_shows_and_is_walked() {
    on_c_and_interpreters(
        "## A Shape is either:\n    A Circle with radius Int.\n    A Dot.\n\n## Main\nLet xs be [a new Circle with radius 2, a new Dot].\nShow xs.\nRepeat for s in xs:\n    Inspect s:\n        When Circle (r): Show r.\n        When Dot: Show \"dot\".\nShow length of xs.\n",
        "[Circle(2), Dot]\n2\ndot\n2",
    );
}

#[test]
fn nested_sequence_literals_three_deep() {
    on_c_and_interpreters(
        "## Main\nLet g be [[1, 2], [3]].\nShow g.\nLet h be [[[\"a\"]], [[\"b\", \"c\"]]].\nShow h.\n",
        "[[1, 2], [3]]\n[[[a]], [[b, c]]]",
    );
}
