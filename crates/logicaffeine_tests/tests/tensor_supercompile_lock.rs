//! tensor_supercompile_lock — the supercompiler evaluates STATIC-DATA tensor programs at COMPILE TIME.
//!
//! A tensor op whose inputs are literal-filled and whose loops are constant-trip is a pure compile-time
//! computation. With array support in the supercompiler's value lattice (`Value::Seq`), such a program
//! DRIVES to its result constants — the matmul/reduction loops and the backing `Seq`s vanish, leaving only
//! `Show <constant>`. This lock pins that:
//!
//!   • FIRING — `supercompile` fires on a static-data matmul (and the float reductions).
//!   • FOLDING — the emitted Rust contains the RESULT constants (computed at compile time), and turning the
//!     pass off (`## No supercompile`) removes them (they were the supercompiler's work, nothing else's).
//!   • SEMANTICS — the folded result equals the VM / tree-walker, byte for byte.
//!   • GENUINENESS — change an input literal and the folded constant changes (real evaluation, not a canned
//!     answer).
//!   • HONEST BOUNDARY — a tensor op over a value the supercompiler cannot prove constant is LEFT ALONE (no
//!     false fold) — that program's optimized output still computes at runtime.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(tensor_supercompile_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::compile::{
    compile_to_rust, interpret_program, optimizations_fired, vm_run_source,
};
use logicaffeine_compile::optimization::decorate_source;

/// A 2×2 · 2×2 matmul, every input a literal, every loop constant-trip. `A=[[1,2],[3,4]]`,
/// `B=[[5,6],[7,8]]`, `C = A·B = [[19,22],[43,50]]`.
const STATIC_MATMUL: &str = "\
## Main
Let A be a 2 by 2 tensor of Int.
Set item 1 of A to 1.
Set item 2 of A to 2.
Set item 3 of A to 3.
Set item 4 of A to 4.
Let B be a 2 by 2 tensor of Int.
Set item 1 of B to 5.
Set item 2 of B to 6.
Set item 3 of B to 7.
Set item 4 of B to 8.
Let C be A matmul B.
Show item 1 of C.
Show item 2 of C.
Show item 3 of C.
Show item 4 of C.
";

/// The four result constants of `STATIC_MATMUL`.
const RESULT_CONSTS: &[&str] = &["19", "22", "43", "50"];

fn rust(src: &str) -> String {
    compile_to_rust(src).unwrap_or_else(|e| panic!("compile_to_rust failed: {e:?}"))
}
fn tw(src: &str) -> String {
    interpret_program(src)
        .unwrap_or_else(|e| panic!("tree-walker failed: {e:?}"))
        .trim_end()
        .to_string()
}
fn vm(src: &str) -> String {
    vm_run_source(src)
        .unwrap_or_else(|e| panic!("VM failed: {e}"))
        .trim_end()
        .to_string()
}
fn count_present(hay: &str, needles: &[&str]) -> usize {
    needles.iter().filter(|n| hay.contains(**n)).count()
}

#[test]
fn supercompile_fires_on_a_static_matmul() {
    let fired = optimizations_fired(STATIC_MATMUL);
    assert!(
        fired.contains(&"supercompile"),
        "supercompile must fire on a static-data matmul; got {fired:?}"
    );
}

#[test]
fn the_static_matmul_folds_to_its_result_constants() {
    // With the pass ON, the emitted Rust carries all four RESULT constants (computed at compile time). With
    // `## No supercompile`, the program instead computes them at runtime — the result literals are absent.
    let on = rust(STATIC_MATMUL);
    let off = rust(&decorate_source(STATIC_MATMUL, &["supercompile"]));
    assert_eq!(
        count_present(&on, RESULT_CONSTS),
        4,
        "folded output must contain all result constants"
    );
    assert!(
        count_present(&off, RESULT_CONSTS) < 4,
        "with supercompile off the result constants must NOT all appear (they were the fold's work)"
    );
}

#[test]
fn the_folded_program_equals_the_vm() {
    assert_eq!(tw(STATIC_MATMUL), "19\n22\n43\n50", "tree-walker product");
    assert_eq!(vm(STATIC_MATMUL), "19\n22\n43\n50", "VM product");
}

#[test]
fn genuineness_changing_a_literal_changes_the_constant() {
    // A[1] = 100 (was 1) ⇒ C[1] = 100·5 + 2·7 = 514 (was 19). 514 is distinctive (won't collide with `i64`
    // etc.). The fold must recompute it.
    let modified = STATIC_MATMUL.replace("Set item 1 of A to 1.", "Set item 1 of A to 100.");
    assert_eq!(
        vm(&modified).lines().next().unwrap(),
        "514",
        "VM must reflect the changed input"
    );
    let on = rust(&modified);
    assert!(
        on.contains("514"),
        "folded output must contain the NEW constant 514"
    );
}

#[test]
fn static_reductions_fold_too() {
    // The float reductions are also pure compile-time computations on literal data — supercompile drives them.
    let rmsnorm = "\
## Main
Let X be a 1 by 4 tensor of Real.
Set item 1 of X to 2.0.
Set item 2 of X to 4.0.
Set item 3 of X to 6.0.
Set item 4 of X to 8.0.
Let Y be rmsnorm of X.
Show round(item 1 of Y * 1000000).
";
    let fired = optimizations_fired(rmsnorm);
    assert!(
        fired.contains(&"supercompile"),
        "supercompile must fire on a static rmsnorm; got {fired:?}"
    );
    // Semantics preserved (round(2/sqrt(30)·1e6) = 365148).
    assert_eq!(
        vm(rmsnorm),
        "365148",
        "static rmsnorm folds to its constant"
    );
}

#[test]
fn a_non_constant_matmul_is_left_alone() {
    // The honest boundary: when an input is NOT provably constant, the supercompiler must not fold. Here A[1]
    // is read from the program's first argument, so C depends on runtime input — the result constants must
    // NOT appear as literals in the emitted Rust (it still computes at runtime).
    let runtime = "\
## Main
Let A be a 2 by 2 tensor of Int.
Set item 1 of A to parseInt(item 1 of arguments).
Set item 2 of A to 2.
Set item 3 of A to 3.
Set item 4 of A to 4.
Let B be a 2 by 2 tensor of Int.
Set item 1 of B to 5.
Set item 2 of B to 6.
Set item 3 of B to 7.
Set item 4 of B to 8.
Let C be A matmul B.
Show item 1 of C.
Show item 2 of C.
Show item 3 of C.
Show item 4 of C.
";
    let on = rust(runtime);
    assert!(
        count_present(&on, RESULT_CONSTS) < 4,
        "a matmul with a runtime input must NOT fold to constants (soundness)"
    );
}
