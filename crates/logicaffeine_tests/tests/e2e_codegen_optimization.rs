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
    // 3-statement swap without If guard should use manual swap with __swap_tmp
    let code = r#"## Main
Let items be [10, 20, 30, 40, 50].
Let tmp be item 2 of items.
Set item 2 of items to item 4 of items.
Set item 4 of items to tmp.
Show items.
"#;
    assert_exact_output(code, "[10, 40, 30, 20, 50]");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("__swap_tmp"),
        "Unconditional swap should use __swap_tmp, got:\n{}", rust);
    assert!(!rust.contains(".swap("),
        "Should not use .swap() method, got:\n{}", rust);
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
    assert!(rust.contains("__swap_tmp"),
        "Unconditional swap with expr indices should use __swap_tmp, got:\n{}", rust);
    assert!(!rust.contains(".swap("),
        "Should not use .swap() method, got:\n{}", rust);
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
    // Should use direct indexing (no LogosIndex) and manual swap
    assert!(!rust.contains("LogosIndex"),
        "quicksort should use direct indexing, got:\n{}", rust);
    assert!(rust.contains("__swap_tmp"),
        "quicksort should use manual swap with __swap_tmp, got:\n{}", rust);
    assert!(!rust.contains(".swap("),
        "quicksort should not use .swap() method, got:\n{}", rust);
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
    // as_bytes() byte-level comparison instead of logos_get_char() method dispatch
    let code = r#"## Main
Let text be "hello".
Let pattern be "el".
If item 2 of text equals item 1 of pattern:
    Show "match".
"#;
    assert_exact_output(code, "match");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("as_bytes()"),
        "String char comparison should use as_bytes() byte comparison, got:\n{}", rust);
    assert!(!rust.contains("logos_get_char"),
        "Should NOT use logos_get_char() anymore, got:\n{}", rust);
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

// =============================================================================
// Optimization: Relaxed Pre-Allocation (intervening statements)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_prealloc_with_intervening_stmt() {
    // Pattern: new Seq, then intervening variable init, then push loop.
    // The with_capacity pattern should fire despite the intervening statement.
    let code = r#"## Main
Let mutable arr be a new Seq of Int.
Let mutable seed be 42.
Let mutable i be 0.
While i is less than 10:
    Set seed to seed * 3 + 7.
    Push seed to arr.
    Set i to i + 1.
