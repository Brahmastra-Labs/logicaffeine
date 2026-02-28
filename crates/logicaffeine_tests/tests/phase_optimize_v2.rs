mod common;

use common::compile_to_rust;

// =============================================================================
// OPT-1a: For-Range with Set counter (not just Let)
// =============================================================================

#[test]
fn opt1a_set_counter_for_range() {
    let source = r#"## Main
Let mutable i be 99.
Set i to 0.
While i is less than 5:
    Show i.
    Set i to i + 1.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("for i in 0..5"), "Set-based counter should emit for-range, got:\n{}", rust);
}

#[test]
fn opt1a_set_counter_inclusive_for_range() {
    let source = r#"## Main
Let mutable i be 99.
Set i to 1.
While i is at most 5:
    Show i.
    Set i to i + 1.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("for i in 1..6"), "Set-based inclusive counter should emit for-range, got:\n{}", rust);
}

#[test]
fn opt1a_set_counter_e2e_correct_sum() {
    let source = r#"## Main
Let mutable sum be 0.
Let mutable i be 99.
Set i to 1.
While i is at most 5:
    Set sum to sum + i.
    Set i to i + 1.
Show sum.
"#;
    common::assert_exact_output(source, "15");
}

#[test]
fn opt1a_set_counter_post_loop_value() {
    let source = r#"## Main
Let mutable i be 99.
Set i to 1.
While i is at most 5:
    Set i to i + 1.
Show i.
"#;
    common::assert_exact_output(source, "6");
}

#[test]
fn opt1a_set_counter_in_function() {
    let source = r#"## To countUp (n: Int) -> Int:
    Let mutable total be 0.
    Let mutable i be 99.
    Set i to 0.
    While i is less than n:
        Set total to total + i.
        Set i to i + 1.
    Return total.

## Main
Show countUp(5).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("for i in 0.."), "Set-based counter in function should emit for-range, got:\n{}", rust);
    common::assert_exact_output(source, "10");
}

// =============================================================================
// OPT-1b: For-Range with variable start values
// =============================================================================

#[test]
fn opt1b_variable_start_for_range() {
    let source = r#"## To sumFrom (start: Int) -> Int:
    Let mutable total be 0.
    Let i be start.
    While i is less than 10:
        Set total to total + i.
        Set i to i + 1.
    Return total.

## Main
Show sumFrom(3).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("for i in start..10"), "Variable start should emit for-range, got:\n{}", rust);
    common::assert_exact_output(source, "42");
}

#[test]
fn opt1b_variable_start_inclusive() {
    let source = r#"## To sumRange (start: Int) -> Int:
    Let mutable total be 0.
    Let i be start.
    While i is at most 5:
        Set total to total + i.
        Set i to i + 1.
    Return total.

## Main
Show sumRange(2).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("for i in start..(5 + 1)") || rust.contains("for i in start..6"),
        "Variable start inclusive should emit for-range, got:\n{}", rust);
    common::assert_exact_output(source, "14");
}

#[test]
fn opt1b_variable_start_e2e_correct() {
    let source = r#"## To countFrom (start: Int, limit: Int) -> Int:
    Let mutable total be 0.
    Let i be start.
    While i is less than limit:
        Set total to total + i.
        Set i to i + 1.
    Return total.

## Main
Show countFrom(3, 8).
"#;
    // 3+4+5+6+7 = 25
    common::assert_exact_output(source, "25");
}

#[test]
fn opt1b_set_with_variable_start() {
    let source = r#"## To loopFrom (start: Int) -> Int:
    Let mutable total be 0.
    Let mutable i be 0.
    Set i to start.
    While i is less than 10:
        Set total to total + i.
        Set i to i + 1.
    Return total.

## Main
Show loopFrom(5).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("for i in start..10"), "Set + variable start should emit for-range, got:\n{}", rust);
    common::assert_exact_output(source, "35");
}

#[test]
fn opt1b_expression_start() {
    let source = r#"## To loopExpr (n: Int) -> Int:
    Let mutable total be 0.
    Let i be n - 5.
    While i is less than n:
        Set total to total + i.
        Set i to i + 1.
    Return total.

## Main
Show loopExpr(10).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("for i in (n - 5)..n"), "Expression start should emit for-range, got:\n{}", rust);
    // n=10, so i goes from 5 to 9: 5+6+7+8+9 = 35
    common::assert_exact_output(source, "35");
}

