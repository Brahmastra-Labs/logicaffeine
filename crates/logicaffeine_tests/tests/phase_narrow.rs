//! i64 → i32 element-width narrowing (on by default; `LOGOS_NO_NARROW` disables).
//!
//! A `Seq of Int` whose every written value provably fits `i32` is stored as
//! `Vec<i32>` — half the footprint and cache pressure. Loads sign-extend, stores
//! truncate, both lossless by the proof. Narrowing fires only for the three
//! sound sources (constant in range, `% m` with a runtime guard, accumulator
//! bounded by enclosing constant loops); anything else stays `Vec<i64>`.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{assert_exact_output, compile_to_rust};

/// Accumulator: `counts[i]` incremented once per pass of a constant 3-pass loop
/// ⟹ bounded by `[0, 3] ⊆ i32` ⟹ `Vec<i32>`, no runtime guard.
const ACCUM: &str = r#"## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt("4").
Let mutable counts be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push 0 to counts.
    Set i to i + 1.
Let mutable p be 1.
While p is at most 3:
    Set i to 0.
    While i is less than n:
        Let cnt be item (i + 1) of counts.
        Set item (i + 1) of counts to cnt + 1.
        Set i to i + 1.
    Set p to p + 1.
Let mutable total be 0.
Set i to 0.
While i is less than n:
    Set total to total + item (i + 1) of counts.
    Set i to i + 1.
Show total.
"#;

#[test]
fn accumulator_bounded_by_const_loop_narrows() {
    let rust = compile_to_rust(ACCUM).unwrap();
    assert!(
        rust.contains("counts: Vec<i32>") || rust.contains("counts:Vec<i32>"),
        "an accumulator bounded by a constant loop count must narrow to Vec<i32>. Got:\n{}",
        rust
    );
    // 4 slots, each incremented 3× ⟹ total 12. The truncating stores must be lossless.
    assert_exact_output(ACCUM, "12");
}

/// `% m` element source: `r[i] = i % n` ⟹ `|i % n| < n`, narrowed under the
/// runtime guard `0 < n <= i32::MAX`.
const MODN: &str = r#"## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt("7").
Let mutable r be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i % n to r.
    Set i to i + 1.
Let mutable total be 0.
Set i to 0.
While i is less than n:
    Set total to total + item (i + 1) of r.
    Set i to i + 1.
Show total.
"#;

#[test]
fn mod_n_element_narrows_with_guard() {
    let rust = compile_to_rust(MODN).unwrap();
    assert!(
        rust.contains("r: Vec<i32>") || rust.contains("r:Vec<i32>"),
        "a `% n`-filled sequence must narrow to Vec<i32>. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("LOGOS i32-narrowing guard"),
        "a variable divisor must emit a runtime `n <= i32::MAX` guard. Got:\n{}",
        rust
    );
    // sum_{i=0}^{6} (i % 7) = 0+1+..+6 = 21.
    assert_exact_output(MODN, "21");
}

/// An out-of-i32-range constant store disqualifies the sequence — stays Vec<i64>.
#[test]
fn out_of_range_constant_stays_i64() {
    let src = r#"## Main
Let mutable big be a new Seq of Int.
Push 5000000000 to big.
Show item 1 of big.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(
        !rust.contains("big: Vec<i32>") && !rust.contains("big:Vec<i32>"),
        "a value above i32::MAX must NOT narrow. Got:\n{}",
        rust
    );
    assert_exact_output(src, "5000000000");
}

/// A value with no provable i32 bound (`n * n`) keeps the sequence as Vec<i64>.
#[test]
fn unprovable_value_stays_i64() {
    let src = r#"## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt("100").
Let mutable xs be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push n * n to xs.
    Set i to i + 1.
Let mutable total be 0.
Set i to 0.
While i is less than n:
    Set total to total + item (i + 1) of xs.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(
        !rust.contains("xs: Vec<i32>") && !rust.contains("xs:Vec<i32>"),
        "`n * n` (unbounded) must NOT narrow. Got:\n{}",
        rust
    );
    // 100 * (100*100) = 1_000_000.
    assert_exact_output(src, "1000000");
}

/// An array passed to a function (which expects `Seq of Int` = i64) escapes —
/// reinterpreting it as `Vec<i32>` would be a type mismatch, so it must NOT narrow.
#[test]
fn escaping_array_not_narrowed() {
    let src = r#"## To native parseInt (s: Text) -> Int
## To sumOf (xs: Seq of Int) -> Int:
    Return item 1 of xs.

## Main
Let n be parseInt("8").
Let mutable arr be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i % n to arr.
    Set i to i + 1.
Show sumOf(arr).
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(
        !rust.contains("arr: Vec<i32>") && !rust.contains("arr:Vec<i32>"),
        "an array passed to a function must NOT narrow. Got:\n{}",
        rust
    );
    assert_exact_output(src, "0");
}

/// A loop counter is loop-carried, not its pre-loop value — `Push i to arr` must
/// not be misread as a constant. (Conservatively stays Vec<i64>; output correct.)
#[test]
fn loop_counter_fill_not_misclassified() {
    let src = r#"## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt("5").
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
Show total.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(
        !rust.contains("arr: Vec<i32>") && !rust.contains("arr:Vec<i32>"),
        "a loop-counter fill must not be misread as constant and narrowed. Got:\n{}",
        rust
    );
    // 0+1+2+3+4 = 10.
    assert_exact_output(src, "10");
}

/// End-to-end on graph_bfs: `adj` (`% n`) and `adjCounts` (accumulator ≤ 5)
/// narrow to Vec<i32>, while `dist` (semantic-invariant bound) stays Vec<i64>.
#[test]
fn graph_bfs_narrows_adj_and_adjcounts_not_dist() {
    const GRAPH_BFS: &str = include_str!("../../../benchmarks/programs/graph_bfs/main.lg");
    let rust = compile_to_rust(GRAPH_BFS).unwrap();
    assert!(
        rust.contains("adj: Vec<i32>"),
        "graph_bfs `adj` (% n elements) must narrow to Vec<i32>. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("adjCounts: Vec<i32>"),
        "graph_bfs `adjCounts` (accumulator ≤ 5) must narrow to Vec<i32>. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("dist: Vec<i64>") || rust.contains("dist: Vec < i64"),
        "graph_bfs `dist` (semantic-invariant bound) must STAY Vec<i64>. Got:\n{}",
        rust
    );
}
