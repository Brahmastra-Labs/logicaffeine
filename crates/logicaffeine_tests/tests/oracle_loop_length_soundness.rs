//! Soundness lock: the Oracle (`optimize::abstract_interp`) must never drop a live statement it
//! guards behind a collection-length condition that a loop mutates.
//!
//! The bug this pins: `Let cur be a new Seq of Int` pinned `length(cur) = [0,0]` in the abstract
//! state, and the `While`/`Repeat` arms widened loop-written *scalar values* but never *collection
//! lengths*. So a `(length of cur) > 0` guard — inside the loop body AND after it — constant-folded
//! to `false` against the stale `[0,0]` length even though `Push b to cur` demonstrably grows it,
//! and the Oracle deleted the live `Push` as dead code. It surfaced on the C tier (its pipeline runs
//! the Oracle over the raw AST; the AOT-Rust pipeline's comptime pre-pass masked it) and silently
//! miscompiled `words` (split_whitespace) and `splitAfter` — exactly the split-builder shape below.
//!
//! `assert_all_tiers` only proves tree-walker ≡ VM, so this ALSO drives `aot`
//! (`assert_compiled_equals_interpreted`) and `c` (`assert_c_output`) — the tiers the default
//! differential ignores and where the drop actually lived. Losing length precision is always sound
//! (it keeps more code); the miscompile was keeping *stale* precision, so widening the mutated
//! length to `top` is the fix and this locks it against regression.

#![allow(unused_imports)]
mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// The real trigger: a split-builder (`words`/`splitAfter` shape). A nested length-guarded `If` in
/// the then-block of an outer `If` with an `Otherwise`, all inside a `While`, plus a final flush
/// guarded on the same length after the loop. `"aa bb"` has two whitespace-delimited runs → 2.
const SPLIT_BUILDER: &str = "\
## To splitparts (s: Text) -> Seq of Text:
    Let sb be text_bytes(s).
    Let result be a new Seq of Text.
    Let mutable cur be a new Seq of Int.
    Let mutable i be 1.
    While i is at most (length of sb):
        Let b be item i of sb.
        If b is equal to 32:
            If (length of cur) is greater than 0:
                Push text_from_bytes(cur) to result.
                Set cur to a new Seq of Int.
        Otherwise:
            Push b to cur.
        Set i to i + 1.
    If (length of cur) is greater than 0:
        Push text_from_bytes(cur) to result.
    Return result.
## Main
Show length of splitparts(\"aa bb\").
";

#[test]
fn split_builder_tiers() {
    assert_all_tiers(SPLIT_BUILDER, "2");
}

#[test]
fn split_builder_aot() {
    assert_compiled_equals_interpreted(SPLIT_BUILDER);
}

#[test]
fn split_builder_c() {
    assert_c_output(SPLIT_BUILDER, "2");
}

/// Pure-integer chunk counter — no text builtins, so it isolates the in-loop length widening.
/// Guard `(length of cur) > 0` fires inside the loop (chunk boundary) and after it (final flush).
/// i=1..10: pushes accumulate into `cur`, every third `i` flushes a non-empty `cur` → 4 chunks.
const CHUNK_COUNTER: &str = "\
## To countchunks (n: Int) -> Int:
    Let mutable cur be a new Seq of Int.
    Let mutable chunks be 0.
    Let mutable i be 1.
    While i is at most n:
        If (i mod 3) is equal to 0:
            If (length of cur) is greater than 0:
                Set chunks to chunks + 1.
                Set cur to a new Seq of Int.
        Otherwise:
            Push i to cur.
        Set i to i + 1.
    If (length of cur) is greater than 0:
        Set chunks to chunks + 1.
    Return chunks.
## Main
Show countchunks(10).
";

#[test]
fn chunk_counter_tiers() {
    assert_all_tiers(CHUNK_COUNTER, "4");
}

#[test]
fn chunk_counter_aot() {
    assert_compiled_equals_interpreted(CHUNK_COUNTER);
}

#[test]
fn chunk_counter_c() {
    assert_c_output(CHUNK_COUNTER, "4");
}

/// Post-loop-only guard: the length-guarded `If` sits AFTER the `While`, so it isolates the
/// *post-loop* length widening (`state.set_length(w, top)` after the loop). Even `i` values get
/// pushed; `n=3` pushes one element → length 1 → returns 1; `n=1` pushes none → returns 0.
const POST_LOOP_GUARD: &str = "\
## To hasitems (n: Int) -> Int:
    Let mutable xs be a new Seq of Int.
    Let mutable i be 1.
    While i is at most n:
        If (i mod 2) is equal to 0:
            Push i to xs.
        Set i to i + 1.
    If (length of xs) is greater than 0:
        Return 1.
    Return 0.
## Main
Show hasitems(3).
Show hasitems(1).
";

#[test]
fn post_loop_guard_tiers() {
    assert_all_tiers(POST_LOOP_GUARD, "1\n0");
}

#[test]
fn post_loop_guard_aot() {
    assert_compiled_equals_interpreted(POST_LOOP_GUARD);
}

#[test]
fn post_loop_guard_c() {
    assert_c_output(POST_LOOP_GUARD, "1\n0");
}
