//! The programs that lock how C stores a value that can be `nothing` hold on Rust too.
//!
//! A list item, a map value, a field, a parameter or a result the program stores `nothing` in is copied, compared,
//! passed, returned and read as the VM reads it, and an operation on `nothing` fails in the VM's words. Each program
//! is one of the `c_nullable_*` locks', named for its test there, and runs on the tree-walker, the VM and the
//! generated Rust, each writing the same bytes to standard output and failing, where it fails, with the same words.
#![cfg(not(target_arch = "wasm32"))]
mod common;
use logicaffeine_compile::compile::{tw_lines, vm_outcome};

fn holds(source: &str, stdout: &str, error: Option<&str>) {
    let tw = tw_lines(source);
    let tw_bytes: String = tw.lines.iter().map(|line| format!("{line}\n")).collect();
    assert_eq!((tw_bytes.as_str(), tw.error.as_deref()), (stdout, error), "tree-walker:\n{source}");

    let vm = vm_outcome(source);
    assert_eq!((vm.output.as_str(), vm.error.as_deref()), (stdout, error), "VM:\n{source}");

    let run = common::run_logos(source);
    assert_eq!(run.stdout, stdout, "Rust stdout:\n{source}\n\n{}\n\n{}", run.stderr, run.rust_code);
    match error {
        None => assert!(run.success, "Rust run:\n{}\n\n{source}\n\n{}", run.stderr, run.rust_code),
        Some(words) => assert!(
            !run.success && run.stderr.contains(words),
            "Rust must stop with {words:?}:\n{}\n\n{source}\n\n{}",
            run.stderr,
            run.rust_code
        ),
    }
}

#[test]
fn a_copy_made_through_a_function_during_a_loan_keeps_the_absence() {
    holds(
        r#"## To identity (items: Seq of Int) -> Seq of Int:
    Return items.
## To change (value: mutable Int) -> Int:
    Set saved to identity(xs).
    Set value to 7.
    Return 1.
## Main
Let xs be [4, 5].
Let mutable saved be a new Seq of Int.
Show change(item 1 of xs).
Show saved.
Show xs.
"#,
        "1\n[nothing, 5]\n[7, 5]\n",
        None,
    );
}

#[test]
fn a_nested_sequence_clears_only_the_lent_leaf_and_rebuilds_it() {
    holds(
        r#"## To change (value: mutable Int) -> Int:
    Show rows.
    Set value to 9.
    Return 1.
## Main
Let rows be [[4, 5], [6, 7]].
Show change(item 1 of item 2 of rows).
Show rows.
"#,
        "[[4, 5], [nothing, 7]]\n1\n[[4, 5], [9, 7]]\n",
        None,
    );
}

#[test]
fn a_shadowing_text_sequence_keeps_its_own_layout() {
    holds(
        r#"## To change (value: mutable Int) -> Int:
    If true:
        Let xs be ["local"].
        Show xs.
    Show xs.
    Set value to 7.
    Return 1.
## Main
Let xs be [4, 5].
Show change(item 1 of xs).
Show xs.
"#,
        "[local]\n[nothing, 5]\n1\n[7, 5]\n",
        None,
    );
}

#[test]
fn an_absent_element_survives_a_sequence_parameter_and_return() {
    holds(
        r#"## To identity (items: Seq of Int) -> Seq of Int:
    Return items.
## To change (value: mutable Int) -> Int:
    Show identity(xs).
    Set value to 7.
    Return 1.
## Main
Let xs be [4, 5].
Show change(item 1 of xs).
Show xs.
"#,
        "[nothing, 5]\n1\n[7, 5]\n",
        None,
    );
}

#[test]
fn an_absent_scalar_survives_a_readonly_parameter_and_return() {
    holds(
        r#"## To identity (value: Int) -> Int:
    Return value.
## To change (value: mutable Int) -> Int:
    Show identity(item 1 of xs).
    Set value to 7.
    Return 1.
## Main
Let xs be [4].
Show change(item 1 of xs).
Show xs.
"#,
        "nothing\n1\n[7]\n",
        None,
    );
}

#[test]
fn an_integer_map_keeps_its_key_while_its_value_is_lent() {
    holds(
        r#"## To change (value: mutable Int) -> Int:
    Show item 3 of values.
    Show length of values.
    Set value to 7.
    Return 1.
## Main
Let values be a new Map of Int to Int.
Set item 3 of values to 4.
Show change(item 3 of values).
Show item 3 of values.
"#,
        "nothing\n1\n1\n7\n",
        None,
    );
}

