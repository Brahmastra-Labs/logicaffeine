//! O1 gate — borrow hoisting (scoped slice extraction).
//!
//! When a loop body indexes Seq handles without rebinding, resizing, or
//! leaking them, the per-access `RefCell` traffic collapses to ONE borrow
//! per loop: a guard + shadow pair in a scope around the loop,
//!
//! ```text
//! {
//!     let __prev_g = prev.borrow();
//!     let prev = &__prev_g[..];
//!     let mut __curr_g = curr.borrow_mut();
//!     let curr = &mut __curr_g[..];
//!     for w in 0..n { curr[w as usize] = prev[w as usize]; }
//! }
//! ```
//!
//! Soundness rests on the alias oracle (phase_alias_oracle.rs): a handle is
//! mut-hoisted only when PROVABLY distinct from every other handle the loop
//! touches. The adversarial half of this suite feeds the hoister aliased
//! handles every way LOGOS can make them — the RefCell runtime makes any
//! soundness slip a loud `already borrowed` panic, so each adversarial
//! program EXECUTING CORRECTLY is itself the proof of refusal.

mod common;

use common::compile_to_rust;

/// The loop body text between the LAST `for <counter> in` header in the
/// generated code and the first `}` line back at the same indentation.
fn last_for_body<'a>(rust: &'a str, counter: &str) -> &'a str {
    let header = format!("for {} in", counter);
    let start = rust
        .rfind(&header)
        .unwrap_or_else(|| panic!("no `{}` loop in:\n{}", header, rust));
    let body_start = start + rust[start..].find('\n').expect("newline after for header") + 1;
    let indent = rust[..start].rfind('\n').map(|p| start - p - 1).unwrap_or(0);
    let closer = format!("\n{}}}", " ".repeat(indent));
    let body_end = rust[body_start..]
        .find(&closer)
        .map(|p| body_start + p)
        .unwrap_or(rust.len());
    &rust[body_start..body_end]
}

// =============================================================================
// Positive: hoisting fires and the hot loop is borrow-free
// =============================================================================

#[test]
fn hoist_read_only_single_seq() {
    // `arr` is a Seq OF Seq — its elements are handles, so de-Rc (scalar-only
    // in v1) leaves it as a reference-semantics `LogosSeq`, and the read loop
    // hoists its borrow exactly once. Each row is a singleton inner Seq.
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("50").
Let mutable arr be a new Seq of (Seq of Int).
Let mutable i be 0.
While i is less than n:
    Let mutable row be a new Seq of Int.
    Push i * 3 to row.
    Push row to arr.
    Set i to i + 1.
Let mutable total be 0.
Set i to 0.
While i is less than n:
    Set total to total + item 1 of (item (i + 1) of arr).
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("let __arr_g = arr.borrow();"),
        "read loop should hoist one shared guard. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("let arr = &__arr_g[..];"),
        "guard should be shadowed as a plain slice. Got:\n{}",
        rust
    );
    let body = last_for_body(&rust, "i");
    assert!(
        !body.contains("arr.borrow"),
        "the hoisted OUTER handle must not be borrowed in the body (inner-handle \
         borrows are separate). Body:\n{}\nFull:\n{}",
        body,
        rust
    );
    common::assert_exact_output(source, "3675");
}

