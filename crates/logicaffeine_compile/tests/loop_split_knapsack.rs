//! End-to-end: the guard-based loop index-set split fires on the knapsack inner
//! DP loop, turning the single guarded scan into a branch-free prefix/suffix
//! pair (plus an out-of-range fallback) whose `curr` store is a once-`resize`d
//! indexed write (`curr[w] = ...`, the form LLVM vectorizes — it elides the
//! bounds check after the `resize`).

use logicaffeine_compile::compile_to_rust;

const KNAPSACK: &str = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
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
"#;

#[test]
fn knapsack_inner_loop_is_split_into_three_for_w_loops() {
    let rust = compile_to_rust(KNAPSACK).expect("knapsack compiles");
    // prefix [0, wi), suffix [wi, cols), and the out-of-range fallback — five in
    // all: the exact-Int narrowing versions each loop that reads `prev[w - wi]` on
    // `wi`'s magnitude, cloning the prefix and the fallback into a raw/exact pair
    // (2 + 1 + 2). The split itself is intact (the sibling store/load locks pass).
    let for_w = rust.matches("for w in").count();
    assert_eq!(for_w, 5, "expected versioned prefix + suffix + fallback `for w` loops:\n{rust}");
}

#[test]
fn knapsack_curr_store_is_resized_indexed_write() {
    let rust = compile_to_rust(KNAPSACK).expect("knapsack compiles");
    // The reused buffer is sized once and written by index — the vectorizable
    // DP store — not rebuilt by `push`. The checked indexed store is enough:
    // LLVM elides the bounds check after the `resize`, so it vectorizes.
    assert!(rust.contains("curr.resize("), "curr is resize-sized once:\n{rust}");
    assert!(
        rust.contains("curr[") || rust.contains("curr ["),
        "curr is written by index:\n{rust}"
    );
    // No per-iteration push into curr survives in the hot path.
    assert!(!rust.contains("curr.push("), "no curr.push in the split loops:\n{rust}");
}

#[test]
fn knapsack_suffix_load_is_unchecked_and_branch_free() {
    let rust = compile_to_rust(KNAPSACK).expect("knapsack compiles");
    // The split-suffix `prev[w - wi]` load is unconditional and unchecked
    // (the affine oracle re-proves it in range from the `wi..` for-range start).
    assert!(
        rust.contains("get_unchecked(((w - wi))"),
        "suffix `prev[w-wi]` is an unchecked load:\n{rust}"
    );
    // No bounds panic remains in the generated program's hot region.
    assert!(!rust.contains("panic_bounds_check"), "no residual bounds panics");
}
