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
fn a_global_write_preserves_the_earlier_operand_and_the_final_binding() {
    check("## To next -> Int:\n    Set counter to counter + 1.\n    Return counter.\n## Main\nLet mutable counter be 0.\nShow counter + next().\nShow counter.\n",
        "1\n1\n", None);
}