#[test]
fn hoist_knapsack_inner_loop() {
    // The headline shape: fresh `curr` per outer iteration, inner loop reads
    // prev and writes curr, handles swapped by rebinding at iteration end.
    // `prev`/`curr` are Seqs OF Seq (singleton inner rows): the inner-handle
    // element type survives de-Rc, so the swap PAIR stays reference-semantics
    // `LogosSeq`s and both hoist their borrows around the inner DP loop. Both
    // are hoisted SHARED there — the DP loop only reads the outer handles (to
    // reach the inner rows); the row writes and the clear/swap are elsewhere.
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("6").
Let cols be 8.
Let mutable prev be a new Seq of (Seq of Int).
Let mutable k be 0.
While k is less than cols:
    Let mutable prow be a new Seq of Int.
    Push 1 to prow.
    Push prow to prev.
    Set k to k + 1.
Let mutable i be 0.
While i is less than n:
    Let mutable curr be a new Seq of (Seq of Int).
    Let mutable f be 0.
    While f is less than cols:
        Let mutable crow be a new Seq of Int.
        Push 0 to crow.
        Push crow to curr.
        Set f to f + 1.
    Let mutable w be 0.
    While w is less than cols:
        Set item 1 of (item (w + 1) of curr) to item 1 of (item (w + 1) of prev) + w.
        Set w to w + 1.
    Set prev to curr.
    Set i to i + 1.
Let mutable total be 0.
Set k to 0.
While k is less than cols:
    Set total to total + item 1 of (item (k + 1) of prev).
    Set k to k + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("let __prev_g = prev.borrow();") && rust.contains("let prev = &__prev_g[..];"),
        "prev should be hoisted shared around the inner loop. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("let __curr_g = curr.borrow();") && rust.contains("let curr = &__curr_g[..];"),
        "curr should also be hoisted shared around the inner loop (its outer \
         handle is read to reach the inner rows). Got:\n{}",
        rust
    );
    let body = last_for_body(&rust, "w");
    assert!(
        !body.contains("prev.borrow") && !body.contains("curr.borrow"),
        "the hoisted OUTER handles must not be borrowed in the inner DP loop \
         (inner singleton-row borrows are separate). Body:\n{}\nFull:\n{}",
        body,
        rust
    );
    // n=6 rounds of curr[w] = prev[w] + w starting from ones:
    // after 6 rounds element w = 1 + 6w; total = 8 + 6*28 = 176.
    common::assert_exact_output(source, "176");
}

#[test]
fn hoist_two_reads_one_write_distinct() {
    // dot-product-with-output shape: two read handles, one write handle, all
    // fresh allocations — all three hoisted. Each is a Seq OF Seq (singleton
    // inner rows), so de-Rc leaves the outer handles reference-semantics
    // `LogosSeq`s and all three hoist exactly as before.
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("20").
Let mutable a be a new Seq of (Seq of Int).
Let mutable b be a new Seq of (Seq of Int).
Let mutable out be a new Seq of (Seq of Int).
Let mutable i be 0.
While i is less than n:
    Let mutable arow be a new Seq of Int.
    Push i to arow.
    Push arow to a.
    Let mutable brow be a new Seq of Int.
    Push i * 2 to brow.
    Push brow to b.
    Let mutable orow be a new Seq of Int.
    Push 0 to orow.
    Push orow to out.
    Set i to i + 1.
Set i to 0.
While i is less than n:
    Set item 1 of (item (i + 1) of out) to item 1 of (item (i + 1) of a) * item 1 of (item (i + 1) of b).
    Set i to i + 1.
Let mutable total be 0.
Set i to 0.
While i is less than n:
    Set total to total + item 1 of (item (i + 1) of out).
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("let __a_g = a.borrow();") && rust.contains("let __b_g = b.borrow();"),
        "both read handles should be hoisted shared. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("let mut __out_g = out.borrow_mut();"),
        "the write handle should be hoisted mutably. Got:\n{}",
        rust
    );
    // sum of 2i^2 for i=0..19 = 2 * 2470 = 4940
    common::assert_exact_output(source, "4940");
}

#[test]
fn hoist_conditional_access_still_fires() {
    // Accesses under an If are still covered by the loop-level borrow. `arr` is
    // a Seq OF Seq (singleton inner rows), so de-Rc leaves the outer handle a
    // reference-semantics `LogosSeq` and its borrow hoists out of the loop.
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("30").
Let mutable arr be a new Seq of (Seq of Int).
Let mutable i be 0.
While i is less than n:
    Let mutable row be a new Seq of Int.
    Push i to row.
    Push row to arr.
    Set i to i + 1.
Let mutable total be 0.
Set i to 0.
While i is less than n:
    If i % 3 equals 0:
        Set total to total + item 1 of (item (i + 1) of arr).
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("let __arr_g = arr.borrow();"),
        "conditional access still hoists at loop level. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "135");
}

#[test]
fn hoist_read_read_alias_is_allowed() {
    // `Let b be arr.` then a read-only loop over both: shared RefCell
    // borrows coexist — both may be hoisted shared.
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("10").
Let mutable arr be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i to arr.
    Set i to i + 1.
Let mutable b be arr.
Let mutable total be 0.
Set i to 0.
While i is less than n:
    Set total to total + item (i + 1) of arr + item (i + 1) of b.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("let __arr_g = arr.borrow();") && rust.contains("let __b_g = b.borrow();"),
        "read-read aliasing is RefCell-legal; both hoist shared. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "90");
}