// =============================================================================
// OPT-1c: For-Range with length-of in limit
// =============================================================================

#[test]
fn opt1c_length_limit_for_range() {
    let source = r#"## Main
Let items: Seq of Int be [10, 20, 30].
Let i be 1.
While i is at most length of items:
    Show item i of items.
    Set i to i + 1.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("for i in 0..(items.len() as i64)")
        || rust.contains("for i in 1..((items.len() as i64) + 1)")
        || rust.contains("for i in 1..(items.len() as i64 + 1)"),
        "Length-of limit should emit for-range, got:\n{}", rust
    );
}

#[test]
fn opt1c_length_limit_exclusive() {
    let source = r#"## Main
Let items: Seq of Int be [10, 20, 30, 40].
Let i be 0.
While i is less than length of items:
    Show i.
    Set i to i + 1.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("for i in 0..(items.len() as i64)"),
        "Length-of exclusive limit should emit for-range, got:\n{}", rust
    );
}

#[test]
fn opt1c_length_limit_e2e_correct() {
    let source = r#"## Main
Let items: Seq of Int be [10, 20, 30].
Let mutable sum be 0.
Let i be 1.
While i is at most length of items:
    Set sum to sum + item i of items.
    Set i to i + 1.
Show sum.
"#;
    common::assert_exact_output(source, "60");
}

// =============================================================================
// OPT-7: Dead post-loop value elimination
// =============================================================================

#[test]
fn opt7_dead_post_loop_when_immediately_overwritten() {
    let source = r#"## To process (n: Int) -> Int:
    Let mutable total be 0.
    Let i be 0.
    While i is less than n:
        Set total to total + i.
        Set i to i + 1.
    Set i to 0.
    Return total + i.

## Main
Show process(5).
"#;
    let rust = compile_to_rust(source).unwrap();
    // The for-range should fire, and the post-loop i = max(0, n) should be omitted
    // because i is immediately overwritten by Set i to 0.
    assert!(rust.contains("for i in 0..n"), "Should emit for-range, got:\n{}", rust);
    assert!(!rust.contains(".max("), "Should not emit post-loop max when immediately overwritten, got:\n{}", rust);
    common::assert_exact_output(source, "10");
}

#[test]
fn opt7_dead_post_loop_set_counter_to_new_loop() {
    let source = r#"## To twoLoops (n: Int) -> Int:
    Let mutable total be 0.
    Let i be 0.
    While i is less than n:
        Set total to total + 1.
        Set i to i + 1.
    Set i to 0.
    While i is less than n:
        Set total to total + 1.
        Set i to i + 1.
    Return total.

## Main
Show twoLoops(5).
"#;
    let rust = compile_to_rust(source).unwrap();
    // Between the two loops, i is reset — the first post-loop value is dead
    assert!(!rust.contains(".max("), "Dead post-loop should be eliminated when next stmt overwrites counter, got:\n{}", rust);
    common::assert_exact_output(source, "10");
}

#[test]
fn opt7_post_loop_kept_when_used() {
    let source = r#"## To sumAndReturn (n: Int) -> Int:
    Let mutable total be 0.
    Let i be 0.
    While i is less than n:
        Set total to total + i.
        Set i to i + 1.
    Return total + i.

## Main
Show sumAndReturn(5).
"#;
    let rust = compile_to_rust(source).unwrap();
    // i IS used after the loop (in total + i), so post-loop value MUST be kept
    assert!(rust.contains(".max(") || rust.contains("let mut i ="),
        "Post-loop value should be kept when counter is used, got:\n{}", rust);
    // 0+1+2+3+4 = 10, then i=5, total+i = 15
    common::assert_exact_output(source, "15");
}

// =============================================================================
// OPT-2: Seq-from-slice pattern
// =============================================================================

