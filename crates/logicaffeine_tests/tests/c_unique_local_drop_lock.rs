//! Direct C must reclaim fresh, non-escaping sequence temporaries just as Rust
//! drops the corresponding `Vec`s. Recursive divide-and-conquer code otherwise
//! retains the whole allocation tree until process exit.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::compile::{compile_to_c, compile_to_c_traced, vm_outcome};
use logicaffeine_compile::optimization::Opt;

const SORT: &str = r#"
## To mergeSort (arr: Seq of Int) -> Seq of Int:
    Let n be length of arr.
    If n is less than 2:
        Return arr.
    Let mid be n / 2.
    Let mutable left be a new Seq of Int.
    Let mutable right be a new Seq of Int.
    Let mutable i be 1.
    While i is at most mid:
        Push item i of arr to left.
        Set i to i + 1.
    While i is at most n:
        Push item i of arr to right.
        Set i to i + 1.
    Set left to mergeSort(left).
    Set right to mergeSort(right).
    Let mutable result be arr.
    Let mutable li be 1.
    Let mutable ri be 1.
    Let mutable k be 1.
    While li is at most length of left and ri is at most length of right:
        If item li of left is at most item ri of right:
            Set item k of result to item li of left.
            Set li to li + 1.
        Otherwise:
            Set item k of result to item ri of right.
            Set ri to ri + 1.
        Set k to k + 1.
    While li is at most length of left:
        Set item k of result to item li of left.
        Set li to li + 1.
        Set k to k + 1.
    While ri is at most length of right:
        Set item k of result to item ri of right.
        Set ri to ri + 1.
        Set k to k + 1.
    Return result.

## Main
Let mutable values be [9, 1, 7, 3, 5, 2, 8, 4, 6].
Set values to mergeSort(values).
Show values.
"#;

const RETURNED_LOCAL: &str = r#"
## To build () -> Seq of Int:
    Let mutable out be a new Seq of Int.
    Push 7 to out.
    Return out.

## Main
Show build().
"#;

fn c_output(source: &str) -> String {
    let c = compile_to_c(source).expect("source compiles to direct C");
    let dir = tempfile::Builder::new()
        .prefix("logos_c_unique_drop_")
        .tempdir()
        .expect("temporary C directory");
    let c_path = dir.path().join("main.c");
    let bin_path = dir.path().join("main");
    std::fs::write(&c_path, c).expect("write generated C");
    let build = common::build_c_release(&c_path, &bin_path).expect("C compiler starts");
    assert!(
        build.status.success(),
        "generated C builds:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new(&bin_path)
        .output()
        .expect("generated C starts");
    assert!(
        run.status.success(),
        "generated C exits cleanly:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    String::from_utf8_lossy(&run.stdout).into_owned()
}

#[test]
fn direct_c_drops_only_the_two_non_escaping_merge_buffers() {
    let (c, trace) = compile_to_c_traced(SORT).expect("sort compiles to traced direct C");
    assert_eq!(
        c.matches("/* unique local drop */").count(),
        2,
        "left and right must be reclaimed, while arr/result still escape:\n{c}"
    );
    assert!(
        trace.lowering.fired(Opt::Unbox),
        "unique-owned lifetime lowering silently disappeared from the C trace: {:?}",
        trace.lowering.keywords()
    );
}

#[test]
fn returned_buffers_are_never_destroyed_and_early_return_is_safe() {
    let returned = compile_to_c(RETURNED_LOCAL).expect("returned-local source compiles");
    assert!(
        !returned.contains("/* unique local drop */"),
        "a buffer handed to the caller must not be destroyed:\n{returned}"
    );

    if !common::require_tool(&common::c_compiler()) {
        return;
    }
    for (name, source) in [("recursive-sort", SORT), ("returned-local", RETURNED_LOCAL)] {
        let vm = vm_outcome(source);
        assert!(vm.error.is_none(), "{name}: VM failed: {:?}", vm.error);
        assert_eq!(
            c_output(source),
            vm.output,
            "{name}: direct C diverged from the VM"
        );
    }
}
