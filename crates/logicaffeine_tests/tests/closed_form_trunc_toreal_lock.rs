//! closed_form_trunc_toreal_lock — the closed-form pass sees THROUGH `trunc(toReal(i))` to `i`.
//!
//! `Σ trunc(toReal(i))` over a loop is `Σ i` (for the counter within the pass's existing i64-safe
//! bound guard, where `toReal(i)` is exact and `trunc` of an exact non-negative integer float is that
//! integer). Recognizing this lets the EXISTING Gauss reduction collapse the loop to `n(n+1)/2` — a
//! real loop-supercompilation win (the C/Go/Rust rivals run O(n)). This is the convert.to_real cap.
//!
//! SOUNDNESS IS THE WHOLE GAME. `trunc(toReal(x)) == x` ONLY when `toReal`/`trunc` are the canonical
//! stdlib int-cast / truncation. A user may SHADOW them (`## To toReal(n)->Real: Return 999.0.` type-
//! checks), so the pass fires ONLY after verifying the in-program `toReal`/`trunc` DEFINITIONS match the
//! canonical bodies. `shadowed_toreal_is_not_reduced` is the adversarial partner: a redefined toReal must
//! compute its real value, never collapse to `Σ i`.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(closed_form_trunc_toreal_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::compile::{
    compile_to_rust, interpret_program, optimizations_fired, vm_run_source,
};
use logicaffeine_compile::optimization::decorate_source;

/// Canonical stdlib `toReal` (Int->Real cast) + `trunc` (toward zero), as auto-imported.
const STDLIB_FNS: &str = "\
## To toReal (n: Int) -> Real:
    Return n + 0.0.
## To trunc (x: Real) -> Int:
    If x is less than 0.0:
        Return 0 - floor(0.0 - x).
    Return floor(x).
";

/// `Σ_{i=1}^{100} trunc(toReal(i)) = Σ i = 5050`.
fn sum_prog() -> String {
    format!("{STDLIB_FNS}## Main\nLet mutable s be 0.\nLet mutable i be 1.\nWhile i is at most 100:\n    Set s to s + trunc(toReal(i)).\n    Set i to i + 1.\nShow s.\n")
}

/// A user SHADOWS toReal to a constant: `Σ trunc(999.0) = 999*100 = 99900`, NOT `Σ i`.
fn shadow_prog() -> String {
    "## To toReal (n: Int) -> Real:\n    Return 999.0.\n## To trunc (x: Real) -> Int:\n    If x is less than 0.0:\n        Return 0 - floor(0.0 - x).\n    Return floor(x).\n## Main\nLet mutable s be 0.\nLet mutable i be 1.\nWhile i is at most 100:\n    Set s to s + trunc(toReal(i)).\n    Set i to i + 1.\nShow s.\n".to_string()
}

fn no_unroll(src: &str) -> String {
    decorate_source(src, &["unroll"])
}
fn tw(src: &str) -> String {
    interpret_program(src)
        .unwrap_or_else(|e| panic!("tw: {e:?}"))
        .trim_end()
        .to_string()
}
fn vm(src: &str) -> String {
    vm_run_source(src)
        .unwrap_or_else(|e| panic!("vm: {e}"))
        .trim_end()
        .to_string()
}
fn rust(src: &str) -> String {
    compile_to_rust(src).unwrap_or_else(|e| panic!("compile_to_rust: {e:?}"))
}

// ---- RED: the pass fires + collapses the loop -------------------------------------------
#[test]
fn closed_form_sees_through_trunc_toreal() {
    let src = no_unroll(&sum_prog());
    let fired = optimizations_fired(&src);
    assert!(
        fired.contains(&"closedform"),
        "closedform must fire on Σ trunc(toReal(i)); got {fired:?}"
    );
}

#[test]
fn the_residual_is_loop_free() {
    let on = rust(&no_unroll(&sum_prog()));
    // The main accumulation loop must be gone (collapsed to arithmetic).
    let main_start = on.find("fn _logos_main").expect("main present");
    assert!(
        !on[main_start..].contains("while "),
        "closedform must remove the accumulation loop from main"
    );
}

// ---- SEMANTICS: exact on every tier, pass on or off ------------------------------------
#[test]
fn the_formula_equals_the_loop() {
    let src = sum_prog();
    assert_eq!(tw(&src), "5050", "tree-walker");
    assert_eq!(vm(&src), "5050", "VM");
    assert_eq!(
        vm(&no_unroll(&src)),
        "5050",
        "VM (unroll off, closedform on)"
    );
    assert_eq!(
        vm(&decorate_source(&src, &["unroll", "closedform"])),
        "5050",
        "VM (both off)"
    );
}

// ---- SOUNDNESS: a shadowed toReal must NEVER be reduced to Σ i --------------------------
#[test]
fn shadowed_toreal_is_not_reduced() {
    let src = shadow_prog();
    // trunc(999.0) = 999; Σ_{1}^{100} 999 = 99900. The pass must NOT collapse this to Σ i = 5050.
    assert_eq!(tw(&src), "99900", "tree-walker (shadowed)");
    assert_eq!(vm(&src), "99900", "VM (shadowed)");
    assert_eq!(
        vm(&no_unroll(&src)),
        "99900",
        "VM (shadowed, unroll off) — must not become 5050"
    );
}
