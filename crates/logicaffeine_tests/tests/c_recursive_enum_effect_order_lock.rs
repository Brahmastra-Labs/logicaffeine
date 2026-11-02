//! Recursive enum allocation must preserve observable field evaluation order.
#![cfg(not(target_arch = "wasm32"))]
mod common;
const SOURCE: &str = r#"## A Tree is either:
    A Leaf.
    A Node with left Tree and stamp Int and right Tree.
## No Inline
## To leaf (n: Int) -> Tree:
    Show n.
    Return a new Leaf.
## No Inline
## To number (n: Int) -> Int:
    Show n.
    Return n.
## Main
Let tree be a new Node with left leaf(1) and stamp number(2) and right leaf(3).
Inspect tree:
    When Leaf: Show 0.
    When Node (left, stamp, right): Show stamp.
"#;
fn check(source: &str) {
    check_output(source, b"1\n2\n3\n2\n");
}
fn check_output(source: &str, wanted: &[u8]) {
    let expected = logicaffeine_compile::compile::vm_outcome(source);
    assert_eq!(expected.error, None);
    assert_eq!(
        expected.output.lines().collect::<Vec<_>>(),
        std::str::from_utf8(wanted)
            .unwrap()
            .lines()
            .collect::<Vec<_>>()
    );
    let c = logicaffeine_compile::compile::compile_to_c(source).expect("valid enum C program");
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("program.c");
    let exe = dir.path().join("program");
    std::fs::write(&file, c).unwrap();
    let built = common::build_c_release(&file, &exe).expect("C compiler starts");
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let got = std::process::Command::new("timeout")
        .arg("10s")
        .arg(&exe)
        .output()
        .expect("C program starts");
    assert!(
        got.status.success(),
        "{}",
        String::from_utf8_lossy(&got.stderr)
    );
    assert_eq!(got.stdout, wanted);
    assert!(got.stderr.is_empty());
}
#[test]
fn recursive_and_scalar_fields_keep_source_order() {
    check(SOURCE);
}
#[test]
fn disabling_allocation_batching_keeps_source_order() {
    check(&logicaffeine_compile::optimization::decorate_source(
        SOURCE,
        &["scalarize"],
    ));
}

#[test]
fn allocation_storage_cannot_capture_a_user_parameter() {
    let source = r#"## A Tree is either:
    A Leaf.
    A Node with left Tree and stamp Int and right Tree.
## No Inline
## To leaf (n: Int) -> Tree:
    Show n.
    Return a new Leaf.
## No Inline
## To make (__recursive_fields: Int) -> Tree:
    Return a new Node with left leaf(1) and stamp __recursive_fields and right leaf(3).
## Main
Let tree be make(7).
Inspect tree:
    When Leaf: Show 0.
    When Node (left, stamp, right): Show stamp.
"#;
    for parameter in [
        "__recursive_fields",
        "__logos_recursive_fields_0",
        "__logos_recursive_fields_1",
    ] {
        check_output(
            &source.replace("__recursive_fields", parameter),
            b"1\n3\n7\n",
        );
    }
}