#[test]
fn arithmetic_on_an_absent_element_fails_as_it_does_on_the_vm() {
    holds(
        r#"## To change (value: mutable Int) -> Int:
    Show item 1 of xs + 1.
    Return 1.
## Main
Let xs be [4].
Show change(item 1 of xs).
"#,
        "",
        Some("Cannot add Nothing and Int"),
    );
}

#[test]
fn a_call_used_in_optional_arithmetic_runs_once() {
    holds(
        r#"## To fetch -> Int:
    Show "fetch".
    Return item 2 of xs.
## To change (value: mutable Int) -> Int:
    Show fetch() + 1.
    Set value to 7.
    Return 1.
## Main
Let xs be [4, 5].
Show change(item 1 of xs).
Show xs.
"#,
        "fetch\n6\n1\n[7, 5]\n",
        None,
    );
}

#[test]
fn an_index_used_in_optional_arithmetic_runs_once() {
    holds(
        r#"## To chooseIndex -> Int:
    Show "index".
    Return 2.
## To change (value: mutable Int) -> Int:
    Show item (chooseIndex()) of xs + 1.
    Set value to 7.
    Return 1.
## Main
Let xs be [4, 5].
Show change(item 1 of xs).
Show xs.
"#,
        "index\n6\n1\n[7, 5]\n",
        None,
    );
}

#[test]
fn an_inferred_collection_return_preserves_absent_elements() {
    holds(
        r#"## To identity (items: Seq of Int):
    Return items.
## To change (value: mutable Int) -> Int:
    Show identity(xs).
    Set value to 7.
    Return 1.
## Main
Let xs be [4, 5].
Show change(item 1 of xs).
Show xs.
"#,
        "[nothing, 5]\n1\n[7, 5]\n",
        None,
    );
}

#[test]
fn copying_an_absent_scalar_preserves_nothing() {
    holds(
        r#"## To change (value: mutable Int) -> Int:
    Let saved be copy of item 1 of xs.
    Show saved is equal to nothing.
    Show saved.
    Set value to 7.
    Return 1.
## Main
Let xs be [4, 5].
Show change(item 1 of xs).
Show xs.
"#,
        "true\nnothing\n1\n[7, 5]\n",
        None,
    );
}

#[test]
fn membership_compares_absent_and_present_integer_elements() {
    holds(
        r#"## To change (value: mutable Int) -> Int:
    Show xs contains nothing.
    Show xs contains 4.
    Show xs contains 5.
    Set value to 7.
    Return 1.
## Main
Let xs be [4, 5].
Show change(item 1 of xs).
Show xs.
"#,
        "true\nfalse\ntrue\n1\n[7, 5]\n",
        None,
    );
}

#[test]
fn membership_compares_absent_and_present_text_elements() {
    holds(
        r#"## To change (value: mutable Text) -> Int:
    Show xs contains nothing.
    Show xs contains "one".
    Show xs contains "two".
    Set value to "new".
    Return 1.
## Main
Let xs be ["one", "two"].
Show change(item 1 of xs).
Show xs.
"#,
        "true\nfalse\ntrue\n1\n[new, two]\n",
        None,
    );
}

#[test]
fn popping_an_absent_element_returns_nothing_without_an_extra_presence_layer() {
    holds(
        r#"## To change (value: mutable Int) -> Int:
    Let mutable snapshot be xs.
    Pop from snapshot into popped.
    Show popped is equal to nothing.
    Show popped.
    Show length of snapshot.
    Set value to 7.
    Return 1.
## Main
Let xs be [4, 5].
Show change(item 2 of xs).
Show xs.
"#,
        "true\nnothing\n1\n1\n[4, 7]\n",
        None,
    );
}

#[test]
fn nullable_integer_equality_retains_absence_and_present_controls() {
    holds(
        r#"## To change (value: mutable Int) -> Int:
    Show item 1 of xs is equal to nothing.
    Show item 2 of xs is equal to nothing.
    Show item 2 of xs is equal to item 3 of xs.
    Show item 2 of xs is equal to 4.
    Set value to 7.
    Return 1.
## Main
Let xs be [4, 5, 5].
Show change(item 1 of xs).
"#,
        "true\nfalse\ntrue\nfalse\n1\n",
        None,
    );
}

