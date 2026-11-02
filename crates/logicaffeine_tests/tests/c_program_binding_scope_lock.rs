//! A function's names on C mean what the tree-walker resolves them to, and its copies of the program's bindings are
//! its own.
//!
//! A `Let` inside a block of a function binds its name for that block only: after the block, the name is the
//! program's binding again. A copy a function makes of one of the program's bindings is a value of its own, unchanged
//! when `## Main` later changes the binding in place. Each program runs on the tree-walker, the VM and a release C
//! build, and each must write the same bytes to standard output.
#![cfg(not(target_arch = "wasm32"))]
mod common;
use logicaffeine_compile::compile::{compile_to_c, tw_lines, vm_outcome};

fn holds(source: &str, stdout: &str, error: Option<&str>) {
    let tw = tw_lines(source);
    let tw_bytes: String = tw.lines.iter().map(|line| format!("{line}\n")).collect();
    assert_eq!((tw_bytes.as_str(), tw.error.as_deref()), (stdout, error), "tree-walker:\n{source}");

    let vm = vm_outcome(source);
    assert_eq!((vm.output.as_str(), vm.error.as_deref()), (stdout, error), "VM:\n{source}");

    let c = compile_to_c(source).unwrap_or_else(|e| panic!("LOGOS→C: {e:?}\n{source}"));
    let dir = tempfile::tempdir().expect("temp dir");
    let (src, bin) = (dir.path().join("main.c"), dir.path().join("main"));
    std::fs::write(&src, &c).expect("write main.c");
    let build = common::build_c_release(&src, &bin).expect("run the C compiler");
    assert!(build.status.success(), "C build:\n{}\n{c}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&bin).output().expect("run the C program");
    assert_eq!(String::from_utf8_lossy(&run.stdout), stdout, "C stdout:\n{source}");
    let want_stderr = error.map(|e| format!("{e}\n")).unwrap_or_default();
    assert_eq!(String::from_utf8_lossy(&run.stderr), want_stderr, "C stderr:\n{source}");
    assert_eq!(run.status.code(), Some(if error.is_some() { 1 } else { 0 }), "C exit:\n{source}");
}

#[test]
fn a_name_a_nested_block_binds_is_the_programs_after_the_block() {
    holds(
        r#"## To look () -> Int:
    If true:
        Let xs be ["local"].
        Show xs.
    Show xs.
    Return 1.
## Main
Let xs be [4, 5].
Show look().
Show xs.
"#,
        "[local]\n[4, 5]\n1\n[4, 5]\n",
        None,
    );
}

#[test]
fn a_functions_copy_of_a_program_list_keeps_its_items_when_main_sets_one() {
    holds(
        r#"## To snap () -> Int:
    Set saved to xs.
    Return 1.
## Main
Let xs be [4, 5].
Let mutable saved be a new Seq of Int.
Show snap().
Set item 1 of xs to 9.
Show saved.
Show xs.
"#,
        "1\n[4, 5]\n[9, 5]\n",
        None,
    );
}

#[test]
fn a_functions_copy_of_a_program_list_keeps_its_length_when_main_pushes() {
    holds(
        r#"## To snap () -> Int:
    Set saved to xs.
    Return length of saved.
## Main
Let xs be [4, 5].
Let mutable saved be a new Seq of Int.
Show snap().
Push 6 to xs.
Set item 2 of xs to 0.
Show saved.
Show xs.
"#,
        "2\n[4, 5]\n[4, 0, 6]\n",
        None,
    );
}

#[test]
fn a_functions_copy_of_a_program_map_keeps_its_entries_when_main_sets_one() {
    holds(
        r#"## To snap () -> Int:
    Set saved to m.
    Return length of saved.
## Main
Let m be a new Map of Text to Int.
Set item "a" of m to 1.
Let mutable saved be a new Map of Text to Int.
Show snap().
Set item "a" of m to 2.
Set item "b" of m to 3.
Show saved.
Show m.
"#,
        "1\n{a: 1}\n{a: 2, b: 3}\n",
        None,
    );
}

#[test]
fn a_list_a_function_hands_back_is_a_copy_of_its_own() {
    holds(
        r#"## To identity (items: Seq of Int) -> Seq of Int:
    Return items.
## Main
Let xs be [4, 5].
Let ys be identity(xs).
Set item 1 of xs to 9.
Show ys.
Show xs.
Let mutable zs be a new Seq of Int.
Set zs to identity(xs).
Set item 2 of xs to 8.
Show zs.
"#,
        "[4, 5]\n[9, 5]\n[9, 5]\n",
        None,
    );
}
