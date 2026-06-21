//! Symbolic element-bounds-through-memory (the graph_bfs lever).
//!
//! The concrete A2 element interval (`phase_oracle_bounds::oracle_hints_element_indexed_scatter`)
//! proves `counts[v+1]` in bounds when the divisor is a CONSTANT (`% 1000` ⟹
//! `v ∈ [0,999]`, `counts.len()==1000`). graph_bfs differs in one way that
//! breaks the concrete domain: the divisor is the RUNTIME variable `n`, so the
//! element bound is the SYMBOLIC interval `[0, n-1]` and the target length is
//! the SYMBOLIC `n` — neither representable as a concrete `Interval`. The proof
//! `u < length(dist)` is therefore relational (`u <= n-1 < n = length(dist)`)
//! and must run through the Fourier–Motzkin prover, fed by a symbolic element
//! upper bound that flows store → array → read exactly as the concrete one does.
//!
//! `n` comes from `args()` so it is genuinely symbolic (a `parseInt("100")`
//! literal would constant-fold to the concrete case and never exercise this).

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::compile_to_rust;

/// `adj` is filled DIRECTLY by `Push (...) % n` (no zero-init), so every element
/// is in `[0, n-1]`; `dist` is a counted build of length `n`. Reading `u` from
/// `adj` carries the symbolic element bound, so `dist[u+1]` is provably in
/// bounds (`u <= n-1 < n = length(dist)`). The read must get an
/// `assert_unchecked` hint against `dist.len()` — the symbolic element bound
/// reached the index through the variable-divisor modulo. This is the core
/// mechanism, isolated from the zero-init join below.
#[test]
fn symbolic_element_bound_direct_modulo_fill() {
    let source = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int
## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable adj be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push (i * 31 + 7) % n to adj.
    Set i to i + 1.
Let mutable dist be a new Seq of Int.
Set i to 0.
While i is less than n:
    Push 0 - 1 to dist.
    Set i to i + 1.
Let mutable total be 0.
Set i to 1.
While i is at most n:
    Let u be item i of adj.
    Set total to total + item (u + 1) of dist.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("std::hint::assert_unchecked(") && rust.contains("dist.len() as i64"),
        "the element-indexed read `dist[u+1]` (u an element of a `% n`-filled \
         array) must get a symbolic bounds hint. Got:\n{}",
        rust
    );
}

/// The graph_bfs shape: `adj` is ZERO-INITIALISED by a counted build loop, then
/// SOME slots overwritten by `Set item _ of adj to (...) % n`. So `adj`'s
/// elements are `{0, (...) % n}`. The symbolic element upper bound is `n-1`,
/// which covers the zero-init slots too (`0 <= n-1` holds whenever the array is
/// non-empty, i.e. `n >= 1` from the build-loop guard; the array is empty and
/// the bound vacuous otherwise). The join of the constant `0` element and the
/// symbolic `n-1` element must resolve to `n-1`, so `dist[u+1]` still hints.
#[test]
fn symbolic_element_bound_zero_init_then_scatter() {
    let source = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int
## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable adj be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push 0 to adj.
    Set i to i + 1.
Set i to 0.
While i is less than n:
    Let neighbor be (i * 31 + 7) % n.
    Set item (i + 1) of adj to neighbor.
    Set i to i + 1.
Let mutable dist be a new Seq of Int.
Set i to 0.
While i is less than n:
    Push 0 - 1 to dist.
    Set i to i + 1.
Let mutable total be 0.
Set i to 1.
While i is at most n:
    Let u be item i of adj.
    Set total to total + item (u + 1) of dist.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("std::hint::assert_unchecked(") && rust.contains("dist.len() as i64"),
        "the element-indexed read `dist[u+1]` (u an element of a zero-init + \
         `% n`-scattered array) must get a symbolic bounds hint. Got:\n{}",
        rust
    );
}

/// The full graph_bfs shape: a zero-init array scattered by `% n` inside a
/// NESTED loop (`while p<=5: while i<n: ... Set item _ of adj to (...) % n`).
/// `n >= 1` is established only in the inner loop, then lost at the outer
/// loop's exit join — so the proof leans on the LENIENT symbolic element join
/// (keep `n-1` over the constant zero-init) plus a POSITIVITY GUARD: the
/// `dist[u+1]` read must get BOTH the `assert_unchecked` hint AND a hoisted
/// `assert!(n > 0)` that discharges the `n >= 1` precondition (so `m <= 0`
/// panics at the would-be out-of-bounds access instead of the elision being UB).
#[test]
fn symbolic_element_bound_nested_scatter_emits_positivity_guard() {
    let source = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int
## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable adj be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push 0 to adj.
    Set i to i + 1.
Let mutable p be 1.
While p is at most 5:
    Set i to 0.
    While i is less than n:
        Let neighbor be (i * 31 + p) % n.
        Set item (i + 1) of adj to neighbor.
        Set i to i + 1.
    Set p to p + 1.
Let mutable dist be a new Seq of Int.
Set i to 0.
While i is less than n:
    Push 0 - 1 to dist.
    Set i to i + 1.
Let mutable total be 0.
Set i to 1.
While i is at most n:
    Let u be item i of adj.
    Set total to total + item (u + 1) of dist.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("std::hint::assert_unchecked(") && rust.contains("dist.len() as i64"),
        "the element-indexed read `dist[u+1]` (nested `% n` scatter) must get a \
         symbolic bounds hint. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("LOGOS positivity guard") && rust.contains("(n) > 0"),
        "the nested-scatter elision must emit a hoisted `assert!(n > 0)` \
         positivity guard discharging the `% n` element bound's precondition. \
         Got:\n{}",
        rust
    );
}

/// graph_bfs's parallel-array shape: a SINGLE counted loop fills MULTIPLE arrays
/// (`Push slots; Push keys` per iteration). Each array pushed exactly once gets
/// `length(_) = trip count = n`; indexing `slots` (length `n`) by an element of
/// `keys` (`< n`, a `% n` fill) is then in bounds. Pins the multi-array build-
/// length inference — without it neither array gets a length fact and the
/// element-indexed read stays bounds-checked.
#[test]
fn symbolic_element_bound_multi_array_build_loop() {
    let source = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int
## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable slots be a new Seq of Int.
Let mutable keys be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push 0 to slots.
    Push (i * 7) % n to keys.
    Set i to i + 1.
Let mutable total be 0.
Set i to 1.
While i is at most n:
    Let v be item i of keys.
    Set total to total + item (v + 1) of slots.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("std::hint::assert_unchecked(") && rust.contains("slots.len() as i64"),
        "indexing `slots` (a co-built parallel array of length n) by an element \
         of a `% n`-filled array must get a symbolic bounds hint — the multi-\
         array build-length inference must establish `length(slots) = n`. \
         Got:\n{}",
        rust
    );
}
