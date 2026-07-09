//! RUNTIME-TRUTH gate: compilation is not execution.
//!
//! `function_counts()` proves the tier COMPILED a function; nothing yet
//! proves the native code ever RAN. A backend that deopts or gets rejected
//! at every call still passes every exactness gate (replay is exact) while
//! delivering zero speedup — the failure mode is invisible. These tests pin
//! the truth: for pure integer recursion with no deopt-able ops, the native
//! path must be ENTERED, must COMPLETE, and must never side-exit.
//!
//! Diagnosed from measurement: with the tier off, nqueens lost 2%, fib 10%,
//! binary_trees 16% — the recursion cluster's native code was barely
//! running. The ratchet below makes that regression class impossible to
//! reintroduce silently.

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

fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("test thread panicked")
}

/// Runs `src` tiered, asserts tree-walker parity, and returns the tier's
/// runtime boundary stats: (entries, completions, plain deopts, precise
/// deopts).
fn tiered_stats(src: &str, expect_out: &str) -> (u64, u64, u64, u64) {
    let src = src.to_string();
    let expect = expect_out.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "tiered VM diverged from tree-walker on:\n{src}"
        );
        assert_eq!(vm.error, None);
        assert_eq!(norm(&vm.output), expect);
        let (_, fn_ok) = tier.function_counts();
        assert!(fn_ok >= 1, "the function must compile (got {fn_ok})");
        tier.runtime_stats()
    })
}

/// fib(24): once hot, ONE boundary entry runs an entire recursive subtree
/// natively. Entries stay near the warmup threshold (the interpreted spine
/// re-enters at most once per live frame), completions match, and a pure
/// integer body with no division, no indexing, and depth ≪ MAX_CALL_DEPTH
/// must never side-exit.
#[test]
fn fib_native_runs_to_completion() {
    let src = "## To fib (n: Int) -> Int:\n\
               \x20   If n is less than 2:\n\
               \x20       Return n.\n\
               \x20   Return fib(n - 1) + fib(n - 2).\n\
               \n\
               ## Main\n\
               Show fib(24).\n";
    let (entries, completions, deopts, deopt_ats) = tiered_stats(src, "46368");
    assert!(entries >= 1, "native fib was never entered (entries = 0)");
    assert_eq!(deopts, 0, "pure integer fib must never plain-deopt");
    assert_eq!(deopt_ats, 0, "pure integer fib must never precise-deopt");
    assert_eq!(completions, entries, "every native entry must complete");
    assert!(
        entries <= 200,
        "recursion must stay INSIDE native code: {entries} boundary entries \
         means self-calls are bouncing through the interpreter"
    );
}

/// The nqueens solver: bitwise ops + a While loop + self-recursion in the
/// loop body. Same contract — enter, stay native, complete, never side-exit.
#[test]
fn nqueens_native_runs_to_completion() {
    let src = "## To solve (row: Int, cols: Int, diag1: Int, diag2: Int, n: Int) -> Int:\n\
               \x20   If row equals n:\n\
               \x20       Return 1.\n\
               \x20   Let all be (1 shifted left by n) - 1.\n\
               \x20   Let mutable available be all & ~(cols | diag1 | diag2).\n\
               \x20   Let mutable count be 0.\n\
               \x20   While available is not 0:\n\
               \x20       Let bit be available & (0 - available).\n\
               \x20       Set available to available xor bit.\n\
               \x20       Set count to count + solve(row + 1, cols | bit, (diag1 | bit) shifted left by 1, (diag2 | bit) shifted right by 1, n).\n\
               \x20   Return count.\n\
               \n\
               ## Main\n\
               Show solve(0, 0, 0, 0, 8).\n";
    let (entries, completions, deopts, deopt_ats) = tiered_stats(src, "92");
    assert!(entries >= 1, "native solve was never entered (entries = 0)");
    assert_eq!(deopts, 0, "the solver has no deopt-able ops at n = 8");
    assert_eq!(deopt_ats, 0, "the solver has no deopt-able ops at n = 8");
    assert_eq!(completions, entries, "every native entry must complete");
    assert!(
        entries <= 250,
        "recursion must stay INSIDE native code: {entries} boundary entries \
         means self-calls are bouncing through the interpreter"
    );
}

/// Ackermann-style double recursion through TWO mutually-visible calls:
/// the call protocol must hold across non-self-call table dispatch too.
/// Cross-function dispatch BOOTSTRAPS: until the partner publishes its
/// entry, calls to it deopt by design (plan D7 — "entry == 0 → deopt;
/// converges as callees tier up"), so a small bootstrap allowance is part
/// of the contract; the steady state must be deopt-free.
#[test]
fn mutual_recursion_stays_native() {
    let src = "## To isEven (n: Int) -> Int:\n\
               \x20   If n equals 0:\n\
               \x20       Return 1.\n\
               \x20   Return isOdd(n - 1).\n\
               \n\
               ## To isOdd (n: Int) -> Int:\n\
               \x20   If n equals 0:\n\
               \x20       Return 0.\n\
               \x20   Return isEven(n - 1).\n\
               \n\
               ## Main\n\
               Let mutable acc be 0.\n\
               Let mutable i be 0.\n\
               While i is less than 600:\n\
               \x20   Set acc to acc + isEven(i).\n\
               \x20   Set i to i + 1.\n\
               Show acc.\n";
    let (entries, completions, deopts, deopt_ats) = tiered_stats(src, "300");
    assert!(entries >= 1, "native code was never entered");
    assert!(
        deopts <= 32,
        "cross-function bootstrap must converge: {deopts} plain deopts \
         means table dispatch is failing in steady state"
    );
    assert_eq!(deopt_ats, 0, "the parity pair has no precise-deopt ops");
    assert_eq!(
        completions + deopts,
        entries,
        "every native entry must complete or bootstrap-deopt"
    );
    assert!(
        completions >= entries - 32,
        "steady state must be deopt-free: only {completions} of {entries} \
         entries completed"
    );
}

/// DivPow2 lowering: a hot Main loop dividing many NEGATIVE dividends by
/// several region-constant powers of two. The side-exit-free sign-correcting
/// shift `(x + ((x>>63) & (2^k-1))) >> k` must be bit-exact with the
/// tree-walker's toward-zero `idiv` for EVERY sign — a wrong bias term would
/// diverge on the negative dividends. The loop tiers up to a native REGION,
/// where `Div` by a constant power of two is replaced with `DivPow2`.
#[test]
fn divpow2_signed_region_parity() {
    let src = "## Main\n\
               Let mutable acc be 0.\n\
               Let mutable i be 0 - 200000.\n\
               While i is at most 200000:\n\
               \x20   Set acc to acc + (i / 2) + (i / 4) + (i / 8) + (i / 64) + (i / 1024).\n\
               \x20   Set i to i + 1.\n\
               Show acc.\n";
    let src = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "DivPow2 (signed /2^k) diverged from the tree-walker on negative dividends:\n{src}"
        );
        assert_eq!(vm.error, None);
        let (_, region_ok) = tier.region_counts();
        assert!(
            region_ok >= 1,
            "the hot loop must tier to a native region (got {region_ok}) — \
             DivPow2 is otherwise never exercised"
        );
    });
}