#[test]
fn hoist_zero_trip_loop_is_safe() {
    // Guards are taken even when the loop body never runs — must not panic.
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("0").
Let mutable arr be a new Seq of Int.
Let mutable total be 0.
Let mutable i be 0.
While i is less than n:
    Set total to total + item (i + 1) of arr.
    Set i to i + 1.
Show total.
"#;
    common::assert_exact_output(source, "0");
}

#[test]
fn hoist_counter_usable_after_loop() {
    // The scope wrapping the loop must not swallow the counter's post-loop
    // value.
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("7").
Let mutable arr be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i to arr.
    Set i to i + 1.
Let mutable total be 0.
Set i to 0.
While i is less than n:
    Set total to total + item (i + 1) of arr.
    Set i to i + 1.
Show i.
Show total.
"#;
    common::assert_exact_output(source, "7\n21");
}

// =============================================================================
// Adversarial: aliasing and leaks must refuse the hoist (execution is the
// soundness check — an unsound hoist panics `already borrowed`)
// =============================================================================

#[test]
fn no_hoist_write_aliased_handles() {
    // `Let b be arr.` then the loop WRITES b and reads arr: a long-lived
    // borrow_mut on b plus any arr access would panic. Must stay per-access.
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("10").
Let mutable arr be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i to arr.
    Set i to i + 1.
Let mutable b be arr.
Set i to 0.
While i is less than n:
    Set item (i + 1) of b to item (i + 1) of arr * 2.
    Set i to i + 1.
Let mutable total be 0.
Set i to 0.
While i is less than n:
    Set total to total + item (i + 1) of arr.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("let mut __b_g = b.borrow_mut();"),
        "write-aliased handle must not be mut-hoisted. Got:\n{}",
        rust
    );
    // b aliases arr: each write doubles in place, reads see updated values.
    // arr = [0,2,4,...,18]; total = 90.
    common::assert_exact_output(source, "90");
}

#[test]
fn no_hoist_conditionally_aliased_handles() {
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let k be parseInt("1").
Let n be parseInt("10").
Let mutable arr be a new Seq of Int.
Let mutable other be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i to arr.
    Push 100 to other.
    Set i to i + 1.
Let mutable b be other.
If k is greater than 0:
    Set b to arr.
Set i to 0.
While i is less than n:
    Set item (i + 1) of b to item (i + 1) of arr + 1.
    Set i to i + 1.
Let mutable total be 0.
Set i to 0.
While i is less than n:
    Set total to total + item (i + 1) of arr.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("let mut __b_g = b.borrow_mut();"),
        "conditionally-aliased handle must not be mut-hoisted. Got:\n{}",
        rust
    );
    // k=1: b aliases arr → arr[i] = arr[i]+1 in place → total = 45+10 = 55.
    common::assert_exact_output(source, "55");
}

#[test]
fn no_hoist_when_handle_passed_to_function_in_body() {
    // A call inside the loop body could touch any handle — no hoisting in
    // that loop. `arr` is a Seq OF Seq (singleton rows) so it survives de-Rc
    // and is genuinely passed by reference-semantics borrow into the call.
    let source = r#"## To native parseInt (s: Text) -> Int

## To peek (xs: Seq of (Seq of Int), j: Int) -> Int:
    Return item 1 of (item j of xs).

## Main
Let n be parseInt("10").
Let mutable arr be a new Seq of (Seq of Int).
Let mutable i be 0.
While i is less than n:
    Let mutable row be a new Seq of Int.
    Push i * 5 to row.
    Push row to arr.
    Set i to i + 1.
Let mutable total be 0.
Set i to 0.
While i is less than n:
    Set total to total + peek(arr, i + 1).
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    // The read loop contains a user call (`peek`) that borrows arr, so it must
    // NOT hoist — the call goes through a per-access borrow. (The separate
    // build loop, which only pushes, vec-hoists; that's fine and unrelated.)
    assert!(
        rust.contains("peek(&*arr.borrow()") || rust.contains("peek(arr.borrow()"),
        "a loop whose body calls a function touching the handle must use a per-access borrow. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "225");
}

#[test]
fn no_hoist_call_returned_handle() {
    // `Let b be pick(arr).` — b may BE arr (it is, here). Mut-hoisting b
    // against arr reads would panic.
    let source = r#"## To native parseInt (s: Text) -> Int

## To pick (xs: Seq of Int) -> Seq of Int:
    Return xs.

## Main
Let n be parseInt("10").
Let mutable arr be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i to arr.
    Set i to i + 1.
Let mutable b be pick(arr).
Set i to 0.
While i is less than n:
    Set item (i + 1) of b to item (i + 1) of arr + 3.
    Set i to i + 1.
Let mutable total be 0.
Set i to 0.
While i is less than n:
    Set total to total + item (i + 1) of arr.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("let mut __b_g = b.borrow_mut();"),
        "call-returned handle must not be mut-hoisted against its possible alias. Got:\n{}",
        rust
    );
    // NOTE (compiled semantics today): the readonly-slice calling convention
    // makes `Return xs` re-wrap into a FRESH Seq (`from_vec(xs.to_vec())`),
    // so in compiled code b is a copy and arr is unchanged: total = 45.
    // The alias ORACLE still treats b as may-alias (it follows reference
    // semantics), which is the conservative direction for hoisting. If the
    // calling convention ever preserves return aliasing (O4), this value
    // becomes 75 — revisit deliberately.
    common::assert_exact_output(source, "45");
}