#[test]
fn nullable_integer_payloads_do_not_equal_booleans() {
    holds(
        r#"## To change (value: mutable Int) -> Int:
    Show item 2 of xs is equal to true.
    Show true is equal to item 2 of xs.
    Show item 3 of xs is equal to false.
    Set value to 7.
    Return 1.
## Main
Let xs be [4, 1, 0].
Show change(item 1 of xs).
"#,
        "false\nfalse\nfalse\n1\n",
        None,
    );
}

#[test]
fn present_nullable_integers_compare_numerically_to_reals() {
    holds(
        r#"## To change (value: mutable Int) -> Int:
    Show item 2 of xs is equal to 2.0.
    Show 2.0 is equal to item 2 of xs.
    Show item 2 of xs is equal to 2.5.
    Show item 3 of xs is equal to 9007199254740993.0.
    Set value to 7.
    Return 1.
## Main
Let xs be [4, 2, 9007199254740993].
Show change(item 1 of xs).
"#,
        "true\ntrue\nfalse\nfalse\n1\n",
        None,
    );
}

#[test]
fn nullable_int_int_maps_check_keys_values_lengths_and_empty_tables() {
    holds(
        r#"## To change (value: mutable Map of Int to Int) -> Int:
    Show item 1 of maps is equal to nothing.
    Show item 2 of maps is equal to item 3 of maps.
    Show item 2 of maps is equal to item 4 of maps.
    Show item 2 of maps is equal to item 5 of maps.
    Show item 2 of maps is equal to item 6 of maps.
    Show item 6 of maps is equal to item 2 of maps.
    Show item 7 of maps is equal to item 8 of maps.
    Set value to a new Map of Int to Int.
    Return 1.
## Main
Let first be a new Map of Int to Int.
Let left be a new Map of Int to Int.
Set item 1 of left to 11.
Set item 2 of left to 22.
Let right be a new Map of Int to Int.
Set item 2 of right to 22.
Set item 1 of right to 11.
Let changed be a new Map of Int to Int.
Set item 1 of changed to 11.
Set item 2 of changed to 33.
Let differentKey be a new Map of Int to Int.
Set item 1 of differentKey to 11.
Set item 3 of differentKey to 22.
Let shorter be a new Map of Int to Int.
Set item 1 of shorter to 11.
Let emptyLeft be a new Map of Int to Int.
Let emptyRight be a new Map of Int to Int.
Let maps be [first, left, right, changed, differentKey, shorter, emptyLeft, emptyRight].
Show change(item 1 of maps).
"#,
        "true\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\n1\n",
        None,
    );
}

#[test]
fn nullable_int_text_maps_check_keys_values_lengths_and_empty_tables() {
    holds(
        r#"## To change (value: mutable Map of Int to Text) -> Int:
    Show item 1 of maps is equal to nothing.
    Show item 2 of maps is equal to item 3 of maps.
    Show item 2 of maps is equal to item 4 of maps.
    Show item 2 of maps is equal to item 5 of maps.
    Show item 2 of maps is equal to item 6 of maps.
    Show item 6 of maps is equal to item 2 of maps.
    Show item 7 of maps is equal to item 8 of maps.
    Set value to a new Map of Int to Text.
    Return 1.
## Main
Let first be a new Map of Int to Text.
Let left be a new Map of Int to Text.
Set item 1 of left to "one".
Set item 2 of left to "two".
Let right be a new Map of Int to Text.
Set item 2 of right to "two".
Set item 1 of right to "one".
Let changed be a new Map of Int to Text.
Set item 1 of changed to "one".
Set item 2 of changed to "three".
Let differentKey be a new Map of Int to Text.
Set item 1 of differentKey to "one".
Set item 3 of differentKey to "two".
Let shorter be a new Map of Int to Text.
Set item 1 of shorter to "one".
Let emptyLeft be a new Map of Int to Text.
Let emptyRight be a new Map of Int to Text.
Let maps be [first, left, right, changed, differentKey, shorter, emptyLeft, emptyRight].
Show change(item 1 of maps).
"#,
        "true\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\n1\n",
        None,
    );
}

