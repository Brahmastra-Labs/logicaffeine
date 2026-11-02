//! The `## Require <opt>` decorator.
//!
//! Ray Myers' critique includes a wish: *"I always wish I could declare that I
//! depend on an optimization so that missing it would become an error."* (Zig
//! forces tail calls with `@call(.always_tail)`.) LOGOS answers it — and goes
//! further: `## Require <opt>` makes compilation FAIL if the named optimization
//! did not fire, for every registered optimization, not just tail calls. It
//! is the exact inverse of the existing `## No <opt>` (disable).
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(e2e_require_optimization)'`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::compile_to_rust;

const RECURSIVE_FIB: &str = "## To fib (n: Int) -> Int:\n\
    \x20   If n is less than 2:\n\
    \x20       Return n.\n\
    \x20   Return fib(n - 1) + fib(n - 2).\n\
## Main\nShow fib(10).\n";

/// `## Require memo` on a pure double-recursive function: memoization DOES fire,
/// so the compile succeeds unchanged.
#[test]
fn require_memo_compiles_when_memo_fires() {
    let code = format!("## Require memo\n{RECURSIVE_FIB}");
    let r = compile_to_rust(&code);
    assert!(
        r.is_ok(),
        "memo fires on recursive fib → `## Require memo` must compile, got: {r:?}"
    );
}

/// `## Require memo` on a program with nothing to memoize: memoization CANNOT
/// fire, so the compile must FAIL loudly — Ray's "missing it becomes an error."
#[test]
fn require_memo_errors_when_it_cannot_fire() {
    let code = "## Require memo\n## Main\nShow 2 + 2.\n";
    let r = compile_to_rust(code);
    assert!(
        r.is_err(),
        "no memoizable function here → `## Require memo` must be a compile error, got: {r:?}"
    );
}

/// The general point: it works for more than tail calls. `## Require closedform`
/// on the affine double-recursion the optimizer collapses to a closed form must
/// compile; on a program with no such shape it must error.
#[test]
fn require_closedform_gates_the_collapse() {
    // `1 + f(d-1) + f(d-1)` is the affine double-recursion → closed-form collapse.
    let collapses = "## Require closedform\n\
## To build (d: Int) -> Int:\n\
    \x20   If d is 0:\n\
    \x20       Return 1.\n\
    \x20   Return 1 + build(d - 1) + build(d - 1).\n\
## Main\nShow build(10).\n";
    assert!(
        compile_to_rust(collapses).is_ok(),
        "closed-form fires → must compile"
    );

    let cannot = "## Require closedform\n## Main\nShow 2 + 2.\n";
    assert!(
        compile_to_rust(cannot).is_err(),
        "no collapsible shape → `## Require closedform` must error"
    );
}

/// Regression: the existing `## No <opt>` decorator is untouched — `## No memo`
/// still DISABLES memoization (no memo table emitted).
#[test]
fn no_memo_decorator_still_disables() {
    let code = format!("## No memo\n{RECURSIVE_FIB}");
    let rust = compile_to_rust(&code).expect("## No memo should still compile");
    assert!(
        !rust.contains("__MEMO") && !rust.to_lowercase().contains("memo"),
        "`## No memo` must suppress the memo table:\n{rust}"
    );
}
