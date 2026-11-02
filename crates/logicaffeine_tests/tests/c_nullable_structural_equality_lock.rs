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
fn nullable_nested_lists_compare_contents_and_absence() {
    check("## To change (value: mutable Seq of Int) -> Int:
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
", "true\ntrue\ntrue\ntrue\nfalse\ntrue\nfalse\n1\n[[7, 8], [1, 2], [1, 2]]\n", None);
}

#[test]
fn nullable_text_lists_compare_elements_not_buffer_addresses() {
    check("## To change (value: mutable Seq of Text) -> Int:
    Show item 2 of rows is equal to item 3 of rows.
    Show item 2 of rows is equal to [\"a\", \"b\"].
    Show item 2 of rows is equal to [\"a\", \"c\"].
    Set value to [\"new\"].
    Return 1.
## Main
Let rows be [[\"old\"], [\"a\", \"b\"], [\"a\", \"b\"]].
Show change(item 1 of rows).
Show rows.
", "true\ntrue\nfalse\n1\n[[new], [a, b], [a, b]]\n", None);
}

#[test]
fn nullable_records_compare_fields_not_allocations() {
    check("## A Box has:
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
", "true\ntrue\ntrue\nfalse\n1\n", None);
}

#[test]
fn nullable_list_equality_checks_empty_and_unequal_lengths() {
    check("## To change (value: mutable Seq of Int) -> Int:
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
", "true\nfalse\nfalse\nfalse\nfalse\n1\n", None);
}

#[test]
fn nullable_record_equality_checks_scalar_fields_independently() {
    check("## A Box has:
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
", "false\ntrue\n1\n", None);
}
