//! ★ LOCK — a one-expression helper over pure builtins inlines, and a compound argument is bound once ahead of it. ★
//!
//! The run path's tiny-function inliner (`optimize::inline_tiny`) turns a call of a helper whose whole body is
//! `Return <expr>` into that expression, so a hot loop pays no call. SHA-256's round helpers are that shape over the
//! word builtins (`rotr`, `word32Shr`, `word_and`, …), which are pure: a builtin call cannot recurse or act, so it is
//! in the inlinable fragment like an operator. A call whose arguments are not all names or literals inlines where it
//! is a statement's whole value (`Let s0 be ssig0(item (t - 14) of sched)`): each compound argument is bound once to a
//! fresh name ahead of the statement, in argument order, which is exactly when the call would have evaluated it, and
//! the body reads the fresh names. A name the program defines as a function is never taken for a builtin.
//!
//! Each row runs on the tree-walker (unoptimized), the VM and the optimized run path; the VM's listing shows whether
//! Main still calls the helper.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test inline_tiny_lock`

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome, vm_outcome, vm_outcome_tiered};
use logicaffeine_compile::optimization::Tier;
use logicaffeine_compile::ui_bridge::vm_listing;

/// `src` answers on the VM and the optimized run path what it answers on the tree-walker: output and failure.
fn same_everywhere(src: &str) {
    let tw = tw_outcome(src);
    let vm = vm_outcome(src);
    let run = vm_outcome_tiered(src, &[], Tier::T3, None);
    assert_eq!(
        (vm.output.trim_end(), &vm.error),
        (tw.output.trim_end(), &tw.error),
        "the VM's answer:\n{src}"
    );
    assert_eq!(
        (run.output.trim_end(), &run.error),
        (tw.output.trim_end(), &tw.error),
        "the run path's answer:\n{src}"
    );
}

/// The instructions of Main in the bytecode the VM runs.
fn main_section(src: &str) -> String {
    let listing = vm_listing(src).expect("the program compiles");
    listing
        .split("\n## To ")
        .next()
        .unwrap_or_default()
        .to_string()
}

const SIG: &str = "## To sig (x: Word32) -> Word32:
    Return rotr(x, 7) xor rotr(x, 18) xor word32Shr(x, 3).

";

#[test]
fn a_helper_built_from_pure_builtins_inlines() {
    let src = format!(
        "{SIG}## Main
Let mutable acc be word32(0).
Let mutable k be 1.
While k is at most 300:
    Let w be word32(k * 2654435761).
    Set acc to acc xor sig(w).
    Set k to k + 1.
Show intOfWord32(acc).
"
    );
    same_everywhere(&src);
    let main = main_section(&src);
    assert!(!main.contains("Call "), "sig is inlined into Main:\n{main}");
}

#[test]
fn a_compound_argument_is_bound_once_ahead_of_the_call() {
    let src = format!(
        "{SIG}## Main
Let mutable ws be a new Seq of Word32.
Let mutable k be 1.
While k is at most 64:
    Push word32(k * 40503) to ws.
    Set k to k + 1.
Let mutable acc be word32(0).
Set k to 2.
While k is at most 64:
    Let s be sig(item (k - 1) of ws).
    Set acc to acc xor s.
    Set k to k + 1.
Show intOfWord32(acc).
"
    );
    same_everywhere(&src);
    let main = main_section(&src);
    assert!(
        !main.contains("Call "),
        "sig(item (k - 1) of ws) is inlined through a fresh binding:\n{main}"
    );
}

#[test]
fn a_failing_argument_fails_as_the_call_did() {
    let src = format!(
        "{SIG}## Main
Let mutable ws be a new Seq of Word32.
Push word32(5) to ws.
Let s be sig(item 9 of ws).
Show intOfWord32(s).
"
    );
    same_everywhere(&src);
}

#[test]
fn a_function_named_like_a_builtin_is_the_programs() {
    let src = "## To rotr (x: Int, k: Int) -> Int:
    Set calls to calls + 1.
    Return x + k.

## To sig (x: Int) -> Int:
    Return rotr(x, 7) + rotr(x, 1).

## Main
Let mutable calls be 0.
Let mutable acc be 0.
Let mutable k be 1.
While k is at most 50:
    Set acc to acc + sig(k).
    Set k to k + 1.
Show acc.
Show calls.
";
    same_everywhere(src);
}

#[test]
fn no_inline_keeps_every_call() {
    let src = format!(
        "## No inline\n{SIG}## Main
Let mutable acc be word32(0).
Let mutable k be 1.
While k is at most 20:
    Set acc to acc xor sig(word32(k)).
    Set k to k + 1.
Show intOfWord32(acc).
"
    );
    same_everywhere(&src);
}
