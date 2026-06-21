//! O5 — the AOT codegen consumes the bounds-elision oracle. The oracle proves
//! `item i of arr` reads in bounds when the loop guard is `i <= length(arr)`
//! (or a length-bound var) and the array is not shrunk — including arrays that
//! GROW in the loop (graph_bfs's queue), which the for-range counter hints
//! (O5a) cannot reach. Codegen must emit an `assert_unchecked` so LLVM drops
//! the bounds check, paired with a `debug_assert!` that panics loudly in debug
//! if the proof is ever unsound.

mod common;

use common::compile_to_rust;

/// counting_sort's scatter: `v = item i of arr` where `arr` is filled with
/// `(seed/65536) % 1000` (a TRUNCATED modulo of a provably-non-negative LCG),
/// so every element is in `[0, 999]`; `counts` has exactly 1000 elements. The
/// element interval flows through the read to bound `v`, so `counts[v+1]` is
/// provably in bounds — value-range analysis carried THROUGH MEMORY (A1
/// modulo/div intervals + A2 element bounds), which no per-loop counter or
/// single-variable relational proof can reach.
#[test]
fn oracle_hints_element_indexed_scatter() {
    let source = r#"## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt("100").
Let mutable arr be a new Seq of Int.
Let mutable seed be 42.
Let mutable i be 0.
While i is less than n:
    Set seed to (seed * 1103515245 + 12345) % 2147483648.
    Push (seed / 65536) % 1000 to arr.
    Set i to i + 1.
Let mutable counts be a new Seq of Int.
Set i to 0.
While i is less than 1000:
    Push 0 to counts.
    Set i to i + 1.
Let mutable total be 0.
Set i to 1.
While i is at most n:
    Let v be item i of arr.
    Set total to total + item (v + 1) of counts.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    // `counts` is indexed ONLY by `v + 1` where `v ∈ [0, 999]` is an element of
    // `arr`; counts.len() == 1000. The scatter read must get an assert_unchecked
    // hint — proof that the element bound reached the index.
    assert!(
        rust.contains("std::hint::assert_unchecked(") && rust.contains("counts.len() as i64"),
        "the element-indexed scatter `counts[v+1]` must get a bounds hint \
         (A1 modulo + A2 element-interval analysis). Got:\n{}",
        rust
    );
}

/// Row-major 2D BCE-hoist (the matrix_mult shape, non-tiled here). `c[i*n+j]`
/// is a BILINEAR index (the `i*n` product defeats the linear Fourier–Motzkin
/// prover), but the access is UNCONDITIONAL and the index is MONOTONIC in the
/// loop counters `i, j ∈ [0, n)` on a NON-RESIZED array. The oracle records it
/// for hoisting: codegen emits a preheader `assert!(maxIndex < len && ...)`
/// (one runtime check, guarded by loop-nonemptiness so it never spuriously
/// panics) which makes the per-iteration `assert_unchecked` SOUND with no
/// static nonlinear proof and no UB. This is the general 2D/grid/DP-table lever.
///
/// AOT BCE-hoist (task #21). The inner-loop offset `i*n` is loop-invariant, so
/// `c[i*n+j]` is monotone in `j`; codegen emits a preheader `assert!(maxIdx <
/// len)` + a per-iteration `assert_unchecked`, eliding the bounds check soundly
/// (the hard `assert!` panics, never UB, on an out-of-range program). Worth
/// ~11% on matrix_mult (hand-elision experiment: 0.61s→0.54s, bit-identical).
#[test]
fn oracle_hoists_row_major_2d_index() {
    let source = r#"## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt("8").
Let mutable c be a new Seq of Int.
Let mutable i be 0.
While i is less than n * n:
    Push 0 to c.
    Set i to i + 1.
Set i to 0.
While i is less than n:
    Let mutable j be 0.
    While j is less than n:
        Set item (i * n + j + 1) of c to i * n + j.
        Set j to j + 1.
    Set i to i + 1.
Let mutable sum be 0.
Set i to 0.
While i is less than n * n:
    Set sum to sum + item (i + 1) of c.
    Set i to i + 1.
Show sum.
"#;
    let rust = compile_to_rust(source).unwrap();
    // The bilinear write `c[i*n+j]` must get a body bounds hint...
    assert!(
        rust.contains("std::hint::assert_unchecked(") && rust.contains("c.len() as i64"),
        "the bilinear row-major write `c[i*n+j]` must get an assert_unchecked hint. Got:\n{}",
        rust
    );
    // ...justified by a hoisted preheader runtime guard (a hard `assert!`, so an
    // OOB program panics rather than hits UB — the soundness anchor).
    assert!(
        rust.contains("assert!(") && rust.contains("LOGOS bounds guard"),
        "a preheader hoisted bounds guard `assert!(...)` must anchor the elision. Got:\n{}",
        rust
    );
    // sum of c[0..n*n] = sum of 0..63 = 63*64/2 = 2016.
    common::assert_exact_output(source, "2016");
}