#[test]
fn vec_hoist_pushed_and_read_in_loop() {
    // A handle read AND pushed in the same loop is VEC-hoisted: the RefMut is
    // held as a `&mut Vec` so both the index read and the push go through it
    // (one borrow_mut for the loop, not one per push). Sound because the
    // `&mut Vec` re-derefs each access — reallocation on push is fine. `arr` is
    // a Seq OF Seq (singleton inner rows), so the outer handle survives de-Rc.
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("8").
Let mutable arr be a new Seq of (Seq of Int).
Let mutable seed be a new Seq of Int.
Push 1 to seed.
Push seed to arr.
Let mutable total be 0.
Let mutable i be 0.
While i is less than n:
    Set total to total + item 1 of (item (i + 1) of arr).
    Let mutable row be a new Seq of Int.
    Push 2 to row.
    Push row to arr.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("let mut __arr_g = arr.borrow_mut();") && rust.contains("let arr = &mut *__arr_g;"),
        "read+pushed handle should be vec-hoisted as a held RefMut. Got:\n{}",
        rust
    );
    let body = last_for_body(&rust, "i");
    assert!(
        !body.contains("arr.borrow"),
        "the vec-hoisted OUTER handle must not be borrowed in the body \
         (inner singleton-row borrows are separate). Body:\n{}",
        body
    );
    common::assert_exact_output(source, "15");
}

#[test]
fn no_hoist_rebound_handle_in_loop() {
    // The loop rebinds the handle each iteration.
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
        !rust.contains("__arr_g"),
        "rebound handles must not be hoisted. Got:\n{}",
        rust
    );
    // total doubles each round after the first: 7, 14, 28, 56, 112.
    common::assert_exact_output(source, "112");
}

#[test]
fn no_hoist_body_local_handle() {
    // A handle declared INSIDE the body is fresh per iteration — the guard
    // would reference a stale or undeclared name.
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("5").
Let mutable total be 0.
Let mutable i be 0.
While i is less than n:
    Let mutable tmp be a new Seq of Int.
    Push i to tmp.
    Push i * 2 to tmp.
    Set total to total + item 2 of tmp.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("__tmp_g"),
        "body-local handles must not be hoisted. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "20");
}

#[test]
fn no_hoist_seq_of_seq_extracted_elements() {
    // Two handles extracted from the same container slot alias each other —
    // both have unknown provenance, so writing through one while reading the
    // other must stay per-access. (A wrong mut-hoist panics `already
    // borrowed` at the first read of the alias.)
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("6").
Let mutable inner be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i to inner.
    Set i to i + 1.
Let mutable nest be a new Seq of (Seq of Int).
Push inner to nest.
Let mutable x be item 1 of nest.
Let mutable y be item 1 of nest.
Set i to 0.
While i is less than n:
    Let v be item (i + 1) of y.
    Set item (i + 1) of x to v + 10.
    Set i to i + 1.
Let mutable total be 0.
Set i to 0.
While i is less than n:
    Set total to total + item (i + 1) of x.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("let mut __x_g = x.borrow_mut();"),
        "extracted element handles must not be mut-hoisted. Got:\n{}",
        rust
    );
    // x and y are the same buffer: each element becomes original + 10,
    // so total = (0+..+5) + 6*10 = 75.
    common::assert_exact_output(source, "75");
}

#[test]
fn no_hoist_whole_handle_shown() {
    // `Show arr.` inside the body borrows the whole handle in opaque code.
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("3").
Let mutable arr be a new Seq of Int.
Push 1 to arr.
Push 2 to arr.
Let mutable total be 0.
Let mutable i be 0.
While i is less than n:
    Set total to total + item 1 of arr.
    Show arr.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("__arr_g"),
        "handles passed whole to Show must not be hoisted. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "[1, 2]\n[1, 2]\n[1, 2]\n3");
}

