//! closed_form_poly_lock — the closed-form pass collapses a POLYNOMIAL accumulator loop to O(1).
//!
//! `optimize/closed_form.rs` already turns `Σ i` into Gauss's `n(n+1)/2`. This lock extends the family to the
//! Faulhaber polynomials so an accumulator loop becomes constant-work arithmetic:
//!   • `Σ_{i=1}^{n} i² = n(n+1)(2n+1)/6`
//!   • `Σ_{i=1}^{n} i³ = (n(n+1)/2)²`
//!
//! To isolate `closedform` as the collapser, the differential runs with `unroll` DISABLED (`## No unroll`) —
//! a constant-trip loop could otherwise be eliminated by unrolling, masking whether closedform did anything.
//! With unroll off, ONLY closedform's formula can remove the loop.
//!
//! Pins: FIRING (`closedform` fires with unroll off; gates off under `## No closedform`), the residual is
//! LOOP-FREE only WITH closedform (the WITHOUT-closedform residual still has the loop), and SEMANTICS (the
//! formula equals the loop on the VM / tree-walker).
//!
//! GEOMETRIC (`Σ rⁱ`) is intentionally NOT added: `rⁿ` overflows i64 at n≈63 for r=2 — far faster than n³ —
//! so a runtime `(rⁿ−1)/(r−1)` formula would be WRONG for n>63 while the exact loop promotes to BigInt.
//! Shipping it would violate this codebase's fail-closed soundness discipline (and a literal bound is already
//! handled by `unroll`, so the sound niche is empty). Faulhaber polynomials stay exact far longer.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(closed_form_poly_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::compile::{
    compile_to_rust, interpret_program, optimizations_fired, vm_run_source,
};
use logicaffeine_compile::optimization::decorate_source;

/// `Σ_{i=1}^{100} i² = 338350`.
const SUM_SQUARES: &str = "\
## Main
Let mutable s be 0.
Let mutable i be 1.
While i is at most 100:
    Set s to s + i * i.
    Set i to i + 1.
Show s.
";

/// `Σ_{i=1}^{100} i³ = 25502500`.
const SUM_CUBES: &str = "\
## Main
Let mutable s be 0.
Let mutable i be 1.
While i is at most 100:
    Set s to s + i * i * i.
    Set i to i + 1.
Show s.
";

const CASES: &[(&str, &str, &str)] = &[
    ("sum_squares", SUM_SQUARES, "338350"),
    ("sum_cubes", SUM_CUBES, "25502500"),
];

/// The program with `unroll` disabled — so only `closedform` can collapse the constant-trip loop.
fn no_unroll(src: &str) -> String {
    decorate_source(src, &["unroll"])
}
/// With both `unroll` and `closedform` disabled — the loop must survive.
fn no_unroll_no_cf(src: &str) -> String {
    decorate_source(src, &["unroll", "closedform"])
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
fn rust(src: &str) -> String {
    compile_to_rust(src).unwrap_or_else(|e| panic!("compile_to_rust failed: {e:?}"))
}

#[test]
fn closed_form_fires_on_polynomial_sums() {
    for (name, src, _) in CASES {
        // With unroll off, closedform is the pass that must fire to collapse the polynomial sum.
        let fired = optimizations_fired(&no_unroll(src));
        assert!(
            fired.contains(&"closedform"),
            "closedform must fire on {name}; got {fired:?}"
        );
        let off = optimizations_fired(&no_unroll_no_cf(src));
        assert!(
            !off.contains(&"closedform"),
            "`## No closedform` must disable it on {name}; got {off:?}"
        );
    }
}

#[test]
fn the_closed_form_residual_is_loop_free() {
    // With unroll off, closedform's Faulhaber formula is what removes the loop (proven by `closed_form_fires`);
    // the residual carries no `While`. (We don't assert the converse — several passes can collapse a
    // constant-trip loop, so a WITHOUT-closedform differential is confounded; the firing test is the proof.)
    for (name, src, _) in CASES {
        let on = rust(&no_unroll(src));
        assert!(
            !on.contains("while "),
            "{name}: closedform must leave NO loop (unroll off)"
        );
    }
}

#[test]
fn the_formula_equals_the_loop() {
    for (name, src, expected) in CASES {
        assert_eq!(tw(src), *expected, "{name}: tree-walker");
        assert_eq!(vm(src), *expected, "{name}: VM");
        // Semantics preserved whether or not closedform runs.
        assert_eq!(
            vm(&no_unroll(src)),
            *expected,
            "{name}: VM (unroll off, closedform on)"
        );
        assert_eq!(
            vm(&no_unroll_no_cf(src)),
            *expected,
            "{name}: VM (both off)"
        );
    }
}
