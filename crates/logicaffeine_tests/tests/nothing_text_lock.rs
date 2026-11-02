//! `nothing` joined into text, or shown, reads as the word `nothing` on every tier, as on the VM.
//!
//! The literal `nothing` (and `none`, the same absence) in a `+` or `combined with` that joins text, in an
//! interpolation hole, or after `Show` is the word the VM writes for it. Each program runs on the tree-walker, the VM,
//! a release C build and the generated Rust, and each must write the same bytes to standard output.
#![cfg(not(target_arch = "wasm32"))]
mod common;
use logicaffeine_compile::compile::{compile_to_c, tw_lines, vm_outcome};

fn holds(source: &str, stdout: &str) {
    let tw = tw_lines(source);
    let tw_bytes: String = tw.lines.iter().map(|line| format!("{line}\n")).collect();
    assert_eq!((tw_bytes.as_str(), tw.error.as_deref()), (stdout, None), "tree-walker:\n{source}");

    let vm = vm_outcome(source);
    assert_eq!((vm.output.as_str(), vm.error.as_deref()), (stdout, None), "VM:\n{source}");

    let c = compile_to_c(source).unwrap_or_else(|e| panic!("LOGOS→C: {e:?}\n{source}"));
    let dir = tempfile::tempdir().expect("temp dir");
    let (src, bin) = (dir.path().join("main.c"), dir.path().join("main"));
    std::fs::write(&src, &c).expect("write main.c");
    let build = common::build_c_release(&src, &bin).expect("run the C compiler");
    assert!(build.status.success(), "C build:\n{}\n{c}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&bin).output().expect("run the C program");
    assert_eq!(String::from_utf8_lossy(&run.stdout), stdout, "C stdout:\n{source}");
    assert!(run.status.success(), "C exit:\n{source}");

    let rust = common::run_logos(source);
    assert!(rust.success, "Rust run:\n{}\n\n{source}\n\n{}", rust.stderr, rust.rust_code);
    assert_eq!(rust.stdout, stdout, "Rust stdout:\n{source}\n\n{}", rust.rust_code);
}

#[test]
fn nothing_joined_after_text_reads_as_the_word_nothing() {
    holds(
        r#"## Main
Let s be "a" + nothing.
Show s.
"#,
        "anothing\n",
    );
}

#[test]
fn nothing_joined_before_text_reads_as_the_word_nothing() {
    holds(
        r#"## Main
Let t be nothing + "b".
Show t.
"#,
        "nothingb\n",
    );
}

#[test]
fn nothing_in_an_interpolation_hole_reads_as_the_word_nothing() {
    holds(
        r#"## Main
Show "x{nothing}y".
"#,
        "xnothingy\n",
    );
}

#[test]
fn showing_nothing_writes_the_word_nothing() {
    holds(
        r#"## Main
Show nothing.
"#,
        "nothing\n",
    );
}

#[test]
fn none_joined_into_text_reads_as_nothing() {
    holds(
        r#"## Main
Let s be "a" + none.
Show s.
"#,
        "anothing\n",
    );
}

#[test]
fn nothing_among_numbers_and_text_in_one_join_reads_as_nothing() {
    holds(
        r#"## Main
Let n be 5.
Show "n=" + n + ", m=" + nothing + "!".
"#,
        "n=5, m=nothing!\n",
    );
}
