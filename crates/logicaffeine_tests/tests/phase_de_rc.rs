//! O2 — de-Rc: a Seq that provably never needs reference semantics is emitted
//! as a plain `Vec<T>` instead of `LogosSeq<T> = Rc<RefCell<Vec<T>>>`.
//!
//! Eliminating the Rc removes the heap-allocated control block, the triple
//! indirection (Rc → RefCell → Vec → buffer), the refcount traffic, and the
//! RefCell borrow-flag bookkeeping. A Seq is de-Rc-eligible when, across its
//! whole scope, it is never aliased by a second live handle and never escapes
//! (call arg, return, stored as an element/field).
//!
//! Soundness backstop: an unsound de-Rc makes the generated Rust fail to
//! compile (two owners, or a moved-from use) — a loud, immediate failure the
//! execution tests catch, exactly like borrow-hoisting's `already borrowed`.

mod common;

use common::compile_to_rust;

// =============================================================================
// Phase 4 — return-type de-Rc: a function whose every Return value is a
// uniquely-owned fresh Seq returns `Vec<T>` instead of `LogosSeq<T>`. This
// unlocks de-Rc on the locals that capture the call result (`Set left to
// msort(left)`), which the current analysis disqualifies (detection.rs ~1544:
// a callee's `LogosSeq` return is "non-fresh"). The mergesort keystone.
// =============================================================================

/// Recursive mergesort: `msort` returns a freshly-built `result` (or a copy of
/// its param in the base case) — never an aliased global — so its return type
/// must de-Rc to `Vec<i64>`, eliminating the per-call `Rc.clone().borrow()` and
/// the Rc box, and letting `left`/`right`/`result` de-Rc to plain Vecs.
///
/// PINNED OBJECTIVE (Phase 4 — return-type de-Rc, the mergesort allocation
/// keystone). Return-type de-Rc is an interprocedural aliasing FIXPOINT (a
/// function may return `Vec` only if every caller, incl. recursive self-calls,
/// treats the result as uniquely-owned, not aliasing the argument), co-analyzed
/// with the callers' de-Rc eligibility. An unsound version silently corrupts
/// output. Implemented via the two-fixpoint `collect_vec_return_fns`
/// (least: return-ownability; greatest: caller soundness) in `detection.rs`.
#[test]
fn de_rc_recursive_fresh_return_becomes_vec() {
    let source = r#"## To msort (arr: Seq of Int) -> Seq of Int:
    Let n be length of arr.
    If n is less than 2:
        Return arr.
    Let mid be n / 2.
    Let mutable left be a new Seq of Int.
    Let mutable right be a new Seq of Int.
    Let mutable i be 1.
    While i is at most mid:
        Push item i of arr to left.
        Set i to i + 1.
    While i is at most n:
        Push item i of arr to right.
        Set i to i + 1.
    Set left to msort(left).
    Set right to msort(right).
    Let mutable result be a new Seq of Int.
    Let mutable li be 1.
    Let mutable ri be 1.
    While li is at most length of left:
        If ri is greater than length of right:
            Push item li of left to result.
            Set li to li + 1.
        Otherwise:
            If item li of left is at most item ri of right:
                Push item li of left to result.
                Set li to li + 1.
            Otherwise:
                Push item ri of right to result.
                Set ri to ri + 1.
    While ri is at most length of right:
        Push item ri of right to result.
        Set ri to ri + 1.
    Return result.
## Main
Let mutable arr be a new Seq of Int.
Push 5 to arr. Push 2 to arr. Push 8 to arr. Push 1 to arr. Push 9 to arr. Push 3 to arr.
Set arr to msort(arr).
Show "" + item 1 of arr + item 2 of arr + item 3 of arr + item 4 of arr + item 5 of arr + item 6 of arr.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("-> Vec<i64>"),
        "msort's every Return is a uniquely-owned fresh Seq — its return type must \
         de-Rc to `Vec<i64>` (not LogosSeq). Got:\n{}",
        rust
    );
    assert!(
        !rust.contains(".borrow()"),
        "a fully de-Rc'd mergesort must have no RefCell borrows. Got:\n{}",
        rust
    );
    // The sort must be CORRECT — an unsound de-Rc (aliasing) corrupts the merge.
    common::assert_exact_output(source, "123589");
}

// =============================================================================
// Phase 1 — non-aliased, non-escaping local Seqs become Vec<T>
// =============================================================================

