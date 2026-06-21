//! M2 RED gate: Div/Mod enter the JIT subset with the side-exit deopt
//! protocol (EXODIA Phase 2/3 — checked native division, discard-and-replay
//! on zero divisors).
//!
//! Three layers under test:
//! - the forge gains checked `MicroOp::Div`/`MicroOp::Mod` stencils whose
//!   zero-divisor path side-exits instead of faulting;
//! - the adapters accept `Op::Div`/`Op::Mod` (functions AND Main-loop
//!   regions) and route the side-exit to the VM's replay;
//! - the VM's replay produces the EXACT kernel error and partial output the
//!   pure bytecode path produces — differentially asserted.
//!
//! Wrapping parity: `i64::MIN / -1` and `i64::MIN % -1` must match the
//! kernel's locked wrapping-i64 spec bit-for-bit (the same contract
//! `fast_path_kernel_differential` pins for add/sub/mul).

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

/// Run on the tiered VM (private tier) and the tree-walker; outcomes must be
/// identical. Returns the tier's (function successes, region successes).
fn tiered_differential(src: &str) -> (u32, u32) {
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "tiered VM diverged from tree-walker on:\n{src}"
    );
    let (_, fn_ok) = tier.function_counts();
    let (_, region_ok) = tier.region_counts();
    (fn_ok, region_ok)
}

/// The loop_sum shape: an accumulating Main loop with a `%` reduction. The
/// region MUST tier up (this is the M2 coverage ratchet) and the result must
/// match the tree-walker.
#[test]
fn main_loop_with_mod_compiles_to_region() {
    let src = "## Main\n\
               Let mutable sum be 0.\n\
               Let mutable i be 1.\n\
               While i is at most 5000:\n\
               \x20   Set sum to (sum + i) % 1000000007.\n\
               \x20   Set i to i + 1.\n\
               Show sum.\n";
    let (_, region_ok) = tiered_differential(src);
    assert!(
        region_ok >= 1,
        "the loop_sum-shaped Main loop must JIT as a region (got {region_ok} successes)"
    );
}

/// The collatz shape: data-dependent Div/Mod in a Main loop.
#[test]
fn main_loop_with_div_and_mod_compiles_to_region() {
    let src = "## Main\n\
               Let mutable steps be 0.\n\
               Let mutable n be 27.\n\
               While n is not 1:\n\
               \x20   If n % 2 equals 0:\n\
               \x20       Set n to n / 2.\n\
               \x20   If n % 2 equals 1:\n\
               \x20       If n is not 1:\n\
               \x20           Set n to 3 * n + 1.\n\
               \x20   Set steps to steps + 1.\n\
               Show steps.\n";
    // collatz(27) = 111 steps — under the 100-back-edge threshold the region
    // never arms, so loop the computation to cross it.
    let looped = "## Main\n\
                  Let mutable total be 0.\n\
                  Let mutable round be 0.\n\
                  While round is less than 300:\n\
                  \x20   Let mutable n be 27.\n\
                  \x20   Let mutable steps be 0.\n\
                  \x20   While n is not 1:\n\
                  \x20       If n % 2 equals 0:\n\
                  \x20           Set n to n / 2.\n\
                  \x20       If n % 2 equals 1:\n\
                  \x20           If n is not 1:\n\
                  \x20               Set n to 3 * n + 1.\n\
                  \x20       Set steps to steps + 1.\n\
                  \x20   Set total to total + steps.\n\
                  \x20   Set round to round + 1.\n\
                  Show total.\n";
    tiered_differential(src);
    let (_, region_ok) = tiered_differential(looped);
    assert!(
        region_ok >= 1,
        "the collatz-shaped nested Main loop must JIT a region (got {region_ok})"
    );
}

/// A pure Int function whose body divides — the gcd shape. Must tier up as a
/// native FUNCTION once hot.
#[test]
fn pure_function_with_mod_compiles() {
    let src = "## To gcd (a: Int, b: Int) -> Int:\n\
               \x20   Let mutable x be a.\n\
               \x20   Let mutable y be b.\n\
               \x20   While y is greater than 0:\n\
               \x20       Let temp be y.\n\
               \x20       Set y to x % y.\n\
               \x20       Set x to temp.\n\
               \x20   Return x.\n\
               \n\
               ## Main\n\
               Let mutable sum be 0.\n\
               Let mutable i be 1.\n\
               While i is at most 400:\n\
               \x20   Set sum to sum + gcd(i, 360).\n\
               \x20   Set i to i + 1.\n\
               Show sum.\n";
    let (fn_ok, _) = tiered_differential(src);
    assert!(
        fn_ok >= 1,
        "the gcd-shaped pure Int function must JIT (got {fn_ok} successes)"
    );
}

