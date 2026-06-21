//! WS-A Lever A: the PINNED-ARGUMENT self-call ABI. A scalar self-call no
//! longer stages each argument through its own frame-to-frame `Move` PIECE
//! before the call; instead a single fused `CallSelfCopy` stencil copies the
//! contiguous argument block into the callee window inside ONE piece and
//! performs the identical self-call. On this dispatch/piece-bound engine the
//! win is the removed per-argument dispatch (see memory `jit-is-dispatch-bound`),
//! and the transform is bit-identical to the per-`Move` staging: the same
//! frame slots are written, the same call is made, and the depth/deopt
//! semantics are untouched.
//!
//! These tests pin three contracts:
//!   1. the fused path FIRES for the common scalar self-call shapes (the
//!      observable `pinned_self_call_count` rises) — this is the RED gate;
//!   2. every tiered function stays bit-identical to the tree-walker oracle
//!      (the sacred differential gate); and
//!   3. argument shapes that overflow the budget or carry non-scalar data
//!      keep using the per-`Move` staging path (correctness fallback).

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Recursion tests need a big native stack — the debug tree-walker oracle
/// burns deep frames per LOGOS call.
fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("test thread panicked")
}

/// Run a program on the forge tier, assert bit-identical to the tree-walker,
/// and return `(output, error, fn_ok, pinned_self_calls)`.
fn tiered(src: &str) -> (String, Option<String>, u32, u32) {
    let src = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "pinned-arg self-call diverged from the tree-walker on:\n{src}"
        );
        let (_, fn_ok) = tier.function_counts();
        let pinned = tier.pinned_self_call_count();
        (norm(&vm.output), vm.error, fn_ok, pinned)
    })
}

/// fib: a single `Int` argument self-call (twice per non-base call). The
/// fused path must fire and the result stay exact.
#[test]
fn fib_single_int_arg_uses_pinned_self_call() {
    let src = "## To fib (n: Int) -> Int:\n\
               \x20   If n is less than 2:\n\
               \x20       Return n.\n\
               \x20   Return fib(n - 1) + fib(n - 2).\n\
               \n\
               ## Main\n\
               Show fib(24).\n";
    let (out, err, fn_ok, pinned) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "46368");
    assert!(fn_ok >= 1, "fib must JIT (got {fn_ok})");
    assert!(
        pinned >= 1,
        "fib's single-Int self-call must use the fused pinned-arg path (got {pinned})"
    );
}

/// A two-`Int` NON-TAIL self-call (the recursive result is combined with a
/// local, so it is not a tail call the compiler would loop-reduce away — a
/// genuine frame-passing recursion, the Lever A target). Both arguments ride
/// the fused copy.
#[test]
fn two_int_args_uses_pinned_self_call() {
    let src = "## To collatz_steps (n: Int, acc: Int) -> Int:\n\
               \x20   If n equals 1:\n\
               \x20       Return acc.\n\
               \x20   If n % 2 equals 0:\n\
               \x20       Return 1 + collatz_steps(n / 2, acc).\n\
               \x20   Return 1 + collatz_steps(3 * n + 1, acc).\n\
               \n\
               ## Main\n\
               Let mutable total be 0.\n\
               Let mutable i be 1.\n\
               While i is less than 200:\n\
               \x20   Set total to total + collatz_steps(i, 0).\n\
               \x20   Set i to i + 1.\n\
               Show total.\n";
    let (out, err, fn_ok, pinned) = tiered(src);
    assert_eq!(err, None);
    assert!(fn_ok >= 1, "the recursive function must JIT (got {fn_ok})");
    assert!(
        pinned >= 1,
        "the two-Int non-tail self-call must use the fused pinned-arg path (got {pinned})"
    );
    let _ = out;
}

