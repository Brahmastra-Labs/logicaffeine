mod common;

use common::{assert_exact_output, assert_output_lines};

// =============================================================================
// Reference Semantics for Maps and Seqs
//
// LOGOS Maps and Seqs have reference semantics: passing them to a function
// shares the same underlying data. Mutations inside the function are visible
// to the caller. `copy of` creates an independent deep copy.
// =============================================================================

#[test]
fn ref_semantics_map_mutation_visible() {
    assert_exact_output(
        r#"
## To modify (m: Map of Text to Int):
    Set item "x" of m to 42.

## Main
    Let m be a new Map of Text to Int.
    Set item "x" of m to 0.
    modify(m).
    Show item "x" of m.
"#,
        "42",
    );
}

#[test]
fn ref_semantics_seq_push_visible() {
    assert_exact_output(
        r#"
## To addItem (items: Seq of Int):
    Push 99 to items.

## Main
    Let items be a new Seq of Int.
    Push 1 to items.
    addItem(items).
    Show length of items.
"#,
        "2",
    );
}

#[test]
fn ref_semantics_copy_of_isolates() {
    assert_exact_output(
        r#"
## To modify (m: Map of Text to Int):
    Set item "x" of m to 42.

## Main
    Let m be a new Map of Text to Int.
    Set item "x" of m to 0.
    Let m2 be copy of m.
    modify(m2).
    Show item "x" of m.
"#,
        "0",
    );
}

#[test]
fn ref_semantics_seq_set_index_visible() {
    assert_exact_output(
        r#"
## To mutate (arr: Seq of Int):
    Set item 1 of arr to 999.

## Main
    Let arr be a new Seq of Int.
    Push 10 to arr.
    Push 20 to arr.
    mutate(arr).
    Show item 1 of arr.
"#,
        "999",
    );
}

#[test]
fn ref_semantics_multiple_aliases() {
    assert_exact_output(
        r#"
## Main
    Let a be a new Seq of Int.
    Push 1 to a.
    Let b be a.
    Push 2 to b.
    Show length of a.
"#,
        "2",
    );
}

#[test]
fn ref_semantics_copy_of_seq_isolates() {
    assert_exact_output(
        r#"
## Main
    Let a be a new Seq of Int.
    Push 1 to a.
    Let b be copy of a.
    Push 2 to b.
    Show length of a.
    Show length of b.
"#,
        "1\n2",
    );
}

#[test]
fn ref_semantics_nested_function_calls() {
    assert_exact_output(
        r#"
## To addOne (items: Seq of Int):
    Push 1 to items.

## To addTwo (items: Seq of Int):
    addOne(items).
    addOne(items).

## Main
    Let items be a new Seq of Int.
    addTwo(items).
    Show length of items.
"#,
        "2",
    );
}

#[test]
fn ref_semantics_map_multiple_mutations() {
    assert_output_lines(
        r#"
## To setup (m: Map of Text to Int):
    Set item "a" of m to 1.
    Set item "b" of m to 2.

## To update (m: Map of Text to Int):
    Set item "a" of m to 10.

## Main
    Let m be a new Map of Text to Int.
    setup(m).
    update(m).
    Show item "a" of m.
    Show item "b" of m.
"#,
        &["10", "2"],
    );
}

#[test]
fn ref_semantics_seq_iteration_after_mutation() {
    assert_exact_output(
        r#"
## To populate (items: Seq of Int):
    Push 10 to items.
    Push 20 to items.
    Push 30 to items.

## Main
    Let items be a new Seq of Int.
    populate(items).
    Let total be 0.
    Repeat for x in items:
        Set total to total + x.
    Show total.
"#,
        "60",
    );
}

#[test]
fn ref_semantics_copy_of_map_deep_copy() {
    assert_output_lines(
        r#"
## Main
    Let original be a new Map of Text to Int.
    Set item "x" of original to 1.
    Let clone be copy of original.
    Set item "x" of clone to 99.
    Show item "x" of original.
    Show item "x" of clone.
"#,
        &["1", "99"],
    );
}