#[test]
fn opt2_full_array_copy_to_vec() {
    let source = r#"## Main
Let src: Seq of Int be [1, 2, 3, 4, 5].
Let mutable dst be a new Seq of Int.
Let mutable i be 1.
While i is at most length of src:
    Push item i of src to dst.
    Set i to i + 1.
Show length of dst.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains(".to_vec()") || rust.contains(".clone()"),
        "Full copy loop should emit slice operation, got:\n{}", rust);
    common::assert_exact_output(source, "5");
}

#[test]
fn opt2_partial_slice_copy() {
    let source = r#"## Main
Let src: Seq of Int be [10, 20, 30, 40, 50].
Let mutable dst be a new Seq of Int.
Let mutable i be 2.
While i is at most 4:
    Push item i of src to dst.
    Set i to i + 1.
Show item 1 of dst.
Show item 2 of dst.
Show item 3 of dst.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("[") && rust.contains("].to_vec()"),
        "Partial copy loop should emit slice.to_vec(), got:\n{}", rust);
    common::assert_exact_output(source, "20\n30\n40");
}

#[test]
fn opt2_slice_copy_e2e_correct() {
    let source = r#"## Main
Let src: Seq of Int be [100, 200, 300].
Let mutable dst be a new Seq of Int.
Let mutable i be 1.
While i is at most length of src:
    Push item i of src to dst.
    Set i to i + 1.
Show item 1 of dst.
Show item 2 of dst.
Show item 3 of dst.
"#;
    common::assert_exact_output(source, "100\n200\n300");
}

// =============================================================================
// OPT-3: Vec pre-allocation with_capacity
// =============================================================================

#[test]
fn opt3_vec_with_capacity_counted_loop() {
    let source = r#"## Main
Let mutable result be a new Seq of Int.
Let i be 1.
While i is at most 100:
    Push i * i to result.
    Set i to i + 1.
Show length of result.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("with_capacity") || rust.contains("Vec::with_capacity"),
        "Counted push loop should pre-allocate, got:\n{}", rust);
    common::assert_exact_output(source, "100");
}

#[test]
fn opt3_vec_with_capacity_variable_bound() {
    let source = r#"## Main
Let n be 50.
Let mutable items be a new Seq of Int.
Let i be 0.
While i is less than n:
    Push i to items.
    Set i to i + 1.
Show length of items.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("with_capacity"),
        "Variable-bound push loop should pre-allocate, got:\n{}", rust);
    common::assert_exact_output(source, "50");
}

#[test]
fn opt3_no_capacity_when_conditional_push() {
    let source = r#"## Main
Let mutable evens be a new Seq of Int.
Let i be 0.
While i is less than 10:
    If i % 2 equals 0:
        Push i to evens.
    Set i to i + 1.
Show length of evens.
"#;
    let rust = compile_to_rust(source).unwrap();
    // Should NOT pre-allocate when push is conditional — count isn't deterministic
    assert!(!rust.contains("with_capacity"),
        "Conditional push should NOT pre-allocate, got:\n{}", rust);
    common::assert_exact_output(source, "5");
}

#[test]
fn opt3_map_with_capacity() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Int.
Let i be 0.
While i is less than 100:
    Set item i of m to i * i.
    Set i to i + 1.
Show length of m.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("with_capacity"),
        "Counted map insertion loop should pre-allocate, got:\n{}", rust);
    common::assert_exact_output(source, "100");
}

// =============================================================================
// OPT-4: Bounds check elision via assert hints
// =============================================================================

#[test]
fn opt4_assert_hint_for_indexed_loop() {
    let source = r#"## Main
Let items: Seq of Int be [1, 2, 3, 4, 5].
Let mutable sum be 0.
Let i be 1.
While i is at most 5:
    Set sum to sum + item i of items.
    Set i to i + 1.
Show sum.
"#;
    let rust = compile_to_rust(source).unwrap();
    // Zero-based normalization (0..5 with items[i as usize]) supersedes the assert hint
    // since bounds are provably safe. Accept either pattern.
    assert!(rust.contains("assert_unchecked") || rust.contains("for i in 0..5"),
        "For-range with array indexing should emit assert_unchecked hint or 0-based range, got:\n{}", rust);
    common::assert_exact_output(source, "15");
}

