//! Dominator-scoped GVN lock.
//!
//! `gvn.rs` value-numbers ACROSS control-flow boundaries: it threads the parent
//! block's value table INTO an `If`'s arms and a loop's body (then restores it
//! and invalidates whatever the region wrote), so a subexpression computed
//! before a branch is reused inside it, and one computed before a loop survives
//! after it. These locks pin that the reuse is (a) present and (b) RELOAD-CORRECT
//! — it must never reuse a value whose dependency was written in between, which
//! is the one way dominator-scoping could go unsound.
//!
//! Verified as opt-vs-raw execution: `interpret_program` runs the OPTIMIZING VM
//! (GVN applied), `vm_run_source` the RAW VM. A wrong reuse changes `opt` and is
//! caught.

use logicaffeine_compile::compile::{interpret_program, optimizations_fired, vm_run_source};

fn opt(src: &str) -> String {
    interpret_program(src)
        .unwrap_or_else(|e| panic!("optimized run failed: {e:?}"))
        .trim_end()
        .to_string()
}
fn raw(src: &str) -> String {
    vm_run_source(src)
        .unwrap_or_else(|e| panic!("raw run failed: {e}"))
        .trim_end()
        .to_string()
}

/// A `Let`-bound subexpression computed before an `If` and recomputed inside an
/// arm is a GVN opportunity across the branch boundary — the parent value table
/// threads into the arm — and it fires (`cse`), semantics preserved. (GVN
/// rewrites `Let` values, so the reuse site is a `Let`.)
#[test]
fn gvn_reuses_across_if_arm() {
    // `x + y` is computed for `a` before the `If`, then again for `b` inside the
    // arm. Values come from array reads so `fold`/`propagate` cannot pre-collapse
    // them into a literal that would need no CSE.
    let src = "## Main\nLet data be a new Seq of Int.\nPush 3 to data.\nPush 4 to data.\nLet x be item 1 of data.\nLet y be item 2 of data.\nLet a be x + y.\nIf a is greater than 5:\n    Let b be x + y.\n    Show b.\nOtherwise:\n    Show 0.\n";
    assert!(
        optimizations_fired(src).contains(&"cse"),
        "GVN must fire across the If boundary"
    );
    assert_eq!(opt(src), "7");
    assert_eq!(
        opt(src),
        raw(src),
        "GVN across If changed the result (unsound!)"
    );
}

/// RELOAD-CORRECTNESS: an `item k of arr` read must NEVER be reused across a
/// write to `arr` — the promoted value would be stale.
#[test]
fn gvn_reload_correct_across_array_write() {
    let src = "## Main\nLet mutable arr be a new Seq of Int.\nPush 10 to arr.\nLet a be item 1 of arr.\nSet item 1 of arr to 99.\nLet b be item 1 of arr.\nShow a + b.\n";
    // a = 10, b = 99 → 109. A stale reuse (b := a) would give 20.
    assert_eq!(opt(src), "109");
    assert_eq!(
        opt(src),
        raw(src),
        "GVN reused an array element across a write (stale!)"
    );
}

/// RELOAD-CORRECTNESS across a loop back-edge: a subexpression whose dependency
/// the loop WRITES must not be reused from before the loop on later iterations.
#[test]
fn gvn_reload_correct_across_loop_backedge() {
    // `base + k` is computed each iteration; `k` is loop-carried. A dominator-
    // scoped reuse of a pre-loop `base + k` would be wrong once `k` advances.
    let src = "## Main\nLet data be a new Seq of Int.\nPush 5 to data.\nLet base be item 1 of data.\nLet mutable k be 0.\nLet pre be base + k.\nLet mutable total be pre.\nWhile k is less than 4:\n    Set k to k + 1.\n    Set total to total + (base + k).\nShow total.\n";
    // pre = 5+0 = 5; then +(5+1)+(5+2)+(5+3)+(5+4) = 5 + 6+7+8+9 = 35.
    assert_eq!(opt(src), "35");
    assert_eq!(
        opt(src),
        raw(src),
        "GVN reused a loop-carried subexpression across the back-edge (stale!)"
    );
}

/// A value computed before a loop the loop does NOT write survives to after it
/// — semantics preserved either way.
#[test]
fn gvn_value_survives_past_unwritten_loop() {
    let src = "## Main\nLet data be a new Seq of Int.\nPush 6 to data.\nPush 7 to data.\nLet p be item 1 of data.\nLet q be item 2 of data.\nLet before be p + q.\nLet mutable i be 0.\nWhile i is less than 3:\n    Set i to i + 1.\nLet after be p + q.\nShow before + after.\n";
    assert_eq!(opt(src), "26"); // (6+7) + (6+7)
    assert_eq!(
        opt(src),
        raw(src),
        "GVN across an unwritten loop changed the result"
    );
}