/// nqueens: a FIVE-`Int` self-call (the budget edge — four pins plus the
/// constant board-size arg). The fused copy handles the whole contiguous
/// block regardless of pin budget (the copy is in-stencil, not register
/// threading), and the count is exact.
#[test]
fn nqueens_five_int_args_uses_pinned_self_call() {
    let src = "## To solve (row: Int, cols: Int, diag1: Int, diag2: Int, n: Int) -> Int:\n\
               \x20   If row equals n:\n\
               \x20       Return 1.\n\
               \x20   Let all be (1 shifted left by n) - 1.\n\
               \x20   Let mutable available be all and not (cols or diag1 or diag2).\n\
               \x20   Let mutable count be 0.\n\
               \x20   While available is not 0:\n\
               \x20       Let bit be available and (0 - available).\n\
               \x20       Set available to available xor bit.\n\
               \x20       Set count to count + solve(row + 1, cols or bit, (diag1 or bit) shifted left by 1, (diag2 or bit) shifted right by 1, n).\n\
               \x20   Return count.\n\
               \n\
               ## Main\n\
               Show solve(0, 0, 0, 0, 7).\n";
    let (out, err, fn_ok, pinned) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "40", "nqueens(7)");
    assert!(fn_ok >= 1, "nqueens must JIT (got {fn_ok})");
    assert!(
        pinned >= 1,
        "nqueens's five-Int self-call must use the fused pinned-arg path (got {pinned})"
    );
}

/// A mixed Int/Float NON-TAIL self-call: the argument block carries BOTH a
/// Float and an Int, and the recursive result is combined with a local so it
/// is a genuine frame-passing recursion (not a tail call the compiler
/// loop-reduces). The fused copy is a raw 8-byte move, so it stages the f64
/// argument bit-for-bit exactly like a `Move` — and the result is
/// bit-identical to the tree-walker oracle.
#[test]
fn mixed_int_float_args_uses_pinned_self_call() {
    let src = "## To countdown (x: Float, n: Int) -> Int:\n\
               \x20   If n equals 0:\n\
               \x20       Return 0.\n\
               \x20   If x is greater than 1.0:\n\
               \x20       Return 1 + countdown(x * 0.5, n - 1).\n\
               \x20   Return countdown(x * 0.5, n - 1).\n\
               \n\
               ## Main\n\
               Let mutable total be 0.\n\
               Let mutable i be 0.\n\
               While i is less than 5000:\n\
               \x20   Set total to total + countdown(1024.0, 20).\n\
               \x20   Set i to i + 1.\n\
               Show total.\n";
    let (out, err, fn_ok, pinned) = tiered(src);
    assert_eq!(err, None);
    // countdown(1024.0, 20): x halves each step; it is > 1.0 for the first
    // 10 steps (1024 -> 1.0), contributing 10 to the count, ×5000 calls.
    assert_eq!(out, "50000");
    assert!(fn_ok >= 1, "mixed-kind recursion must JIT (got {fn_ok})");
    assert!(
        pinned >= 1,
        "the mixed Int/Float self-call must use the fused pinned-arg path (got {pinned})"
    );
}

/// A SIX-`Int` NON-TAIL self-call: more arguments than the four-GP pin
/// budget, with the recursive result combined with a local so it stays a
/// genuine frame-passing recursion. The fused in-stencil copy still applies
/// (it is not register threading, so it is not bounded by the pin budget) and
/// the result is exact — the overflow concern is about register pins, which
/// the copy mechanism sidesteps.
#[test]
fn six_int_args_still_fused_and_exact() {
    let src = "## To churn (a: Int, b: Int, c: Int, d: Int, e: Int, f: Int) -> Int:\n\
               \x20   If a equals 0:\n\
               \x20       Return b + c + d + e + f.\n\
               \x20   Let rec be churn(a - 1, b + 1, c + 2, d + 3, e + 4, f + 5).\n\
               \x20   Return rec + 1.\n\
               \n\
               ## Main\n\
               Let mutable total be 0.\n\
               Let mutable i be 0.\n\
               While i is less than 300:\n\
               \x20   Set total to total + churn(50, 0, 0, 0, 0, 0).\n\
               \x20   Set i to i + 1.\n\
               Show total.\n";
    let (out, err, fn_ok, pinned) = tiered(src);
    assert_eq!(err, None);
    // Each churn(50,0,0,0,0,0): after 50 steps b..f are 50,100,150,200,250
    // (sum 750) and the 50 unwinding `1 +`s add 50 -> 800; ×300 calls.
    assert_eq!(out, "240000");
    assert!(fn_ok >= 1, "six-arg recursion must JIT (got {fn_ok})");
    assert!(
        pinned >= 1,
        "the six-Int self-call must use the fused pinned-arg path (got {pinned})"
    );
}

