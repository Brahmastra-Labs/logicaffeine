//! Affine read-only array scalarization.
//!
//! An array `A` built as `push f(i) to A` in a counted loop — where `f(i)` is an
//! AFFINE function of the loop induction variable (`i*5`, `i*3 + 7`, a constant)
//! — and never mutated afterward, holds `A[p] = f(p)` for every position `p`. A
//! read `item k of A` is therefore the pure arithmetic `f(k-1)`, so codegen
//! deletes the array and substitutes the closed form. This turns a CSR-style
//! offset array (graph_bfs's `adjStarts[v] == v*5`) — a 24 MB random-access load
//! per dequeue — into C's `v * 5` shift. Anything outside that exact shape stays
//! an ordinary `Vec`: the pass only ever removes an array whose every value it
//! can reproduce.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{assert_exact_output, compile_to_rust};

/// A minimal CSR offset array: `starts[i] == i*5`, read-only after the build,
/// summed afterward. The array must be deleted and `item (v+1) of starts` must
/// become `v * 5` arithmetic.
const CSR: &str = r#"## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt("100").
Let mutable starts be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i * 5 to starts.
    Set i to i + 1.
Let mutable v be 0.
Let mutable acc be 0.
While v is less than n:
    Set acc to acc + item (v + 1) of starts.
    Set v to v + 1.
Show acc.
"#;

#[test]
fn affine_offset_array_is_deleted() {
    let rust = compile_to_rust(CSR).unwrap();
    assert!(
        !rust.contains("starts"),
        "the affine offset array must be deleted (no decl, push, or index). Got:\n{}",
        rust
    );
}

#[test]
fn affine_offset_array_reads_become_arithmetic() {
    let rust = compile_to_rust(CSR).unwrap();
    assert!(
        rust.contains("* 5"),
        "a read `item (v+1) of starts` must become `v * 5` arithmetic. Got:\n{}",
        rust
    );
}

/// The load-bearing correctness gate: the substituted arithmetic must produce
/// the identical result. sum_{v=0}^{99} 5*v = 5 * 4950 = 24750.
#[test]
fn affine_offset_array_preserves_semantics() {
    assert_exact_output(CSR, "24750");
}

/// An affine value WITH an offset (`i*3 + 7`), read via `item`. The array is
/// deleted and reads become `3*v + 7`. sum_{v=0}^{9} (3*v + 7) = 3*45 + 70 = 205.
/// (The drain bound is `n`, not `length of starts`: affine only fires on
/// item-read-only arrays — a `length of` use makes it decline, since the
/// for-range loop-bound path renders `.len()` outside our rewrite.)
const OFFSET: &str = r#"## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt("10").
Let mutable starts be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i * 3 + 7 to starts.
    Set i to i + 1.
Let mutable v be 0.
Let mutable acc be 0.
While v is less than n:
    Set acc to acc + item (v + 1) of starts.
    Set v to v + 1.
Show acc.
"#;

#[test]
fn affine_with_offset() {
    let rust = compile_to_rust(OFFSET).unwrap();
    assert!(
        !rust.contains("starts"),
        "the affine array (with offset) must be deleted. Got:\n{}",
        rust
    );
    assert_exact_output(OFFSET, "205");
}

/// A `length of A` use makes affine DECLINE (the for-range loop-bound path
/// renders `.len()` outside the rewrite, so deleting `A` would dangle it). The
/// array stays a `Vec` and the program is correct.
#[test]
fn affine_with_length_use_declines() {
    let src = r#"## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt("10").
Let mutable starts be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i * 3 to starts.
    Set i to i + 1.
Let mutable v be 1.
Let mutable acc be 0.
While v is at most length of starts:
    Set acc to acc + item v of starts.
    Set v to v + 1.
Show acc.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(rust.contains("starts"), "an array read via `length of` must stay a Vec. Got:\n{}", rust);
    // v=1..10 reads item v (1-based, starts[v-1]=3*(v-1)) → 3*(0..9) = 135.
    assert_exact_output(src, "135");
}

// ---------------------------------------------------------------------------
// Adversarial disqualifiers. Each must KEEP an ordinary `Vec` (the array name
// survives) and stay semantically correct — the pass must not misfire.
// ---------------------------------------------------------------------------

/// An in-place write after the build breaks `A[p] == f(p)` — must not scalarize.
/// element 0 becomes 99: 99 + 5*(1+..+9) = 99 + 225 = 324 (a wrong scalarization
/// that ignored the store would give 225).
#[test]
fn setindex_after_build_disqualifies() {
    let src = r#"## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt("10").
Let mutable starts be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i * 5 to starts.
    Set i to i + 1.
Set item 1 of starts to 99.
Let mutable v be 0.
Let mutable acc be 0.
While v is less than n:
    Set acc to acc + item (v + 1) of starts.
    Set v to v + 1.
Show acc.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(rust.contains("starts"), "a mutated array must stay a Vec. Got:\n{}", rust);
    assert_exact_output(src, "324");
}