#[test]
fn nullable_integer_sets_compare_contents_after_removal() {
    holds(
        r#"## To change (value: mutable Set of Int) -> Int:
    Show item 1 of sets is equal to nothing.
    Show item 2 of sets is equal to item 3 of sets.
    Show item 2 of sets is equal to item 4 of sets.
    Show item 2 of sets is equal to item 5 of sets.
    Show item 5 of sets is equal to item 2 of sets.
    Show item 6 of sets is equal to item 7 of sets.
    Set value to a new Set of Int.
    Return 1.
## Main
Let first be a new Set of Int.
Let left be a new Set of Int.
Add 1 to left.
Add 2 to left.
Let right be a new Set of Int.
Add 3 to right.
Add 2 to right.
Add 1 to right.
Remove 3 from right.
Let changed be a new Set of Int.
Add 1 to changed.
Add 3 to changed.
Let shorter be a new Set of Int.
Add 1 to shorter.
Let emptyLeft be a new Set of Int.
Let emptyRight be a new Set of Int.
Add 3 to emptyRight.
Remove 3 from emptyRight.
Let sets be [first, left, right, changed, shorter, emptyLeft, emptyRight].
Show change(item 1 of sets).
"#,
        "true\ntrue\nfalse\nfalse\nfalse\ntrue\n1\n",
        None,
    );
}

#[test]
fn nullable_maps_compare_entries_independently_of_insertion_order() {
    holds(
        r#"## To change (value: mutable Map of Text to Int) -> Int:
    Show item 1 of maps is equal to nothing.
    Show item 2 of maps is equal to item 3 of maps.
    Show item 2 of maps is equal to item 4 of maps.
    Show item 2 of maps is not equal to item 4 of maps.
    Set value to a new Map of Text to Int.
    Return 1.
## Main
Let first be a new Map of Text to Int.
Set item "old" of first to 9.
Let left be a new Map of Text to Int.
Set item "a" of left to 1.
Set item "b" of left to 2.
Let right be a new Map of Text to Int.
Set item "b" of right to 2.
Set item "a" of right to 1.
Let changed be a new Map of Text to Int.
Set item "a" of changed to 1.
Set item "b" of changed to 3.
Let maps be [first, left, right, changed].
Show change(item 1 of maps).
"#,
        "true\ntrue\nfalse\ntrue\n1\n",
        None,
    );
}

#[test]
fn nullable_sets_compare_members_independently_of_insertion_order() {
    holds(
        r#"## To change (value: mutable Set of Int) -> Int:
    Show item 1 of sets is equal to nothing.
    Show item 2 of sets is equal to item 3 of sets.
    Show item 2 of sets is equal to item 4 of sets.
    Show item 2 of sets is not equal to item 4 of sets.
    Set value to a new Set of Int.
    Return 1.
## Main
Let first be a new Set of Int.
Add 9 to first.
Let left be a new Set of Int.
Add 1 to left.
Add 2 to left.
Let right be a new Set of Int.
Add 2 to right.
Add 1 to right.
Let changed be a new Set of Int.
Add 1 to changed.
Add 3 to changed.
Let sets be [first, left, right, changed].
Show change(item 1 of sets).
"#,
        "true\ntrue\nfalse\ntrue\n1\n",
        None,
    );
}

#[test]
fn nullable_text_int_maps_probe_colliding_keys() {
    holds(
        r#"## To change (value: mutable Map of Text to Int) -> Int:
    Show item 1 of maps is equal to nothing.
    Show item 2 of maps is equal to item 3 of maps.
    Show item 2 of maps is equal to item 4 of maps.
    Show item 2 of maps is equal to item 5 of maps.
    Show item 2 of maps is equal to item 6 of maps.
    Show item 6 of maps is equal to item 2 of maps.
    Show item 7 of maps is equal to item 8 of maps.
    Set value to a new Map of Text to Int.
    Return 1.
## Main
Let first be a new Map of Text to Int.
Let left be a new Map of Text to Int.
Set item "a" of left to 11.
Set item "q" of left to 22.
Let right be a new Map of Text to Int.
Set item "q" of right to 22.
Set item "a" of right to 11.
Let changed be a new Map of Text to Int.
Set item "a" of changed to 11.
Set item "q" of changed to 33.
Let differentKey be a new Map of Text to Int.
Set item "a" of differentKey to 11.
Set item "z" of differentKey to 22.
Let shorter be a new Map of Text to Int.
Set item "a" of shorter to 11.
Let emptyLeft be a new Map of Text to Int.
Let emptyRight be a new Map of Text to Int.
Let maps be [first, left, right, changed, differentKey, shorter, emptyLeft, emptyRight].
Show change(item 1 of maps).
"#,
        "true\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\n1\n",
        None,
    );
}