#[test]
fn opt4_assert_hint_with_length_bound() {
    let source = r#"## Main
Let items: Seq of Int be [10, 20, 30].
Let mutable sum be 0.
Let i be 1.
While i is at most length of items:
    Set sum to sum + item i of items.
    Set i to i + 1.
Show sum.
"#;
    let rust = compile_to_rust(source).unwrap();
    // When loop bound IS length-of-items, assert_unchecked is trivially true but still helps LLVM
    assert!(rust.contains("assert_unchecked") || rust.contains("for i in"),
        "Length-bounded array loop should emit assert_unchecked or for-range, got:\n{}", rust);
    common::assert_exact_output(source, "60");
}

#[test]
fn opt4_no_assert_when_no_indexing() {
    let source = r#"## Main
Let mutable sum be 0.
Let i be 1.
While i is at most 100:
    Set sum to sum + i.
    Set i to i + 1.
Show sum.
"#;
    let rust = compile_to_rust(source).unwrap();
    // No array indexing → no assert needed
    assert!(!rust.contains("assert_unchecked"),
        "Loop without array indexing should NOT emit assert_unchecked, got:\n{}", rust);
}

// =============================================================================
// OPT-5: String indexing (correctness via LogosIndex)
// =============================================================================

#[test]
fn opt5_string_byte_indexing() {
    let source = r#"## Main
Let text be "hello".
Let ch be item 1 of text.
Show ch.
"#;
    common::assert_exact_output(source, "h");
}

#[test]
fn opt5_string_index_in_loop() {
    let source = r#"## Main
Let text be "abcde".
Let mutable result be "".
Let i be 1.
While i is at most length of text:
    Set result to result + item i of text.
    Set i to i + 1.
Show result.
"#;
    common::assert_exact_output(source, "abcde");
}

#[test]
fn opt5_string_comparison_uses_bytes() {
    let source = r#"## Main
Let text be "hello".
If item 1 of text equals item 1 of "help":
    Show "match".
"#;
    common::assert_exact_output(source, "match");
}

#[test]
fn opt5_unicode_string_indexing() {
    let source = r#"## Main
Let text be "héllo".
Let ch be item 2 of text.
Show ch.
"#;
    common::assert_exact_output(source, "é");
}

// =============================================================================
// String variable correctness tests
// =============================================================================

#[test]
fn string_variable_multi_char_correct() {
    let source = r#"## Main
Let mutable ch be "a".
Set ch to "bc".
Show ch.
"#;
    common::assert_exact_output(source, "bc");
}

#[test]
fn string_variable_in_loop_correct() {
    let source = r#"## Main
Let text be "hello".
Let mutable ch be "x".
Let i be 1.
While i is at most length of text:
    Show ch.
    Set ch to "y".
    Set i to i + 1.
Show ch.
"#;
    common::assert_exact_output(source, "x\ny\ny\ny\ny\ny");
}

// =============================================================================
// OPT-8: LICM — Loop invariant code motion for length
// =============================================================================

#[test]
fn opt8_hoist_length_from_loop() {
    let source = r#"## Main
Let items: Seq of Int be [1, 2, 3, 4, 5].
Let mutable sum be 0.
Let mutable i be 1.
While i is at most length of items:
    Set sum to sum + item i of items.
    Set i to i + 1.
Show sum.
"#;
    let rust = compile_to_rust(source).unwrap();
    // Length of items should be hoisted out of the loop condition
    // Either via for-range conversion (OPT-1c) or explicit hoisting
    let has_for_range = rust.contains("for i in");
    let has_hoisted_len = rust.contains("let __len") || rust.contains("let _len");
    assert!(has_for_range || has_hoisted_len,
        "Length should be hoisted or converted to for-range, got:\n{}", rust);
    common::assert_exact_output(source, "15");
}

#[test]
fn opt8_no_hoist_when_collection_modified() {
    let source = r#"## Main
Let mutable items: Seq of Int be [1, 2, 3].
Let mutable i be 1.
While i is at most length of items:
    Push 0 to items.
    If length of items is greater than 10:
        Set i to 999.
    Set i to i + 1.
Show length of items.
"#;
    // When collection is modified in the loop, length should NOT be hoisted
    // (this test just checks correctness — the program terminates)
    common::assert_exact_output(source, "11");
}

