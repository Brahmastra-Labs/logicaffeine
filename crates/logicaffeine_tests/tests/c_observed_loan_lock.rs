//! A loan something can see leaves `nothing` in the lent place on C, as on the VM.
//!
//! The VM moves what a `mutable` parameter is lent out of its place for as long as the call holds it: a callee that
//! reads the lent global reads `nothing`, an assignment to it is seen until the parameter's value is stored back, and
//! an operation on it fails in the VM's words for that operation on Nothing. Each program runs on the tree-walker, the
//! VM and a release C build, and each must write the same bytes to standard output and fail, where it fails, with the
//! same words and exit status.
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
fn a_lent_global_integer_reads_nothing_until_the_call_returns() {
    holds(
        r#"## To change (value: mutable Int) -> Int:
    Show n.
    Set value to 7.
    Return 1.
## Main
Let n be 4.
Show change(n).
Show n.
"#,
        "nothing\n1\n7\n",
        None,
    );
}

#[test]
fn a_lent_global_text_reads_nothing_until_the_call_returns() {
    holds(
        r#"## To append (value: mutable Text) -> Int:
    Show s.
    Set value to value + "!".
    Return length of value.
## Main
Let s be "hello".
Show append(s).
Show s.
"#,
        "nothing\n6\nhello!\n",
        None,
    );
}

#[test]
fn a_lent_global_list_reads_nothing_until_the_call_returns() {
    holds(
        r#"## To change (value: mutable Seq of Int) -> Int:
    Show xs.
    Push 9 to value.
    Return length of value.
## Main
Let xs be [1].
Push 2 to xs.
Show change(xs).
Push 10 to xs.
Show xs.
"#,
        "nothing\n3\n[1, 2, 9, 10]\n",
        None,
    );
}

#[test]
fn assigning_the_lent_global_is_seen_until_the_parameter_is_stored_back() {
    holds(
        r#"## To change (value: mutable Int) -> Int:
    Show n.
    Set n to 99.
    Show n.
    Set value to 7.
    Return 1.
## Main
Let n be 4.
Show change(n).
Show n.
"#,
        "nothing\n99\n1\n7\n",
        None,
    );
}

#[test]
fn a_loan_nothing_sees_keeps_its_value_in_place() {
    holds(
        r#"## To bump (value: mutable Int) -> Int:
    Set value to value + 1.
    Return value.
## Main
Let n be 4.
Let xs be [1, 2, 3].
Show bump(n).
Show bump(item 2 of xs).
Show n.
Show xs.
"#,
        "5\n3\n5\n[1, 3, 3]\n",
        None,
    );
}

#[test]
fn two_items_lent_at_different_literal_keys_see_each_other_whole() {
    holds(
        r#"## To swap (a: mutable Int, b: mutable Int) -> Int:
    Let t be a.
    Set a to b.
    Set b to t.
    Return 1.
## Main
Let xs be [1, 2, 3].
Show swap(item 1 of xs, item 3 of xs).
Show xs.
"#,
        "",
        Some("`xs` is lent to `swap`'s `mutable` parameter `a` for the whole call — it is `swap`'s alone until the call returns — and argument 2 of the same call lends it again. Pass `copy of xs` there, or take what you need from `xs` into a `Let` before the call."),
    );
}

#[test]
fn a_list_and_a_set_lent_whole_work_as_before_once_returned() {
    holds(
        r#"## To change (value: mutable Seq of Int) -> Int:
    Show xs.
    Push 9 to value.
    Return length of value.
## To grow (value: mutable Set of Int) -> Int:
    Show s.
    Add 7 to value.
    Return length of value.
## Main
Let xs be [3, 1, 2].
Show change(xs).
Show item 2 of xs.
Set item 1 of xs to 5.
Pop from xs.
Show xs contains 2.
Show items 1 through 2 of xs.
Repeat for x in xs:
    Show x.
Let s be a new Set of Int.
Add 1 to s.
Show grow(s).
Remove 1 from s.
Show s.
"#,
        "nothing\n4\n1\ntrue\n[5, 1]\n5\n1\n2\nnothing\n2\n{7}\n",
        None,
    );
}

#[test]
fn copies_made_before_during_and_after_a_loan_are_their_own() {
    holds(
        r#"## To change (value: mutable Seq of Int) -> Int:
    Let seen be xs.
    Show seen.
    Push 9 to value.
    Return length of value.
## Main
Let xs be [1, 2].
Let before be xs.
Show change(xs).
Let after be xs.
Push 4 to after.
Push 5 to xs.
Show before.
Show after.
Show xs.
"#,
        "nothing\n3\n[1, 2]\n[1, 2, 9, 4]\n[1, 2, 9, 5]\n",
        None,
    );
}

