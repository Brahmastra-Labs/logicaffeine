//! Lock for `Opt::BranchlessAccumulate` — if-conversion of a predicate-guarded increment.
//!
//! `If <cond>: Set X to X + <k>` (single-statement `then`, NO `else`, `X` a plain integer
//! accumulator, `k` a literal) lowers to `X = X + (<cond> as i64) * k` — no branch, so the counting
//! loop auto-vectorizes (branches block the vectorizer). Byte-identical: `cond` is `{0,1}`, so
//! `(cond as i64) * k` is `k` when true and `0` when false, and `cond` is evaluated exactly once
//! either way. Gates off under `## No branchlessaccumulate`.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use logicaffeine_compile::compile::{compile_to_c, compile_to_c_via_bytecode};
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

#[cfg(not(target_arch = "wasm32"))]
fn c_output(c: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("main.c");
    let bin = dir.path().join("main");
    std::fs::write(&src, c).expect("write generated C");
    let build = common::build_c_release(&src, &bin).expect("C compiler starts");
    assert!(build.status.success(), "C build failed: {build:?}");
    let output = std::process::Command::new(&bin).output().expect("C runs");
    assert!(output.status.success(), "C process failed: {output:?}");
    String::from_utf8(output.stdout)
        .expect("C output is UTF-8")
        .trim()
        .to_string()
}

/// Count matches over a runtime loop with a predicate-guarded `+1`. `sum` is a plain i64 accumulator;
/// the guard `i < 50000` over i=0..99999 yields 50000.
const PROG: &str = "\
## Main
Let mutable sum be 0.
Let mutable i be 0.
While i is less than 100000:
    If i is less than 50000:
        Set sum to sum + 1.
    Set i to i + 1.
Show sum.
";

#[test]
fn branchless_accumulate_removes_the_branch() {
    let on = compile_to_rust(PROG).expect("PROG compiles");
    let off =
        compile_to_rust(&decorate_source(PROG, &["branchlessaccumulate"])).expect("off compiles");
    // The guarded-increment branch is gone: ON has strictly fewer `if ` than OFF (same helpers, one
    // fewer body branch). And the branchless form casts the condition to i64.
    assert!(
        on.matches("if ").count() < off.matches("if ").count(),
        "the predicate-guarded increment's branch must be eliminated; got ON:\n{on}"
    );
    assert!(
        on.contains(") as i64)"),
        "the branchless form must cast the condition to i64; got:\n{on}"
    );
}

#[test]
fn branchless_accumulate_gates_off() {
    let off =
        compile_to_rust(&decorate_source(PROG, &["branchlessaccumulate"])).expect("off compiles");
    assert!(
        off.contains("if "),
        "`## No branchlessaccumulate` must keep the guarded-increment branch; got:\n{off}"
    );
}

/// PIT-OF-SUCCESS correctness lock: the branchless form must be BYTE-IDENTICAL to the VM oracle (the
/// real `if`) AND to the AOT path with the opt disabled — the transform is valid ONLY because
/// `cond ∈ {0,1}`, so this catches any misfire (a non-`+literal` body, an `else`, a non-i64 X).
#[test]
fn branchless_accumulate_byte_identical_all_tiers_and_toggle() {
    let vm = vm_output(PROG);
    let on = aot_output(PROG);
    let off = aot_output(&decorate_source(PROG, &["branchlessaccumulate"]));
    assert_eq!(on, "50000", "count of i<50000 over 0..99999 must be 50000");
    assert_eq!(on, vm, "AOT branchless diverged from the VM oracle");
    assert_eq!(
        on, off,
        "BranchlessAccumulate ON changed output vs OFF — not behavior-preserving"
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn c_backends_match_the_oracle_and_kill_switch() {
    let disabled = decorate_source(PROG, &["branchlessaccumulate"]);
    let direct_on = c_output(&compile_to_c(PROG).expect("direct C compiles"));
    let direct_off = c_output(&compile_to_c(&disabled).expect("direct C disabled compiles"));
    let bytecode = c_output(&compile_to_c_via_bytecode(PROG).expect("bytecode C compiles"));

    assert_eq!(direct_on, "50000");
    assert_eq!(direct_on, direct_off, "direct C toggle changed output");
    assert_eq!(direct_on, bytecode, "the C backends disagree");
}