/// knapsack's DP scan — the hardest single access in the suite. `prev[w-wi+1]`
/// is indexed by a TWO-variable affine expression under a path guard, into an
/// array whose length is a SYMBOLIC variable: provable only by relating the
/// guard `w >= wi`, the loop bound `w <= capacity`, the element bound
/// `wi ∈ [1,50]` (reached through `Let wi be item _ of weights`, A2), and the
/// scalar definition `length(prev) = cols = capacity + 1` — a multi-variable
/// linear-arithmetic proof discharged by the kernel's Fourier–Motzkin engine.
#[test]
fn oracle_hints_knapsack_data_dependent_index() {
    let source = r#"## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt("100").
Let capacity be n * 5.
Let mutable weights be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push (i * 17 + 3) % 50 + 1 to weights.
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
    Let mutable w be 0.
    While w is at most capacity:
        Let mutable best be item (w + 1) of prev.
        If w is at least wi:
            Let take be item (w - wi + 1) of prev.
            If take is greater than best:
                Set best to take.
        Push best to curr.
        Set w to w + 1.
    Set prev to curr.
    Set i to i + 1.
Show item (capacity + 1) of prev.
"#;
    let rust = compile_to_rust(source).unwrap();
    // The data-dependent read `prev[w-wi+1]` (0-based `w - wi`) must get a
    // bounds hint — the multi-variable LIA proof fired. The `>= 0` lower half
    // is emitted only by the oracle hint, never by the access itself.
    assert!(
        rust.contains("std::hint::assert_unchecked(((w - wi)) >= 0")
            && rust.contains("((w - wi)) < (prev.len() as i64)"),
        "knapsack's `prev[w-wi+1]` must get a multi-variable bounds hint \
         (kernel LIA: path guard + scalar def + element bound). Got:\n{}",
        rust
    );
}

/// string_search's window scan — a NESTED-loop, TWO-variable affine index into
/// a string whose length is bound to a variable. `text[i+j]` is provable from
/// the outer guard `i <= textLen - needleLen + 1`, the inner `j < needleLen`,
/// the monotone IV lowers (`i >= 1`, `j >= 0` — the latter surviving the
/// `Set j to needleLen` break), and the A3 length binding
/// `length(text) = textLen` (from `Let textLen be length of text`). The kernel
/// LIA proves `i + j ∈ [1, textLen]` = text's exact valid range.
#[test]
fn oracle_hints_string_search_window() {
    let source = r#"## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt("200").
Let mutable text be "".
Let mutable pos be 0.
While pos is less than n:
    Set text to text + "a".
    Set pos to pos + 1.
Let needle be "abc".
Let needleLen be 3.
Let textLen be length of text.
Let mutable last be 0.
Let mutable i be 1.
While i is at most textLen - needleLen + 1:
    Let mutable matched be 1.
    Let mutable j be 0.
    While j is less than needleLen:
        If item (i + j) of text is not item (j + 1) of needle:
            Set matched to 0.
            Set j to needleLen.
        Set j to j + 1.
    If matched is equal to 1:
        Set last to i.
    Set i to i + 1.
Show last.
"#;
    // NOTE: this records the last match POSITION (not an occurrence count), so
    // the naive-search SIMD kernel (peephole::try_emit_naive_search) declines and
    // the window `text[i+j]` is still emitted — exactly where the oracle's nested
    // bounds proof must fire. (A full count-nest is subsumed by the kernel; see
    // phase_strsearch.)
    let rust = compile_to_rust(source).unwrap();
    // `text[i+j]` (0-based `i + j - 1`) must get a bounds hint — the nested
    // multi-variable LIA proof using the A3 length binding fired.
    assert!(
        rust.contains("std::hint::assert_unchecked") && rust.contains("(text.len() as i64)"),
        "string_search's `text[i+j]` must get a nested multi-variable bounds \
         hint (A3 length binding + kernel LIA). Got:\n{}",
        rust
    );
}

/// The graph_bfs queue shape: a growing FIFO read by a cursor bounded by its
/// own length. Not a for-range (the bound changes), so only the relational
/// oracle proof reaches it.
#[test]
fn oracle_hints_growing_queue_cursor_read() {
    let source = r#"## Main
Let mutable q be a new Seq of Int.
Push 1 to q.
Let mutable sum be 0.
Let mutable front be 1.
While front is at most length of q:
    Let v be item front of q.
    Set sum to sum + v.
    If v is less than 10:
        Push v + 1 to q.
    Set front to front + 1.
Show sum.
"#;
    let rust = compile_to_rust(source).unwrap();
    // `q` is GROW-ONLY (pushed, never popped/aliased), so `length(q)` only
    // increases and the guard `front <= length(q)` keeps holding — the read is
    // provably in bounds and must get the assert_unchecked hint.
    assert!(
        rust.contains("std::hint::assert_unchecked(")
            && rust.contains("(front - 1)) < (q.len() as i64)"),
        "the grow-only queue cursor read must get an assert_unchecked bounds \
         hint. Got:\n{}",
        rust
    );
    // q grows to [1,2,3,4,5,6,7,8,9,10]; sum = 55.
    common::assert_exact_output(source, "55");
}

/// A fixed array iterated by `while i <= length(arr)` (not for-range-converted)
/// — the oracle proves the read; the program must stay correct.
#[test]
fn oracle_hints_fixed_array_while_index() {
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("50").
Let mutable arr be a new Seq of Int.
Let mutable k be 0.
While k is less than n:
    Push k * 3 to arr.
    Set k to k + 1.
Let mutable total be 0.
Let mutable i be 1.
While i is at most length of arr:
    Set total to total + item i of arr.
    Set i to i + 1.
Show total.
"#;
    // 3*(0+..+49) = 3*1225 = 3675.
    common::assert_exact_output(source, "3675");
}
