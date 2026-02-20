//! E2E Codegen Tests: Optimization Correctness
//!
//! Verifies that optimizations produce correct runtime output.
//! These don't test generated code shape — they test that
//! TCO, constant propagation, DCE, and other optimizations
//! don't break correctness.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_exact_output;

#[cfg(not(target_arch = "wasm32"))]
use common::compile_to_rust;

// =============================================================================
// TCO (Tail Call Optimization)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_tco_factorial() {
    assert_exact_output(
        r#"## To factorial (n: Int) and (acc: Int) -> Int:
    If n is at most 1:
        Return acc.
    Return factorial(n - 1, acc * n).

## Main
Show factorial(5, 1).
"#,
        "120",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_tco_deep_recursion() {
    assert_exact_output(
        r#"## To countDown (n: Int) -> Int:
    If n is at most 0:
        Return 0.
    Return countDown(n - 1).

## Main
Show countDown(10000).
"#,
        "0",
    );
}

// =============================================================================
// Constant Propagation
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_const_prop_simple() {
    assert_exact_output(
        r#"## Main
Let x be 10.
Let y be x + 5.
Show y.
"#,
        "15",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_const_prop_chain() {
    assert_exact_output(
        r#"## Main
Let a be 1.
Let b be a + 1.
Let c be b + 1.
Let d be c + 1.
Show d.
"#,
        "4",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_const_prop_loop_safe() {
    assert_exact_output(
        r#"## Main
Let x be 0.
Repeat for i from 1 to 5:
    Set x to x + i.
Show x.
"#,
        "15",
    );
}

// =============================================================================
// Dead Code Elimination
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_dce_unused_var() {
    assert_exact_output(
        r#"## Main
Let unused be 999.
Let result be 42.
Show result.
"#,
        "42",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_dce_after_return() {
    assert_exact_output(
        r#"## To getValue -> Int:
    Return 42.

## Main
Show getValue().
"#,
        "42",
    );
}

// =============================================================================
// Vec Fill Pattern
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_vec_fill_bool() {
    assert_exact_output(
        r#"## Main
Let flags be a new Seq of Bool.
Repeat for i from 1 to 5:
    Push true to flags.
Show length of flags.
"#,
        "5",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_vec_fill_int() {
    assert_exact_output(
        r#"## Main
Let nums be a new Seq of Int.
Repeat for i from 1 to 10:
    Push 0 to nums.
Show length of nums.
"#,
        "10",
    );
}

// =============================================================================
// Swap Pattern
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_swap_correct() {
    assert_exact_output(
        r#"## Main
Let items be [3, 1, 2].
Let temp be item 1 of items.
Set item 1 of items to item 2 of items.
Set item 2 of items to temp.
Show items.
"#,
        "[1, 3, 2]",
    );
}

// =============================================================================
// Constant Folding
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_fold_expression() {
    assert_exact_output("## Main\nShow 2 + 3 * 4.", "14");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_fold_subtraction() {
    assert_exact_output("## Main\nShow 10 - 3 - 2.", "5");
}

// =============================================================================
// Index Simplification
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_index_simplification() {
    assert_exact_output(
        r#"## Main
Let items be [10, 20, 30].
Let i be 2.
Show item (i + 1 - 1) of items.
"#,
        "20",
    );
}

// =============================================================================
// WithCapacity Runtime
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_with_capacity_runtime() {
    assert_exact_output(
        r#"## Main
Let items be a new Seq of Int.
Repeat for i from 1 to 100:
    Push i to items.
Show length of items.
"#,
        "100",
    );
}

// =============================================================================
// String Append in Loop
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_string_append_large() {
    assert_exact_output(
        r#"## Main
Let s be "".
Repeat for i from 1 to 100:
    Set s to s + "x".
Show length of s.
"#,
        "100",
    );
}

// =============================================================================
// Dead Post-Loop Counter Elimination (Optimization Phase 1)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_dead_counter_eliminated() {
    // When the loop counter is not used after the loop, the post-loop
    // `let mut i = ...;` binding should be eliminated from generated code.
    let code = r#"## Main
Let sum be 0.
Let i be 0.
While i is less than 10:
    Set sum to sum + i.
    Set i to i + 1.
Show sum.
"#;
    assert_exact_output(code, "45");
    let rust = compile_to_rust(code).unwrap();
    // The for-range pattern should fire, and since `i` is not used after the loop,
    // there should be no dead `let mut i = 10;` post-loop counter.
    assert!(rust.contains("for i in 0..10"), "Should use for-range pattern, got:\n{}", rust);
    // After the for loop, i should NOT be re-declared since it's unused
    assert!(!rust.contains("let mut i = 10;"), "Dead post-loop counter should be eliminated, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_counter_kept_when_used() {
    // When the loop counter IS used after the loop, the post-loop binding
    // must be kept for correctness.
    let code = r#"## Main
Let i be 0.
While i is less than 5:
    Set i to i + 1.
Show i.
"#;
    assert_exact_output(code, "5");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("for i in 0..5"), "Should use for-range pattern, got:\n{}", rust);
    // i IS used after the loop (in Show), so the post-loop binding must exist
    assert!(rust.contains("let mut i = 5;"), "Post-loop counter should be kept when used, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_dead_counter_in_function() {
    // Dead counter elimination should also work inside function bodies
    let code = r#"## To sumTo (n: Int) -> Int:
    Let total be 0.
    Let i be 1.
    While i is at most n:
        Set total to total + i.
        Set i to i + 1.
    Return total.

## Main
Show sumTo(10).
"#;
    assert_exact_output(code, "55");
    let rust = compile_to_rust(code).unwrap();
    // i is not used after the while loop (only total is returned), so dead counter should be eliminated
    assert!(rust.contains("for i in"), "Should use for-range pattern, got:\n{}", rust);
    // The post-loop `let mut i = ...` for the dead counter should not appear
    // (total's post-loop value IS needed since it's returned)
    let for_pos = rust.find("for i in").unwrap();
    let after_for = &rust[for_pos..];
    let closing_brace = after_for.find("\n    }").unwrap();
    let after_loop = &after_for[closing_brace..];
    assert!(!after_loop.contains("let mut i ="), "Dead counter `i` should be eliminated after loop, got:\n{}", rust);
}

// =============================================================================
// FxHashMap (Optimization Phase 2)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_fxhashmap_codegen() {
    // Maps should use FxHashMap (via the Map type alias) instead of std::collections::HashMap
    let code = r#"## Main
Let counts be a new Map of Int to Int.
Set item 1 of counts to 10.
Set item 2 of counts to 20.
Show item 1 of counts.
"#;
    assert_exact_output(code, "10");
    let rust = compile_to_rust(code).unwrap();
    // The codegen should NOT emit std::collections::HashMap — it uses the Map alias
    // which now resolves to FxHashMap in the runtime
    assert!(!rust.contains("std::collections::HashMap"),
        "Map codegen should not use std::collections::HashMap, got:\n{}", rust);
    // The type registration should use FxHashMap
    assert!(rust.contains("FxHashMap") || rust.contains("Map::<"),
        "Map codegen should use FxHashMap or Map alias, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_fxhashset_codegen() {
    // Sets should use FxHashSet (via the Set type alias) instead of std::collections::HashSet
    let code = r#"## Main
Let items be a new Set of Int.
Add 42 to items.
If items contains 42:
    Show "found".
"#;
    assert_exact_output(code, "found");
    let rust = compile_to_rust(code).unwrap();
    // The codegen should NOT emit std::collections::HashSet
    assert!(!rust.contains("std::collections::HashSet"),
        "Set codegen should not use std::collections::HashSet, got:\n{}", rust);
    assert!(rust.contains("FxHashSet") || rust.contains("Set::<"),
        "Set codegen should use FxHashSet or Set alias, got:\n{}", rust);
}

// =============================================================================
// Read-Only Vec Params → &[T] (Optimization Phase 3)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_readonly_vec_borrow() {
    // A function that only reads a Vec parameter should accept &[T] instead of Vec<T>
    let code = r#"## To sumList (items: Seq of Int) -> Int:
    Let total be 0.
    Let i be 1.
    While i is at most length of items:
        Set total to total + item i of items.
        Set i to i + 1.
    Return total.

## Main
Let nums be [1, 2, 3, 4, 5].
Show sumList(nums).
"#;
    assert_exact_output(code, "15");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("items: &[i64]"),
        "Read-only Vec param should be &[T], got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_mutated_vec_stays_owned() {
    // A function that mutates a Vec parameter should keep Vec<T>
    let code = r#"## To addAndSum (items: Seq of Int) -> Int:
    Push 99 to items.
    Let total be 0.
    Let i be 1.
    While i is at most length of items:
        Set total to total + item i of items.
        Set i to i + 1.
    Return total.

## Main
Let nums be [1, 2, 3].
Show addAndSum(nums).
"#;
    assert_exact_output(code, "105");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("items: Vec<i64>"),
        "Mutated Vec param should stay Vec<T>, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_readonly_vec_no_clone_at_callsite() {
    // When passing a Vec to a &[T] param, no .clone() should be emitted
    let code = r#"## To isSafe (queens: Seq of Int) and (row: Int) and (col: Int) -> Bool:
    Let i be 1.
    While i is at most length of queens:
        Let q be item i of queens.
        If q equals col:
            Return false.
        If q - col equals i - row:
            Return false.
        If col - q equals i - row:
            Return false.
        Set i to i + 1.
    Return true.

## Main
Let queens be [1, 3].
If isSafe(queens, 3, 5):
    Show "safe".
"#;
    assert_exact_output(code, "safe");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("queens: &[i64]"),
        "Read-only Vec param should be &[T], got:\n{}", rust);
    // At the call site, should pass &queens instead of queens.clone()
    assert!(!rust.contains("queens.clone()"),
        "Call site should not clone for borrow param, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_borrow_param_return_to_owned() {
    // When a function returns a borrowed param, it should use .to_vec()
    let code = r#"## To identity (items: Seq of Int) -> Seq of Int:
    Return items.

## Main
Let nums be [1, 2, 3].
Show identity(nums).
"#;
    assert_exact_output(code, "[1, 2, 3]");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("items: &[i64]"),
        "Read-only Vec param should be &[T], got:\n{}", rust);
    assert!(rust.contains(".to_vec()") || rust.contains(".to_owned()"),
        "Returning borrowed param should convert to owned, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_mergesort_borrow() {
    // Classic mergesort: merge() reads both halves, sort() returns new Vec
    let code = r#"## To merge (left: Seq of Int) and (right: Seq of Int) -> Seq of Int:
    Let result be a new Seq of Int.
    Let i be 1.
    Let j be 1.
    While i is at most length of left and j is at most length of right:
        If item i of left is at most item j of right:
            Push item i of left to result.
            Set i to i + 1.
        Else:
            Push item j of right to result.
            Set j to j + 1.
    While i is at most length of left:
        Push item i of left to result.
        Set i to i + 1.
    While j is at most length of right:
        Push item j of right to result.
        Set j to j + 1.
    Return result.

## To mergeSort (arr: Seq of Int) -> Seq of Int:
    If length of arr is at most 1:
        Return arr.
    Let mid be length of arr / 2.
    Let left be Copy of items 1 through mid of arr.
    Let right be Copy of items (mid + 1) through length of arr of arr.
    Let sortedLeft be mergeSort(left).
    Let sortedRight be mergeSort(right).
    Return merge(sortedLeft, sortedRight).

## Main
Let nums be [5, 3, 8, 1, 2].
Show mergeSort(nums).
"#;
    assert_exact_output(code, "[1, 2, 3, 5, 8]");
    let rust = compile_to_rust(code).unwrap();
    // merge() only reads left and right — they should be &[i64]
    assert!(rust.contains("left: &[i64]"),
        "merge() read-only param 'left' should be &[T], got:\n{}", rust);
    assert!(rust.contains("right: &[i64]"),
        "merge() read-only param 'right' should be &[T], got:\n{}", rust);
}

// =============================================================================
// __set_tmp Reduction (Optimization Phase 4)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_set_tmp_different_indices() {
    // arr[i] = arr[j] + 1 where i != j should NOT need __set_tmp
    let code = r#"## Main
Let items be [10, 20, 30, 40].
Set item 1 of items to item 3 of items + 5.
Show item 1 of items.
"#;
    assert_exact_output(code, "35");
    let rust = compile_to_rust(code).unwrap();
    assert!(!rust.contains("__set_tmp"),
        "Different index assignment should not use __set_tmp, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_set_tmp_different_collections() {
    // a[i] = b[i] + 1 where a and b are different collections — no tmp needed
    let code = r#"## Main
Let source be [100, 200, 300].
Let dest be [0, 0, 0].
Set item 1 of dest to item 1 of source + 5.
Show item 1 of dest.
"#;
    assert_exact_output(code, "105");
    let rust = compile_to_rust(code).unwrap();
    assert!(!rust.contains("__set_tmp"),
        "Different-collection assignment should not use __set_tmp, got:\n{}", rust);
}

// =============================================================================
// Type Propagation Through Variable Assignment (Optimization Phase 5)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_type_propagation_let_from_param() {
    // `Let mutable result be arr` should inherit arr's Vec<i64> type
    // so that subsequent indexing uses direct [] instead of LogosIndex trait
    let code = r#"## To doubleFirst (arr: Seq of Int) -> Seq of Int:
    Let mutable result be arr.
    Set item 1 of result to item 1 of result * 2.
    Return result.

## Main
Let nums be [10, 20, 30].
Show doubleFirst(nums).
"#;
    assert_exact_output(code, "[20, 20, 30]");
    let rust = compile_to_rust(code).unwrap();
    // result should use direct indexing, not LogosIndex/LogosIndexMut
    assert!(!rust.contains("LogosIndex"),
        "Should use direct indexing for result, not LogosIndex, got:\n{}", rust);
    assert!(!rust.contains("LogosIndexMut"),
        "Should use direct indexing for result, not LogosIndexMut, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_type_propagation_copy_slice() {
    // `Copy of items ... of arr` should produce Vec<T> type
    let code = r#"## To firstHalf (arr: Seq of Int) -> Seq of Int:
    Let mid be length of arr / 2.
    Let left be Copy of items 1 through mid of arr.
    Return left.

## Main
Let nums be [1, 2, 3, 4, 5, 6].
Show firstHalf(nums).
"#;
    assert_exact_output(code, "[1, 2, 3]");
}

// =============================================================================
// Unconditional Swap Pattern (Optimization Phase 5)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_unconditional_swap() {
    // 3-statement swap without If guard should use .swap()
    let code = r#"## Main
Let items be [10, 20, 30, 40, 50].
Let tmp be item 2 of items.
Set item 2 of items to item 4 of items.
Set item 4 of items to tmp.
Show items.
"#;
    assert_exact_output(code, "[10, 40, 30, 20, 50]");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains(".swap("),
        "Unconditional swap should use .swap(), got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_unconditional_swap_expressions() {
    // Swap with expression indices like (i+1) and (swapIdx+1)
    let code = r#"## Main
Let items be [10, 20, 30, 40, 50].
Let i be 1.
Let j be 3.
Let tmp be item (i + 1) of items.
Set item (i + 1) of items to item (j + 1) of items.
Set item (j + 1) of items to tmp.
Show items.
"#;
    assert_exact_output(code, "[10, 40, 30, 20, 50]");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains(".swap("),
        "Unconditional swap with expr indices should use .swap(), got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_quicksort_correctness() {
    // Full quicksort implementation should produce correct sorted output
    let code = r#"## To qs (arr: Seq of Int, lo: Int, hi: Int) -> Seq of Int:
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
Let arr be [5, 3, 8, 1, 9, 2, 7].
Set arr to qs(arr, 1, 7).
Show arr.
"#;
    assert_exact_output(code, "[1, 2, 3, 5, 7, 8, 9]");
    let rust = compile_to_rust(code).unwrap();
    // Should use direct indexing (no LogosIndex) and .swap()
    assert!(!rust.contains("LogosIndex"),
        "quicksort should use direct indexing, got:\n{}", rust);
    assert!(rust.contains(".swap("),
        "quicksort should use .swap(), got:\n{}", rust);
}

// =============================================================================
// Last-Use Clone Elimination (Optimization Phase 5)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_last_use_clone_elimination() {
    // When `Set x to f(x, ...)` where f takes ownership (param is mutated),
    // x should be moved, not cloned, since its old value is immediately overwritten.
    let code = r#"## To append42 (arr: Seq of Int) -> Seq of Int:
    Push 42 to arr.
    Return arr.

## Main
Let mutable data be [1, 2, 3].
Set data to append42(data).
Show data.
"#;
    assert_exact_output(code, "[1, 2, 3, 42]");
    let rust = compile_to_rust(code).unwrap();
    // The call in main should NOT clone data since it's immediately reassigned.
    // Look for `append42(data)` without `.clone()`.
    // The function takes ownership (arr is mutated via Push), so it's not a borrow param.
    assert!(!rust.contains("append42(data.clone())"),
        "Last-use clone elimination: should move data, not clone, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_last_use_clone_not_applied_when_unsafe() {
    // Clone should NOT be eliminated when the variable appears in other arg
    // sub-expressions (would cause use-after-move).
    let code = r#"## To process (arr: Seq of Int, n: Int) -> Seq of Int:
    Push n to arr.
    Return arr.

## Main
Let mutable data be [1, 2, 3].
Set data to process(data, length of data).
Show data.
"#;
    assert_exact_output(code, "[1, 2, 3, 3]");
    // `data` appears in arg 0 (direct) AND in arg 1 (sub-expression via length of data).
    // The clone should be preserved for safety.
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("data.clone()"),
        "Clone should be preserved when target appears in other arg exprs, got:\n{}", rust);
}

// =============================================================================
// String Byte Comparison (Optimization Phase 5)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_string_byte_comparison() {
    // Comparing individual characters of two strings via indexing should use
    // as_bytes() comparison instead of allocating String per character
    let code = r#"## Main
Let text be "hello".
Let pattern be "el".
If item 2 of text equals item 1 of pattern:
    Show "match".
"#;
    assert_exact_output(code, "match");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("as_bytes()"),
        "String char comparison should use as_bytes(), got:\n{}", rust);
}

// =============================================================================
// Extended Vec-Fill: Prefix Push + Fill Loop (Optimization Phase 5)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_vec_fill_with_prefix_push() {
    // The "coins DP" pattern: new Seq → Push 1 → loop Push 0.
    // Should emit `vec![0; total]` with dp[0] = 1 override.
    let code = r#"## Main
Let mutable dp be a new Seq of Int.
Push 1 to dp.
Let mutable i be 1.
While i is at most 5:
    Push 0 to dp.
    Set i to i + 1.
Show dp.
"#;
    assert_exact_output(code, "[1, 0, 0, 0, 0, 0]");
    let rust = compile_to_rust(code).unwrap();
    // Should use vec![] allocation, not a push loop
    assert!(rust.contains("vec!["),
        "Prefix+fill pattern should use vec![], got:\n{}", rust);
    // Should override dp[0] = 1
    assert!(rust.contains("dp[0] = 1"),
        "Should override prefix element, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_vec_fill_with_prefix_same_as_fill() {
    // When prefix value equals fill value, no override needed.
    let code = r#"## Main
Let mutable arr be a new Seq of Int.
Push 0 to arr.
Let mutable i be 1.
While i is at most 4:
    Push 0 to arr.
    Set i to i + 1.
Show arr.
"#;
    assert_exact_output(code, "[0, 0, 0, 0, 0]");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("vec!["),
        "Same-fill prefix should use vec![], got:\n{}", rust);
    // No override needed since prefix value matches fill value
    assert!(!rust.contains("arr[0] = 0"),
        "Should skip redundant override when prefix == fill, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_vec_fill_coins_dp_correctness() {
    // Full coins DP pattern: verify correct results with the vec-fill optimization.
    let code = r#"## Main
Let mutable dp be a new Seq of Int.
Push 1 to dp.
Let mutable i be 1.
While i is at most 10:
    Push 0 to dp.
    Set i to i + 1.
Let coins be [1, 5, 10].
Let mutable c be 1.
While c is at most 3:
    Let coin be item c of coins.
    Let mutable j be coin.
    While j is at most 10:
        Set item (j + 1) of dp to item (j + 1) of dp + item (j - coin + 1) of dp.
        Set j to j + 1.
    Set c to c + 1.
Show item 11 of dp.
"#;
    // Ways to make change for 10 cents with coins [1, 5, 10]:
    // 10x1, 5x1+5x1, 5x2, 10x1 = 4 ways
    assert_exact_output(code, "4");
}

// =============================================================================
// Sentinel → break (OPT-5)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_sentinel_to_break() {
    // Sentinel pattern: `Set j to limit` inside `While j < limit:` should produce `break`
    let code = r#"## Main
Let mutable found be 0.
Let mutable j be 0.
While j is less than 5:
    If j equals 3:
        Set found to 1.
        Set j to 5.
    Set j to j + 1.
Show found.
"#;
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("break"), "Expected `break` in generated code for sentinel pattern, got:\n{}", rust);
    assert!(!rust.contains("j = 5"), "Sentinel `j = 5` should have been replaced by `break`");
    assert_exact_output(code, "1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_sentinel_break_string_search_pattern() {
    // Mimics the string_search benchmark's inner loop pattern.
    // Searches for the position of "c" in "abcde".
    let code = r#"## Main
Let text be "abcde".
Let needle be "c".
Let needleLen be 1.
Let textLen be 5.
Let mutable count be 0.
Let mutable i be 1.
While i is at most textLen:
    Let mutable matched be 1.
    Let mutable j be 0.
    While j is less than needleLen:
        If item (i + j) of text is not item (j + 1) of needle:
            Set matched to 0.
            Set j to needleLen.
        Set j to j + 1.
    If matched equals 1:
        Set count to count + 1.
    Set i to i + 1.
Show count.
"#;
    assert_exact_output(code, "1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_sentinel_not_applied_with_else() {
    // Sentinel pattern should NOT match when the If has an else block.
    let code = r#"## Main
Let mutable result be 0.
Let mutable j be 0.
While j is less than 5:
    If j equals 3:
        Set result to 1.
        Set j to 5.
    Otherwise:
        Set result to result + 1.
    Set j to j + 1.
Show result.
"#;
    let rust = compile_to_rust(code).unwrap();
    assert!(!rust.contains("break"), "Sentinel should NOT be applied when If has an else block");
    assert_exact_output(code, "1");
}

// =============================================================================
// Seq Copy Pattern (Optimization Phase 6)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_seq_copy_emits_to_vec() {
    // `Let mutable dst = new Seq; Set i = 1; While i <= length of src: Push item i of src to dst; Set i++`
    // should emit `let mut dst = src.to_vec();` instead of a push loop.
    let code = r#"## Main
Let src be [10, 20, 30].
Let mutable i be 1.
Let mutable dst be a new Seq of Int.
Set i to 1.
While i is at most length of src:
    Push item i of src to dst.
    Set i to i + 1.
Show dst.
"#;
    assert_exact_output(code, "[10, 20, 30]");
    let rust = compile_to_rust(code).unwrap();
    assert!(
        rust.contains(".to_vec()"),
        "Seq-copy loop should emit .to_vec(), got:\n{}",
        rust
    );
}

// =============================================================================
// Rotate-Left Pattern (Optimization Phase 6)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_rotate_left_emits_rotate_left() {
    // `Let tmp = item 1 of arr; Set i = 1; While i <= r: Set item i = item (i+1); Set i++; Set item (r+1) = tmp`
    // should emit `let tmp = arr[0]; arr[0..=(r as usize)].rotate_left(1);`
    let code = r#"## Main
Let mutable arr be [1, 2, 3, 4, 5].
Let mutable i be 1.
Let mutable r be 3.
Let perm0 be item 1 of arr.
Set i to 1.
While i is at most r:
    Set item i of arr to item (i + 1) of arr.
    Set i to i + 1.
Set item (r + 1) of arr to perm0.
Show arr.
"#;
    assert_exact_output(code, "[2, 3, 4, 1, 5]");
    let rust = compile_to_rust(code).unwrap();
    assert!(
        rust.contains(".rotate_left(1)"),
        "Rotate-left loop should emit .rotate_left(1), got:\n{}",
        rust
    );
}

// =============================================================================
// Consumed Seq Parameter: Ownership Transfer (no clone on recursive calls)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_quicksort_ownership_correct() {
    // Quicksort with in-place mutation pattern: `Let mutable result be arr`
    // then `Set result to qs(result, ...)`. The consumed parameter optimization
    // means `arr` is taken by value, enabling moves instead of clones.
    assert_exact_output(
        r#"## To qs (arr: Seq of Int, lo: Int, hi: Int) -> Seq of Int:
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
Let mutable arr be [5, 3, 1, 4, 2].
Set arr to qs(arr, 1, 5).
Show arr.
"#,
        "[1, 2, 3, 4, 5]",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_heapsort_siftdown_ownership_correct() {
    // siftDown with consumed param pattern: `Let mutable result be arr`
    assert_exact_output(
        r#"## To siftDown (arr: Seq of Int, start: Int, end: Int) -> Seq of Int:
    Let mutable result be arr.
    Let mutable root be start.
    While 2 * root + 1 is at most end:
        Let child be 2 * root + 1.
        Let mutable swapIdx be root.
        If item (swapIdx + 1) of result is less than item (child + 1) of result:
            Set swapIdx to child.
        If child + 1 is at most end:
            If item (swapIdx + 1) of result is less than item (child + 2) of result:
                Set swapIdx to child + 1.
        If swapIdx equals root:
            Return result.
        Let tmp be item (root + 1) of result.
        Set item (root + 1) of result to item (swapIdx + 1) of result.
        Set item (swapIdx + 1) of result to tmp.
        Set root to swapIdx.
    Return result.

## Main
Let mutable arr be [1, 5, 3, 4, 2].
Set arr to siftDown(arr, 0, 4).
Show item 1 of arr.
"#,
        "5",
    );
}