/// A runtime-sized Seq (loop-built, so O3 scalarization does NOT claim it),
/// pushed and indexed locally, never aliased, never escaping → plain Vec<i64>.
#[test]
fn de_rc_nonaliased_runtime_seq_becomes_vec() {
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("5").
Let mutable xs be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i * i to xs.
    Set i to i + 1.
Let s be item 3 of xs.
Show s.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("xs: Vec<i64>"),
        "xs is non-aliased and non-escaping — it must de-Rc to a plain Vec<i64>. Got:\n{}",
        rust
    );
    assert!(
        !rust.contains("xs.borrow"),
        "a de-Rc'd Vec must never be borrowed (no RefCell). Got:\n{}",
        rust
    );
    // item 3 (1-based) = index 2 = 2*2 = 4.
    common::assert_exact_output(source, "4");
}

/// De-Rc'd Vec supports the full local access vocabulary: push, index, set,
/// length, read-after. None of these introduce reference semantics.
#[test]
fn de_rc_vec_supports_index_set_length() {
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("6").
Let mutable xs be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push 0 to xs.
    Set i to i + 1.
Set i to 0.
While i is less than n:
    Set item (i + 1) of xs to i * 10.
    Set i to i + 1.
Let total be 0.
Set i to 0.
While i is less than length of xs:
    Set total to total + item (i + 1) of xs.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("xs: Vec<i64>"),
        "xs must de-Rc to Vec<i64>. Got:\n{}",
        rust
    );
    assert!(!rust.contains("xs.borrow"), "no RefCell borrows. Got:\n{}", rust);
    // 0+10+20+30+40+50 = 150.
    common::assert_exact_output(source, "150");
}

// =============================================================================
// Soundness controls — these must stay correct (de-Rc must REFUSE them in
// Phase 1, so reference semantics are preserved).
// =============================================================================

/// A `Let ys be xs` binding is an INDEPENDENT value under value semantics:
/// pushing through `ys` grows only `ys`, never the original `xs`. Whether or
/// not de-Rc fires, both engines must isolate.
#[test]
fn aliased_seq_isolates_under_value_semantics() {
    let source = r#"## Main
Let mutable xs be a new Seq of Int.
Push 1 to xs.
Let mutable ys be xs.
Push 2 to ys.
Push 3 to ys.
Show length of xs.
"#;
    // ys is an independent value; pushing through it leaves xs at length 1.
    common::assert_exact_output(source, "1");
}

/// A Seq passed to a user function escapes. Phase 1 must keep it sound
/// (Phase 3 will de-Rc it to a slice param). Pin semantics, not representation.
#[test]
fn escaping_seq_stays_sound() {
    let source = r#"## To sumSeq (data: Seq of Int) -> Int:
    Let mutable total be 0.
    Let mutable i be 0.
    While i is less than length of data:
        Set total to total + item (i + 1) of data.
        Set i to i + 1.
    Return total.

## Main
Let mutable xs be a new Seq of Int.
Push 5 to xs.
Push 7 to xs.
Push 9 to xs.
Let r be sumSeq(xs).
Show r.
"#;
    // 5+7+9 = 21.
    common::assert_exact_output(source, "21");
}

// =============================================================================
// Function-local de-Rc — a Seq local to a function body (not a parameter, not
// returned) de-Rc's to Vec<T> exactly like a Main-local one. This is the
// sieve/scratch-buffer case: the hot Seq lives inside a helper.
// =============================================================================

#[test]
fn de_rc_function_local_seq_becomes_vec() {
    let source = r#"## To countTrue (n: Int) -> Int:
    Let mutable flags be a new Seq of Bool.
    Let mutable i be 0.
    While i is less than n:
        Push false to flags.
        Set i to i + 1.
    Set item 1 of flags to true.
    Set item 3 of flags to true.
    Let mutable c be 0.
    Set i to 0.
    While i is less than n:
        If item (i + 1) of flags equals true:
            Set c to c + 1.
        Set i to i + 1.
    Return c.

## Main
Show countTrue(5).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("flags: Vec<bool>"),
        "a function-local, non-escaping scalar Seq must de-Rc to Vec<bool>. Got:\n{}",
        rust
    );
    assert!(
        !rust.contains("flags.borrow"),
        "a de-Rc'd Vec must never be borrowed. Got:\n{}",
        rust
    );
    // items 1 and 3 set true → count 2.
    common::assert_exact_output(source, "2");
}

/// A function-local Seq that IS returned escapes — it stays LogosSeq (Phase 3
/// will handle return-by-value). Pin semantics.
#[test]
fn returned_function_local_stays_sound() {
    let source = r#"## To buildSeq (n: Int) -> Seq of Int:
    Let mutable xs be a new Seq of Int.
    Let mutable i be 0.
    While i is less than n:
        Push i * i to xs.
        Set i to i + 1.
    Return xs.

## Main
Let r be buildSeq(4).
Show item 3 of r.
"#;
    // item 3 (1-based) = index 2 = 2*2 = 4.
    common::assert_exact_output(source, "4");
}

