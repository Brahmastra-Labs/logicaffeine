//! Lock for `Opt::TextSliceReturn` — the zero-copy sub-slice return pass (the trim family).
//!
//! A fn that returns a CONTIGUOUS sub-slice of a borrowed Text param (compute lo/hi by scans, then
//! copy `item lo..hi of text_bytes(s)` into a builder and `text_from_bytes(out)`) lowers to a
//! borrowed `&'a str` return (`fn f<'a>(s: &'a str) -> &'a str { &s[lo-1..hi] }`) — Rust's O(1)
//! `str::trim`/`strip_prefix` technique instead of the O(n) build-copy. Gates off under
//! `## No textslicereturn`, and BAILS to the owned path when any caller lets the result escape.

use logicaffeine_compile::compile::{
    compile_to_program, compile_to_rust, program_outcome, run_logos_source,
};
use logicaffeine_compile::optimization::decorate_source;

/// The VM tier runs the real (unchanged) trim fn — the correctness ORACLE.
fn vm_output(src: &str) -> String {
    program_outcome(&compile_to_program(src).expect("VM compiles"))
        .output
        .trim()
        .to_string()
}
/// The AOT Rust tier — where TextSliceReturn actually fires.
fn aot_output(src: &str) -> String {
    run_logos_source(src)
        .expect("AOT compiles and runs")
        .trim()
        .to_string()
}

/// Exercises the trim edge cases with RUNTIME inputs (a Seq iterated — no const-folding, and the fn
/// is called by value each iteration): leading-space, no-space, all-space, empty, interior-space.
const EDGE_PROG: &str = "\
## To trimStartOne (s: Text) -> Text:
    Let sb be text_bytes(s).
    Let n be length of sb.
    Let mutable lo be 1.
    Let mutable go be true.
    While go is equal to true:
        If lo is greater than n:
            Set go to false.
        Otherwise:
            If (item lo of sb) is equal to 32:
                Set lo to lo + 1.
            Otherwise:
                Set go to false.
    Let out be a new Seq of Int.
    Let mutable i be lo.
    While i is at most n:
        Push item i of sb to out.
        Set i to i + 1.
    Return text_from_bytes(out).

## Main
Let inputs be a new Seq of Text.
Push \"  hello\" to inputs.
Push \"world\" to inputs.
Push \"   \" to inputs.
Push \"\" to inputs.
Push \" a b \" to inputs.
Repeat for x in inputs:
    Show trimStartOne(x).
";

/// A trimStart-shaped fn (borrowed Text param `s`, single contiguous-slice return), driven by a
/// runtime arg and used INLINE by `Show` (read-only) so the caller-gate keeps the slice return.
const TRIM: &str = "\
## To native args () -> Seq of Text

## To trimStartOne (s: Text) -> Text:
    Let sb be text_bytes(s).
    Let n be length of sb.
    Let mutable lo be 1.
    Let mutable go be true.
    While go is equal to true:
        If lo is greater than n:
            Set go to false.
        Otherwise:
            If (item lo of sb) is equal to 32:
                Set lo to lo + 1.
            Otherwise:
                Set go to false.
    Let out be a new Seq of Int.
    Let mutable i be lo.
    While i is at most n:
        Push item i of sb to out.
        Set i to i + 1.
    Return text_from_bytes(out).

## Main
Let arguments be args().
Show trimStartOne(item 2 of arguments).
";

/// Same slice fn, but a caller BINDS the result (`Let t be trimStartOne(s)`) — a borrowed &str must
/// not escape into an owned binding, so the caller-gate must drop it back to the owned String path.
const STORED: &str = "\
## To native args () -> Seq of Text

## To trimStartOne (s: Text) -> Text:
    Let sb be text_bytes(s).
    Let n be length of sb.
    Let mutable lo be 1.
    Let mutable go be true.
    While go is equal to true:
        If lo is greater than n:
            Set go to false.
        Otherwise:
            If (item lo of sb) is equal to 32:
                Set lo to lo + 1.
            Otherwise:
                Set go to false.
    Let out be a new Seq of Int.
    Let mutable i be lo.
    While i is at most n:
        Push item i of sb to out.
        Set i to i + 1.
    Return text_from_bytes(out).

## To useIt (s: Text) -> Text:
    Let t be trimStartOne(s).
    Return t.

## Main
Let arguments be args().
Show useIt(item 2 of arguments).
";

#[test]
fn text_slice_return_lowers_to_borrowed_slice() {
    let rust = compile_to_rust(TRIM).expect("TRIM compiles");
    assert!(
        rust.contains("&'a str"),
        "the slice fn must gain a borrowed `&'a str` signature; got:\n{rust}"
    );
    assert!(
        rust.contains("return logicaffeine_data::logos_text_run_through(&s, lo, n)"),
        "the return must use the borrowed text range helper; got:\n{rust}"
    );
    assert!(
        !rust.contains("text_from_bytes(&"),
        "the O(n) build + text_from_bytes round-trip must be eliminated; got:\n{rust}"
    );
}