/// Division by zero at a data-dependent iteration: the native region must
/// side-exit BEFORE the faulting op and the replay must produce the exact
/// kernel error with the exact partial output — bit-identical to the pure
/// bytecode run and the tree-walker.
#[test]
fn div_by_zero_mid_loop_error_parity() {
    // d hits zero at i == 150 — after the region went hot at 100.
    let src = "## Main\n\
               Show 7.\n\
               Let mutable acc be 0.\n\
               Let mutable i be 1.\n\
               While i is at most 5000:\n\
               \x20   Let d be 150 - i.\n\
               \x20   Set acc to acc + 1000 / d.\n\
               \x20   Set i to i + 1.\n\
               Show acc.\n";
    let tier = ForgeTier::new();
    let vm_tiered = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm_tiered.output), &vm_tiered.error),
        (norm(&tw.output), &tw.error),
        "tiered deopt replay diverged from tree-walker"
    );
    assert!(vm_tiered.error.is_some(), "division by zero must error");
    assert_eq!(norm(&vm_tiered.output), "7", "partial output before the error must survive");
}

/// Same for `%` — the modulo error string is distinct in the kernel.
#[test]
fn mod_by_zero_mid_loop_error_parity() {
    let src = "## Main\n\
               Let mutable acc be 0.\n\
               Let mutable i be 1.\n\
               While i is at most 5000:\n\
               \x20   Let d be 200 - i.\n\
               \x20   Set acc to acc + 1000 % d.\n\
               \x20   Set i to i + 1.\n\
               Show acc.\n";
    let tier = ForgeTier::new();
    let vm_tiered = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm_tiered.output), &vm_tiered.error),
        (norm(&tw.output), &tw.error),
        "tiered mod-deopt replay diverged from tree-walker"
    );
    assert!(vm_tiered.error.is_some(), "modulo by zero must error");
}

/// A slot written only on SOME paths through the region (the primes
/// `isPrime`-flag shape: set before the loop, conditionally cleared inside,
/// never read inside) must keep its pre-region value whenever the completing
/// path did not write it. The write-back contract may not stamp the native
/// frame's default over it.
#[test]
fn conditionally_written_flag_survives_region_completion() {
    // Exactly the primes trial-division shape, small enough to verify by eye:
    // π(500) = 95.
    let src = "## Main\n\
               Let n be 500.\n\
               Let mutable count be 0.\n\
               Let mutable i be 2.\n\
               While i is at most n:\n\
               \x20   Let mutable isPrime be 1.\n\
               \x20   Let mutable d be 2.\n\
               \x20   While d * d is at most i:\n\
               \x20       If i % d equals 0:\n\
               \x20           Set isPrime to 0.\n\
               \x20           Break.\n\
               \x20       Set d to d + 1.\n\
               \x20   If isPrime equals 1:\n\
               \x20       Set count to count + 1.\n\
               \x20   Set i to i + 1.\n\
               Show count.\n";
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    assert_eq!(vm.error, None);
    assert_eq!(norm(&vm.output), "95", "π(500) — conditional write must not be clobbered");
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(norm(&tw.output), "95");
}

/// Wrapping edges through a hot region: `i64::MIN / -1`, `i64::MIN % -1`,
/// truncation toward zero on negatives — every value the kernel's locked
/// wrapping spec pins, computed natively, must display identically.
#[test]
fn div_mod_wrapping_edge_grid_through_hot_loop() {
    // Each (numerator, divisor) pair runs inside a hot loop so the natively
    // compiled code (not the bytecode fallback) computes it.
    let cases = [
        ("-9223372036854775808", "-1"),
        ("-9223372036854775808", "1"),
        ("9223372036854775807", "-1"),
        ("-7", "2"),
        ("7", "-2"),
        ("-7", "-2"),
        ("1", "9223372036854775807"),
    ];
    for (n, d) in cases {
        let src = format!(
            "## Main\n\
             Let mutable q be 0.\n\
             Let mutable r be 0.\n\
             Let mutable i be 0.\n\
             While i is less than 300:\n\
             \x20   Set q to {n} / {d}.\n\
             \x20   Set r to {n} % {d}.\n\
             \x20   Set i to i + 1.\n\
             Show q.\n\
             Show r.\n"
        );
        tiered_differential(&src);
    }
}