// =============================================================================
// Phase 3 — interprocedural: a Seq passed ONLY to borrow-params (&[T]/&mut [T])
// does not escape, so the caller's Seq de-Rc's and is passed by reference.
// =============================================================================

#[test]
fn de_rc_seq_passed_to_readonly_param() {
    let source = r#"## To sumSeq (data: Seq of Int) -> Int:
    Let mutable total be 0.
    Let mutable i be 0.
    While i is less than length of data:
        Set total to total + item (i + 1) of data.
        Set i to i + 1.
    Return total.

## Main
Let mutable xs be a new Seq of Int.
Let mutable i be 0.
While i is less than 5:
    Push i * i to xs.
    Set i to i + 1.
Show sumSeq(xs).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("xs: Vec<i64>"),
        "xs is passed only to a read-only param — it should de-Rc to Vec<i64>. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("sumSeq(&xs)") || rust.contains("sumSeq(&xs["),
        "the de-Rc'd Vec must be passed by reference to the &[T] param. Got:\n{}",
        rust
    );
    assert!(
        !rust.contains("xs.borrow"),
        "no RefCell borrow on a de-Rc'd Vec. Got:\n{}",
        rust
    );
    // sum of 0,1,4,9,16 = 30.
    common::assert_exact_output(source, "30");
}

// =============================================================================
// Phase 3b — interprocedural mut-borrow: a Seq passed ONLY to &mut [T] params
// (in-place element mutation, never retained/aliased) de-Rc's and is passed by
// mutable reference. The in-place-sort shape (quicksort/heap_sort `arr`).
// =============================================================================

/// The in-place-sort shape (quicksort/heap_sort): `sortStep` mutates its Seq
/// param's elements and returns it, so the param is a `&mut [T]` borrow and the
/// call `Set xs to sortStep(xs)` lowers to a void in-place call. The caller's
/// `xs` is therefore passed only at a mutable-BORROW position (not retained,
/// not aliased) + read by index — so it must de-Rc to `Vec<i64>` and the call
/// site passes `&mut xs`, with no RefCell borrow anywhere.
#[test]
fn de_rc_seq_passed_to_mut_borrow_param() {
    // The mut-borrow de-Rc is a REFERENCE-semantics optimization: it would let
    // `sortStep` mutate the caller's `xs` in place through a plain param, which
    // the value-semantics liveness gate forbids. The opt remains valuable when
    // value semantics is off ("fastest when we don't need value semantics"), so
    // this test pins it in reference mode. nextest runs it process-isolated, so
    // the flag set here is local to this test.
    std::env::set_var("LOGOS_VALUE_SEMANTICS", "0");
    let source = r#"## To sortStep (arr: Seq of Int) -> Seq of Int:
    Let mutable result be arr.
    Let mutable i be 1.
    While i is less than length of result:
        If item i of result is greater than item (i + 1) of result:
            Let tmp be item i of result.
            Set item i of result to item (i + 1) of result.
            Set item (i + 1) of result to tmp.
        Set i to i + 1.
    Return result.

## Main
Let mutable xs be a new Seq of Int.
Push 3 to xs. Push 1 to xs. Push 2 to xs.
Set xs to sortStep(xs).
Show "" + item 1 of xs + item 2 of xs + item 3 of xs.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("arr: &mut [i64]"),
        "sortStep mutates its param in place and returns it — the param is &mut [i64]. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("xs: Vec<i64>"),
        "xs is passed only to a &mut [T] param (in-place) — it should de-Rc to Vec<i64>. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("sortStep(&mut xs)"),
        "the de-Rc'd Vec must be passed by mutable reference to the &mut [T] param. Got:\n{}",
        rust
    );
    assert!(
        !rust.contains("xs.borrow"),
        "no RefCell borrow on a de-Rc'd Vec. Got:\n{}",
        rust
    );
    // one bubble pass over [3,1,2]: 3>1 swap → [1,3,2]; 3>2 swap → [1,2,3].
    common::assert_exact_output(source, "123");
}

// =============================================================================
// Phase 2 — swap / buffer-reuse pairs de-Rc to Vec<T>
// =============================================================================