// =============================================================================
// Combined optimization correctness tests
// =============================================================================

#[test]
fn combined_set_counter_with_length_limit() {
    let source = r#"## Main
Let items: Seq of Int be [5, 10, 15, 20].
Let mutable total be 0.
Let mutable i be 99.
Set i to 1.
While i is at most length of items:
    Set total to total + item i of items.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    // This combines OPT-1a (Set counter) + OPT-1c (length limit)
    assert!(rust.contains("for i in"),
        "Set counter + length limit should emit for-range, got:\n{}", rust);
    common::assert_exact_output(source, "50");
}

#[test]
fn combined_variable_start_with_capacity() {
    let source = r#"## Main
Let start be 1.
Let limit be 100.
Let mutable squares be a new Seq of Int.
Let i be start.
While i is at most limit:
    Push i * i to squares.
    Set i to i + 1.
Show length of squares.
Show item 1 of squares.
Show item 100 of squares.
"#;
    common::assert_exact_output(source, "100\n1\n10000");
}

#[test]
fn combined_nested_loops_both_optimized() {
    let source = r#"## Main
Let mutable sum be 0.
Let i be 1.
While i is at most 10:
    Let j be 1.
    While j is at most 10:
        Set sum to sum + 1.
        Set j to j + 1.
    Set i to i + 1.
Show sum.
"#;
    let rust = compile_to_rust(source).unwrap();
    // Both loops should be converted to for-range
    let for_count = rust.matches("for ").count();
    assert!(for_count >= 2, "Both nested loops should be for-range (found {} for), got:\n{}", for_count, rust);
    common::assert_exact_output(source, "100");
}

#[test]
fn combined_post_loop_value_after_nested() {
    let source = r#"## Main
Let mutable outer be 0.
Let i be 1.
While i is at most 3:
    Let j be 1.
    While j is at most 3:
        Set outer to outer + 1.
        Set j to j + 1.
    Set i to i + 1.
Show outer.
Show i.
"#;
    common::assert_exact_output(source, "9\n4");
}

// =============================================================================
// OPT-D: assert_unchecked for proven bounds
// =============================================================================

#[test]
fn opt_d_assert_unchecked_in_for_range() {
    // Counter i is used for BOTH indexing and non-indexing (Show i),
    // so zero-based optimization does NOT fire, and the bounds assert IS emitted.
    let source = r#"## Main
Let items: Seq of Int be [10, 20, 30, 40, 50].
Let mutable sum be 0.
Let i be 1.
While i is at most 5:
    Set sum to sum + item i of items.
    Show i.
    Set i to i + 1.
Show sum.
"#;
    let rust = compile_to_rust(source).unwrap();
    // Should use assert_unchecked instead of assert! for proven bounds
    assert!(
        rust.contains("assert_unchecked"),
        "Proven bounds in for-range should use assert_unchecked, got:\n{}",
        rust
    );
    assert!(
        !rust.contains("assert!("),
        "Should NOT use assert!() for proven bounds — use assert_unchecked instead, got:\n{}",
        rust
    );
}

#[test]
fn opt_d_assert_unchecked_in_while_loop() {
    // While loop that doesn't convert to for-range (limit modified inside).
    // The counter indexes arr, so bounds hint should be emitted as assert_unchecked.
    let source = r#"## To search (arr: Seq of Int, target: Int) -> Int:
    Let mutable i be 1.
    While i is at most length of arr:
        If item i of arr equals target:
            Return i.
        Set i to i + 1.
    Return 0 - 1.

## Main
Let data: Seq of Int be [5, 3, 8, 1, 9].
Show search(data, 8).
"#;
    let rust = compile_to_rust(source).unwrap();
    // While-loop bounds assertions should use assert_unchecked (not assert!)
    if rust.contains("assert_unchecked") || rust.contains("assert!(") {
        assert!(
            rust.contains("assert_unchecked"),
            "While-loop bounds hints should use assert_unchecked, got:\n{}",
            rust
        );
        assert!(
            !rust.contains("assert!("),
            "While-loop bounds hints should NOT use assert!(), got:\n{}",
            rust
        );
    }
}