Show length of arr.
"#;
    assert_exact_output(code, "10");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("Vec::with_capacity(") || rust.contains("vec!["),
        "Pre-allocation should fire with intervening stmt, got:\n{}", rust);
    assert!(!rust.contains("Seq::<i64>::default()") && !rust.contains("Vec::<i64>::new()"),
        "Should not use default/new (no pre-alloc), got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_prealloc_two_intervening_stmts() {
    // Two intervening statements between Seq creation and push loop.
    let code = r#"## Main
Let mutable arr be a new Seq of Int.
Let x be 10.
Let y be 20.
Let mutable i be 0.
While i is less than 5:
    Push x + y + i to arr.
    Set i to i + 1.
Show length of arr.
"#;
    assert_exact_output(code, "5");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("Vec::with_capacity(") || rust.contains("vec!["),
        "Pre-allocation should fire with 2 intervening stmts, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_prealloc_copy_type_vec_fill() {
    // For Copy types (i64), pushing a constant should use vec![0; N].
    let code = r#"## Main
Let mutable arr be a new Seq of Int.
Let mutable i be 0.
While i is less than 10:
    Push 0 to arr.
    Set i to i + 1.
Show length of arr.
"#;
    assert_exact_output(code, "10");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("vec![0; ") || rust.contains("vec![0i64; "),
        "Copy type constant fill should use vec![0; N], got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_prealloc_copy_type_with_computation() {
    // Push with computed value should use with_capacity + push (not vec![0; N]).
    let code = r#"## Main
Let mutable arr be a new Seq of Int.
Let mutable i be 0.
While i is less than 5:
    Push i * i to arr.
    Set i to i + 1.
Show arr.
"#;
    assert_exact_output(code, "[0, 1, 4, 9, 16]");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("Vec::with_capacity("),
        "Computed push should use Vec::with_capacity, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_prealloc_inner_loop() {
    // Pre-allocation inside a for-loop body (knapsack pattern).
    // The outer `prev` Seq gets pre-allocated at the top level.
    // The inner `curr` Seq is created inside the while body — its with_capacity
    // fires within the while body's peephole pass.
    let code = r#"## Main
Let n be 3.
Let capacity be 5.
Let mutable prev be a new Seq of Int.
Let mutable i be 0.
While i is at most capacity:
    Push 0 to prev.
    Set i to i + 1.
Let mutable outer be 1.
While outer is at most n:
    Let mutable curr be a new Seq of Int.
    Let mutable w be 0.
    While w is at most capacity:
        Push item (w + 1) of prev to curr.
        Set w to w + 1.
    Set prev to curr.
    Set outer to outer + 1.
Show length of prev.
"#;
    assert_exact_output(code, "6");
    let rust = compile_to_rust(code).unwrap();
    // At minimum, the outer `prev` should be pre-allocated
    let prealloc_count = rust.matches("Vec::with_capacity(").count()
        + rust.matches("vec![").count();
    assert!(prealloc_count >= 1,
        "At least outer Seq should be pre-allocated, found {} pre-allocs, got:\n{}", prealloc_count, rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_prealloc_bool_vec_fill() {
    // Bool is a Copy type.
    let code = r#"## Main
Let mutable flags be a new Seq of Bool.
Let mutable i be 0.
While i is less than 8:
    Push true to flags.
    Set i to i + 1.
Show length of flags.
"#;
    assert_exact_output(code, "8");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("vec![true; ") || rust.contains("Vec::with_capacity("),
        "Bool fill should use vec![true; N] or with_capacity, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_prealloc_prefix_sum_pattern() {
    // Mimics the prefix_sum benchmark pattern: Seq, seed init, push loop.
    let code = r#"## Main
Let n be 20.
Let mutable arr be a new Seq of Int.
Let mutable seed be 42.
Let mutable i be 0.
While i is less than n:
    Set seed to (seed * 37 + 13) % 10000.
    Push seed to arr.
    Set i to i + 1.
Show length of arr.
"#;
    assert_exact_output(code, "20");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("Vec::with_capacity(") || rust.contains("vec!["),
        "prefix_sum pattern should pre-allocate, got:\n{}", rust);
}

// =============================================================================
// Optimization: Index Arithmetic Simplification
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_index_literal_1_simplified() {
    // `item 1 of arr` should produce arr[0] not arr[(1 - 1) as usize]
    let code = r#"## Main
Let arr be [10, 20, 30].
Show item 1 of arr.
"#;
    assert_exact_output(code, "10");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("arr[0]"),
        "Literal index 1 should simplify to [0], got:\n{}", rust);
    assert!(!rust.contains("(1 - 1)"),
        "Should not contain (1 - 1), got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_index_literal_3_simplified() {
    // `item 3 of arr` should produce arr[2] not arr[(3 - 1) as usize]
    let code = r#"## Main
Let arr be [10, 20, 30].
Show item 3 of arr.
"#;
    assert_exact_output(code, "30");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("arr[2]"),
        "Literal index 3 should simplify to [2], got:\n{}", rust);
    assert!(!rust.contains("(3 - 1)"),
        "Should not contain (3 - 1), got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_index_plus_k_simplified() {
    // `item (child + 2) of arr` should produce arr[(child + 1) as usize]
    let code = r#"## Main
Let arr be [10, 20, 30, 40, 50].
Let child be 1.
Show item (child + 2) of arr.
"#;
    assert_exact_output(code, "30");
    let rust = compile_to_rust(code).unwrap();
    assert!(!rust.contains("((child + 2) - 1)"),
        "(child+2)-1 should simplify to (child+1), got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_setindex_literal_simplified() {
    // `Set item 1 of arr to X` should produce arr[0] = X
    let code = r#"## Main
Let mutable arr be [10, 20, 30].
Set item 1 of arr to 99.
Show arr.
"#;
    assert_exact_output(code, "[99, 20, 30]");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("arr[0]"),
        "SetIndex literal 1 should simplify to [0], got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_setindex_plus_k_simplified() {
    // `Set item (child + 2) of arr to X` should produce arr[(child + 1) as usize] = X
    let code = r#"## Main
Let mutable arr be [10, 20, 30, 40, 50].
Let child be 1.
Set item (child + 2) of arr to 99.
Show arr.
"#;
    assert_exact_output(code, "[10, 20, 99, 40, 50]");
    let rust = compile_to_rust(code).unwrap();
    assert!(!rust.contains("((child + 2) - 1)"),
        "SetIndex (child+2)-1 should simplify to (child+1), got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_swap_index_simplified() {
    // Swap with (x+1) indices should simplify to x in the manual swap.
    let code = r#"## Main
Let mutable arr be [10, 20, 30, 40, 50].
Let i be 1.
Let j be 3.
Let tmp be item (i + 1) of arr.
Set item (i + 1) of arr to item (j + 1) of arr.
Set item (j + 1) of arr to tmp.
Show arr.
"#;
    assert_exact_output(code, "[10, 40, 30, 20, 50]");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("__swap_tmp"),
        "Should use __swap_tmp, got:\n{}", rust);
    // The swap indices should be simplified: (i+1)-1 -> i, (j+1)-1 -> j
    assert!(!rust.contains("((i + 1) - 1)"),
        "swap index (i+1)-1 should simplify to i, got:\n{}", rust);
    assert!(!rust.contains("((j + 1) - 1)"),
        "swap index (j+1)-1 should simplify to j, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_swap_literal_index_simplified() {
    // Swap with literal indices should simplify completely.
    let code = r#"## Main
Let mutable arr be [10, 20, 30].
Let tmp be item 1 of arr.
Set item 1 of arr to item 3 of arr.
Set item 3 of arr to tmp.
Show arr.
"#;
    assert_exact_output(code, "[30, 20, 10]");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("__swap_tmp"),
        "Should use __swap_tmp, got:\n{}", rust);
    // Literal swap indices should simplify: item 1 -> [0], item 3 -> [2]
    assert!(rust.contains("arr[0]"),
        "Literal swap index 1 should simplify to [0], got:\n{}", rust);
    assert!(rust.contains("arr[2]"),
        "Literal swap index 3 should simplify to [2], got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_conditional_swap_index_simplified() {
    // Conditional swap (bubble sort pattern) should use manual swap with simplified indices.
    let code = r#"## Main
Let mutable arr be [3, 1, 2].
Let j be 1.
Let a be item j of arr.
Let b be item (j + 1) of arr.
If a is greater than b:
    Set item j of arr to b.
    Set item (j + 1) of arr to a.
Show arr.
"#;
    assert_exact_output(code, "[1, 3, 2]");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("__swap_tmp"),
        "Conditional swap should use __swap_tmp, got:\n{}", rust);
    // The comparison should use simplified indices
    assert!(!rust.contains("((j + 1) - 1)"),
        "Conditional swap comparison should simplify (j+1)-1, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_heap_sort_index_simplified() {
    // Heap sort pattern: item (swapIdx + 1) and item (child + 2) should simplify.
    let code = r#"## To siftDown2 (arr: Seq of Int, start: Int, end: Int) -> Seq of Int:
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
Set arr to siftDown2(arr, 0, 4).
Show item 1 of arr.
"#;
    assert_exact_output(code, "5");
    let rust = compile_to_rust(code).unwrap();
    // (child + 2) - 1 should simplify to (child + 1)
    assert!(!rust.contains("((child + 2) - 1)"),
        "heap_sort (child+2)-1 should simplify, got:\n{}", rust);
}

// =============================================================================
// Optimization: String Byte-Level Comparison
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_string_byte_comparison_correctness() {
    // String character equality comparison should produce correct results.
    // This tests that the byte-level optimization doesn't break correctness.
    assert_exact_output(
        r#"## Main
Let text be "hello".
Let pattern be "hello".
Let mutable matches be 0.
Let mutable i be 1.
While i is at most 5:
    If item i of text equals item i of pattern:
        Set matches to matches + 1.
    Set i to i + 1.
Show matches.
"#,
        "5",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_string_byte_comparison_codegen() {
    // When comparing indexed characters of two strings for equality,
    // codegen should use as_bytes() for direct byte comparison instead
    // of logos_get_char() which has method dispatch + ASCII branch overhead.
    let code = r#"## Main
Let text be "abcde".
Let pattern be "abcde".
Let mutable i be 1.
While i is at most 5:
    If item i of text is not item i of pattern:
        Show "mismatch".
    Set i to i + 1.
Show "done".
"#;
    assert_exact_output(code, "done");
    let rust = compile_to_rust(code).unwrap();
    assert!(!rust.contains("logos_get_char"),
        "String char comparison should use as_bytes() instead of logos_get_char(), got:\n{}", rust);
    assert!(rust.contains("as_bytes()"),
        "String char comparison should use as_bytes(), got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_string_byte_comparison_not_equal() {
    // NotEq comparisons should also use byte-level access.
    let code = r#"## Main
Let text be "abcde".
Let pattern be "axcde".
Let mutable mismatches be 0.
Let mutable i be 1.
While i is at most 5:
    If item i of text is not item i of pattern:
        Set mismatches to mismatches + 1.
    Set i to i + 1.
Show mismatches.
"#;
    assert_exact_output(code, "1");
    let rust = compile_to_rust(code).unwrap();
    assert!(!rust.contains("logos_get_char"),
        "NotEq string comparison should use as_bytes(), got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_string_byte_search_pattern() {
    // Full string search pattern (naive substring search) should use byte comparison.
    let code = r#"## Main
Let text be "hello world".
Let pattern be "world".
Let n be 11.
Let m be 5.
Let mutable found be 0.
Let mutable i be 1.
While i is at most (n - m + 1):
    Let mutable matched be 1.
    Let mutable j be 1.
    While j is at most m:
        If item (i + j - 1) of text is not item j of pattern:
            Set matched to 0.
        Set j to j + 1.
    If matched equals 1:
        Set found to found + 1.
    Set i to i + 1.
Show found.
"#;
    assert_exact_output(code, "1");
    let rust = compile_to_rust(code).unwrap();
    assert!(!rust.contains("logos_get_char"),
        "String search should use byte comparison, got:\n{}", rust);
}

// =============================================================================
// Optimization: String Index vs Single-Char Literal (Phase 6)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_string_char_literal_comparison_correctness() {
    // Comparing item i of text with a single-character string literal " "
    // should produce correct results.
    assert_exact_output(
        r#"## Main
Let text be "hello world".
Let mutable count be 0.
Let mutable i be 1.
While i is at most 11:
    If item i of text equals " ":
        Set count to count + 1.
    Set i to i + 1.
Show count.
"#,
        "1",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_string_char_literal_comparison_codegen() {
    // When comparing `item i of text` with a single-character string literal,
    // codegen should use logos_get_char() == 'c' instead of
    // LogosIndex::logos_get() == String::from("c") (avoids 2 allocations per comparison).
    let code = r#"## Main
Let text be "hello world".
Let mutable count be 0.
Let mutable i be 1.
While i is at most 11:
    If item i of text equals " ":
        Set count to count + 1.
    Set i to i + 1.
Show count.
"#;
    assert_exact_output(code, "1");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("logos_get_char"),
        "String-vs-char-literal comparison should use logos_get_char(), got:\n{}", rust);
    assert!(!rust.contains("String::from(\" \")"),
        "Should NOT allocate String::from for single char comparison, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_string_char_literal_reverse_order() {
    // Literal on the left side: `" " == item i of text`
    let code = r#"## Main
Let text be "a b c".
Let mutable spaces be 0.
Let mutable i be 1.
While i is at most 5:
    If " " equals item i of text:
        Set spaces to spaces + 1.
    Set i to i + 1.
Show spaces.
"#;
    assert_exact_output(code, "2");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("logos_get_char"),
        "Reverse-order string-vs-char comparison should use logos_get_char(), got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_string_char_literal_not_equal() {
    // NotEq comparison should also use logos_get_char.
    let code = r#"## Main
Let text be "aXbXc".
Let mutable non_x be 0.
Let mutable i be 1.
While i is at most 5:
    If item i of text is not "X":
        Set non_x to non_x + 1.
    Set i to i + 1.
Show non_x.
"#;
    assert_exact_output(code, "3");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("logos_get_char"),
        "NotEq string-vs-char-literal should use logos_get_char(), got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_string_char_literal_multi_char_no_opt() {
    // Multi-character string literals should NOT trigger the char optimization.
    let code = r#"## Main
Let text be "hello".
If item 1 of text equals "he":
    Show "match".
Otherwise:
    Show "no match".
"#;
    assert_exact_output(code, "no match");
    let rust = compile_to_rust(code).unwrap();
    // Multi-char literal should NOT use logos_get_char
    assert!(!rust.contains("logos_get_char"),
        "Multi-char literal should NOT use logos_get_char(), got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_string_char_literal_empty_no_opt() {
    // Empty string literal should NOT trigger the char optimization.
    let code = r#"## Main
Let text be "hello".
If item 1 of text equals "":
    Show "empty".
Otherwise:
    Show "not empty".
"#;
    assert_exact_output(code, "not empty");
    let rust = compile_to_rust(code).unwrap();
    assert!(!rust.contains("logos_get_char"),
        "Empty string literal should NOT use logos_get_char(), got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_string_char_literal_in_loop_count_spaces() {
    // Real-world pattern: counting spaces in a string (hot loop).
    // This is the exact pattern from the strings benchmark.
    let code = r#"## Main
Let text be "the quick brown fox jumps over the lazy dog".
Let n be 43.
Let mutable spaces be 0.
Let mutable i be 1.
While i is at most n:
    If item i of text equals " ":
        Set spaces to spaces + 1.
    Set i to i + 1.
Show spaces.
"#;
    assert_exact_output(code, "8");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("logos_get_char"),
        "Hot loop string comparison should use logos_get_char(), got:\n{}", rust);
    assert!(rust.contains("'"),
        "Should use char literal with single quotes, got:\n{}", rust);
}

// =============================================================================
// Optimization: Manual Swap (Phase 6)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_manual_swap_correctness() {
    // Swap pattern should produce correct results with manual swap codegen.
    assert_exact_output(
        r#"## Main
Let mutable arr be [5, 3, 1, 4, 2].
Let mutable i be 1.
While i is at most 4:
    Let mutable j be 5.
    While j is greater than i:
        Let a be item (j - 1) of arr.
        Let b be item j of arr.
        If a is greater than b:
            Set item (j - 1) of arr to b.
            Set item j of arr to a.
        Set j to j - 1.
    Set i to i + 1.
Show arr.
"#,
        "[1, 2, 3, 4, 5]",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_manual_swap_codegen() {
    // Swap pattern should emit manual swap with direct indexing instead of .swap()
    // to give LLVM better bounds-check elision opportunities.
    let code = r#"## Main
Let mutable arr be [5, 3, 1, 4, 2].
Let mutable i be 1.
While i is at most 4:
    Let mutable j be 5.
    While j is greater than i:
        Let a be item (j - 1) of arr.
        Let b be item j of arr.
        If a is greater than b:
            Set item (j - 1) of arr to b.
            Set item j of arr to a.
        Set j to j - 1.
    Set i to i + 1.
Show arr.
"#;
    assert_exact_output(code, "[1, 2, 3, 4, 5]");
    let rust = compile_to_rust(code).unwrap();
    // Should use direct indexing swap, not .swap() method
    assert!(!rust.contains(".swap("),
        "Should use manual swap instead of .swap() method, got:\n{}", rust);
    assert!(rust.contains("__swap_tmp"),
        "Manual swap should use __swap_tmp variable, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_unconditional_swap_manual() {
    // Unconditional swap (quicksort/heapsort pattern) should also use manual swap.
    let code = r#"## Main
Let mutable arr be [3, 1, 2].
Let tmp be item 1 of arr.
Set item 1 of arr to item 3 of arr.
Set item 3 of arr to tmp.
Show arr.
"#;
    assert_exact_output(code, "[2, 1, 3]");
    let rust = compile_to_rust(code).unwrap();
    assert!(!rust.contains(".swap("),
        "Unconditional swap should use manual swap, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_conditional_swap_ascending_indices() {
    // Pattern (j, j+1): standard ascending bubble sort should use manual swap.
    let code = r#"## Main
Let mutable arr be [3, 1, 2].
Let mutable i be 1.
While i is at most 2:
    Let mutable j be 1.
    While j is at most 2:
        Let a be item j of arr.
        Let b be item (j + 1) of arr.
        If a is greater than b:
            Set item j of arr to b.
            Set item (j + 1) of arr to a.
        Set j to j + 1.
    Set i to i + 1.
Show arr.
"#;
    assert_exact_output(code, "[1, 2, 3]");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("__swap_tmp"),
        "Ascending (j, j+1) swap should use __swap_tmp, got:\n{}", rust);
    assert!(!rust.contains(".swap("),
        "Ascending swap should not use .swap(), got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_conditional_swap_arbitrary_indices() {
    // Pattern (i, j) with arbitrary non-adjacent indices (heapsort-style).
    let code = r#"## Main
Let mutable arr be [1, 5, 3, 4, 2].
Let a be item 2 of arr.
Let b be item 5 of arr.
If a is greater than b:
    Set item 2 of arr to b.
    Set item 5 of arr to a.
Show arr.
"#;
    assert_exact_output(code, "[1, 2, 3, 4, 5]");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("__swap_tmp"),
        "Arbitrary index conditional swap should use __swap_tmp, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_conditional_swap_less_than() {
    // Conditional swap with < operator instead of >.
    let code = r#"## Main
Let mutable arr be [2, 1].
Let a be item 1 of arr.
Let b be item 2 of arr.
If a is less than b:
    Set item 1 of arr to b.
    Set item 2 of arr to a.
Show arr.
"#;
    // a=2, b=1, 2 < 1 is false, so no swap
    assert_exact_output(code, "[2, 1]");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("__swap_tmp"),
        "Less-than conditional swap should use __swap_tmp, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_unconditional_swap_variable_indices() {
    // Unconditional swap with variable indices (not just literals).
    let code = r#"## Main
Let mutable arr be [10, 20, 30, 40, 50].
Let i be 2.
Let j be 4.
Let tmp be item i of arr.
Set item i of arr to item j of arr.
Set item j of arr to tmp.
Show arr.
"#;
    assert_exact_output(code, "[10, 40, 30, 20, 50]");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("__swap_tmp"),
        "Unconditional swap with variable indices should use __swap_tmp, got:\n{}", rust);
    assert!(!rust.contains(".swap("),
        "Should not use .swap(), got:\n{}", rust);
}

// =============================================================================
// Optimization: Optimization Annotations (Phase 6)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_no_memo_annotation() {
    // ## No Memo should prevent memoization on a recursive function.
    // Without ## No Memo, fib would be memoized. With it, it shouldn't be.
    let code = r#"## No Memo
## To fib (n: Int) -> Int:
    If n is at most 1:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Show fib(10).
"#;
    assert_exact_output(code, "55");
    let rust = compile_to_rust(code).unwrap();
    // Should NOT contain memoization infrastructure
    assert!(!rust.contains("thread_local!"),
        "## No Memo should prevent memoization, got:\n{}", rust);
    assert!(!rust.contains("MEMO_"),
        "## No Memo should prevent memoization cache, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_memo_without_annotation() {
    // Without ## No Memo, fib should still be memoized normally.
    let code = r#"## To fib (n: Int) -> Int:
    If n is at most 1:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Show fib(10).
"#;
    assert_exact_output(code, "55");
    let rust = compile_to_rust(code).unwrap();
    // Should contain memoization since no annotation prevents it
    assert!(rust.contains("thread_local!") || rust.contains("MEMO_"),
        "Fib without ## No Memo should be memoized, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_no_tco_annotation() {
    // ## No TCO should prevent tail-call optimization.
    let code = r#"## No TCO
## To countDown (n: Int) -> Int:
    If n is at most 0:
        Return 0.
    Return countDown(n - 1).

## Main
Show countDown(100).
"#;
    assert_exact_output(code, "0");
    let rust = compile_to_rust(code).unwrap();
    // Without TCO, the function should use regular recursion (not a loop)
    assert!(!rust.contains("loop {"),
        "## No TCO should prevent tail-call loop conversion, got:\n{}", rust);
    assert!(rust.contains("countDown("),
        "Should still have recursive call, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_no_optimize_annotation() {
    // ## No Optimize should disable ALL optimizations (memo, TCO, peephole, borrow).
    let code = r#"## No Optimize
## To fib (n: Int) -> Int:
    If n is at most 1:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Show fib(10).
"#;
    assert_exact_output(code, "55");
    let rust = compile_to_rust(code).unwrap();
    // No Optimize implies No Memo
    assert!(!rust.contains("thread_local!"),
        "## No Optimize should prevent memoization, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_annotation_only_affects_annotated_function() {
    // ## No Memo on one function should not affect other functions.
    let code = r#"## No Memo
## To makeCheck (depth: Int) -> Int:
    If depth equals 0:
        Return 1.
    Return 1 + makeCheck(depth - 1) + makeCheck(depth - 1).

## To fib (n: Int) -> Int:
    If n is at most 1:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Show makeCheck(3).
Show fib(10).
"#;
    assert_exact_output(code, "15\n55");
    let rust = compile_to_rust(code).unwrap();
    // fib should still be memoized (no annotation on it)
    assert!(rust.contains("MEMO_") || rust.contains("thread_local!"),
        "Fib should still be memoized when only makeCheck has ## No Memo, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_no_borrow_annotation() {
    // ## No Borrow should prevent readonly borrow optimization.
    // Without it, a function that only reads its Seq param gets &[T].
    // With ## No Borrow, it should take owned Vec<T>.
    let code = r#"## No Borrow
## To sumAll (items: Seq of Int) -> Int:
    Let mutable total be 0.
    Let mutable i be 1.
    While i is at most length of items:
        Set total to total + item i of items.
        Set i to i + 1.
    Return total.

## Main
Let data be [1, 2, 3, 4, 5].
Show sumAll(data).
"#;
    assert_exact_output(code, "15");
    let rust = compile_to_rust(code).unwrap();
    // Should NOT use borrow param — takes owned Vec
    assert!(!rust.contains("&[i64]"),
        "## No Borrow should prevent borrow optimization, got:\n{}", rust);
    assert!(rust.contains("Vec<i64>") || rust.contains("items: Vec<i64>"),
        "Should take owned Vec<i64> with ## No Borrow, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_multiple_annotations() {
    // Multiple annotations can be stacked on a single function.
    let code = r#"## No Memo
## No TCO
## To helper (n: Int) -> Int:
    If n is at most 0:
        Return 0.
    Return helper(n - 1).

## Main
Show helper(10).
"#;
    assert_exact_output(code, "0");
    let rust = compile_to_rust(code).unwrap();
    // Both memo and TCO should be disabled
    assert!(!rust.contains("thread_local!"),
        "## No Memo should prevent memoization with stacked annotations, got:\n{}", rust);
    assert!(!rust.contains("loop {"),
        "## No TCO should prevent loop conversion with stacked annotations, got:\n{}", rust);
}

// =============================================================================
// Optimization: Mutable Borrow Parameters (&mut [T])
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_mut_borrow_param_correctness() {
    // Function that takes a Seq, mutates elements via SetIndex, and returns it.
    // Should produce correct results regardless of codegen strategy.
    assert_exact_output(
        r#"## To swapItems (arr: Seq of Int) and (i: Int) and (j: Int) -> Seq of Int:
    Let tmp be item i of arr.
    Set item i of arr to item j of arr.
    Set item j of arr to tmp.
    Return arr.

## Main
Let mutable data be [10, 20, 30, 40, 50].
Set data to swapItems(data, 2, 4).
Show data.
"#,
        "[10, 40, 30, 20, 50]",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_mut_borrow_param_codegen() {
    // When a function takes a Seq, only mutates elements (SetIndex),
    // and returns the same Seq, codegen should use &mut [T] parameter
    // instead of the move-return ownership pattern.
    let code = r#"## To setFirst (arr: Seq of Int) and (val: Int) -> Seq of Int:
    Set item 1 of arr to val.
    Return arr.

## Main
Let mutable data be [10, 20, 30].
Set data to setFirst(data, 99).
Show data.
"#;
    assert_exact_output(code, "[99, 20, 30]");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("&mut [i64]"),
        "Element-only mutation should use &mut [T] param, got:\n{}", rust);
    assert!(!rust.contains("fn setFirst(arr: Vec<i64>"),
        "Should NOT take owned Vec, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_mut_borrow_param_no_structural_change() {
    // If a function Pushes to the vec (structural change), it should NOT
    // use &mut [T] — it needs ownership to resize.
    let code = r#"## To addItem (arr: Seq of Int) and (val: Int) -> Seq of Int:
    Push val to arr.
    Return arr.

## Main
Let mutable data be [10, 20].
Set data to addItem(data, 30).
Show data.
"#;
    assert_exact_output(code, "[10, 20, 30]");
    let rust = compile_to_rust(code).unwrap();
    assert!(!rust.contains("&mut [i64]"),
        "Push function should NOT use &mut [T], got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_mut_borrow_param_siftdown_pattern() {
    // Heap sort siftDown pattern: takes arr, does SetIndex + swap, returns arr.
    // This is the critical pattern for heap_sort benchmark.
    let code = r#"## To siftDown (arr: Seq of Int) and (start: Int) and (endIdx: Int) -> Seq of Int:
    Let mutable root be start.
    While (2 * root + 1) is at most endIdx:
        Let mutable child be 2 * root + 1.
        If (child + 1) is at most endIdx:
            If item (child + 2) of arr is greater than item (child + 1) of arr:
                Set child to child + 1.
        If item (child + 1) of arr is greater than item (root + 1) of arr:
            Let tmp be item (root + 1) of arr.
            Set item (root + 1) of arr to item (child + 1) of arr.
            Set item (child + 1) of arr to tmp.
            Set root to child.
        Otherwise:
            Return arr.
    Return arr.

## Main
Let mutable data be [1, 5, 3, 4, 2].
Set data to siftDown(data, 0, 4).
Show item 1 of data.
"#;
    assert_exact_output(code, "5");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("&mut [i64]"),
        "siftDown should use &mut [T] for element-only mutation, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_mut_borrow_param_multiple_calls() {
    // Multiple calls to the same &mut borrow function should work.
    let code = r#"## To setAt (arr: Seq of Int) and (idx: Int) and (val: Int) -> Seq of Int:
    Set item idx of arr to val.
    Return arr.

## Main
Let mutable data be [0, 0, 0].
Set data to setAt(data, 1, 10).
Set data to setAt(data, 2, 20).
Set data to setAt(data, 3, 30).
Show data.
"#;
    assert_exact_output(code, "[10, 20, 30]");
}

// =============================================================================
// Optimization: 0-Based Loop Counter Normalization
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_zero_based_loop_correctness() {
    // A simple loop from 1 to N accessing array elements by counter.
    // Verify correct results with any counter normalization applied.
    assert_exact_output(
        r#"## Main
Let data be [10, 20, 30, 40, 50].
Let mutable total be 0.
Let mutable i be 1.
While i is at most 5:
    Set total to total + item i of data.
    Set i to i + 1.
Show total.
"#,
        "150",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_zero_based_loop_codegen() {
    // When a for-range loop starts at 1 and the counter is ONLY used for
    // array indexing, the codegen should normalize to 0-based to eliminate
    // the `(i - 1) as usize` subtraction on every access.
    let code = r#"## Main
Let data be [10, 20, 30, 40, 50].
Let mutable total be 0.
Let mutable i be 1.
While i is at most 5:
    Set total to total + item i of data.
    Set i to i + 1.
Show total.
"#;
    assert_exact_output(code, "150");
    let rust = compile_to_rust(code).unwrap();
    // Should use 0-based range
    assert!(rust.contains("0..5") || rust.contains("0..5_"),
        "Loop starting at 1 with index-only counter should be 0-based, got:\n{}", rust);
    // Should NOT have the (i - 1) subtraction pattern
    assert!(!rust.contains("(i - 1)"),
        "0-based counter should not need (i - 1) subtraction, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_zero_based_loop_setindex() {
    // 0-based normalization should also work for SetIndex operations,
    // when the counter is ONLY used for array indexing (not arithmetic).
    let code = r#"## Main
Let mutable data be [0, 0, 0, 0, 0].
Let values be [10, 20, 30, 40, 50].
Let mutable i be 1.
While i is at most 5:
    Set item i of data to item i of values.
    Set i to i + 1.
Show data.
"#;
    assert_exact_output(code, "[10, 20, 30, 40, 50]");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("0..5") || rust.contains("0..5_"),
        "SetIndex loop should use 0-based range, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_zero_based_not_applied_when_counter_used_in_arithmetic() {
    // If the counter is used in arithmetic (not just array indexing),
    // normalization should NOT be applied since changing the counter's
    // value range would affect the computation.
    let code = r#"## Main
Let mutable total be 0.
Let mutable i be 1.
While i is at most 5:
    Set total to total + i.
    Set i to i + 1.
Show total.
"#;
    assert_exact_output(code, "15");
    let rust = compile_to_rust(code).unwrap();
    // Should still use 1-based range since counter is used in addition
    assert!(rust.contains("1..6") || rust.contains("1..(5 + 1)"),
        "Counter used in arithmetic should keep 1-based range, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_zero_based_not_applied_when_counter_not_starting_at_1() {
    // When the loop counter starts at 0, no normalization needed.
    let code = r#"## Main
Let data be [10, 20, 30].
Let mutable total be 0.
Let mutable i be 0.
While i is less than 3:
    Set total to total + item (i + 1) of data.
    Set i to i + 1.
Show total.
"#;
    assert_exact_output(code, "60");
}

#[test]
fn e2e_opt_zero_based_counter_reused_across_sequential_loops() {
    // Counter `i` is used in two sequential loops.
    // The first loop gets 0-based normalization, but the counter must
    // revert to 1-based semantics for the second loop.
    let code = r#"## Main
Let arr be [10, 20, 30, 40, 50].
Let mutable left be a new Seq of Int.
Let mutable right be a new Seq of Int.
Let mutable i be 1.
While i is at most 3:
    Push item i of arr to left.
    Set i to i + 1.
While i is at most 5:
    Push item i of arr to right.
    Set i to i + 1.
Show left.
Show right.
"#;
    assert_exact_output(code, "[10, 20, 30]\n[40, 50]");
}

// =============================================================================
// Zero-Based with Comparison (Step 4)
// =============================================================================

/// When a 1-based counter is used for BOTH indexing and a simple comparison
/// (where the counter appears as a bare identifier operand), zero-based should
/// still fire. The comparison operand gets (counter + 1) to compensate.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_zero_based_with_comparison_correctness() {
    // Counter `i` starts at 1, used for indexing and in a comparison.
    // Zero-based: for i in 0..5, comparison `(i + 1) > 2` replaces `i > 2`.
    assert_exact_output(
        r#"## Main
Let arr be [10, 20, 30, 40, 50].
Let mutable sum be 0.
Let mutable i be 1.
While i is at most 5:
    If i is greater than 2:
        Set sum to sum + item i of arr.
    Set i to i + 1.
Show sum.
"#,
        "120",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_zero_based_with_comparison_codegen() {
    let code = r#"## Main
Let arr be [10, 20, 30, 40, 50].
Let mutable sum be 0.
Let mutable i be 1.
While i is at most 5:
    If i is greater than 2:
        Set sum to sum + item i of arr.
    Set i to i + 1.
Show sum.
"#;
    assert_exact_output(code, "120");
    let rust = compile_to_rust(code).unwrap();
    // Zero-based should fire: range should be 0..5
    assert!(rust.contains("0..5"),
        "Zero-based with comparison should produce 0..5 range, got:\n{}", rust);
    // The index should use `i as usize` (no -1 subtraction)
    assert!(rust.contains("i as usize") || rust.contains("(i) as usize"),
        "Zero-based should skip -1 subtraction in index, got:\n{}", rust);
}

/// When a counter is used in arithmetic (not just a bare comparison), zero-based
/// should NOT fire for comparisons containing the counter in sub-expressions.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_zero_based_not_applied_when_counter_in_arithmetic_comparison() {
    // `i - 1` in the comparison means the counter is in arithmetic, not a bare comparison.
    let code = r#"## Main
Let arr be [10, 20, 30, 40, 50].
Let mutable sum be 0.
Let mutable i be 1.
While i is at most 5:
    Set sum to sum + item i of arr * (i - 1).
    Set i to i + 1.
Show sum.
"#;
    assert_exact_output(code, "400");
    let rust = compile_to_rust(code).unwrap();
    // Should NOT use 0-based: counter appears in arithmetic `(i - 1)`
    assert!(!rust.contains("0..5"),
        "Counter in arithmetic should block zero-based, got:\n{}", rust);
}

// =============================================================================
// Consume-Alias &mut [T] Detection
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_consume_alias_basic_codegen() {
    // Function takes Seq, consumes into mutable alias, does SetIndex, returns alias.
    // Should detect consume-alias pattern and use &mut [T].
    let code = r#"## To setFirst (arr: Seq of Int) and (val: Int) -> Seq of Int:
    Let mutable result be arr.
    Set item 1 of result to val.
    Return result.

## Main
Let mutable data be [10, 20, 30].
Set data to setFirst(data, 99).
Show data.
"#;
    assert_exact_output(code, "[99, 20, 30]");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("&mut [i64]"),
        "Consume-alias with SetIndex-only should use &mut [T], got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_consume_alias_quicksort_codegen() {
    // Quicksort pattern: consume param into alias, SetIndex mutations,
    // self-recursive calls reassigning alias, return alias.
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
Let mutable arr be [5, 3, 8, 1, 9, 2, 7].
Set arr to qs(arr, 1, 7).
Show arr.
"#;
    assert_exact_output(code, "[1, 2, 3, 5, 7, 8, 9]");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("fn qs(arr: &mut [i64]"),
        "Quicksort consume-alias should use &mut [T], got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_consume_alias_heapsort_codegen() {
    // Heapsort siftDown pattern: consume param into alias, SetIndex in while loop,
    // early returns + final return of alias.
    let code = r#"## To siftDown (arr: Seq of Int, start: Int, end: Int) -> Seq of Int:
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
"#;
    assert_exact_output(code, "5");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("fn sift_down(arr: &mut [i64]") || rust.contains("fn siftDown(arr: &mut [i64]"),
        "Heapsort consume-alias should use &mut [T], got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_consume_alias_not_eligible_push() {
    // Alias has structural mutation (Push) — should NOT use &mut [T].
    let code = r#"## To extend (arr: Seq of Int) and (val: Int) -> Seq of Int:
    Let mutable result be arr.
    Push val to result.
    Return result.

## Main
Let mutable data be [10, 20].
Set data to extend(data, 30).
Show data.
"#;
    assert_exact_output(code, "[10, 20, 30]");
    let rust = compile_to_rust(code).unwrap();
    assert!(!rust.contains("fn extend(arr: &mut [i64]"),
        "Push on alias should NOT use &mut [T], got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_consume_alias_not_eligible_no_setindex() {
    // Alias is consumed but never mutated via SetIndex — no need for &mut.
    // Just reads from the alias and returns it.
    let code = r#"## To identity (arr: Seq of Int) -> Seq of Int:
    Let mutable result be arr.
    Return result.

## Main
Let mutable data be [10, 20, 30].
Set data to identity(data).
Show data.
"#;
    assert_exact_output(code, "[10, 20, 30]");
    let rust = compile_to_rust(code).unwrap();
    assert!(!rust.contains("fn identity(arr: &mut [i64]"),
        "No SetIndex on alias should NOT use &mut [T], got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_consume_alias_full_quicksort() {
    // Full quicksort with larger array to verify correctness at scale.
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
Let mutable arr be [9, 1, 7, 3, 5, 2, 8, 4, 6, 10].
Set arr to qs(arr, 1, 10).
Show arr.
"#,
        "[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_consume_alias_full_heapsort() {
    // Full heapsort to verify correctness.
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

## To heapSort (arr: Seq of Int) -> Seq of Int:
    Let n be length of arr.
    Let mutable result be arr.
    Let mutable start be n / 2 - 1.
    While start is at least 0:
        Set result to siftDown(result, start, n - 1).
        Set start to start - 1.
    Let mutable end be n - 1.
    While end is greater than 0:
        Let tmp be item 1 of result.
        Set item 1 of result to item (end + 1) of result.
        Set item (end + 1) of result to tmp.
        Set end to end - 1.
        Set result to siftDown(result, 0, end).
    Return result.

## Main
Let mutable arr be [9, 1, 7, 3, 5, 2, 8, 4, 6, 10].
Set arr to heapSort(arr).
Show arr.
"#,
        "[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]",
    );
}

// =============================================================================
// Relaxed Seq-From-Slice Pattern
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_seq_from_slice_with_intervening() {
    // Seq creation followed by intervening statement, then counter + While push loop.
    // The seq-from-slice optimization should fire despite the gap.
    let code = r#"## Main
Let arr be [10, 20, 30, 40, 50].
Let mutable left be a new Seq of Int.
Let dummy be 42.
Let mutable i be 1.
While i is at most 3:
    Push item i of arr to left.
    Set i to i + 1.
Show left.
"#;
    assert_exact_output(code, "[10, 20, 30]");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains(".to_vec()"),
        "Seq-from-slice should fire with intervening stmt, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_seq_from_slice_two_halves() {
    // Two-Seq split pattern: left and right halves from same array.
    // Both should use .to_vec() slice copies.
    let code = r#"## Main
Let arr be [10, 20, 30, 40, 50].
Let mid be 3.
Let n be 5.
Let mutable left be a new Seq of Int.
Let mutable right be a new Seq of Int.
Let mutable i be 1.
While i is at most mid:
    Push item i of arr to left.
    Set i to i + 1.
While i is at most n:
    Push item i of arr to right.
    Set i to i + 1.
Show left.
Show right.
"#;
    assert_exact_output(code, "[10, 20, 30]\n[40, 50]");
    let rust = compile_to_rust(code).unwrap();
    // At minimum, the first (left) half should use .to_vec()
    assert!(rust.contains(".to_vec()"),
        "Two-seq split should use .to_vec() slice, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_seq_from_slice_mergesort_correctness() {
    // Full mergesort using the split pattern.
    assert_exact_output(
        r#"## To merge (left: Seq of Int, right: Seq of Int) -> Seq of Int:
    Let mutable result be a new Seq of Int.
    Let mutable i be 1.
    Let mutable j be 1.
    Let nl be length of left.
    Let nr be length of right.
    While i is at most nl:
        If j is greater than nr:
            Push item i of left to result.
            Set i to i + 1.
        Otherwise:
            If item i of left is at most item j of right:
                Push item i of left to result.
                Set i to i + 1.
            Otherwise:
                Push item j of right to result.
                Set j to j + 1.
    While j is at most nr:
        Push item j of right to result.
        Set j to j + 1.
    Return result.

## To mergeSort (arr: Seq of Int) -> Seq of Int:
    Let n be length of arr.
    If n is at most 1:
        Return arr.
    Let mid be n / 2.
    Let mutable left be a new Seq of Int.
    Let mutable right be a new Seq of Int.
    Let mutable i be 1.
    While i is at most mid:
        Push item i of arr to left.
        Set i to i + 1.
    While i is at most n:
        Push item i of arr to right.
        Set i to i + 1.
    Set left to mergeSort(left).
    Set right to mergeSort(right).
    Return merge(left, right).

## Main
Let mutable arr be [9, 1, 7, 3, 5, 2, 8, 4, 6, 10].
Set arr to mergeSort(arr).
Show arr.
"#,
        "[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]",
    );
}

// =============================================================================
// Self-Append push_str/push Optimization
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_push_str_literal() {
    // Single string literal append should use push_str instead of write!
    let code = r#"## Main
Let mutable text be "hello".
Set text to text + " world".
Show text.
"#;
    assert_exact_output(code, "hello world");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("push_str(\" world\")"),
        "Single literal append should use push_str, got:\n{}", rust);
    assert!(!rust.contains("write!("),
        "Single literal append should NOT use write!, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_push_single_char() {
    // Single char literal append should use push instead of push_str
    let code = r#"## Main
Let mutable text be "abc".
Set text to text + "x".
Show text.
"#;
    assert_exact_output(code, "abcx");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("push('x')"),
        "Single char append should use push('c'), got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_push_str_variable() {
    // Variable append should use push_str(&var)
    let code = r#"## Main
Let mutable text be "hello".
Let suffix be " world".
Set text to text + suffix.
Show text.
"#;
    assert_exact_output(code, "hello world");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("push_str(&suffix)") || rust.contains("push_str(&*suffix"),
        "Variable append should use push_str, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_push_str_multi_operand_keeps_write() {
    // Multi-operand append should still use write!
    let code = r#"## Main
Let mutable text be "a".
Let b be "b".
Let c be "c".
Set text to text + b + c.
Show text.
"#;
    assert_exact_output(code, "abc");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("write!("),
        "Multi-operand append should use write!, got:\n{}", rust);
}

// =============================================================================
// WithCapacity for Complex Push Loops
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_with_capacity_conditional_push() {
    // All-branch push: every If/Otherwise path pushes to result.
    // Push count is deterministic (= loop count), so with_capacity is valid.
    let code = r#"## Main
Let arr be [5, 3, 8, 1, 9].
Let mutable result be a new Seq of Int.
Let mutable i be 1.
While i is at most 5:
    If item i of arr is greater than 4:
        Push item i of arr to result.
    Otherwise:
        Push 0 to result.
    Set i to i + 1.
Show result.
"#;
    assert_exact_output(code, "[5, 0, 8, 0, 9]");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("Vec::with_capacity("),
        "All-branch push loop should use Vec::with_capacity, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_mergesort_with_capacity() {
    // Full mergesort with merge function — result should use with_capacity.
    assert_exact_output(
        r#"## To merge (left: Seq of Int, right: Seq of Int) -> Seq of Int:
    Let mutable result be a new Seq of Int.
    Let mutable i be 1.
    Let mutable j be 1.
    Let nl be length of left.
    Let nr be length of right.
    While i is at most nl:
        If j is greater than nr:
            Push item i of left to result.
            Set i to i + 1.
        Otherwise:
            If item i of left is at most item j of right:
                Push item i of left to result.
                Set i to i + 1.
            Otherwise:
                Push item j of right to result.
                Set j to j + 1.
    While j is at most nr:
        Push item j of right to result.
        Set j to j + 1.
    Return result.

## To mergeSort (arr: Seq of Int) -> Seq of Int:
    Let n be length of arr.
    If n is at most 1:
        Return arr.
    Let mid be n / 2.
    Let mutable left be a new Seq of Int.
    Let mutable right be a new Seq of Int.
    Let mutable i be 1.
    While i is at most mid:
        Push item i of arr to left.
        Set i to i + 1.
    While i is at most n:
        Push item i of arr to right.
        Set i to i + 1.
    Set left to mergeSort(left).
    Set right to mergeSort(right).
    Return merge(left, right).

## Main
Let mutable arr be [9, 1, 7, 3, 5, 2, 8, 4, 6, 10].
Set arr to mergeSort(arr).
Show arr.
"#,
        "[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]",
    );
}

// =============================================================================
// Single-char text variable → u8 byte
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_single_char_var_basic() {
    assert_exact_output(
        r#"## Main
Let mutable text be "".
Let mutable ch be "a".
Set ch to "b".
Set text to text + ch.
Show text.
"#,
        "b",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_single_char_var_loop_build() {
    assert_exact_output(
        r#"## Main
Let mutable text be "".
Let mutable pos be 0.
While pos is less than 10:
    Let mutable ch be "a".
    If pos % 5 equals 1:
        Set ch to "b".
    If pos % 5 equals 2:
        Set ch to "c".
    If pos % 5 equals 3:
        Set ch to "d".
    If pos % 5 equals 4:
        Set ch to "e".
    Set text to text + ch.
    Set pos to pos + 1.
Show text.
"#,
        "abcdeabcde",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_single_char_var_show() {
    assert_exact_output(
        r#"## Main
Let mutable ch be "x".
Set ch to "y".
Show ch.
"#,
        "y",
    );
}

// =============================================================================
// String with_capacity from loop
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_string_with_capacity_loop() {
    assert_exact_output(
        r#"## Main
Let n be 100.
Let mutable text be "".
Let mutable i be 0.
While i is less than n:
    Set text to text + "x".
    Set i to i + 1.
Show length of text.
"#,
        "100",
    );
}

// =============================================================================
// Bare Slice Push Pattern (extend_from_slice)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_bare_slice_push_basic() {
    assert_exact_output(
        r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## To split (arr: Seq of Int, start: Int, end: Int) -> Seq of Int:
    Let mutable result be a new Seq of Int.
    Let mutable i be start.
    While i is at most end:
        Push item i of arr to result.
        Set i to i + 1.
    Return result.

## Main
Let items be [10, 20, 30, 40, 50].
Let half be split(items, 1, 3).
Show item 1 of half.
Show item 2 of half.
Show item 3 of half.
"#,
        "10\n20\n30",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_bare_slice_push_exclusive() {
    assert_exact_output(
        r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## To split (arr: Seq of Int, start: Int, end: Int) -> Seq of Int:
    Let mutable result be a new Seq of Int.
    Let mutable i be start.
    While i is less than end:
        Push item i of arr to result.
        Set i to i + 1.
    Return result.

## Main
Let items be [10, 20, 30, 40, 50].
Let half be split(items, 1, 3).
Show item 1 of half.
Show item 2 of half.
Show length of half.
"#,
        "10\n20\n2",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_bare_slice_push_counter_after() {
    assert_exact_output(
        r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## To split (arr: Seq of Int, mid: Int) -> Seq of Int:
    Let mutable result be a new Seq of Int.
    Let mutable i be 1.
    While i is at most mid:
        Push item i of arr to result.
        Set i to i + 1.
    Show i.
    Return result.

## Main
Let items be [10, 20, 30, 40, 50].
Let half be split(items, 3).
Show length of half.
"#,
        "4\n3",
    );
}

// =============================================================================
// Merge Vec Pre-allocation (capacity from source Vec lengths)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_merge_capacity_codegen() {
    let code = r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## To merge (left: Seq of Int, right: Seq of Int) -> Seq of Int:
    Let mutable result be a new Seq of Int.
    Let mutable i be 1.
    Let mutable j be 1.
    While i is at most length of left:
        If j is greater than length of right:
            Push item i of left to result.
            Set i to i + 1.
        Otherwise if item i of left is at most item j of right:
            Push item i of left to result.
            Set i to i + 1.
        Otherwise:
            Push item j of right to result.
            Set j to j + 1.
    While j is at most length of right:
        Push item j of right to result.
        Set j to j + 1.
    Return result.

## Main
Let a be [1, 3, 5].
Let b be [2, 4, 6].
Show merge(a, b).
"#;
    assert_exact_output(code, "[1, 2, 3, 4, 5, 6]");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("Vec::with_capacity("),
        "Merge result should use Vec::with_capacity, got:\n{}", rust);
    assert!(rust.contains("left.len()") && rust.contains("right.len()"),
        "Capacity should reference left.len() + right.len(), got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_merge_capacity_correctness() {
    assert_exact_output(
        r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## To merge (left: Seq of Int, right: Seq of Int) -> Seq of Int:
    Let mutable result be a new Seq of Int.
    Let mutable i be 1.
    Let mutable j be 1.
    While i is at most length of left:
        If j is greater than length of right:
            Push item i of left to result.
            Set i to i + 1.
        Otherwise if item i of left is at most item j of right:
            Push item i of left to result.
            Set i to i + 1.
        Otherwise:
            Push item j of right to result.
            Set j to j + 1.
    While j is at most length of right:
        Push item j of right to result.
        Set j to j + 1.
    Return result.

## To mergesort (arr: Seq of Int) -> Seq of Int:
    Let n be length of arr.
    If n is at most 1:
        Return arr.
    Let mid be n / 2.
    Let mutable left be a new Seq of Int.
    Let mutable i be 1.
    While i is at most mid:
        Push item i of arr to left.
        Set i to i + 1.
    Let mutable right be a new Seq of Int.
    While i is at most n:
        Push item i of arr to right.
        Set i to i + 1.
    Let sortedLeft be mergesort(left).
    Let sortedRight be mergesort(right).
    Return merge(sortedLeft, sortedRight).

## Main
Let items be [5, 3, 8, 1, 9, 2, 7, 4, 6].
Let sorted be mergesort(items).
Let mutable k be 1.
While k is at most length of sorted:
    Show item k of sorted.
    Set k to k + 1.
"#,
        "1\n2\n3\n4\n5\n6\n7\n8\n9",
    );
}

// =============================================================================
// Loop Bounds Hoisting
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_loop_bounds_hoisted_codegen() {
    // This merge function has counters modified inside branches,
    // so the for-range pattern won't fire — it stays as a while loop.
    // The `length of left` should be hoisted out of the condition.
    let code = r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## To merge (left: Seq of Int, right: Seq of Int) -> Seq of Int:
    Let mutable result be a new Seq of Int.
    Let mutable i be 1.
    Let mutable j be 1.
    While i is at most length of left:
        If j is greater than length of right:
            Push item i of left to result.
            Set i to i + 1.
        Otherwise if item i of left is at most item j of right:
            Push item i of left to result.
            Set i to i + 1.
        Otherwise:
            Push item j of right to result.
            Set j to j + 1.
    While j is at most length of right:
        Push item j of right to result.
        Set j to j + 1.
    Return result.

## Main
Show merge([1, 3, 5], [2, 4, 6]).
"#;
    assert_exact_output(code, "[1, 2, 3, 4, 5, 6]");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("let left_len"),
        "Unmodified collection length should be hoisted, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_loop_bounds_not_hoisted_when_modified() {
    // When a collection IS modified in the loop body (pushed to),
    // its length should NOT be hoisted out of the while condition.
    // Unmodified collections in the same loop SHOULD still be hoisted.
    let code = r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## To collect (left: Seq of Int, right: Seq of Int) -> Seq of Int:
    Let mutable result be a new Seq of Int.
    Let mutable i be 1.
    Let mutable j be 1.
    While i is at most length of left:
        If j is greater than length of right:
            Push item i of left to result.
            Set i to i + 1.
        Otherwise if item i of left is at most item j of right:
            Push item i of left to result.
            Set i to i + 1.
        Otherwise:
            Push item j of right to result.
            Set j to j + 1.
    Return result.

## Main
Show collect([1, 3, 5], [2, 4]).
"#;
    assert_exact_output(code, "[1, 2, 3, 4, 5]");
    let rust = compile_to_rust(code).unwrap();
    // `left` and `right` are NOT modified — their lengths SHOULD be hoisted.
    // `result` IS modified (pushed to) — if it appeared in a condition, it should NOT be hoisted.
    assert!(rust.contains("let left_len"),
        "Unmodified left length should be hoisted, got:\n{}", rust);
    assert!(rust.contains("let right_len"),
        "Unmodified right length should be hoisted, got:\n{}", rust);
}

// =============================================================================
// Continuation Slice Pattern (shared counter, no re-init)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_continuation_slice_codegen() {
    let code = r#"## To split (arr: Seq of Int) -> Seq of Int:
    Let n be length of arr.
    Let mid be n / 2.
    Let mutable left be a new Seq of Int.
    Let mutable i be 1.
    While i is at most mid:
        Push item i of arr to left.
        Set i to i + 1.
    Let mutable right be a new Seq of Int.
    While i is at most n:
        Push item i of arr to right.
        Set i to i + 1.
    Return right.

## Main
Show split([10, 20, 30, 40, 50, 60]).
"#;
    assert_exact_output(code, "[40, 50, 60]");
    let rust = compile_to_rust(code).unwrap();
    let to_vec_count = rust.matches(".to_vec()").count();
    assert!(to_vec_count >= 2,
        "Both left (standard) and right (continuation) slices should use .to_vec(), found {} occurrences, got:\n{}",
        to_vec_count, rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_continuation_slice_correctness() {
    assert_exact_output(
        r#"## To splitAndJoin (arr: Seq of Int) -> Seq of Int:
    Let n be length of arr.
    Let mid be n / 2.
    Let mutable left be a new Seq of Int.
    Let mutable i be 1.
    While i is at most mid:
        Push item i of arr to left.
        Set i to i + 1.
    Let mutable right be a new Seq of Int.
    While i is at most n:
        Push item i of arr to right.
        Set i to i + 1.
    Let mutable result be a new Seq of Int.
    Let mutable j be 1.
    While j is at most length of right:
        Push item j of right to result.
        Set j to j + 1.
    Set j to 1.
    While j is at most length of left:
        Push item j of left to result.
        Set j to j + 1.
    Return result.

## Main
Show splitAndJoin([1, 2, 3, 4, 5, 6, 7, 8]).
"#,
        "[5, 6, 7, 8, 1, 2, 3, 4]",
    );
}

// =============================================================================
// Loop-Scoped Buffer Reuse (mem::swap + clear)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_buffer_reuse_codegen() {
    let code = r#"## Main
Let n be 5.
Let mutable outer be a new Seq of Int.
Push 0 to outer.
Let mutable i be 0.
While i is less than n:
    Let mutable inner be a new Seq of Int.
    Let mutable j be 0.
    While j is less than 3:
        Push (i + 1) * (j + 1) to inner.
        Set j to j + 1.
    Set outer to inner.
    Set i to i + 1.
Show outer.
"#;
    assert_exact_output(code, "[5, 10, 15]");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("mem::swap") || rust.contains("std::mem::swap"),
        "Buffer reuse should emit mem::swap, got:\n{}", rust);
    assert!(rust.contains(".clear()"),
        "Buffer reuse should emit .clear() instead of new allocation, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_buffer_reuse_correctness() {
    assert_exact_output(
        r#"## Main
Let n be 4.
Let mutable prev be a new Seq of Int.
Let mutable k be 0.
While k is less than 3:
    Push 1 to prev.
    Set k to k + 1.
Let mutable i be 0.
While i is less than n:
    Let mutable curr be a new Seq of Int.
    Let mutable j be 0.
    While j is less than 3:
        Push item (j + 1) of prev + (i + 1) to curr.
        Set j to j + 1.
    Set prev to curr.
    Set i to i + 1.
Show prev.
"#,
        "[11, 11, 11]",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_buffer_reuse_different_types_no_fire() {
    let code = r#"## Main
Let mutable outer be a new Seq of Int.
Push 0 to outer.
Let mutable i be 0.
While i is less than 3:
    Let mutable inner be a new Seq of Float.
    Let mutable j be 0.
    While j is less than 2:
        Push 1.5 to inner.
        Set j to j + 1.
    Set i to i + 1.
Show outer.
"#;
    assert_exact_output(code, "[0]");
    let rust = compile_to_rust(code).unwrap();
    assert!(!rust.contains("mem::swap") && !rust.contains("std::mem::swap"),
        "Buffer reuse should NOT fire for different element types, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_buffer_reuse_inner_escapes_no_fire() {
    // When the Set is NOT the last meaningful statement before the counter
    // increment (there's a Push to outer after the Set), the pattern must NOT fire.
    let code = r#"## Main
Let mutable outer be a new Seq of Int.
Let mutable i be 0.
While i is less than 3:
    Let mutable inner be a new Seq of Int.
    Let mutable j be 0.
    While j is less than 2:
        Push j to inner.
        Set j to j + 1.
    Set outer to inner.
    Push 99 to outer.
    Set i to i + 1.
Show outer.
"#;
    assert_exact_output(code, "[0, 1, 99]");
    let rust = compile_to_rust(code).unwrap();
    assert!(!rust.contains("mem::swap") && !rust.contains("std::mem::swap"),
        "Buffer reuse should NOT fire when Set is not last meaningful stmt, got:\n{}", rust);
}

// =============================================================================
// Double-Buffer DP Pattern (mem::swap for pre-allocated buffers)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_double_buffer_correctness() {
    let code = r#"## Main
Let n be 3.
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
While i is less than n:
    Let mutable w be 0.
    While w is at most capacity:
        Set item (w + 1) of curr to item (w + 1) of prev + 1.
        Set w to w + 1.
    Set prev to curr.
    Set i to i + 1.
Show item (capacity + 1) of prev.
"#;
    assert_exact_output(code, "3");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_double_buffer_codegen() {
    let code = r#"## Main
Let n be 3.
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
While i is less than n:
    Let mutable w be 0.
    While w is at most capacity:
        Set item (w + 1) of curr to item (w + 1) of prev + 1.
        Set w to w + 1.
    Set prev to curr.
    Set i to i + 1.
Show item (capacity + 1) of prev.
"#;
    assert_exact_output(code, "3");
    let rust = compile_to_rust(code).unwrap();
    assert!(rust.contains("mem::swap") || rust.contains("std::mem::swap"),
        "Double-buffer should emit mem::swap, got:\n{}", rust);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_double_buffer_knapsack_correctness() {
    assert_exact_output(
        r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
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
Let mutable curr be a new Seq of Int.
Set i to 0.
While i is less than cols:
    Push 0 to prev.
    Push 0 to curr.
    Set i to i + 1.
Set i to 0.
While i is less than n:
    Let wi be item (i + 1) of weights.
    Let vi be item (i + 1) of vals.
    Let mutable w be 0.
    While w is at most capacity:
        Set item (w + 1) of curr to item (w + 1) of prev.
        If w is at least wi:
            Let take be item (w - wi + 1) of prev + vi.
            If take is greater than item (w + 1) of curr:
                Set item (w + 1) of curr to take.
        Set w to w + 1.
    Set prev to curr.
    Set i to i + 1.
Show item (capacity + 1) of prev.
"#,
        "47",
    );
}

// =============================================================================
// Drain Tail (merge loop bulk copy)
// =============================================================================

/// When a while loop body starts with an If whose then-branch is a simple
/// sequential drain (Push item counter of array to target; Set counter to counter + 1),
/// and the If condition is loop-invariant within that branch, the optimization should
/// emit extend_from_slice + break instead.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_drain_tail_merge_correctness() {
    assert_exact_output(
        r#"## To merge (left: Seq of Int, right: Seq of Int) -> Seq of Int:
    Let mutable result be a new Seq of Int.
    Let mutable li be 1.
    Let mutable ri be 1.
    While li is at most length of left:
        If ri is greater than length of right:
            Push item li of left to result.
            Set li to li + 1.
        Otherwise:
            If item li of left is at most item ri of right:
                Push item li of left to result.
                Set li to li + 1.
            Otherwise:
                Push item ri of right to result.
                Set ri to ri + 1.
    While ri is at most length of right:
        Push item ri of right to result.
        Set ri to ri + 1.
    Return result.

## Main
Let left be [1, 3, 5, 7, 9].
Let right be [2, 4, 6].
Let merged be merge(left, right).
Let mutable i be 1.
Let mutable out be "".
While i is at most length of merged:
    If i is greater than 1:
        Set out to out + " ".
    Set out to out + item i of merged.
    Set i to i + 1.
Show out.
"#,
        "1 2 3 4 5 6 7 9",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_drain_tail_codegen() {
    let code = r#"## To merge (left: Seq of Int, right: Seq of Int) -> Seq of Int:
    Let mutable result be a new Seq of Int.
    Let mutable li be 1.
    Let mutable ri be 1.
    While li is at most length of left:
        If ri is greater than length of right:
            Push item li of left to result.
            Set li to li + 1.
        Otherwise:
            If item li of left is at most item ri of right:
                Push item li of left to result.
                Set li to li + 1.
            Otherwise:
                Push item ri of right to result.
                Set ri to ri + 1.
    While ri is at most length of right:
        Push item ri of right to result.
        Set ri to ri + 1.
    Return result.

## Main
Let left be [1, 3, 5, 7, 9].
Let right be [2, 4, 6].
Let merged be merge(left, right).
Show item 1 of merged.
"#;
    assert_exact_output(code, "1");
    let rust = compile_to_rust(code).unwrap();
    // The drain-tail optimization should emit extend_from_slice + break
    // inside the while loop's If branch, replacing the element-by-element push.
    // Without this optimization, the If-then branch has push() + increment.
    assert!(rust.contains("break"),
        "Drain tail should emit break (inside while loop), got:\n{}", rust);
    // The while body should NOT have a push for the drain side
    // after the optimization fires — it should be replaced with extend_from_slice.
    // Check that within the while block, we have extend_from_slice.
    let while_block = rust.split("while (li").nth(1).expect("should have while loop");
    assert!(while_block.contains("extend_from_slice"),
        "While loop body should have extend_from_slice for drain tail, got:\n{}", while_block);
}

// =============================================================================
// Bare Slice Push (existing tests below)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_bare_slice_push_mergesort() {
    assert_exact_output(
        r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## To merge (left: Seq of Int, right: Seq of Int) -> Seq of Int:
    Let mutable result be a new Seq of Int.
    Let mutable i be 1.
    Let mutable j be 1.
    While i is at most length of left:
        If j is greater than length of right:
            Push item i of left to result.
            Set i to i + 1.
        Otherwise if item i of left is at most item j of right:
            Push item i of left to result.
            Set i to i + 1.
        Otherwise:
            Push item j of right to result.
            Set j to j + 1.
    While j is at most length of right:
        Push item j of right to result.
        Set j to j + 1.
    Return result.

## To mergesort (arr: Seq of Int) -> Seq of Int:
    Let n be length of arr.
    If n is at most 1:
        Return arr.
    Let mid be n / 2.
    Let mutable left be a new Seq of Int.
    Let mutable i be 1.
    While i is at most mid:
        Push item i of arr to left.
        Set i to i + 1.
    Let mutable right be a new Seq of Int.
    While i is at most n:
        Push item i of arr to right.
        Set i to i + 1.
    Let sortedLeft be mergesort(left).
    Let sortedRight be mergesort(right).
    Return merge(sortedLeft, sortedRight).

## Main
Let items be [5, 3, 8, 1, 9, 2, 7, 4, 6].
Let sorted be mergesort(items).
Let mutable k be 1.
While k is at most length of sorted:
    Show item k of sorted.
    Set k to k + 1.
"#,
        "1\n2\n3\n4\n5\n6\n7\n8\n9",
    );
}

// =============================================================================
// While-loop bounds check assert hints (OPT-9)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_while_bounds_assert_codegen() {
    // While loop with array indexing should get assert! hint.
    // The intervening `Set total to 0.` prevents for-range conversion.
    let source = r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## To sumArray (arr: Seq of Int) -> Int:
    Let n be length of arr.
    Let mutable total be 0.
    Let mutable i be 1.
    Set total to 0.
    While i is at most n:
        Set total to total + item i of arr.
        Set i to i + 1.
    Return total.

## Main
Let items be [10, 20, 30].
Show sumArray(items).
"#;
    let code = compile_to_rust(source).unwrap();
    // Should have an assert_unchecked hint before the while loop for arr
    assert!(
        code.contains("assert_unchecked") && code.contains(".len()"),
        "Expected assert_unchecked bounds hint before while loop, got:\n{}", code
    );
    // Should still be a while loop (NOT for-range)
    assert!(
        code.contains("while "),
        "Expected a while loop (not for-range), got:\n{}", code
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_while_bounds_assert_correctness() {
    // While loop with assert hint should still produce correct output.
    // Multiple arrays indexed by the same counter.
    assert_exact_output(
        r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## To dotProduct (a: Seq of Int, b: Seq of Int) -> Int:
    Let n be length of a.
    Let mutable total be 0.
    Let mutable i be 1.
    Set total to 0.
    While i is at most n:
        Set total to total + item i of a * item i of b.
        Set i to i + 1.
    Return total.

## Main
Let x be [1, 2, 3].
Let y be [4, 5, 6].
Show dotProduct(x, y).
"#,
        "32",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_while_bounds_no_assert_when_no_indexing() {
    // While loop without array indexing should NOT get assert! hint.
    let source = r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## To countUp (n: Int) -> Int:
    Let mutable total be 0.
    Let mutable i be 1.
    Set total to 0.
    While i is at most n:
        Set total to total + i.
        Set i to i + 1.
    Return total.

## Main
Show countUp(5).
"#;
    let code = compile_to_rust(source).unwrap();
    // Count how many assert! there are — there should be none related to while loop bounds
    // (The function has no array indexing in the while body.)
    let while_pos = code.find("while ").expect("Should have a while loop");
    let code_before_while = &code[..while_pos];
    // No assert! should appear right before this while
    let last_assert = code_before_while.rfind("assert!");
    if let Some(pos) = last_assert {
        // If there is an assert, it must be far from the while (not related)
        let between = &code_before_while[pos..];
        assert!(
            between.contains("for ") || between.len() > 200,
            "Found assert! hint near while loop with no array indexing:\n{}", code
        );
    }
}

// =============================================================================
// FIX-1: assert_unchecked for ZERO-BASED for-range loops
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_assert_unchecked_on_zero_based_loop() {
    // A 1-based counter that is ONLY used for array indexing gets converted to
    // a zero-based for-range loop (0..n). The assert_unchecked hint should
    // STILL be emitted so LLVM can elide bounds checks through the i64→usize cast.
    let source = r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## To sumArray (arr: Seq of Int) -> Int:
    Let n be length of arr.
    Let mutable total be 0.
    Let mutable i be 1.
    While i is at most n:
        Set total to total + item i of arr.
        Set i to i + 1.
    Return total.

## Main
Let items be [10, 20, 30].
Show sumArray(items).
"#;
    let code = compile_to_rust(source).unwrap();
    // This loop should be converted to zero-based (for i in 0..n)
    assert!(
        code.contains("for ") && code.contains("0.."),
        "Expected zero-based for-range loop, got:\n{}", code
    );
    // AND it should have assert_unchecked for the indexed array
    assert!(
        code.contains("assert_unchecked"),
        "Expected assert_unchecked hint on zero-based loop with array indexing, got:\n{}", code
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_assert_unchecked_zero_based_correctness() {
    // Ensure the zero-based loop with assert_unchecked produces correct output.
    assert_exact_output(
        r#"## Main
Let items be [10, 20, 30, 40, 50].
Let mutable total be 0.
Let mutable i be 1.
While i is at most length of items:
    Set total to total + item i of items.
    Set i to i + 1.
Show total.
"#,
        "150",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_assert_unchecked_zero_based_multiple_arrays() {
    // Zero-based loop indexing multiple arrays — each should get an assert_unchecked.
    let source = r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## To addArrays (a: Seq of Int, b: Seq of Int) -> Seq of Int:
    Let n be length of a.
    Let mutable result be a new Seq of Int.
    Let mutable i be 1.
    While i is at most n:
        Push item i of a + item i of b to result.
        Set i to i + 1.
    Return result.

## Main
Let x be [1, 2, 3].
Let y be [4, 5, 6].
Show addArrays(x, y).
"#;
    let code = compile_to_rust(source).unwrap();
    // Both arrays should get assert_unchecked hints
    let assert_count = code.matches("assert_unchecked").count();
    assert!(
        assert_count >= 2,
        "Expected assert_unchecked for each indexed array (a, b), found {} occurrences in:\n{}", assert_count, code
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_assert_unchecked_zero_based_multiple_arrays_correctness() {
    assert_exact_output(
        r#"## To addArrays (a: Seq of Int, b: Seq of Int) -> Seq of Int:
    Let n be length of a.
    Let mutable result be a new Seq of Int.
    Let mutable i be 1.
    While i is at most n:
        Push item i of a + item i of b to result.
        Set i to i + 1.
    Return result.

## Main
Let x be [1, 2, 3].
Let y be [4, 5, 6].
Show addArrays(x, y).
"#,
        "[5, 7, 9]",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_assert_unchecked_zero_based_setindex() {
    // Zero-based loop that writes via SetIndex with counter-based indexing.
    // The counter is used for both reading and writing — should get asserts.
    let source = r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## To doubleInPlace (arr: Seq of Int) -> Seq of Int:
    Let n be length of arr.
    Let mutable i be 1.
    While i is at most n:
        Set item i of arr to item i of arr * 2.
        Set i to i + 1.
    Return arr.

## Main
Show doubleInPlace([3, 5, 7]).
"#;
    let code = compile_to_rust(source).unwrap();
    assert!(
        code.contains("assert_unchecked"),
        "Expected assert_unchecked on zero-based loop with SetIndex, got:\n{}", code
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_no_assert_unchecked_zero_based_no_indexing() {
    // Zero-based loop with NO array indexing should NOT get assert_unchecked.
    let source = r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## To sumRange (n: Int) -> Int:
    Let mutable total be 0.
    Let mutable i be 1.
    While i is at most n:
        Set total to total + i.
        Set i to i + 1.
    Return total.

## Main
Show sumRange(10).
"#;
    let code = compile_to_rust(source).unwrap();
    // This loop doesn't index any arrays — no assert needed.
    // Note: this loop uses counter in arithmetic, so it won't be zero-based anyway.
    assert!(
        !code.contains("assert_unchecked"),
        "Should NOT have assert_unchecked when no array indexing, got:\n{}", code
    );
}

// =============================================================================
// FIX-3: Multi-push with_capacity
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_with_capacity_multi_push_correctness() {
    // A loop that pushes to two different collections should produce correct output.
    assert_exact_output(
        r#"## Main
Let mutable evens be a new Seq of Int.
Let mutable odds be a new Seq of Int.
Let mutable i be 1.
While i is at most 6:
    If i % 2 equals 0:
        Push i to evens.
    Otherwise:
        Push i to odds.
    Set i to i + 1.
Show evens.
Show odds.
"#,
        "[2, 4, 6]\n[1, 3, 5]",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_with_capacity_multi_push_codegen() {
    // When two Seqs are created before the same loop and both are pushed to
    // unconditionally in the loop body, both should get Vec::with_capacity.
    let source = r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## To fillTwo (n: Int) -> Seq of Int:
    Let mutable a be a new Seq of Int.
    Let mutable b be a new Seq of Int.
    Let mutable i be 1.
    While i is at most n:
        Push i to a.
        Push i * 2 to b.
        Set i to i + 1.
    Return a.

## Main
Show fillTwo(5).
"#;
    let code = compile_to_rust(source).unwrap();
    // Both a and b should get with_capacity (not Vec::new() or Seq::default())
    let with_cap_count = code.matches("Vec::with_capacity(").count();
    assert!(
        with_cap_count >= 2,
        "Expected Vec::with_capacity for both a and b, found {} occurrences in:\n{}", with_cap_count, code
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_with_capacity_multi_push_init_loop() {
    // Graph-BFS style: two arrays initialized to same value in same loop.
    let source = r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## To initArrays (n: Int) -> Seq of Int:
    Let mutable starts be a new Seq of Int.
    Let mutable counts be a new Seq of Int.
    Let mutable i be 1.
    While i is at most n:
        Push 0 to starts.
        Push 0 to counts.
        Set i to i + 1.
    Return starts.

## Main
Show initArrays(5).
"#;
    let code = compile_to_rust(source).unwrap();
    // Both should get vec![0; n] or Vec::with_capacity
    assert!(
        !code.contains("Seq::<i64>::default()"),
        "Should not have Seq::default() for either collection, got:\n{}", code
    );
}

// =============================================================================
// FIX-4: Constant folding inside Push value arguments
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_fold_push_argument() {
    // `Push (0 - 1) to arr` should fold to pushing -1, not emitting `(0 - 1)`.
    let source = r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## Main
Let mutable arr be a new Seq of Int.
Push (0 - 1) to arr.
Push (2 + 3) to arr.
Show arr.
"#;
    let code = compile_to_rust(source).unwrap();
    // The folded value should appear as -1 and 5, not as (0 - 1) and (2 + 3)
    assert!(
        code.contains("-1") && !code.contains("0 - 1"),
        "Expected folded push argument (0 - 1 -> -1), got:\n{}", code
    );
    assert!(
        code.contains("5") && !code.contains("2 + 3"),
        "Expected folded push argument (2 + 3 -> 5), got:\n{}", code
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_fold_push_argument_correctness() {
    assert_exact_output(
        r#"## Main
Let mutable arr be a new Seq of Int.
Push (0 - 1) to arr.
Push (2 + 3) to arr.
Push (10 * 2) to arr.
Show arr.
"#,
        "[-1, 5, 20]",
    );
}