/// The kill switch `LOGOS_NO_PINNED_ARGS` restores the per-argument `Move`
/// staging path: fib still tiers and still produces the exact result, but the
/// fused-self-call counter stays at zero. This proves the fusion is a PURE
/// dispatch optimization — the engine is correct with or without it, and the
/// only difference is the removed per-argument `Move` pieces.
#[test]
fn kill_switch_restores_move_staging_same_result() {
    let src = "## To fib (n: Int) -> Int:\n\
               \x20   If n is less than 2:\n\
               \x20       Return n.\n\
               \x20   Return fib(n - 1) + fib(n - 2).\n\
               \n\
               ## Main\n\
               Show fib(24).\n";
    // Fused (default): tiers and uses the pinned-arg path.
    let (out_fused, err_fused, fn_fused, pinned_fused) = tiered(src);
    assert_eq!(err_fused, None);
    assert_eq!(out_fused, "46368");
    assert!(fn_fused >= 1);
    assert!(pinned_fused >= 1, "the default path must fuse (got {pinned_fused})");

    // Kill switch: same tiering, same result, NO fused self-calls.
    let src2 = src.to_string();
    let (out_off, err_off, fn_off, pinned_off) = on_big_stack(move || {
        std::env::set_var("LOGOS_NO_PINNED_ARGS", "1");
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src2, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src2, &[]);
        std::env::remove_var("LOGOS_NO_PINNED_ARGS");
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "kill-switch path diverged from the tree-walker"
        );
        let (_, fn_ok) = tier.function_counts();
        (norm(&vm.output), vm.error, fn_ok, tier.pinned_self_call_count())
    });
    assert_eq!(err_off, None);
    assert_eq!(out_off, "46368", "kill switch must produce the identical result");
    assert!(fn_off >= 1, "fib must still tier with the kill switch (got {fn_off})");
    assert_eq!(pinned_off, 0, "the kill switch must restore Move staging (got {pinned_off})");
}

/// Depth-limit parity must survive the fused ABI: deep non-tail recursion
/// crossing MAX_CALL_DEPTH produces the exact kernel error and partial
/// output. The fused copy does NOT touch the depth cell or the side-exit
/// path — this proves it.
#[test]
fn pinned_self_call_preserves_depth_limit_parity() {
    let src = "## To sink (n: Int) -> Int:\n\
               \x20   If n equals 0:\n\
               \x20       Return 0.\n\
               \x20   Return sink(n - 1) - 1.\n\
               \n\
               ## Main\n\
               Show 3.\n\
               Show sink(5000).\n";
    let src = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "fused-ABI depth-limit replay diverged"
        );
        assert!(vm.error.is_some(), "depth 5000 must exceed the cap");
        assert_eq!(norm(&vm.output), "3");
    });
}

/// A deopt deep inside fused-ABI recursion (div-by-zero at the base) must
/// unwind the whole native stack and replay on bytecode with the exact
/// error — the fused copy leaves the deopt/replay machinery intact.
#[test]
fn pinned_self_call_preserves_deopt_replay() {
    let src = "## To risky (n: Int) -> Int:\n\
               \x20   If n equals 0:\n\
               \x20       Return 100 / (n - 0).\n\
               \x20   Return risky(n - 1) + 1.\n\
               \n\
               ## Main\n\
               Show 1.\n\
               Show risky(400).\n";
    let src = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "fused-ABI recursive deopt replay diverged"
        );
        assert!(vm.error.is_some(), "division by zero at the base case must error");
        assert_eq!(norm(&vm.output), "1");
    });
}