#[test]
fn nullable_text_sequence_maps_compare_values_recursively() {
    holds(
        r#"## To change (value: mutable Map of Text to Seq of Int) -> Int:
    Show item 1 of maps is equal to nothing.
    Show item 2 of maps is equal to item 3 of maps.
    Show item 2 of maps is equal to item 4 of maps.
    Show item 2 of maps is equal to item 5 of maps.
    Show item 2 of maps is equal to item 6 of maps.
    Show item 6 of maps is equal to item 2 of maps.
    Show item 7 of maps is equal to item 8 of maps.
    Set value to a new Map of Text to Seq of Int.
    Return 1.
## Main
Let first be a new Map of Text to Seq of Int.
Let left be a new Map of Text to Seq of Int.
Set item "a" of left to [1, 2].
Set item "q" of left to [3, 4].
Let right be a new Map of Text to Seq of Int.
Set item "q" of right to [3, 4].
Set item "a" of right to [1, 2].
Let changed be a new Map of Text to Seq of Int.
Set item "a" of changed to [1, 2].
Set item "q" of changed to [3, 5].
Let differentKey be a new Map of Text to Seq of Int.
Set item "a" of differentKey to [1, 2].
Set item "z" of differentKey to [3, 4].
Let shorter be a new Map of Text to Seq of Int.
Set item "a" of shorter to [1, 2].
Let emptyLeft be a new Map of Text to Seq of Int.
Let emptyRight be a new Map of Text to Seq of Int.
Let maps be [first, left, right, changed, differentKey, shorter, emptyLeft, emptyRight].
Show change(item 1 of maps).
"#,
        "true\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\n1\n",
        None,
    );
}

#[test]
fn nullable_text_sets_compare_contents_after_removal_and_collision() {
    holds(
        r#"## To change (value: mutable Set of Text) -> Int:
    Show item 1 of sets is equal to nothing.
    Show item 2 of sets is equal to item 3 of sets.
    Show item 2 of sets is equal to item 4 of sets.
    Show item 2 of sets is equal to item 5 of sets.
    Show item 5 of sets is equal to item 2 of sets.
    Show item 6 of sets is equal to item 7 of sets.
    Set value to a new Set of Text.
    Return 1.
## Main
Let first be a new Set of Text.
Let left be a new Set of Text.
Add "a" to left.
Add "q" to left.
Let right be a new Set of Text.
Add "z" to right.
Add "q" to right.
Add "a" to right.
Remove "z" from right.
Let changed be a new Set of Text.
Add "a" to changed.
Add "z" to changed.
Let shorter be a new Set of Text.
Add "a" to shorter.
Let emptyLeft be a new Set of Text.
Let emptyRight be a new Set of Text.
Add "z" to emptyRight.
Remove "z" from emptyRight.
Let sets be [first, left, right, changed, shorter, emptyLeft, emptyRight].
Show change(item 1 of sets).
"#,
        "true\ntrue\nfalse\nfalse\nfalse\ntrue\n1\n",
        None,
    );
}

#[test]
fn nullable_text_text_maps_check_keys_values_lengths_and_empty_tables() {
    holds(
        r#"## To change (value: mutable Map of Text to Text) -> Int:
    Show item 1 of maps is equal to nothing.
    Show item 2 of maps is equal to item 3 of maps.
    Show item 2 of maps is equal to item 4 of maps.
    Show item 2 of maps is equal to item 5 of maps.
    Show item 2 of maps is equal to item 6 of maps.
    Show item 6 of maps is equal to item 2 of maps.
    Show item 7 of maps is equal to item 8 of maps.
    Set value to a new Map of Text to Text.
    Return 1.
## Main
Let first be a new Map of Text to Text.
Let left be a new Map of Text to Text.
Set item "a" of left to "one".
Set item "q" of left to "two".
Let right be a new Map of Text to Text.
Set item "q" of right to "two".
Set item "a" of right to "one".
Let changed be a new Map of Text to Text.
Set item "a" of changed to "one".
Set item "q" of changed to "three".
Let differentKey be a new Map of Text to Text.
Set item "a" of differentKey to "one".
Set item "z" of differentKey to "two".
Let shorter be a new Map of Text to Text.
Set item "a" of shorter to "one".
Let emptyLeft be a new Map of Text to Text.
Let emptyRight be a new Map of Text to Text.
Let maps be [first, left, right, changed, differentKey, shorter, emptyLeft, emptyRight].
Show change(item 1 of maps).
"#,
        "true\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\n1\n",
        None,
    );
}

