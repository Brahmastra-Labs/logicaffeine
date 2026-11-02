//! Nullable C values compare by the same exact value equality as the VM.
#![cfg(not(target_arch = "wasm32"))]
mod common;
use logicaffeine_compile::compile::{compile_to_c, vm_outcome};

fn check(source: &str, expected_stdout: &str, expected_error: Option<&str>) {
    let vm = vm_outcome(source);
    assert_eq!(vm.error.as_deref(), expected_error, "VM error:\n{source}");
    assert_eq!(vm.output, expected_stdout, "VM output:\n{source}");
    let generated = compile_to_c(source).expect("C emission");
    let dir = tempfile::tempdir().expect("isolated C run");
    let source_path = dir.path().join("main.c");
    let executable = dir.path().join("main");
    std::fs::write(&source_path, &generated).expect("write generated C");
    let built = common::build_c_release(&source_path, &executable).expect("start C compiler");
    assert!(built.status.success(), "{}\n{generated}", String::from_utf8_lossy(&built.stderr));
    let result = std::process::Command::new(&executable).output().expect("run C");
    assert_eq!(String::from_utf8(result.stdout).expect("UTF-8 output"), expected_stdout, "{source}");
    assert_eq!(result.status.code(), Some(if expected_error.is_some() { 1 } else { 0 }), "{source}");
    assert_eq!(String::from_utf8(result.stderr).expect("UTF-8 error"),
        expected_error.map(|e| format!("{e}\n")).unwrap_or_default(), "{source}");
}




#[test]
fn nullable_maps_compare_entries_independently_of_insertion_order() {
    check(r#"## To change (value: mutable Map of Text to Int) -> Int:
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
"#, "true\ntrue\nfalse\ntrue\n1\n", None);
}

#[test]
fn nullable_sets_compare_members_independently_of_insertion_order() {
    check(r#"## To change (value: mutable Set of Int) -> Int:
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
"#, "true\ntrue\nfalse\ntrue\n1\n", None);
}

fn map_case(key: &str, value: &str, k1: &str, k2: &str, k3: &str, v1: &str, v2: &str, v3: &str) {
    let source = format!(r#"## To change (value: mutable Map of {key} to {value}) -> Int:
    Show item 1 of maps is equal to nothing.
    Show item 2 of maps is equal to item 3 of maps.
    Show item 2 of maps is equal to item 4 of maps.
    Show item 2 of maps is equal to item 5 of maps.
    Show item 2 of maps is equal to item 6 of maps.
    Show item 6 of maps is equal to item 2 of maps.
    Show item 7 of maps is equal to item 8 of maps.
    Set value to a new Map of {key} to {value}.
    Return 1.
## Main
Let first be a new Map of {key} to {value}.
Let left be a new Map of {key} to {value}.
Set item {k1} of left to {v1}.
Set item {k2} of left to {v2}.
Let right be a new Map of {key} to {value}.
Set item {k2} of right to {v2}.
Set item {k1} of right to {v1}.
Let changed be a new Map of {key} to {value}.
Set item {k1} of changed to {v1}.
Set item {k2} of changed to {v3}.
Let differentKey be a new Map of {key} to {value}.
Set item {k1} of differentKey to {v1}.
Set item {k3} of differentKey to {v2}.
Let shorter be a new Map of {key} to {value}.
Set item {k1} of shorter to {v1}.
Let emptyLeft be a new Map of {key} to {value}.
Let emptyRight be a new Map of {key} to {value}.
Let maps be [first, left, right, changed, differentKey, shorter, emptyLeft, emptyRight].
Show change(item 1 of maps).
"#);
    check(&source, "true\ntrue\nfalse\nfalse\nfalse\nfalse\ntrue\n1\n", None);
}

#[test]
fn nullable_int_int_maps_check_keys_values_lengths_and_empty_tables() {
    map_case("Int", "Int", "1", "2", "3", "11", "22", "33");
}

#[test]
fn nullable_int_text_maps_check_keys_values_lengths_and_empty_tables() {
    map_case("Int", "Text", "1", "2", "3", r#""one""#, r#""two""#, r#""three""#);
}

#[test]
fn nullable_text_int_maps_probe_colliding_keys() {
    // Both keys hash to the same initial bucket in the C table.
    map_case("Text", "Int", r#""a""#, r#""q""#, r#""z""#, "11", "22", "33");
}

#[test]
fn nullable_text_text_maps_check_keys_values_lengths_and_empty_tables() {
    map_case("Text", "Text", r#""a""#, r#""q""#, r#""z""#,
        r#""one""#, r#""two""#, r#""three""#);
}

#[test]
fn nullable_text_sequence_maps_compare_values_recursively() {
    map_case("Text", "Seq of Int", r#""a""#, r#""q""#, r#""z""#,
        "[1, 2]", "[3, 4]", "[3, 5]");
}

fn set_case(ty: &str, a: &str, b: &str, c: &str) {
    let source = format!(r#"## To change (value: mutable Set of {ty}) -> Int:
    Show item 1 of sets is equal to nothing.
    Show item 2 of sets is equal to item 3 of sets.
    Show item 2 of sets is equal to item 4 of sets.
    Show item 2 of sets is equal to item 5 of sets.
    Show item 5 of sets is equal to item 2 of sets.
    Show item 6 of sets is equal to item 7 of sets.
    Set value to a new Set of {ty}.
    Return 1.
## Main
Let first be a new Set of {ty}.
Let left be a new Set of {ty}.
Add {a} to left.
Add {b} to left.
Let right be a new Set of {ty}.
Add {c} to right.
Add {b} to right.
Add {a} to right.
Remove {c} from right.
Let changed be a new Set of {ty}.
Add {a} to changed.
Add {c} to changed.
Let shorter be a new Set of {ty}.
Add {a} to shorter.
Let emptyLeft be a new Set of {ty}.
Let emptyRight be a new Set of {ty}.
Add {c} to emptyRight.
Remove {c} from emptyRight.
Let sets be [first, left, right, changed, shorter, emptyLeft, emptyRight].
Show change(item 1 of sets).
"#);
    check(&source, "true\ntrue\nfalse\nfalse\nfalse\ntrue\n1\n", None);
}

#[test]
fn nullable_integer_sets_compare_contents_after_removal() {
    set_case("Int", "1", "2", "3");
}

#[test]
fn nullable_text_sets_compare_contents_after_removal_and_collision() {
    set_case("Text", r#""a""#, r#""q""#, r#""z""#);
}
