//! Record integer fields carry the same exact values through stores, copies and calls as the VM.
#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{compile_to_c_via_bytecode, vm_outcome};
use std::process::Command;

fn check(label: &str, body: &str, expected: &str) {
    let source = format!("## A Box has:\n    a value, which is Int.\n\n{body}");
    let vm = vm_outcome(&source);
    assert_eq!(vm.error, None, "{label}: VM rejected the fixture");
    assert_eq!(vm.output, expected, "{label}: independent expected output");
    let c = compile_to_c_via_bytecode(&source).expect("bytecode C must lower record integers");
    let dir = tempfile::tempdir().expect("record fixture directory");
    let src = dir.path().join("probe.c");
    let bin = dir.path().join("probe");
    std::fs::write(&src, c).unwrap();
    let built = Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&src)
        .arg("-lm")
        .output()
        .expect("GCC must be available");
    assert!(
        built.status.success(),
        "{label}: GCC failed: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    let out = Command::new("timeout")
        .arg("20s")
        .arg(bin)
        .output()
        .expect("run bytecode C");
    assert!(
        out.status.success(),
        "{label}: C failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.stderr, b"",
        "{label}: successful run must have empty stderr"
    );
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        expected,
        "{label}: exact C output"
    );
}

#[test]
fn exact_record_fields_accept_integers_beyond_a_word() {
    check(
        "wide construction",
        r#"## Main
Let p be 2 ** 70.
Let b be a new Box with value p.
Show b's value.
"#,
        "1180591620717411303424\n",
    );
}

#[test]
fn exact_record_fields_keep_small_integer_values() {
    check(
        "small control",
        r#"## Main
Let mutable b be a new Box with value 7.
Set b's value to b's value + 4.
Show b's value.
"#,
        "11\n",
    );
}

#[test]
fn exact_record_fields_replace_small_and_large_values() {
    check(
        "overwrite",
        r#"## Main
Let p be 2 ** 70.
Let mutable b be a new Box with value 7.
Set b's value to p.
Show b's value.
Set b's value to 0 - p.
Show b's value.
Set b's value to 0.
Show b's value.
"#,
        "1180591620717411303424\n-1180591620717411303424\n0\n",
    );
}

#[test]
fn exact_record_fields_survive_copy_and_function_boundaries() {
    check(
        "copy and call",
        r#"## No Inline
## To boxValue (b: Box) -> Int:
    Return b's value.

## Main
Let p be 2 ** 70.
Let original be a new Box with value p.
Let mutable copied be copy of original.
Set copied's value to p + 1.
Show boxValue(original).
Show boxValue(copied).
"#,
        "1180591620717411303424\n1180591620717411303425\n",
    );
}