#[test]
fn nullable_list_equality_checks_empty_and_unequal_lengths() {
    holds(
        r#"## To change (value: mutable Seq of Int) -> Int:
    Show item 2 of rows is equal to item 3 of rows.
    Show item 2 of rows is equal to item 4 of rows.
    Show item 4 of rows is equal to item 2 of rows.
    Show item 4 of rows is equal to item 5 of rows.
    Show item 5 of rows is equal to item 4 of rows.
    Set value to [9].
    Return 1.
## Main
Let emptyA be a new Seq of Int.
Let emptyB be a new Seq of Int.
Let rows be [[7], emptyA, emptyB, [1], [1, 2]].
Show change(item 1 of rows).
"#,
        "true\nfalse\nfalse\nfalse\nfalse\n1\n",
        None,
    );
}

#[test]
fn nullable_nested_lists_compare_contents_and_absence() {
    holds(
        r#"## To change (value: mutable Seq of Int) -> Int:
    Show item 1 of rows is equal to nothing.
    Show item 2 of rows is equal to item 3 of rows.
    Show item 2 of rows is equal to [1, 2].
    Show [1, 2] is equal to item 2 of rows.
    Show item 2 of rows is equal to [1, 3].
    Show item 2 of rows is not equal to [1, 3].
    Show item 2 of rows is equal to nothing.
    Set value to [7, 8].
    Return 1.
## Main
Let rows be [[4, 5], [1, 2], [1, 2]].
Show change(item 1 of rows).
Show rows.
"#,
        "true\ntrue\ntrue\ntrue\nfalse\ntrue\nfalse\n1\n[[7, 8], [1, 2], [1, 2]]\n",
        None,
    );
}

#[test]
fn nullable_record_equality_checks_scalar_fields_independently() {
    holds(
        r#"## A Box has:
    a number, which is Int.
    a tape, which is Seq of Int.
## To change (value: mutable Box) -> Int:
    Show item 2 of boxes is equal to (a new Box with number 3 and tape [4, 5]).
    Show item 2 of boxes is not equal to (a new Box with number 3 and tape [4, 5]).
    Set value to a new Box with number 7 and tape [8].
    Return 1.
## Main
Let boxes be [a new Box with number 1 and tape [2], a new Box with number 2 and tape [4, 5]].
Show change(item 1 of boxes).
"#,
        "false\ntrue\n1\n",
        None,
    );
}

#[test]
fn nullable_records_compare_fields_not_allocations() {
    holds(
        r#"## A Box has:
    a number, which is Int.
    a tape, which is Seq of Int.
## To change (value: mutable Box) -> Int:
    Show item 1 of boxes is equal to nothing.
    Show item 2 of boxes is equal to item 3 of boxes.
    Show item 2 of boxes is equal to (a new Box with number 2 and tape [4, 5]).
    Show item 2 of boxes is equal to (a new Box with number 2 and tape [4, 6]).
    Set value to a new Box with number 7 and tape [8].
    Return 1.
## Main
Let boxes be [a new Box with number 1 and tape [2], a new Box with number 2 and tape [4, 5], a new Box with number 2 and tape [4, 5]].
Show change(item 1 of boxes).
"#,
        "true\ntrue\ntrue\nfalse\n1\n",
        None,
    );
}

#[test]
fn nullable_text_lists_compare_elements_not_buffer_addresses() {
    holds(
        r#"## To change (value: mutable Seq of Text) -> Int:
    Show item 2 of rows is equal to item 3 of rows.
    Show item 2 of rows is equal to ["a", "b"].
    Show item 2 of rows is equal to ["a", "c"].
    Set value to ["new"].
    Return 1.
## Main
Let rows be [["old"], ["a", "b"], ["a", "b"]].
Show change(item 1 of rows).
Show rows.
"#,
        "true\ntrue\nfalse\n1\n[[new], [a, b], [a, b]]\n",
        None,
    );
}
