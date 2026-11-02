//! Lock for `Opt::StringConcatReuse` — the `Let`-form string concat buffer-reuse pass.
//!
//! `Let X be A + B...` where A is a fresh owned `String` (a call returning `String`) lowers to
//! `let mut X = A; X.push_str(&B)...` — reusing A's buffer — instead of `format!("{}{}", A, B)`
//! (which allocates a new String and copies every operand). Matches Rust's `build() + &tail`
//! buffer-reuse. Gates off under `## No stringconcatreuse`; byte-identical across all tiers.

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

/// `rep` returns an owned `String`; Main's `Let s be rep("a", r) + base` is the concat-reuse target,
/// driven by a runtime loop var (no folding), exercising empty-A (r=0) and non-empty cases.
const PROG: &str = "\
## To rep (c: Text, n: Int) -> Text:
    Let mutable out be \"\".
    Let mutable i be 0.
    While i is less than n:
        Set out to out combined with c.
        Set i to i + 1.
    Return out.

## Main
Let base be \"hello world\".
Let mutable acc be 0.
Let mutable r be 0.
While r is less than 5:
    Let s be rep(\"a\", r) + base.
    Set acc to acc + length of s.
    Set r to r + 1.
Show acc.
";

#[test]
fn concat_reuse_lowers_let_concat_to_push_str() {
    let rust = compile_to_rust(PROG).expect("PROG compiles");
    assert!(
        rust.contains(".push_str("),
        "the Let-concat must lower to push_str; got:\n{rust}"
    );
    assert!(
        !rust.contains("format!(\"{}{}\""),
        "the Let-concat must NOT use format! (concat-reuse should have fired); got:\n{rust}"
    );
}

#[test]
fn concat_reuse_gates_off() {
    let off = decorate_source(PROG, &["stringconcatreuse"]);
    let rust = compile_to_rust(&off).expect("disabled PROG compiles");
    assert!(
        rust.contains("format!(\"{}{}\""),
        "`## No stringconcatreuse` must keep the format! concat; got:\n{rust}"
    );
}

/// PIT-OF-SUCCESS correctness lock: the reused-buffer concat must be BYTE-IDENTICAL to the VM oracle
/// (which runs the plain `format!` concat) AND to the AOT owned/format! path (opt disabled). Catches
/// any operand-order or aliasing bug and any output drift — the class a "does it fire" test misses.
#[test]
fn concat_reuse_is_byte_identical_all_tiers_and_toggle() {
    let vm = vm_output(PROG);
    let on = aot_output(PROG);
    let off = aot_output(&decorate_source(PROG, &["stringconcatreuse"]));
    assert_eq!(on, vm, "AOT concat-reuse diverged from the VM oracle");
    assert_eq!(
        on, off,
        "StringConcatReuse ON changed output vs OFF — not behavior-preserving"
    );
}
