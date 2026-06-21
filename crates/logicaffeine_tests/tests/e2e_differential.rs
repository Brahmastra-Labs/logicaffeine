//! DIFFERENTIAL-CORRECTNESS GATE
//!
//! Every program here is run BOTH compiled (codegen -> rustc -> execute) and
//! interpreted (VM + tree-walker), and the two outcomes must be identical. The
//! interpreter is the reference for LOGOS reference semantics (and is itself
//! cross-checked VM-vs-tree-walker by the debug shadow oracle); codegen must
//! match it. This is the invariant that catches the entire class of bug where a
//! codegen optimization silently changes a program's meaning — exactly how the
//! unsound double-buffer swap (`Set X to Y` -> `mem::swap`) and the aliased
//! `SetIndex` double-borrow slipped through "passing against C" while diverging
//! from the interpreter.
//!
//! When adding an optimization, add a program here that exercises it under
//! aliasing — if compiled and interpreted disagree, the optimization is unsound.

mod common;

use common::{assert_compiled_equals_interpreted, assert_compiled_equals_interpreted_eq};

// ---------------------------------------------------------------------------
// The knapsack bug class: a bare `Set prev to curr` double-buffer with a
// CROSS-INDEX read (`item w of prev` while writing `item (w+1) of curr`). The
// removed `detect_double_buffer_swap` turned this into a swap (distinct
// buffers) while the interpreter aliased — different answers. Now both alias;
// they must agree.
// ---------------------------------------------------------------------------
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn diff_cross_index_double_buffer_alias() {
    assert_compiled_equals_interpreted(
        r#"## Main
Let mutable prev be a new Seq of Int.
Let mutable j be 0.
While j is less than 5:
    Push 0 to prev.
    Set j to j + 1.
Let mutable curr be a new Seq of Int.
Set j to 0.
While j is less than 5:
    Push 0 to curr.
    Set j to j + 1.
Let mutable i be 0.
While i is less than 3:
    Let mutable w be 1.
    While w is at most 4:
        Set item (w + 1) of curr to item (w + 1) of prev.
        Let take be item w of prev + 1.
        If take is greater than item (w + 1) of curr:
            Set item (w + 1) of curr to take.
        Set w to w + 1.
    Set prev to curr.
    Set i to i + 1.
Show item 5 of prev.
"#,
    );
}

// ---------------------------------------------------------------------------
// The aliased-SetIndex double-borrow class: after `Set prev to curr` they are
// the SAME RefCell, so `curr.borrow_mut()[w] = prev.borrow()[w] + 1` used to
// panic in compiled code while the interpreter returned 3. SAME-index, so the
// value is well-defined: both must produce 3.
// ---------------------------------------------------------------------------
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn diff_same_index_aliased_setindex() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let capacity be 4.
Let cols be capacity + 1.
Let mutable prev be a new Seq of Int.
Let mutable curr be a new Seq of Int.
Let mutable j be 0.
While j is less than cols:
    Push 0 to prev.
    Push 0 to curr.
    Set j to j + 1.
Let mutable i be 0.
While i is less than 3:
    Let mutable w be 0.
    While w is at most capacity:
        Set item (w + 1) of curr to item (w + 1) of prev + 1.
        Set w to w + 1.
    Set prev to curr.
    Set i to i + 1.
Show item (capacity + 1) of prev.
"#,
        "3",
    );
}

// ---------------------------------------------------------------------------
// Reference semantics: `Let b be a` aliases — a push through b is visible via a.
// ---------------------------------------------------------------------------
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn diff_let_binding_aliases() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let mutable a be a new Seq of Int.
Push 1 to a.
Let mutable b be a.
Push 2 to b.
Show length of a.
"#,
        "2",
    );
}

// ---------------------------------------------------------------------------
// `copy of` makes an independent deep copy — mutating the copy must not touch
// the original. Both engines must agree (1 then 2).
// ---------------------------------------------------------------------------
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn diff_copy_of_isolates() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let mutable a be a new Seq of Int.
Push 1 to a.
Let mutable b be copy of a.
Push 2 to b.
Show length of a.
Show length of b.
"#,
        "1\n2",
    );
}

// ---------------------------------------------------------------------------
// The SOUND double-buffer idiom: `curr` is allocated FRESH (`new Seq`) each
// iteration (built via Push, reading only prev). Correct 0/1 knapsack on both
// engines, AND the compiler may legally reuse the buffer (mem::swap + clear).
// ---------------------------------------------------------------------------
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn diff_fresh_buffer_knapsack() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let n be 5.
Let capacity be n * 5.
Let mutable weights be a new Seq of Int.
Let mutable vals be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push (i * 17 + 3) % 50 + 1 to weights.
    Push (i * 31 + 7) % 100 + 1 to vals.
    Set i to i + 1.
Let cols be capacity + 1.
Let mutable prev be a new Seq of Int.
Set i to 0.
While i is less than cols:
    Push 0 to prev.
    Set i to i + 1.
Set i to 0.
While i is less than n:
    Let mutable curr be a new Seq of Int.
    Let wi be item (i + 1) of weights.
    Let vi be item (i + 1) of vals.
    Let mutable w be 0.
    While w is at most capacity:
        Let mutable best be item (w + 1) of prev.
        If w is at least wi:
            Let take be item (w - wi + 1) of prev + vi.
            If take is greater than best:
                Set best to take.
        Push best to curr.
        Set w to w + 1.
    Set prev to curr.
    Set i to i + 1.
Show item (capacity + 1) of prev.
"#,
        "47",
    );
}

// ---------------------------------------------------------------------------
// Aliased Map SetIndex with a cross-key read: same RefCell-aliasing hazard as
// Seqs. Both engines must agree.
// ---------------------------------------------------------------------------
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn diff_map_alias_setindex() {
    assert_compiled_equals_interpreted(
        r#"## Main
Let mutable a be a new Map of Int to Int.
Set item 1 of a to 10.
Set item 2 of a to 20.
Let mutable b be a.
Set item 1 of b to item 2 of a.
Show item 1 of a.
"#,
    );
}
