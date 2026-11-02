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
fn membership_compares_absent_and_present_integer_elements() {
    check("## To change (value: mutable Int) -> Int:\n    Show xs contains nothing.\n    Show xs contains 4.\n    Show xs contains 5.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [4, 5].\nShow change(item 1 of xs).\nShow xs.\n",
        "true\nfalse\ntrue\n1\n[7, 5]\n", None);
}

#[test]
fn membership_compares_absent_and_present_text_elements() {
    check("## To change (value: mutable Text) -> Int:\n    Show xs contains nothing.\n    Show xs contains \"one\".\n    Show xs contains \"two\".\n    Set value to \"new\".\n    Return 1.\n## Main\nLet xs be [\"one\", \"two\"].\nShow change(item 1 of xs).\nShow xs.\n",
        "true\nfalse\ntrue\n1\n[new, two]\n", None);
}

#[test]
fn popping_an_absent_element_returns_nothing_without_an_extra_presence_layer() {
    check("## To change (value: mutable Int) -> Int:\n    Let mutable snapshot be xs.\n    Pop from snapshot into popped.\n    Show popped is equal to nothing.\n    Show popped.\n    Show length of snapshot.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [4, 5].\nShow change(item 2 of xs).\nShow xs.\n",
        "true\nnothing\n1\n1\n[4, 7]\n", None);
}

#[test]
fn copying_an_absent_scalar_preserves_nothing() {
    check("## To change (value: mutable Int) -> Int:\n    Let saved be copy of item 1 of xs.\n    Show saved is equal to nothing.\n    Show saved.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [4, 5].\nShow change(item 1 of xs).\nShow xs.\n",
        "true\nnothing\n1\n[7, 5]\n", None);
}

#[test]
fn an_inferred_collection_return_preserves_absent_elements() {
    check("## To identity (items: Seq of Int):\n    Return items.\n## To change (value: mutable Int) -> Int:\n    Show identity(xs).\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [4, 5].\nShow change(item 1 of xs).\nShow xs.\n",
        "[nothing, 5]\n1\n[7, 5]\n", None);
}

#[test]
fn an_index_used_in_optional_arithmetic_runs_once() {
    check("## To chooseIndex -> Int:\n    Show \"index\".\n    Return 2.\n## To change (value: mutable Int) -> Int:\n    Show item (chooseIndex()) of xs + 1.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [4, 5].\nShow change(item 1 of xs).\nShow xs.\n",
        "index\n6\n1\n[7, 5]\n", None);
}

#[test]
fn a_call_used_in_optional_arithmetic_runs_once() {
    check("## To fetch -> Int:\n    Show \"fetch\".\n    Return item 2 of xs.\n## To change (value: mutable Int) -> Int:\n    Show fetch() + 1.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [4, 5].\nShow change(item 1 of xs).\nShow xs.\n",
        "fetch\n6\n1\n[7, 5]\n", None);
}
