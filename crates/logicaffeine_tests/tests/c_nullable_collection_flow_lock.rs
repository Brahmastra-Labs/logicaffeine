//! C collection loans preserve absence in storage and through value copies.
#![cfg(not(target_arch = "wasm32"))]
mod common;
use logicaffeine_compile::compile::{compile_to_c, vm_outcome};

fn check(source: &str, expected_stdout: &str, expected_error: Option<&str>) {
    let vm = vm_outcome(source);
    assert_eq!(vm.output, expected_stdout, "VM output:\n{source}");
    assert_eq!(vm.error.as_deref(), expected_error, "VM error:\n{source}");
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
fn a_nested_sequence_clears_only_the_lent_leaf_and_rebuilds_it() {
    check("## To change (value: mutable Int) -> Int:
    Show rows.
    Set value to 9.
    Return 1.
## Main
Let rows be [[4, 5], [6, 7]].
Show change(item 1 of item 2 of rows).
Show rows.
", "[[4, 5], [nothing, 7]]\n1\n[[4, 5], [9, 7]]\n", None);
}

#[test]
fn a_copy_made_through_a_function_during_a_loan_keeps_the_absence() {
    check("## To identity (items: Seq of Int) -> Seq of Int:
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
", "1\n[nothing, 5]\n[7, 5]\n", None);
}

#[test]
fn a_shadowing_text_sequence_keeps_its_own_layout() {
    check("## To change (value: mutable Int) -> Int:
    If true:
        Let xs be [\"local\"].
        Show xs.
    Show xs.
    Set value to 7.
    Return 1.
## Main
Let xs be [4, 5].
Show change(item 1 of xs).
Show xs.
", "[local]\n[nothing, 5]\n1\n[7, 5]\n", None);
}

#[test]
fn an_integer_map_keeps_its_key_while_its_value_is_lent() {
    check("## To change (value: mutable Int) -> Int:
    Show item 3 of values.
    Show length of values.
    Set value to 7.
    Return 1.
## Main
Let values be a new Map of Int to Int.
Set item 3 of values to 4.
Show change(item 3 of values).
Show item 3 of values.
", "nothing\n1\n1\n7\n", None);
}

#[test]
fn an_absent_element_survives_a_sequence_parameter_and_return() {
    check("## To identity (items: Seq of Int) -> Seq of Int:
    Return items.
## To change (value: mutable Int) -> Int:
    Show identity(xs).
    Set value to 7.
    Return 1.
## Main
Let xs be [4, 5].
Show change(item 1 of xs).
Show xs.
", "[nothing, 5]\n1\n[7, 5]\n", None);
}

#[test]
fn arithmetic_on_an_absent_element_fails_as_it_does_on_the_vm() {
    check("## To change (value: mutable Int) -> Int:
    Show item 1 of xs + 1.
    Return 1.
## Main
Let xs be [4].
Show change(item 1 of xs).
", "", Some("Cannot add Nothing and Int"));
}

#[test]
fn an_absent_scalar_survives_a_readonly_parameter_and_return() {
    check("## To identity (value: Int) -> Int:
    Return value.
## To change (value: mutable Int) -> Int:
    Show identity(item 1 of xs).
    Set value to 7.
    Return 1.
## Main
Let xs be [4].
Show change(item 1 of xs).
Show xs.
", "nothing\n1\n[7]\n", None);
}