/// A conditional push breaks `position == iteration` — must not scalarize.
#[test]
fn conditional_push_disqualifies() {
    let src = r#"## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt("10").
Let mutable starts be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    If i is less than 5:
        Push i * 5 to starts.
    Set i to i + 1.
Let mutable v be 0.
Let mutable acc be 0.
While v is less than length of starts:
    Set acc to acc + item (v + 1) of starts.
    Set v to v + 1.
Show acc.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(rust.contains("starts"), "a conditionally-built array must stay a Vec. Got:\n{}", rust);
    // i=0..4 push 0,5,10,15,20 → sum 50.
    assert_exact_output(src, "50");
}

/// A non-affine push value (`i * i`) has no closed affine form — must not scalarize.
#[test]
fn non_affine_value_disqualifies() {
    let src = r#"## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt("5").
Let mutable starts be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i * i to starts.
    Set i to i + 1.
Let mutable v be 0.
Let mutable acc be 0.
While v is less than n:
    Set acc to acc + item (v + 1) of starts.
    Set v to v + 1.
Show acc.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(rust.contains("starts"), "a non-affine array must stay a Vec. Got:\n{}", rust);
    // 0+1+4+9+16 = 30.
    assert_exact_output(src, "30");
}

/// An induction step of +2 makes `position != iteration` (`A[p] = f(2p)`) — the
/// naive `f(p)` substitution would be wrong, so it must not scalarize. The loop
/// pushes at i=0,2,4,6,8 → A = [0,10,20,30,40]; sum = 100 (a wrong `5*v` would be 50).
#[test]
fn increment_by_two_disqualifies() {
    let src = r#"## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt("10").
Let mutable starts be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i * 5 to starts.
    Set i to i + 2.
Let mutable v be 0.
Let mutable acc be 0.
While v is less than length of starts:
    Set acc to acc + item (v + 1) of starts.
    Set v to v + 1.
Show acc.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(rust.contains("starts"), "a step-2 build must stay a Vec. Got:\n{}", rust);
    assert_exact_output(src, "100");
}

/// A bare reference (aliasing the array into another binding) escapes it — the
/// alias could observe the materialized array, so it must not be deleted.
#[test]
fn aliased_array_disqualifies() {
    let src = r#"## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt("10").
Let mutable starts be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i * 5 to starts.
    Set i to i + 1.
Let other be starts.
Show length of other.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(rust.contains("starts"), "an aliased array must stay a Vec. Got:\n{}", rust);
    assert_exact_output(src, "10");
}

/// End-to-end on the real graph_bfs benchmark: its CSR offset array `adjStarts`
/// (`adjStarts[i] == i*5`, read random-access per dequeued vertex) must be
/// deleted and every `item (v+1) of adjStarts` become C's `v * 5` shift.
#[test]
fn graph_bfs_adjstarts_is_eliminated() {
    const GRAPH_BFS: &str = include_str!("../../../benchmarks/programs/graph_bfs/main.lg");
    let rust = compile_to_rust(GRAPH_BFS).unwrap();
    assert!(
        !rust.contains("adjStarts"),
        "graph_bfs `adjStarts` (the CSR offset array) must be deleted — no decl, push, or load. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("* 5"),
        "graph_bfs `item (v+1) of adjStarts` must become `v * 5` arithmetic. Got:\n{}",
        rust
    );
}

// ---------------------------------------------------------------------------
// Phase 2: a Seq pushed k>1 times per loop iteration needs `k * N` capacity, not
// `N` — under-sizing forces a chain of reallocations C avoids with one malloc.
// ---------------------------------------------------------------------------

/// Three pushes per iteration → capacity scaled by 3 (and the array is NOT
/// affine-deleted, since affine scalarization requires exactly one push).
#[test]
fn multi_push_per_iter_scales_capacity() {
    let src = r#"## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt("4").
Let mutable xs be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i to xs.
    Push i to xs.
    Push i to xs.
    Set i to i + 1.
Show length of xs.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(
        rust.contains("* 3) as usize"),
        "a Seq pushed 3×/iter must reserve `N * 3`. Got:\n{}",
        rust
    );
    // 4 iterations × 3 pushes = 12 elements.
    assert_exact_output(src, "12");
}

/// graph_bfs's `adj` is pushed 5×/iter (the 5 edge slots); its buffer must be
/// sized to `n * 5`, matching C's `malloc(n * MAX_EDGES * sizeof(long))`.
#[test]
fn graph_bfs_adj_buffer_sized_for_five_edges() {
    const GRAPH_BFS: &str = include_str!("../../../benchmarks/programs/graph_bfs/main.lg");
    let rust = compile_to_rust(GRAPH_BFS).unwrap();
    assert!(
        rust.contains("* 5) as usize"),
        "graph_bfs `adj` (5 pushes/vertex) must reserve `n * 5`, not `n`. Got:\n{}",
        rust
    );
}