#[test]
fn no_hoist_map_handles() {
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("5").
Let mutable m be a new Map of Int to Int.
Let mutable i be 0.
While i is less than n:
    Set item i of m to i * 2.
    Set i to i + 1.
Let mutable total be 0.
Set i to 0.
While i is less than n:
    Set total to total + item i of m.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("__m_g"),
        "Map handles must never be borrow-hoisted. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "20");
}

#[test]
fn kill_switch_disables_hoisting() {
    // The kill switch must turn the optimization off entirely. Driven via the
    // thread-local override (race-free in the parallel test runner) rather
    // than the process-global LOGOS_HOIST env var. `arr` is a Seq OF Seq
    // (singleton inner rows) so the outer handle survives de-Rc and would hoist
    // by default — the kill switch must suppress it.
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("10").
Let mutable arr be a new Seq of (Seq of Int).
Let mutable i be 0.
While i is less than n:
    Let mutable row be a new Seq of Int.
    Push i to row.
    Push row to arr.
    Set i to i + 1.
Let mutable total be 0.
Set i to 0.
While i is less than n:
    Set total to total + item 1 of (item (i + 1) of arr).
    Set i to i + 1.
Show total.
"#;
    // Sanity: hoisting is ON by default for this shape.
    assert!(
        compile_to_rust(source).unwrap().contains("__arr_g"),
        "expected hoisting on by default"
    );
    logicaffeine_compile::codegen::force_disable_borrow_hoist_for_test(true);
    let rust = compile_to_rust(source).unwrap();
    logicaffeine_compile::codegen::force_disable_borrow_hoist_for_test(false);
    assert!(
        !rust.contains("__arr_g"),
        "kill switch must disable borrow hoisting. Got:\n{}",
        rust
    );
}

// =============================================================================
// Composition with existing peephole patterns
// =============================================================================

#[test]
fn hoist_composes_with_swap_fusion() {
    // Bubble-sort compare-swap on a hoisted array: the swap must go through
    // the slice (no per-swap borrow blocks), and the result must be sorted.
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("12").
Let mutable arr be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push (n - i) * 7 % 13 to arr.
    Set i to i + 1.
Set i to 0.
While i is less than n - 1:
    Let mutable j be 1.
    While j is less than n - i:
        Let a be item j of arr.
        Let b be item (j + 1) of arr.
        If a is greater than b:
            Set item j of arr to b.
            Set item (j + 1) of arr to a.
        Set j to j + 1.
    Set i to i + 1.
Let mutable out be 0.
Set i to 0.
While i is less than n:
    Set out to out * 13 + item (i + 1) of arr.
    Set i to i + 1.
Show out.
"#;
    // Sorted [1..12] read back in base 13 (Horner).
    common::assert_exact_output(source, "2103299351334");
}

#[test]
fn hoist_composes_with_zero_based_lowering() {
    // OPT-8 zero-basing and hoisting together: raw `arr[i]` over a slice. `arr`
    // is a Seq OF Seq (singleton inner rows) so the outer handle survives de-Rc
    // as a reference-semantics `LogosSeq` and still hoists.
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("9").
Let mutable arr be a new Seq of (Seq of Int).
Let mutable k be 0.
While k is less than n:
    Let mutable row be a new Seq of Int.
    Push k * k to row.
    Push row to arr.
    Set k to k + 1.
Let mutable total be 0.
Let mutable i be 1.
While i is at most n:
    Set total to total + item 1 of (item i of arr).
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("let __arr_g = arr.borrow();"),
        "zero-based loop should still hoist. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "204");
}

