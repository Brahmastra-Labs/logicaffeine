//! Self-tail-call optimization (TCO) as a LANGUAGE semantic, identical across
//! the two interpreter tiers. A self-call in TAIL position — `Return f(args)`,
//! or `Set/Let x to f(args); Return x` — is lowered to a parameter-reassign +
//! jump-to-entry instead of a real call+return, so tail recursion runs in
//! constant stack (a loop), eliminating the per-call frame round-trip. V8 does
//! NOT tail-call-optimize JS, so this is a place the LOGOS interpreter can beat
//! it (quicksort's second recursion is a tail call).
//!
//! The observable here: recursion 5000 deep is FAR past `MAX_CALL_DEPTH` (1000),
//! so WITHOUT TCO a tier hits the call-depth limit and errors. With TCO it is a
//! loop and completes — and CRUCIALLY both the bytecode VM and the tree-walker
//! agree (same value, no error), because TCO is a shared semantic, not a VM-only
//! optimization. The shallow case pins that TCO is result-preserving.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("test thread panicked")
}

/// Run `src` on BOTH tiers and return `(vm_out, vm_err, tw_out, tw_err)`.
fn both_tiers(src: &str) -> (String, Option<String>, String, Option<String>) {
    let src = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src, &[]);
        (
            vm.output.trim().to_string(),
            vm.error,
            tw.output.trim().to_string(),
            tw.error,
        )
    })
}

/// Assert deep tail recursion completes on BOTH tiers with the same value and
/// no call-depth error — the (B) constant-stack semantic.
fn assert_constant_stack(src: &str, expected: &str) {
    let (vm_out, vm_err, tw_out, tw_err) = both_tiers(src);
    assert_eq!(vm_err, None, "VM must not hit the call-depth limit (TCO)");
    assert_eq!(tw_err, None, "tree-walker must not hit the call-depth limit (TCO)");
    assert_eq!(vm_out, tw_out, "the two tiers must agree under TCO");
    assert_eq!(vm_out, expected);
}

/// A DIRECT self-tail-call `Return sumto(n-1, acc+n)`, 5000 deep. Without TCO a
/// tier errors at the depth limit; with TCO both loop and return 5000·5001/2.
#[test]
fn direct_tail_recursion_constant_stack_both_tiers() {
    let src = "## To sumto (n: Int, acc: Int) -> Int:\n\
               \x20   If n is at most 0:\n\
               \x20       Return acc.\n\
               \x20   Return sumto(n - 1, acc + n).\n\
               ## Main\n\
               Show sumto(5000, 0).\n";
    assert_constant_stack(src, "12502500");
}

/// The quicksort-shape `Set x to f(args); Return x` pair is also a tail call.
#[test]
fn set_return_pair_tail_recursion_constant_stack_both_tiers() {
    let src = "## To sump (n: Int, acc: Int) -> Int:\n\
               \x20   If n is at most 0:\n\
               \x20       Return acc.\n\
               \x20   Let mutable r be 0.\n\
               \x20   Set r to sump(n - 1, acc + n).\n\
               \x20   Return r.\n\
               ## Main\n\
               Show sump(5000, 0).\n";
    assert_constant_stack(src, "12502500");
}

/// The `Let x be f(args); Return x` pair (immutable binding) is a tail call too.
#[test]
fn let_return_pair_tail_recursion_constant_stack_both_tiers() {
    let src = "## To sumL (n: Int, acc: Int) -> Int:\n\
               \x20   If n is at most 0:\n\
               \x20       Return acc.\n\
               \x20   Let r be sumL(n - 1, acc + n).\n\
               \x20   Return r.\n\
               ## Main\n\
               Show sumL(5000, 0).\n";
    assert_constant_stack(src, "12502500");
}

/// The tail call sits in the THEN branch of an `If` (not the body's last
/// statement). It is still in tail position and must be optimized on both tiers.
#[test]
fn tail_call_in_if_branch_constant_stack_both_tiers() {
    let src = "## To countdown (n: Int, acc: Int) -> Int:\n\
               \x20   If n is greater than 0:\n\
               \x20       Return countdown(n - 1, acc + 1).\n\
               \x20   Return acc.\n\
               ## Main\n\
               Show countdown(5000, 0).\n";
    assert_constant_stack(src, "5000");
}

/// ACCUMULATOR recursion: `Return n + sumAcc(n - 1)` is NOT a tail call (the
/// add happens after the call), but it is single-linear recursion that strength-
/// reduces to a constant-stack loop with an accumulator. The AOT already does
/// this; for cross-tier consistency the VM and tree-walker must too, so deep
/// accumulator recursion completes (and agrees) on both instead of hitting the
/// call-depth limit. (op on the RIGHT: `n + self(...)`.)
#[test]
fn accumulator_recursion_constant_stack_both_tiers() {
    let src = "## To sumAcc (n: Int) -> Int:\n\
               \x20   If n is at most 0:\n\
               \x20       Return 0.\n\
               \x20   Return n + sumAcc(n - 1).\n\
               ## Main\n\
               Show sumAcc(5000).\n";
    assert_constant_stack(src, "12502500");
}

/// Accumulator with the self-call on the LEFT and a `*` fold: `Return prodAcc(n
/// - 1) * 1` style is trivial, so use a running product that stays in range:
/// factorial-shaped but additive to avoid i64 overflow — `Return sumAcc2(n-1) +
/// n` (self-call LEFT, nonRec RIGHT).
#[test]
fn accumulator_recursion_self_call_left_both_tiers() {
    let src = "## To sumAcc2 (n: Int) -> Int:\n\
               \x20   If n is at most 0:\n\
               \x20       Return 0.\n\
               \x20   Return sumAcc2(n - 1) + n.\n\
               ## Main\n\
               Show sumAcc2(5000).\n";
    assert_constant_stack(src, "12502500");
}

/// TCO must PRESERVE results: shallow tail recursion (both engines complete
/// natively, below the depth limit) matches exactly.
#[test]
fn shallow_tail_recursion_matches_tree_walker() {
    let (vm_out, vm_err, tw_out, tw_err) = both_tiers(
        "## To sumto (n: Int, acc: Int) -> Int:\n\
         \x20   If n is at most 0:\n\
         \x20       Return acc.\n\
         \x20   Return sumto(n - 1, acc + n).\n\
         ## Main\n\
         Show sumto(100, 0).\n",
    );
    assert_eq!(vm_err, None);
    assert_eq!(tw_err, None);
    assert_eq!(vm_out, tw_out);
    assert_eq!(vm_out, "5050");
}

/// A self-call that is NOT in tail position (its result is used in an arithmetic
/// expression) must NOT be optimized — it recurses, so deep recursion hits the
/// call-depth limit on both tiers. This guards against over-eager TCO turning
/// genuine recursion into a wrong-answer loop.
#[test]
fn non_tail_recursion_still_bounded_both_tiers() {
    let src = "## To spin (n: Int) -> Int:\n\
               \x20   Return spin(n + 1) - 1.\n\
               ## Main\n\
               Show spin(0).\n";
    let (_, vm_err, _, tw_err) = both_tiers(src);
    assert_eq!(
        vm_err.as_deref(),
        Some("Stack overflow: maximum call depth exceeded"),
        "non-tail recursion must stay bounded on the VM"
    );
    assert_eq!(
        tw_err.as_deref(),
        Some("Stack overflow: maximum call depth exceeded"),
        "non-tail recursion must stay bounded on the tree-walker"
    );
}