#[test]
fn text_slice_return_gates_off() {
    let off = decorate_source(TRIM, &["textslicereturn"]);
    let rust = compile_to_rust(&off).expect("disabled TRIM compiles");
    assert!(
        rust.contains("fn trimStartOne(s: String) -> String")
            && rust.contains("return String::from_utf8(out)"),
        "`## No textslicereturn` must keep the owned build path; got:\n{rust}"
    );
    assert!(
        !rust.contains("&'a str"),
        "disabled: no borrowed-slice signature; got:\n{rust}"
    );
}

/// PIT-OF-SUCCESS correctness lock: the AOT slice-return must be BYTE-IDENTICAL to the VM oracle
/// (which runs the real copy-loop trim) AND to the AOT owned path (opt disabled). This catches a
/// wrong slice range (off-by-one lo/hi, bad char boundary) and any output drift — the exact class of
/// silent bug a "does it compile / does it fire" test would miss.
#[test]
fn text_slice_return_is_byte_identical_all_tiers_and_toggle() {
    let vm = vm_output(EDGE_PROG);
    let on = aot_output(EDGE_PROG);
    let off = aot_output(&decorate_source(EDGE_PROG, &["textslicereturn"]));
    assert_eq!(
        on, vm,
        "AOT slice-return diverged from the VM oracle (miscompiled trim range?)"
    );
    assert_eq!(
        on, off,
        "TextSliceReturn ON changed output vs OFF — the optimization is not behavior-preserving"
    );
}

#[test]
fn text_slice_return_bails_when_result_stored() {
    let rust = compile_to_rust(STORED).expect("STORED compiles");
    assert!(
        rust.contains("fn trimStartOne(s: String) -> String")
            && rust.contains("return String::from_utf8(out)"),
        "a caller binding the result must force the owned String path; got:\n{rust}"
    );
}

/// ★ THE SHAPE THAT BROKE EVERY LOGOS WEB SERVER. `assets/std/web.lg`'s `bodyParam` reads
/// `If startsWith(trim(body), brace):` — the slice fn's result passed as an ARGUMENT to a
/// `Text`-taking function, inside a CONDITION. The caller-gate saw neither position: it inspected
/// only the top-level expression of a statement, and its `Stmt::If` arm recursed into the blocks
/// while never looking at the condition. So `trim` kept its borrowed `-> &'a str`, `startsWith`
/// still wanted an owned `String`, and the generated Rust did not compile:
///
/// ```text
/// error[E0308]: mismatched types
///   if logos_truthy(&(startsWith(trim(&body), brace.clone()))) {
///                     ---------- ^^^^^^^^^^^ expected `String`, found `&str`
/// error: the generated Rust failed to compile — this is a LOGOS compiler bug, not an error in your program
/// ```
///
/// `bodyParam` is pulled into every `## Route` program, so on 2026-09-11 this one gap failed
/// `todo_deploy_e2e`, `real_site_deploy_lock` and `json_decode_deploys_lock` — ~991 seconds of the
/// baseline — and no LOGOS web server could be built at all. The gate's own note had predicted the
/// outcome ("any missed escape surfaces as a LOUD `&str`-vs-`String` compile error") and shipped it.
///
/// This runs the program rather than only compiling it, so the bail is proved to be CORRECT and not
/// merely buildable.
#[test]
fn a_slice_result_passed_to_a_text_parameter_still_builds_and_runs() {
    const ARG_POSITION: &str = "\
## To trimStartOne (s: Text) -> Text:
    Let sb be text_bytes(s).
    Let n be length of sb.
    Let mutable lo be 1.
    Let mutable go be true.
    While go is equal to true:
        If lo is greater than n:
            Set go to false.
        Otherwise:
            If (item lo of sb) is equal to 32:
                Set lo to lo + 1.
            Otherwise:
                Set go to false.
    Let out be a new Seq of Int.
    Let mutable i be lo.
    While i is at most n:
        Push item i of sb to out.
        Set i to i + 1.
    Return text_from_bytes(out).

## To startsWithOne (s: Text) and (p: Text) -> Bool:
    Let sb be text_bytes(s).
    Let pb be text_bytes(p).
    If (length of pb) is greater than (length of sb):
        Return false.
    Let mutable i be 1.
    While i is at most (length of pb):
        If (item i of sb) is not equal to (item i of pb):
            Return false.
        Set i to i + 1.
    Return true.

## Main
Let body be \"  hello world\".
If startsWithOne(trimStartOne(body), \"hello\"):
    Show \"greeting\".
Otherwise:
    Show \"other\".
";
    assert_eq!(
        aot_output(ARG_POSITION),
        "greeting",
        "the AOT tier must build AND answer correctly"
    );
    assert_eq!(vm_output(ARG_POSITION), "greeting", "the VM oracle agrees");

    // The bail is the mechanism: `startsWithOne` takes an owned `Text`, so the borrowed return
    // must NOT survive to its call site.
    let rust = compile_to_rust(ARG_POSITION).expect("compiles");
    assert!(
        rust.contains("fn trimStartOne(s: String) -> String")
            && rust.contains("return String::from_utf8(out)"),
        "a slice result consumed by a Text parameter must fall back to the owned path; got:\n{rust}"
    );
}