#[test]
fn hoist_while_site_graph_drain_shape() {
    // BFS-drain shape: the queue grows in the body and is read in the loop
    // condition (`length of queue`) and body, so it VEC-hoists — one held
    // RefMut covers the `len`, the index reads, and the pushes. dist is
    // indexed read/written (mut-slice-hoist). What MUST hold: the drain runs
    // borrow-free, nothing double-borrows, and the output is exact.
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("12").
Let mutable queue be a new Seq of Int.
Let mutable dist be a new Seq of Int.
Let mutable k be 0.
While k is less than n:
    Push 0 - 1 to dist.
    Set k to k + 1.
Push 1 to queue.
Set item 1 of dist to 0.
Let mutable head be 1.
While head is at most length of queue:
    Let v be item head of queue.
    Let d be item v of dist.
    Let child be v * 2.
    If child is at most n:
        If item child of dist is less than 0:
            Set item child of dist to d + 1.
            Push child to queue.
    Let child2 be v * 2 + 1.
    If child2 is at most n:
        If item child2 of dist is less than 0:
            Set item child2 of dist to d + 1.
            Push child2 to queue.
    Set head to head + 1.
Let mutable total be 0.
Set k to 0.
While k is less than n:
    Set total to total + item (k + 1) of dist.
    Set k to k + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    // The growing queue is vec-hoisted (held `&mut Vec`), never as a slice
    // (a slice would dangle on push).
    assert!(
        !rust.contains("let queue = &mut __queue_g[..]") && !rust.contains("let queue = &__queue_g[..]"),
        "the growing queue must not be slice-hoisted (would dangle on push). Got:\n{}",
        rust
    );
    // Distances on the binary-heap tree of 12 nodes:
    // d(1)=0, d(2)=d(3)=1, d(4..7)=2, d(8..12)=3 → 0+2+8+15 = 25.
    common::assert_exact_output(source, "25");
}

// =============================================================================
// Hoisting inside a Zone — the oracle analyzes zone interiors for alias
// snapshots (expr facts suppressed so the JIT is unperturbed), so borrow
// hoisting fires inside zones too.
// =============================================================================

#[test]
fn hoist_inside_zone_fires() {
    // `arr` is a Seq OF Seq (singleton inner rows): de-Rc leaves the outer
    // handle a reference-semantics `LogosSeq`, so the read loop inside the Zone
    // hoists its borrow exactly once.
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("20").
Inside a new zone called "Scratch":
    Let mutable arr be a new Seq of (Seq of Int).
    Let mutable i be 0.
    While i is less than n:
        Let mutable row be a new Seq of Int.
        Push i * 2 to row.
        Push row to arr.
        Set i to i + 1.
    Let mutable total be 0.
    Set i to 0.
    While i is less than n:
        Set total to total + item 1 of (item (i + 1) of arr).
        Set i to i + 1.
    Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("let __arr_g = arr.borrow();"),
        "a read loop inside a Zone should hoist. Got:\n{}",
        rust
    );
    // sum of 2i for i=0..19 = 2 * 190 = 380
    common::assert_exact_output(source, "380");
}

#[test]
fn no_hoist_aliased_handles_inside_zone() {
    // `Let b be arr.` inside a zone aliases; a write loop through b reading
    // arr must NOT mut-hoist b (the runtime would panic `already borrowed`).
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("10").
Inside a new zone called "Scratch":
    Let mutable arr be a new Seq of Int.
    Let mutable i be 0.
    While i is less than n:
        Push i to arr.
        Set i to i + 1.
    Let mutable b be arr.
    Set i to 0.
    While i is less than n:
        Set item (i + 1) of b to item (i + 1) of arr * 2.
        Set i to i + 1.
    Let mutable total be 0.
    Set i to 0.
    While i is less than n:
        Set total to total + item (i + 1) of arr.
        Set i to i + 1.
    Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("let mut __b_g = b.borrow_mut();"),
        "aliased handle inside a Zone must not be mut-hoisted. Got:\n{}",
        rust
    );
    // b aliases arr: arr[i] doubles in place → total = 2 * 45 = 90.
    common::assert_exact_output(source, "90");
}

// =============================================================================
// OPT-TILE composition — the tiled matrix-multiply nest hoists a/b/c once
// around the whole 32-tile nest instead of per-FMA.
// =============================================================================