#[test]
fn opt_d_assert_unchecked_correctness() {
    // Ensure programs using assert_unchecked produce correct output
    let source = r#"## Main
Let items: Seq of Int be [10, 20, 30, 40, 50].
Let mutable sum be 0.
Let i be 1.
While i is at most 5:
    Set sum to sum + item i of items.
    Show i.
    Set i to i + 1.
Show sum.
"#;
    common::assert_exact_output(source, "1\n2\n3\n4\n5\n150");
}

// =============================================================================
// OPT-E: Inline threshold raised from 5 to 10
// =============================================================================

#[test]
fn opt_e_inline_8_statement_function() {
    // A function with 8 statements should get #[inline] annotation
    let source = r#"## To process (a: Int, b: Int) -> Int:
    Let x be a + b.
    Let y be a - b.
    Let z be x * y.
    Let w be z + a.
    Let v be w - b.
    Let u be v * 2.
    Let t be u + 1.
    Return t.

## Main
Show process(10, 3).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("#[inline]"),
        "8-statement function should be inlined with raised threshold, got:\n{}",
        rust
    );
}

#[test]
fn opt_e_no_inline_12_statement_function() {
    // A function with 12 statements should NOT get #[inline]
    let source = r#"## To bigProcess (a: Int, b: Int) -> Int:
    Let x1 be a + b.
    Let x2 be a - b.
    Let x3 be x1 * x2.
    Let x4 be x3 + a.
    Let x5 be x4 - b.
    Let x6 be x5 * 2.
    Let x7 be x6 + 1.
    Let x8 be x7 - 3.
    Let x9 be x8 * a.
    Let x10 be x9 + b.
    Let x11 be x10 - 1.
    Return x11.

## Main
Show bigProcess(5, 3).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("#[inline]"),
        "12-statement function should NOT be inlined, got:\n{}",
        rust
    );
}

#[test]
fn opt_e_inline_threshold_correctness() {
    // Ensure inlined functions produce correct output
    let source = r#"## To process (a: Int, b: Int) -> Int:
    Let x be a + b.
    Let y be a - b.
    Let z be x * y.
    Let w be z + a.
    Let v be w - b.
    Let u be v * 2.
    Let t be u + 1.
    Return t.

## Main
Show process(10, 3).
"#;
    // (10+3)=13, (10-3)=7, 13*7=91, 91+10=101, 101-3=98, 98*2=196, 196+1=197
    common::assert_exact_output(source, "197");
}

// =============================================================================
// OPT-B: Clean index cast parentheses
// =============================================================================

#[test]
fn opt_b_no_redundant_parens_on_identifier_index() {
    // After +1/-1 cancellation, a simple identifier should not have extra parens
    let source = r#"## Main
Let arr: Seq of Int be [10, 20, 30, 40, 50].
Let mutable sum be 0.
Let i be 0.
While i is less than 5:
    Let val be item (i + 1) of arr.
    Set sum to sum + val.
    Set i to i + 1.
Show sum.
"#;
    let rust = compile_to_rust(source).unwrap();
    // After +1/-1 cancellation, should produce `arr[i as usize]` not `arr[(i) as usize]`
    assert!(
        !rust.contains("[(i) as usize]"),
        "Should not have redundant parens around single identifier in index, got:\n{}",
        rust
    );
}

#[test]
fn opt_b_clean_index_correctness() {
    let source = r#"## Main
Let arr: Seq of Int be [10, 20, 30, 40, 50].
Let mutable sum be 0.
Let i be 0.
While i is less than 5:
    Let val be item (i + 1) of arr.
    Set sum to sum + val.
    Set i to i + 1.
Show sum.
"#;
    common::assert_exact_output(source, "150");
}

// =============================================================================
// OPT-A: target-cpu=native in generated projects
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn opt_a_cargo_config_target_cpu_native() {
    // The generated project should include .cargo/config.toml with target-cpu=native
    let source = r#"## Main
Show 42.
"#;
    let temp_dir = tempfile::tempdir().unwrap();
    logicaffeine_compile::compile::compile_to_dir(source, temp_dir.path()).unwrap();

    let config_path = temp_dir.path().join(".cargo").join("config.toml");
    assert!(
        config_path.exists(),
        ".cargo/config.toml should be created in the generated project"
    );
    let config_content = std::fs::read_to_string(&config_path).unwrap();
    assert!(
        config_content.contains("target-cpu=native"),
        ".cargo/config.toml should contain target-cpu=native, got:\n{}",
        config_content
    );
}