#[test]
fn a_lent_global_equals_nothing_until_returned_and_its_value_after() {
    holds(
        r#"## To change (value: mutable Seq of Int) -> Int:
    Show xs is equal to nothing.
    Show xs is equal to [1, 2].
    Push 9 to value.
    Return 1.
## To bump (value: mutable Int) -> Int:
    Show n is equal to nothing.
    Show n is equal to 4.
    Show n is equal to 4.0.
    Set value to value + 1.
    Return 1.
## Main
Let xs be [1, 2].
Let ys be [1, 2, 9].
Show change(xs).
Show xs is equal to ys.
Show xs is equal to [1, 2, 9].
Show xs is not equal to ys.
Let n be 4.
Show bump(n).
Show n is equal to 5.
Show n is equal to 5.0.
Show n is equal to nothing.
"#,
        "true\nfalse\n1\ntrue\ntrue\nfalse\ntrue\nfalse\nfalse\n1\ntrue\ntrue\nfalse\n",
        None,
    );
}

#[test]
fn arithmetic_on_a_lent_global_fails_as_the_vm_fails() {
    holds(
        r#"## To change (value: mutable Int) -> Int:
    Show n + 1.
    Return 1.
## Main
Let n be 4.
Show change(n).
"#,
        "",
        Some("Cannot add Nothing and Int"),
    );
}

#[test]
fn the_length_of_a_lent_list_fails_as_the_vm_fails() {
    holds(
        r#"## To change (value: mutable Seq of Int) -> Int:
    Show length of xs.
    Push 9 to value.
    Return length of value.
## Main
Let xs be [3, 1, 2].
Show change(xs).
Show xs.
"#,
        "",
        Some("Cannot get length of Nothing"),
    );
}

#[test]
fn an_item_of_a_lent_list_fails_as_the_vm_fails() {
    holds(
        r#"## To change (value: mutable Seq of Int) -> Int:
    Show item 1 of xs.
    Push 9 to value.
    Return length of value.
## Main
Let xs be [3, 1, 2].
Show change(xs).
Show xs.
"#,
        "",
        Some("Cannot index Nothing with Int"),
    );
}

#[test]
fn setting_an_item_of_a_lent_list_fails_as_the_vm_fails() {
    holds(
        r#"## To change (value: mutable Seq of Int) -> Int:
    Set item 1 of xs to 4.
    Push 9 to value.
    Return length of value.
## Main
Let xs be [3, 1, 2].
Show change(xs).
Show xs.
"#,
        "",
        Some("Cannot index into Nothing"),
    );
}

#[test]
fn pushing_to_a_lent_list_fails_as_the_vm_fails() {
    holds(
        r#"## To change (value: mutable Seq of Int) -> Int:
    Push 1 to xs.
    Push 9 to value.
    Return length of value.
## Main
Let xs be [3, 1, 2].
Show change(xs).
Show xs.
"#,
        "",
        Some("Can only push to a List"),
    );
}

#[test]
fn popping_from_a_lent_list_fails_as_the_vm_fails() {
    holds(
        r#"## To change (value: mutable Seq of Int) -> Int:
    Pop from xs.
    Push 9 to value.
    Return length of value.
## Main
Let xs be [3, 1, 2].
Show change(xs).
Show xs.
"#,
        "",
        Some("Cannot get length of Nothing"),
    );
}

#[test]
fn membership_in_a_lent_list_fails_as_the_vm_fails() {
    holds(
        r#"## To change (value: mutable Seq of Int) -> Int:
    Show xs contains 1.
    Push 9 to value.
    Return length of value.
## Main
Let xs be [3, 1, 2].
Show change(xs).
Show xs.
"#,
        "",
        Some("Cannot check contains on Nothing"),
    );
}

#[test]
fn a_slice_of_a_lent_list_fails_as_the_vm_fails() {
    holds(
        r#"## To change (value: mutable Seq of Int) -> Int:
    Show items 1 through 1 of xs.
    Push 9 to value.
    Return length of value.
## Main
Let xs be [3, 1, 2].
Show change(xs).
Show xs.
"#,
        "",
        Some("Slice requires List and Int indices"),
    );
}

#[test]
fn iterating_a_lent_list_fails_as_the_vm_fails() {
    holds(
        r#"## To change (value: mutable Seq of Int) -> Int:
    Repeat for x in xs:
        Show x.
    Return length of value.
## Main
Let xs be [3, 1, 2].
Show change(xs).
"#,
        "",
        Some("Cannot iterate over Nothing"),
    );
}