#[test]
fn hoist_inside_tiled_matmul() {
    // a/b/c are Seqs OF Seq (each flat matrix cell is a singleton inner row),
    // so the outer handles survive de-Rc as reference-semantics `LogosSeq`s and
    // hoist once around the whole 32-tile nest exactly as before.
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("8").
Let mutable a be a new Seq of (Seq of Int).
Let mutable b be a new Seq of (Seq of Int).
Let mutable c be a new Seq of (Seq of Int).
Let mutable i be 0.
While i is less than n * n:
    Let mutable arow be a new Seq of Int.
    Push i % 7 to arow.
    Push arow to a.
    Let mutable brow be a new Seq of Int.
    Push (i + 1) % 5 to brow.
    Push brow to b.
    Let mutable crow be a new Seq of Int.
    Push 0 to crow.
    Push crow to c.
    Set i to i + 1.
Set i to 0.
While i is less than n:
    Let mutable k be 0.
    While k is less than n:
        Let mutable j be 0.
        While j is less than n:
            Let idx be i * n + j + 1.
            Set item 1 of (item idx of c) to (item 1 of (item idx of c)) + (item 1 of (item (i * n + k + 1) of a)) * (item 1 of (item (k * n + j + 1) of b)).
            Set j to j + 1.
        Set k to k + 1.
    Set i to i + 1.
Show item 1 of (item 1 of c).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("step_by"), "tiling should still fire. Got:\n{}", rust);
    assert!(
        rust.contains("let __a_g = a.borrow();")
            && rust.contains("let __b_g = b.borrow();")
            && rust.contains("let mut __c_g = c.borrow_mut();"),
        "tile body should hoist a/b (shared) and c (mut). Got:\n{}",
        rust
    );
    // The innermost FMA must index the hoisted OUTER slices directly (the outer
    // handles a/b/c must not be re-borrowed inside the tile body — inner
    // singleton-row borrows are separate).
    let inner = last_for_body(&rust, "j");
    assert!(
        inner.contains("c[(idx - 1) as usize]")
            && !inner.contains("c.borrow_mut()[(idx")
            && !inner.contains("a.borrow()[")
            && !inner.contains("b.borrow()["),
        "tiled FMA must go through the hoisted OUTER slices, not re-borrow them. Body:\n{}\nFull:\n{}",
        inner,
        rust
    );
    // The matmul writes a Seq-of-Seq cell via a NESTED SetIndex
    // (`Set item 1 of (item idx of c) to …`). The compiler lowers this through
    // the hoisted slice, but the reference tree-walker rejects an indexed
    // SetIndex target outright ("SetIndex collection must be an identifier"), so
    // the differential gate cannot execute this exact construct. We therefore
    // pin the compiled result against its hand-computed value instead of the
    // interpreter. Tiled matmul of the two 8×8 matrices a[r]=idx%7,
    // b[r]=(idx+1)%5; c[0][0] = Σ_k a[0][k]·b[k][0] = 49.
    common::assert_exact_output(source, "49");
}

// =============================================================================
// Pure scalar builtins (sqrt/abs/min/…) don't block hoisting — they can't
// touch a Seq's RefCell. nbody's force loop is the motivating case.
// =============================================================================

#[test]
fn hoist_loop_with_sqrt_builtin() {
    // `xs` is a Seq OF Seq (singleton inner rows): de-Rc leaves the outer
    // handle a reference-semantics `LogosSeq`, so the sqrt read loop hoists its
    // borrow once. The inner singleton row is read per access; sqrt of the Int
    // element promotes into the Float accumulator exactly as before.
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("30").
Let mutable xs be a new Seq of (Seq of Int).
Let mutable i be 0.
While i is less than n:
    Let mutable row be a new Seq of Int.
    Push (i * 3 + 1) to row.
    Push row to xs.
    Set i to i + 1.
Let mutable total be 0.0.
Set i to 0.
While i is less than n:
    Set total to total + sqrt(item 1 of (item (i + 1) of xs)).
    Set i to i + 1.
Show "{total:.4}".
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("let __xs_g = xs.borrow();"),
        "a loop whose only call is sqrt should still hoist. Got:\n{}",
        rust
    );
    let body = last_for_body(&rust, "i");
    assert!(
        !body.contains("xs.borrow"),
        "the hoisted OUTER handle must not be borrowed in the sqrt loop body \
         (inner-handle borrows are separate). Body:\n{}",
        body
    );
    common::assert_compiled_equals_interpreted(source);
}

