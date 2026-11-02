//! Lock for `Opt::StringRepeatLoop` — the string repeat-idiom pass.
//!
//! `Let out be ""` + a counted loop whose sole body is `Set out to out + chunk` (chunk
//! loop-invariant) lowers to `let mut out = chunk.repeat(count)` — one presized alloc + memset,
//! matching Rust's `chunk.repeat(n)` — instead of `count` un-presized `push_str`es. Gates off under
//! `## No stringrepeatloop`; byte-identical across all tiers (including n=0 / empty-chunk edges).

use logicaffeine_compile::compile::{
    compile_to_program, compile_to_rust, program_outcome, run_logos_source,
};
use logicaffeine_compile::optimization::decorate_source;

fn vm_output(src: &str) -> String {
    program_outcome(&compile_to_program(src).expect("VM compiles"))
        .output
        .trim()
        .to_string()
}
fn aot_output(src: &str) -> String {
    run_logos_source(src)
        .expect("AOT compiles and runs")
        .trim()
        .to_string()
}

/// `rep` is the repeatText shape (counted `out += c` loop). Main drives it with a runtime count
/// (no folding), exercising n=0 (empty) through n=5.
const PROG: &str = "\
## To rep (c: Text, n: Int) -> Text:
    Let mutable out be \"\".
    Let mutable i be 1.
    While i is at most n:
        Set out to out combined with c.
        Set i to i + 1.
    Return out.

## Main
Let mutable acc be 0.
Let mutable r be 0.
While r is less than 6:
    Let x be rep(\"ab\", r).
    Set acc to acc + length of x.
    Set r to r + 1.
Show acc.
";

#[test]
fn repeat_loop_lowers_to_repeat_call() {
    let rust = compile_to_rust(PROG).expect("PROG compiles");
    assert!(
        rust.contains(".repeat("),
        "the counted append loop must lower to a `.repeat(..)` call; got:\n{rust}"
    );
    assert!(
        !rust.contains(".push_str("),
        "the un-presized push_str loop must be eliminated; got:\n{rust}"
    );
}

#[test]
fn repeat_loop_gates_off() {
    let off = decorate_source(PROG, &["stringrepeatloop"]);
    let rust = compile_to_rust(&off).expect("disabled PROG compiles");
    assert!(
        !rust.contains(".repeat("),
        "`## No stringrepeatloop` must keep the loop (no .repeat lowering); got:\n{rust}"
    );
}

/// PIT-OF-SUCCESS correctness lock: `chunk.repeat(count)` must be BYTE-IDENTICAL to the VM oracle
/// (which runs the real append loop) AND to the AOT loop path (opt disabled) — across n=0..5,
/// catching any count off-by-one (inclusive/exclusive, negative clamp) or output drift.
#[test]
fn repeat_loop_is_byte_identical_all_tiers_and_toggle() {
    let vm = vm_output(PROG);
    let on = aot_output(PROG);
    let off = aot_output(&decorate_source(PROG, &["stringrepeatloop"]));
    assert_eq!(
        on, vm,
        "AOT repeat-idiom diverged from the VM oracle (count off-by-one?)"
    );
    assert_eq!(
        on, off,
        "StringRepeatLoop ON changed output vs OFF — not behavior-preserving"
    );
}

/// `spaces` is `center`'s exact vulnerable shape: the repeated chunk is a STRING LITERAL (`" "`),
/// not a variable. Regression guard — the pass emitted `(_).repeat(..)` (invalid Rust) because the
/// chunk emitter had no Text-literal arm and fell to the `_` sentinel, breaking every stdlib fn that
/// builds a run of a literal (center/pad/…). n=0..5 → 0+1+2+3+4+5 = 15.
const LIT_PROG: &str = "\
## To spaces (n: Int) -> Text:
    Let mutable out be \"\".
    Let mutable i be 1.
    While i is at most n:
        Set out to out combined with \" \".
        Set i to i + 1.
    Return out.

## Main
Let mutable acc be 0.
Let mutable r be 0.
While r is less than 6:
    Let x be spaces(r).
    Set acc to acc + length of x.
    Set r to r + 1.
Show acc.
";

#[test]
fn repeat_loop_literal_chunk_emits_valid_rust() {
    let rust = compile_to_rust(LIT_PROG).expect("LIT_PROG compiles");
    assert!(
        rust.contains(".repeat("),
        "the literal-chunk append loop must still lower to `.repeat(..)`; got:\n{rust}"
    );
    assert!(
        !rust.contains("(_)"),
        "the chunk must emit as a real string literal, never the `_` sentinel; got:\n{rust}"
    );
}

/// The bug was AOT-only (the pass is AOT|CODEGEN — dev/`largo check` never fired it), so this lock
/// asserts the emitted Rust actually BUILDS AND RUNS and matches the VM oracle + the opt-off loop.
#[test]
fn repeat_loop_literal_chunk_byte_identical_all_tiers_and_toggle() {
    let vm = vm_output(LIT_PROG);
    let on = aot_output(LIT_PROG);
    let off = aot_output(&decorate_source(LIT_PROG, &["stringrepeatloop"]));
    assert_eq!(on, "15", "spaces(0..5) length sum must be 15");
    assert_eq!(
        on, vm,
        "AOT literal-chunk repeat diverged from the VM oracle"
    );
    assert_eq!(
        on, off,
        "StringRepeatLoop ON changed output vs OFF for a literal chunk"
    );
}
