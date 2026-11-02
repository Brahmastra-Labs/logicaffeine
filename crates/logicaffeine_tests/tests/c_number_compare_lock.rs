//! An integer and a real compare on C as the numbers they are, as on the VM.
//!
//! `is equal to`, `<`, `<=`, `>` and `>=` between an `Int` (a machine word or an exact integer past one) and a `Real`
//! compare the mathematical values, however far past 2^53 the integer is: 9007199254740993 is not 9007199254740993.0,
//! which is the double 9007199254740992. Each program runs on the tree-walker, the VM and a release C build, and each
//! must write the same bytes to standard output.
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
fn an_integer_equals_a_real_only_when_it_is_that_number() {
    holds(
        r#"## Main
Show 2 is equal to 2.0.
Show 9007199254740993 is equal to 9007199254740993.0.
Let x be 3.
Let y be 3.0.
Show x is equal to y.
Show y is equal to x.
Show x is equal to 3.5.
"#,
        "true\nfalse\ntrue\ntrue\nfalse\n",
        None,
    );
}

#[test]
fn an_integer_and_a_real_order_as_the_numbers_do_past_two_to_the_fifty_three() {
    holds(
        r#"## Main
Let big be 9007199254740993.
Let edge be 9007199254740992.0.
Show big > edge.
Show big >= edge.
Show big < edge.
Show edge < big.
Show edge > big.
Show big is equal to edge.
Show 3 < 3.5.
Show 3.5 > 3.
Show 4 <= 4.0.
Show -9007199254740993 < -9007199254740992.0.
Show 9223372036854775807 < 9223372036854775807.0.
"#,
        "true\ntrue\nfalse\ntrue\nfalse\nfalse\ntrue\ntrue\ntrue\ntrue\ntrue\n",
        None,
    );
}

#[test]
fn an_exact_integer_past_a_word_compares_with_a_real_as_the_number_it_is() {
    holds(
        r#"## Main
Let huge be 9223372036854775807 + 1.
Let odd be 9223372036854775807 + 2.
Show huge is equal to 9223372036854775808.0.
Show odd is equal to 9223372036854775808.0.
Show odd > 9223372036854775808.0.
Show odd >= 9223372036854775808.0.
Show odd < 9223372036854775808.0.
Show 9223372036854775808.0 < odd.
Show huge < 9300000000000000000.0.
Show odd is not equal to 9223372036854775808.0.
"#,
        "true\nfalse\ntrue\ntrue\nfalse\ntrue\ntrue\ntrue\n",
        None,
    );
}