#[test]
fn no_hoist_loop_with_user_call() {
    // A non-builtin call could touch any handle — bail. `xs` is a Seq OF Seq
    // (singleton inner rows) so the outer handle survives de-Rc, but the user
    // call in the read loop still forces a per-access borrow of the outer
    // handle rather than a hoist.
    let source = r#"## To native parseInt (s: Text) -> Int

## To twice (x: Int) -> Int:
    Return x * 2.

## Main
Let n be parseInt("20").
Let mutable xs be a new Seq of (Seq of Int).
Let mutable i be 0.
While i is less than n:
    Let mutable row be a new Seq of Int.
    Push i to row.
    Push row to xs.
    Set i to i + 1.
Let mutable total be 0.
Set i to 0.
While i is less than n:
    Set total to total + twice(item 1 of (item (i + 1) of xs)).
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    // The read loop calls `twice(item … of xs)` — the call must go through a
    // per-access borrow (the read loop bails on the call), so the OUTER handle
    // is NOT hoisted as a shared read guard around that loop and the access
    // borrows `xs` inline. The separate build loop only pushes, so it
    // vec-hoists; that is fine and unrelated.
    assert!(
        !rust.contains("let __xs_g = xs.borrow();"),
        "the read loop with a user call must NOT hoist a shared read guard. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("twice(") && rust.contains("xs.borrow()["),
        "the read access inside the user call must use a per-access borrow. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "380");
}

// =============================================================================
// Vec-hoist — a handle that is only PUSHED (resized) in the loop is hoisted
// as one held RefMut (`&mut Vec`), so the per-element borrow_mut collapses to
// one borrow per loop. (knapsack's curr; any push-built collection.)
// =============================================================================

#[test]
fn vec_hoist_push_loop() {
    // `src` and `dst` are Seqs OF Seq (singleton inner rows): the inner handle
    // element type survives de-Rc, so both outer handles stay reference-
    // semantics `LogosSeq`s and `dst` (pushed-only) vec-hoists as a held RefMut.
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("100").
Let mutable src be a new Seq of (Seq of Int).
Let mutable i be 0.
While i is less than n:
    Let mutable srow be a new Seq of Int.
    Push i * 2 to srow.
    Push srow to src.
    Set i to i + 1.
Let mutable dst be a new Seq of (Seq of Int).
Set i to 0.
While i is less than n:
    Let mutable drow be a new Seq of Int.
    Push item 1 of (item (i + 1) of src) to drow.
    Push drow to dst.
    Set i to i + 1.
Show item 1 of (item n of dst).
"#;
    let rust = compile_to_rust(source).unwrap();
    // dst is push-built in the second loop → vec-hoisted (one borrow_mut).
    assert!(
        rust.contains("let mut __dst_g = dst.borrow_mut();") && rust.contains("let dst = &mut *__dst_g;"),
        "push-only handle should be vec-hoisted as a held RefMut. Got:\n{}",
        rust
    );
    // The hot loop body's OUTER pushed handle must not be borrowed (inner
    // singleton-row construction is separate).
    let body = last_for_body(&rust, "i");
    assert!(
        !body.contains("dst.borrow"),
        "push loop body must not borrow the vec-hoisted OUTER handle. Body:\n{}",
        body
    );
    common::assert_exact_output(source, "198"); // dst[n-1] = src[n-1] = (n-1)*2 = 198
}

#[test]
fn vec_hoist_read_and_push_distinct() {
    // src read (slice-hoist), dst push (vec-hoist) — distinct fresh handles.
    // Both are Seqs OF Seq (singleton inner rows), so the outer handles survive
    // de-Rc as reference-semantics `LogosSeq`s and hoist as before.
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("50").
Let mutable src be a new Seq of (Seq of Int).
Let mutable i be 0.
While i is less than n:
    Let mutable srow be a new Seq of Int.
    Push i to srow.
    Push srow to src.
    Set i to i + 1.
Let mutable dst be a new Seq of (Seq of Int).
Let mutable total be 0.
Set i to 0.
While i is less than n:
    Let mutable drow be a new Seq of Int.
    Push (item 1 of (item (i + 1) of src)) * 3 to drow.
    Push drow to dst.
    Set total to total + item 1 of (item (i + 1) of src).
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("let __src_g = src.borrow();") && rust.contains("let mut __dst_g = dst.borrow_mut();"),
        "src slice-hoisted (read), dst vec-hoisted (push). Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "1225"); // sum 0..49
}

#[test]
fn no_vec_hoist_when_handle_leaked() {
    // dst is pushed AND passed to a function — opaque, must not hoist. It is a
    // Seq OF Seq (singleton rows) so it survives de-Rc and stays a genuine
    // reference-semantics handle the call could retain.
    let source = r#"## To native parseInt (s: Text) -> Int

## To peek (xs: Seq of (Seq of Int)) -> Int:
    Return item 1 of (item 1 of xs).

## Main
Let n be parseInt("10").
Let mutable dst be a new Seq of (Seq of Int).
Let mutable acc be 0.
Let mutable i be 0.
While i is less than n:
    Let mutable row be a new Seq of Int.
    Push i to row.
    Push row to dst.
    Set acc to acc + peek(dst).
    Set i to i + 1.
Show acc.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("__dst_g"),
        "a pushed handle also passed to a call must not hoist. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "0"); // peek always returns dst[0][0]=0
}
