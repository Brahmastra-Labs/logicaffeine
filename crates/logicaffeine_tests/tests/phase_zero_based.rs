//! General 0-based index normalization.
//!
//! LOGOS source is 1-based (`item 1 of arr` = first element), so codegen emits
//! `arr[(i - 1)]`. When a counted loop's counter is used ONLY to index `Vec`s,
//! the whole loop can be rebased so the generated Rust indexes `arr[i]` directly
//! — uniformly 0-based. The existing OPT-8 did this only for loops starting at
//! the literal `1`; this generalizes it to any start (`lo..=hi` rebased to
//! `(lo-1)..hi`), which is sound because the counter never escapes the index.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::assert_exact_output;
use logicaffeine_compile::compile::compile_to_rust;

/// A counted loop whose counter starts at a runtime `lo` (not the literal 1) and
/// is used only to read `arr[i]` must still rebase to 0-based: `for i in
/// (lo - 1)..hi` indexing `arr[i as usize]`, with NO `(i - 1)` subtraction left.
const NON_ONE_START: &str = r#"## To sumRange (arr: Seq of Int, lo: Int, hi: Int) -> Int:
    Let mutable total be 0.
    Let mutable i be lo.
    While i is at most hi:
        Set total to total + item i of arr.
        Set i to i + 1.
    Return total.
## Main
Let x be 0.
Show x.
"#;

#[test]
fn non_one_start_loop_rebases_to_zero_based() {
    let rust = compile_to_rust(NON_ONE_START).unwrap();
    let body = rust
        .split_once("fn sumRange")
        .map(|(_, b)| b)
        .unwrap_or(&rust);
    assert!(
        body.contains("(lo - 1)..hi") || body.contains("((lo - 1))..hi") || body.contains("(lo - 1)..(hi)"),
        "a non-1-start indexing loop must rebase its range to 0-based \
         (`for i in (lo - 1)..hi`). Got:\n{}",
        rust
    );
    assert!(
        !body.contains("[(i - 1) as usize]"),
        "the 1-based `arr[(i - 1)]` subtraction must be gone (0-based `arr[i]`). Got:\n{}",
        rust
    );
}

/// The rebase must be exactly value-preserving for a non-1 (literal) start.
/// `i` runs 2..=4 over `[10,20,30,40,50]`, summing the 2nd..4th elements
/// (20 + 30 + 40 = 90). The 0-based rebase indexes `arr[1..=3]` — same elements.
#[test]
fn non_one_start_loop_is_value_preserving() {
    let src = r#"## Main
Let mutable arr be a new Seq of Int.
Push 10 to arr.
Push 20 to arr.
Push 30 to arr.
Push 40 to arr.
Push 50 to arr.
Let mutable total be 0.
Let mutable i be 2.
While i is at most 4:
    Set total to total + item i of arr.
    Set i to i + 1.
Show total.
"#;
    assert_exact_output(src, "90");
}

/// A counter used for something OTHER than indexing (here, added into the
/// running total) must NOT be rebased — its value is observed directly, so a
/// `-1` shift would change the result. `i` runs 1..=3, summing i (1+2+3 = 6).
#[test]
fn counter_observed_directly_is_not_rebased() {
    let src = r#"## Main
Let mutable total be 0.
Let mutable i be 1.
While i is at most 3:
    Set total to total + i.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(
        !rust.contains("(0)..") && !rust.contains("(i - 1)") || rust.contains("for i in 1.."),
        "a directly-observed counter must keep its 1-based values. Got:\n{}",
        rust
    );
    assert_exact_output(src, "6");
}
