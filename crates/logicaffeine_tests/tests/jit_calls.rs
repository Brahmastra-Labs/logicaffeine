//! M8 RED gate: native→native SELF-RECURSIVE calls through the per-program
//! AtomicPtr function table (EXODIA 4.7's hot-swap seam). The call stencil
//! reads the callee entry from the table at runtime (self-calls resolve the
//! moment the function's own compile lands), windows the callee frame at
//! `base + args_start` exactly like the VM, counts depth against the SAME
//! `MAX_CALL_DEPTH` contract (side-exit at the precise crossing bytecode
//! would error at), and propagates any callee side-exit up the native stack
//! so the caller's `try_native` replays the whole call on bytecode.

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

/// The debug tree-walker burns enormous native frames per LOGOS call —
/// recursion tests need a big-stack thread (the bench-corpus pattern).
fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("test thread panicked")
}

fn tiered(src: &str) -> (String, Option<String>, u32, u32) {
    let src = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "tiered VM diverged from tree-walker on:\n{src}"
        );
        let (_, fn_ok) = tier.function_counts();
        let (_, region_ok) = tier.region_counts();
        (norm(&vm.output), vm.error, fn_ok, region_ok)
    })
}

/// Naive recursive fib: the function must tier up and the recursion must run
/// native-to-native through the table.
#[test]
fn recursive_fib_tiers_and_matches() {
    let src = "## To fib (n: Int) -> Int:\n\
               \x20   If n is less than 2:\n\
               \x20       Return n.\n\
               \x20   Return fib(n - 1) + fib(n - 2).\n\
               \n\
               ## Main\n\
               Show fib(24).\n";
    let (out, err, fn_ok, _) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "46368");
    assert!(fn_ok >= 1, "recursive fib must JIT (got {fn_ok})");
}

/// Ackermann with two accumulating self-calls (nested recursion) — depth
/// stays under the language's MAX_CALL_DEPTH at this size.
#[test]
fn ackermann_tiers_and_matches() {
    let src = "## To ackermann (m: Int) and (n: Int) -> Int:\n\
               \x20   If m equals 0:\n\
               \x20       Return n + 1.\n\
               \x20   If n equals 0:\n\
               \x20       Return ackermann(m - 1, 1).\n\
               \x20   Return ackermann(m - 1, ackermann(m, n - 1)).\n\
               \n\
               ## Main\n\
               Show ackermann(3, 5).\n";
    let (out, err, fn_ok, _) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "253");
    assert!(fn_ok >= 1, "ackermann must JIT (got {fn_ok})");
}

/// The nqueens kernel: self-recursion + the full bitwise family + a While
/// loop inside the recursive function.
#[test]
fn nqueens_solver_tiers_and_matches() {
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
    let (out, err, fn_ok, _) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "40", "nqueens(7)");
    assert!(fn_ok >= 1, "the nqueens solver must JIT (got {fn_ok})");
}

/// Depth-limit parity straight through the native path: recursion that
/// crosses MAX_CALL_DEPTH must produce the exact kernel error and partial
/// output — the native depth cell side-exits at precisely the crossing
/// bytecode would error at.
#[test]
fn native_recursion_depth_limit_parity() {
    // NON-accumulator, non-tail recursion (`sink(n-1) - 1`) so real frames stack
    // and depth 5000 crosses the cap. `sink(n-1) + 1` is accumulator-shaped
    // (single linear `+`), now strength-reduced to a constant-stack loop on every
    // tier; subtraction keeps it genuine recursion.
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
            "native depth-limit replay diverged"
        );
        assert!(vm.error.is_some(), "depth 5000 must exceed the cap");
        assert_eq!(norm(&vm.output), "3");
    });
}

/// A deopt deep inside native recursion (div-by-zero on a data-dependent
/// branch) must unwind the whole native call stack and replay on bytecode
/// with the exact error.
#[test]
fn deopt_inside_native_recursion_unwinds_and_replays() {
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
            "recursive deopt replay diverged"
        );
        assert!(vm.error.is_some(), "division by zero at the base case must error");
        assert_eq!(norm(&vm.output), "1");
    });
}