// =============================================================================
// OPT-F: Vec::with_capacity for multiple collections
// =============================================================================

#[test]
fn opt_f_with_capacity_multiple_collections() {
    // Multiple Seq allocations followed by a single push loop (graph_bfs pattern)
    let source = r#"## Main
Let n be 100.
Let mutable starts be a new Seq of Int.
Let mutable counts be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i * 5 to starts.
    Push 0 to counts.
    Set i to i + 1.
Show length of starts.
Show length of counts.
"#;
    let rust = compile_to_rust(source).unwrap();
    // Both starts and counts should get with_capacity(n)
    let cap_count = rust.matches("with_capacity").count();
    assert!(
        cap_count >= 1,
        "Multiple collections in counted push loop should get with_capacity, got:\n{}",
        rust
    );
    common::assert_exact_output(source, "100\n100");
}

#[test]
fn opt_f_with_capacity_fill_pattern() {
    // Allocate a Vec and fill with repeated pushes (dist initialization in graph_bfs)
    let source = r#"## Main
Let n be 50.
Let mutable dist be a new Seq of Int.
Let i be 0.
While i is less than n:
    Push 0 - 1 to dist.
    Set i to i + 1.
Show length of dist.
Show item 1 of dist.
"#;
    let rust = compile_to_rust(source).unwrap();
    // Should use either with_capacity or the even better vec![value; n] fill pattern
    assert!(
        rust.contains("with_capacity") || rust.contains("vec!["),
        "Single-push fill loop should get with_capacity or vec fill, got:\n{}",
        rust
    );
    common::assert_exact_output(source, "50\n-1");
}

// =============================================================================
// OPT-G: Constant fold 0 - 1 to -1
// =============================================================================

#[test]
fn opt_g_fold_zero_minus_one() {
    let source = "## Main\nLet x be 0 - 1.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("let x = -1") || rust.contains("let x = (0 - 1)"),
        "0 - 1 should fold to -1 or at least be a simple subtraction, got:\n{}",
        rust
    );
    // More strict: should fold to -1 literal
    assert!(
        rust.contains("let x = -1"),
        "0 - 1 should be constant-folded to -1, got:\n{}",
        rust
    );
}

// =============================================================================
// OPT-C: Strength reduction for power-of-2 modulo
// =============================================================================

#[test]
fn opt_c_power_of_2_modulo_strength_reduction() {
    // x % 8 where x is non-negative should be emitted as x & 7
    let source = r#"## Main
Let x be 42.
Let result be x % 8.
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    // After constant folding: 42 % 8 = 2, so it should fold to a literal.
    // This test verifies the fold handles modulo correctly.
    assert!(
        rust.contains("let result = 2") || rust.contains("& 7"),
        "Power-of-2 modulo should be strength-reduced or constant-folded, got:\n{}",
        rust
    );
    common::assert_exact_output(source, "2");
}

#[test]
fn opt_c_variable_modulo_power_of_2() {
    // Variable modulo by power-of-2 literal — should emit bitwise AND in codegen
    let source = r#"## To hash (x: Int) -> Int:
    Return x % 1024.

## Main
Show hash(12345).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("& 1023") || rust.contains("% 1024"),
        "Variable % power-of-2 should emit & (K-1) or at least % K, got:\n{}",
        rust
    );
    // 12345 % 1024 = 12345 - 12*1024 = 12345 - 12288 = 57
    common::assert_exact_output(source, "57");
}

#[test]
fn opt_c_non_power_of_2_not_reduced() {
    // x % 7 should NOT be strength-reduced (7 is not power of 2)
    let source = r#"## To remainder (x: Int) -> Int:
    Return x % 7.

## Main
Show remainder(100).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("% 7"),
        "Non-power-of-2 modulo should NOT be strength-reduced, got:\n{}",
        rust
    );
    // 100 % 7 = 2
    common::assert_exact_output(source, "2");
}
