//! Maps and sets held in a list compare on C as the VM compares them: by their entries, whatever order they were
//! inserted in.
//!
//! A list of maps or of sets is a list of tables on C (`Seq_Map_…`, `Seq_Set_…`), and `is equal to` between two of
//! its items compares the tables entry by entry: the same size, and every key of one found in the other with an
//! equal value. Neither slot positions nor insertion order are values. Each program runs on the tree-walker, the VM
//! and a release C build, and each must write exactly the expected bytes to standard output.
#![cfg(not(target_arch = "wasm32"))]
mod common;
use logicaffeine_compile::compile::{compile_to_c, tw_lines, vm_outcome};

fn holds(source: &str, expected: &str) {
    let tw = tw_lines(source);
    assert_eq!(tw.error, None, "tree-walker:\n{source}");
    let tw_bytes: String = tw.lines.iter().map(|line| format!("{line}\n")).collect();
    assert_eq!(tw_bytes, expected, "tree-walker:\n{source}");

    let vm = vm_outcome(source);
    assert_eq!(vm.error, None, "VM:\n{source}");
    assert_eq!(vm.output, expected, "VM:\n{source}");

    let c = compile_to_c(source).unwrap_or_else(|e| panic!("LOGOS→C: {e:?}\n{source}"));
    let dir = tempfile::tempdir().expect("temp dir");
    let (src, bin) = (dir.path().join("main.c"), dir.path().join("main"));
    std::fs::write(&src, &c).expect("write main.c");
    let build = common::build_c_release(&src, &bin).expect("run the C compiler");
    assert!(build.status.success(), "C build:\n{}\n{c}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&bin).output().expect("run the C program");
    assert!(run.status.success(), "C run: {:?}\n{}\n{c}", run.status, String::from_utf8_lossy(&run.stderr));
    assert_eq!(String::from_utf8_lossy(&run.stdout), expected, "C:\n{source}\n{c}");
}

#[test]
fn maps_in_a_list_compare_by_their_entries_not_their_insertion_order() {
    holds(
        r#"## Main
Let left be a new Map of Text to Int.
Set item "a" of left to 1.
Set item "b" of left to 2.
Let right be a new Map of Text to Int.
Set item "b" of right to 2.
Set item "a" of right to 1.
Let changed be a new Map of Text to Int.
Set item "a" of changed to 1.
Set item "b" of changed to 3.
Let maps be [left, right, changed].
Show item 1 of maps is equal to item 2 of maps.
Show item 1 of maps is equal to item 3 of maps.
Show item 2 of maps is not equal to item 3 of maps.
Show length of maps.
"#,
        "true\nfalse\ntrue\n3\n",
    );
}

#[test]
fn integer_keyed_maps_in_a_list_compare_by_key_and_value() {
    holds(
        r#"## Main
Let first be a new Map of Int to Int.
Set item 3 of first to 30.
Set item 1 of first to 10.
Let second be a new Map of Int to Int.
Set item 1 of second to 10.
Set item 3 of second to 30.
Let shorter be a new Map of Int to Int.
Set item 1 of shorter to 10.
Let maps be [first, second, shorter].
Show item 1 of maps is equal to item 2 of maps.
Show item 1 of maps is equal to item 3 of maps.
Show item 3 of maps is equal to item 1 of maps.
"#,
        "true\nfalse\nfalse\n",
    );
}

#[test]
fn sets_in_a_list_compare_by_their_members() {
    holds(
        r#"## Main
Let first be a new Set of Int.
Add 3 to first.
Add 1 to first.
Let second be a new Set of Int.
Add 1 to second.
Add 3 to second.
Let other be a new Set of Int.
Add 1 to other.
Add 4 to other.
Let sets be [first, second, other].
Show item 1 of sets is equal to item 2 of sets.
Show item 1 of sets is equal to item 3 of sets.
"#,
        "true\nfalse\n",
    );
}

#[test]
fn text_sets_and_empty_tables_compare_by_their_members() {
    holds(
        r#"## Main
Let first be a new Set of Text.
Add "x" to first.
Add "y" to first.
Let second be a new Set of Text.
Add "y" to second.
Add "x" to second.
Let empty be a new Set of Text.
Let also_empty be a new Set of Text.
Let sets be [first, second, empty, also_empty].
Show item 1 of sets is equal to item 2 of sets.
Show item 3 of sets is equal to item 4 of sets.
Show item 1 of sets is equal to item 3 of sets.
Show item 3 of sets is equal to item 1 of sets.
"#,
        "true\ntrue\nfalse\nfalse\n",
    );
}

#[test]
fn a_removed_key_leaves_a_map_equal_to_one_that_never_had_it() {
    holds(
        r#"## Main
Let grown be a new Map of Text to Int.
Set item "a" of grown to 1.
Set item "gone" of grown to 5.
Set item "b" of grown to 2.
Remove "gone" from grown.
Let plain be a new Map of Text to Int.
Set item "b" of plain to 2.
Set item "a" of plain to 1.
Let maps be [grown, plain].
Show item 1 of maps is equal to item 2 of maps.
"#,
        "true\n",
    );
}

#[test]
fn a_map_that_grew_and_shrank_equals_one_that_never_grew() {
    holds(
        r#"## Main
Let grown be a new Map of Int to Int.
Repeat for i from 1 to 40:
    Set item i of grown to i * 10.
Repeat for i from 3 to 40:
    Remove i from grown.
Let small be a new Map of Int to Int.
Set item 2 of small to 20.
Set item 1 of small to 10.
Let maps be [grown, small].
Show item 1 of maps is equal to item 2 of maps.
Show item 2 of maps is equal to item 1 of maps.
Show length of grown.
"#,
        "true\ntrue\n2\n",
    );
}

#[test]
fn a_set_that_grew_and_shrank_equals_one_that_never_grew() {
    holds(
        r#"## Main
Let grown be a new Set of Text.
Repeat for i from 1 to 40:
    Add "k{i}" to grown.
Repeat for i from 3 to 40:
    Remove "k{i}" from grown.
Let small be a new Set of Text.
Add "k2" to small.
Add "k1" to small.
Let sets be [grown, small].
Show item 1 of sets is equal to item 2 of sets.
Show item 2 of sets is equal to item 1 of sets.
"#,
        "true\ntrue\n",
    );
}

#[test]
fn text_valued_maps_in_a_list_compare_their_texts() {
    holds(
        r#"## Main
Let first be a new Map of Int to Text.
Set item 2 of first to "two".
Set item 1 of first to "one".
Let second be a new Map of Int to Text.
Set item 1 of second to "one".
Set item 2 of second to "two".
Let renamed be a new Map of Int to Text.
Set item 1 of renamed to "one".
Set item 2 of renamed to "deux".
Let maps be [first, second, renamed].
Show item 1 of maps is equal to item 2 of maps.
Show item 1 of maps is equal to item 3 of maps.
"#,
        "true\nfalse\n",
    );
}

#[test]
fn maps_of_lists_in_a_list_compare_their_lists_item_by_item() {
    holds(
        r#"## Main
Let first be a new Map of Text to List of Int.
Set item "odd" of first to [1, 3].
Set item "even" of first to [2, 4].
Let second be a new Map of Text to List of Int.
Set item "even" of second to [2, 4].
Set item "odd" of second to [1, 3].
Let reordered be a new Map of Text to List of Int.
Set item "even" of reordered to [4, 2].
Set item "odd" of reordered to [1, 3].
Let maps be [first, second, reordered].
Show item 1 of maps is equal to item 2 of maps.
Show item 1 of maps is equal to item 3 of maps.
"#,
        "true\nfalse\n",
    );
}

#[test]
fn two_maps_and_two_sets_compare_by_their_entries_outside_a_list() {
    holds(
        r#"## Main
Let left be a new Map of Text to Int.
Set item "a" of left to 1.
Set item "b" of left to 2.
Let right be a new Map of Text to Int.
Set item "b" of right to 2.
Set item "a" of right to 1.
Show left is equal to right.
Show left is not equal to right.
Set item "a" of right to 9.
Show left is equal to right.
Let s be a new Set of Int.
Add 2 to s.
Add 1 to s.
Let t be a new Set of Int.
Add 1 to t.
Add 2 to t.
Show s is equal to t.
Remove 2 from t.
Show s is equal to t.
"#,
        "true\nfalse\nfalse\ntrue\nfalse\n",
    );
}