/// The knapsack DP shape: `prev` and `curr` are "aliased" only through
/// `Set prev to curr`, which codegen lowers to `std::mem::swap` (a content
/// exchange, not a shared handle). Both buffers therefore de-Rc to plain
/// `Vec<T>` and the swap stays.
#[test]
fn de_rc_buffer_reuse_swap_pair_becomes_vec() {
    let source = r#"## Main
Let cols be 8.
Let mutable prev be a new Seq of Int.
Let mutable k be 0.
While k is less than cols:
    Push 1 to prev.
    Set k to k + 1.
Let mutable i be 0.
While i is less than 6:
    Let mutable curr be a new Seq of Int.
    Let mutable f be 0.
    While f is less than cols:
        Push 0 to curr.
        Set f to f + 1.
    Let mutable w be 0.
    While w is less than cols:
        Set item (w + 1) of curr to item (w + 1) of prev + w.
        Set w to w + 1.
    Set prev to curr.
    Set i to i + 1.
Let mutable total be 0.
Set k to 0.
While k is less than cols:
    Set total to total + item (k + 1) of prev.
    Set k to k + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("prev: Vec<i64>"),
        "prev (a buffer-reuse swap partner) must de-Rc to Vec<i64>. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("curr: Vec<i64>"),
        "curr (the fresh-each-iteration buffer) must de-Rc to Vec<i64>. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("std::mem::swap(&mut prev, &mut curr)"),
        "the rebind must lower to a Vec content swap. Got:\n{}",
        rust
    );
    assert!(
        !rust.contains("prev.borrow") && !rust.contains("curr.borrow"),
        "neither de-Rc'd buffer may be borrowed. Got:\n{}",
        rust
    );
    // After 6 rounds element w = 1 + 6w; total over w=0..7 = 8 + 6*28 = 176.
    common::assert_exact_output(source, "176");
}

/// Wave-2 Phase A1 (knapsack 1.97x keystone). The knapsack DP refills `curr` (the inner
/// partner of a `Set prev to curr` ping-pong swap pair) via `Push best to curr`
/// in a counted w-loop. Codegen today hoists `curr` to `Vec::new()` + `clear()`
/// + `curr.push(best)` — and the `push` MUTATES `curr.len()` every iteration, a
/// loop-carried dependency on the Vec metadata that blocks LLVM from
/// vectorizing the DP scan (C writes `curr[w] = …` into a fixed array). The fix:
/// when a reused buffer is refilled by push in a counted loop whose trip count
/// equals the partner buffer's size, emit it SIZED (`vec![default; size]`, no
/// `clear()`) + INDEXED WRITES (`curr[w] = best`). Then `curr.push` disappears
/// and the loop vectorizes.
#[test]
fn buffer_reuse_push_refill_becomes_indexed_write() {
    let source = r#"## Main
Let cols be 8.
Let mutable prev be a new Seq of Int.
Let mutable k be 0.
While k is less than cols:
    Push 0 to prev.
    Set k to k + 1.
Let mutable i be 0.
While i is less than 4:
    Let mutable curr be a new Seq of Int.
    Let mutable w be 0.
    While w is less than cols:
        Let v be item (w + 1) of prev + i.
        Push v to curr.
        Set w to w + 1.
    Set prev to curr.
    Set i to i + 1.
Show item 1 of prev.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("curr.push"),
        "the counted push-refill of the reused buffer must become indexed writes, \
         not `curr.push` (the length mutation blocks vectorization). Got:\n{}",
        rust
    );
    assert!(
        rust.contains("curr[") || rust.contains("curr [")
            || rust.contains("= v;") && rust.contains("Vec::with_capacity"),
        "curr must be sized + written by index. Got:\n{}",
        rust
    );
    // prev[0] accumulates += i each round: 0 + 0 + 1 + 2 + 3 = 6.
    common::assert_exact_output(source, "6");
}

/// A1 soundness: the fill conversion must NOT fire when the buffer is pushed at
/// the TOP LEVEL of the swap loop (a fresh length-1 Seq rebound into a handle
/// each iteration). Such a buffer never accumulates trip-count elements, so
/// sizing it to the trip count and writing `fresh[i]` would index past its true
/// length. It must keep `clear()` + `push()`. Distinct from the knapsack shape
/// above, where the push lives in a NESTED loop that builds the buffer to length.
#[test]
fn buffer_reuse_top_level_push_swap_keeps_clear_push() {
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("5").
Let mutable arr be a new Seq of Int.
Push 7 to arr.
Let mutable total be 0.
Let mutable i be 0.
While i is less than n:
    Set total to total + item 1 of arr.
    Let mutable fresh be a new Seq of Int.
    Push total to fresh.
    Set arr to fresh.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("fresh.clear"),
        "a top-level push-once-then-swap buffer must keep `fresh.clear()` so it \
         does not accumulate across iterations. Got:\n{}",
        rust
    );
    assert!(
        !rust.contains("fresh.resize") && !rust.contains("fresh["),
        "the buffer is NOT filled by a nested loop, so it must not be sized to \
         the trip count nor written by index (that would over-index after the \
         swap rolls in a shorter buffer). Got:\n{}",
        rust
    );
    // arr is always [total]: total = 7,14,28,56,112 across the 5 iterations.
    common::assert_exact_output(source, "112");
}
