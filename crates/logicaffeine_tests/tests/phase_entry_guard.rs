//! Function-entry precondition guard (quicksort BCE).
//!
//! A recursive partition like quicksort indexes its slice param by `item j of
//! arr` where `j ∈ [lo, hi]` — 1-based, so codegen emits `arr[(j-1)]`. LLVM
//! cannot elide those bounds checks because it cannot prove `lo >= 1` (needed so
//! `(j-1) as usize` does not wrap) nor `hi <= len`. Those facts ARE the
//! function's precondition; making them explicit with one entry assert lets LLVM
//! drop the per-access checks in the hot partition loop (C's partition has none).
//!
//! The guard is emitted as `if lo < hi { assert!(lo >= 1 && hi <= arr.len()) }`
//! so it runs only on the indexing path (when the base case `if lo >= hi:
//! return` would fall through) — it never fires for a valid sort, and on a
//! genuinely out-of-range call it aborts exactly where the access would, so it
//! is behavior-preserving for the pure (no-I/O) functions it targets.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::assert_exact_output;
use logicaffeine_compile::compile::compile_to_rust;

const QUICKSORT: &str = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## To qs (arr: Seq of Int, lo: Int, hi: Int) -> Seq of Int:
    If lo is at least hi:
        Return arr.
    Let pivot be item hi of arr.
    Let mutable result be arr.
    Let mutable i be lo.
    Let mutable j be lo.
    While j is less than hi:
        If item j of result is at most pivot:
            Let tmp be item i of result.
            Set item i of result to item j of result.
            Set item j of result to tmp.
            Set i to i + 1.
        Set j to j + 1.
    Let tmp be item i of result.
    Set item i of result to item hi of result.
    Set item hi of result to tmp.
    Set result to qs(result, lo, i - 1).
    Set result to qs(result, i + 1, hi).
    Return result.

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable arr be a new Seq of Int.
Let mutable seed be 42.
Let mutable i be 0.
While i is less than n:
    Set seed to (seed * 1103515245 + 12345) % 2147483648.
    Push (seed / 65536) % 32768 to arr.
    Set i to i + 1.
Set arr to qs(arr, 1, n).
Let mutable checksum be 0.
Set i to 1.
While i is at most n:
    Set checksum to (checksum + item i of arr) % 1000000007.
    Set i to i + 1.
Show "" + item 1 of arr + " " + item n of arr + " " + checksum.
"#;

#[test]
fn quicksort_emits_entry_precondition_guard() {
    let rust = compile_to_rust(QUICKSORT).unwrap();
    let qs = rust
        .split_once("fn qs")
        .map(|(_, b)| b.split("\nfn ").next().unwrap_or(b))
        .expect("qs function must exist");
    assert!(
        qs.contains("assert!((lo) >= 1 && (hi) <= ")
            || qs.contains("assert!((lo) >= 1i64 && (hi) <= "),
        "qs must assert its 1-based index precondition at entry so LLVM elides \
         the partition bounds checks. Got qs:\n{}",
        qs
    );
    assert!(
        qs.contains("if (lo) < (hi)") || qs.contains("if (lo < hi)") || qs.contains("if lo < hi"),
        "the precondition guard must be gated on the indexing path (`if lo < hi`) \
         so it never fires for the empty/base-case range. Got qs:\n{}",
        qs
    );
}

/// The PAYOFF of the entry-guard precondition: with `1 <= lo` and `hi <= len`
/// seeded, the relational BCE proves `1 <= i <= j < hi <= len`, so every
/// partition access proves in range and codegen lowers it to UNCHECKED indexing
/// — eliminating the `panic_bounds_check` branches that are the gap vs C. Reads
/// (`item j/i of result`) become `get_unchecked`; stores (`Set item i/j of
/// result`) become `get_unchecked_mut`.
#[test]
fn quicksort_partition_accesses_elide_to_unchecked() {
    let rust = compile_to_rust(QUICKSORT).unwrap();
    let qs = rust
        .split_once("fn qs")
        .map(|(_, b)| b.split("\nfn ").next().unwrap_or(b))
        .expect("qs function must exist");
    assert!(
        qs.contains(".get_unchecked("),
        "qs's partition reads must lower to `get_unchecked` once the entry-guard \
         precondition proves `1 <= i <= j < hi <= len`. Got qs:\n{}",
        qs
    );
    assert!(
        qs.contains(".get_unchecked_mut("),
        "qs's partition stores must lower to `get_unchecked_mut`. Got qs:\n{}",
        qs
    );
}

/// The `Unchecked` toggle keeps every bounds check. Disabling it
/// (`LOGOS_OPT_OFF=unchecked`, the config form of `## No Unchecked`) must remove
/// ALL oracle-elided `get_unchecked`/`get_unchecked_mut` from the generated Rust:
/// the same quicksort that elides by default stays fully checked. This is the
/// headline safety guarantee of the Unchecked toggle (the Safety profile relies
/// on it).
#[test]
fn disabling_unchecked_keeps_all_bounds_checks() {
    std::env::set_var("LOGOS_OPT_OFF", "unchecked");
    let rust = compile_to_rust(QUICKSORT).unwrap();
    std::env::remove_var("LOGOS_OPT_OFF");
    assert!(
        !rust.contains(".get_unchecked("),
        "with Unchecked disabled, no read may lower to get_unchecked. Got:\n{rust}"
    );
    assert!(
        !rust.contains(".get_unchecked_mut("),
        "with Unchecked disabled, no store may lower to get_unchecked_mut. Got:\n{rust}"
    );
}

/// A short recursive partition (no I/O before the indexing) must still sort
/// correctly with the guard in place — the guard never fires for valid input.
/// qs over a fixed list, checksum of the sorted result is deterministic.
const SORT3: &str = r#"## To qs (arr: Seq of Int, lo: Int, hi: Int) -> Seq of Int:
    If lo is at least hi:
        Return arr.
    Let pivot be item hi of arr.
    Let mutable result be arr.
    Let mutable i be lo.
    Let mutable j be lo.
    While j is less than hi:
        If item j of result is at most pivot:
            Let tmp be item i of result.
            Set item i of result to item j of result.
            Set item j of result to tmp.
            Set i to i + 1.
        Set j to j + 1.
    Let tmp be item i of result.
    Set item i of result to item hi of result.
    Set item hi of result to tmp.
    Set result to qs(result, lo, i - 1).
    Set result to qs(result, i + 1, hi).
    Return result.

## Main
Let mutable arr be a new Seq of Int.
Push 5 to arr.
Push 2 to arr.
Push 8 to arr.
Push 1 to arr.
Push 9 to arr.
Push 3 to arr.
Set arr to qs(arr, 1, 6).
Show "" + item 1 of arr + item 2 of arr + item 3 of arr + item 4 of arr + item 5 of arr + item 6 of arr.
"#;

#[test]
fn quicksort_with_entry_guard_sorts_correctly() {
    // [5,2,8,1,9,3] sorted ascending -> 1 2 3 5 8 9 -> concatenated "123589".
    assert_exact_output(SORT3, "123589");
}
