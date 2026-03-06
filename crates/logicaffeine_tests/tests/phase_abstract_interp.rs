mod common;

use common::compile_to_rust;

// =============================================================================
// Camp 3: Abstract Interpretation (Value Range Analysis)
// =============================================================================
//
// Forward analysis tracking integer value ranges (intervals) through the
// program. Eliminates dead branches when conditions are provably always-true
// or always-false based on range information. Also tracks collection lengths.

// ---------------------------------------------------------------------------
// Phase 1: Literal + Arithmetic Ranges
// ---------------------------------------------------------------------------

#[test]
fn range_literal_dead_branch() {
    // Let x = 42. If x > 100: → dead branch eliminated
    // Constant propagation already handles this, but range analysis should too
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let x be 42.
If x is greater than 100:
    Show "unreachable".
Show "done".
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("unreachable"),
        "Dead branch (x=42, x>100) should be eliminated. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "done");
}

#[test]
fn range_arithmetic_dead_branch() {
    // Let x = 3. Let y = x + 7 → y in [10, 10].
    // If y > 20: → dead branch
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let x be 3.
Let y be x + 7.
If y is greater than 20:
    Show "unreachable".
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("unreachable"),
        "Dead branch (y=10, y>20) should be eliminated. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "10");
}

#[test]
fn range_multiply_dead_branch() {
    // Let x = 5. Let y = x * 3 → y = 15.
    // If y is less than 10: → dead branch
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let x be 5.
Let y be x * 3.
If y is less than 10:
    Show "unreachable".
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("unreachable"),
        "Dead branch (y=15, y<10) should be eliminated. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "15");
}

#[test]
fn range_unknown_variable_preserved() {
    // parseInt returns unknown range → both branches preserved
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("42").
If n is greater than 100:
    Show "big".
Show "done".
"#;
    common::assert_exact_output(source, "done");
}

// ---------------------------------------------------------------------------
// Phase 2: Conditionals + Narrowing
// ---------------------------------------------------------------------------

#[test]
fn range_conditional_narrowing_then() {
    // If n > 0: then n is at least 1 inside the branch
    // Within the branch, if n < 0: → dead (n >= 1, so n < 0 is impossible)
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("5").
If n is greater than 0:
    If n is less than 0:
        Show "unreachable".
    Show n.
Show "done".
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("unreachable"),
        "Nested dead branch after narrowing should be eliminated. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "5\ndone");
}

#[test]
fn range_conditional_narrowing_else() {
    // If n > 10: ... Else: n <= 10
    // In else branch, if n > 10: → dead
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("3").
If n is greater than 10:
    Show "big".
Otherwise:
    If n is greater than 10:
        Show "unreachable".
    Show n.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("unreachable"),
        "Dead branch in else (n<=10, n>10) should be eliminated. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "3");
}

#[test]
fn range_set_updates_range() {
    // Let x = 5. Set x to x + 10 → x = 15.
    // If x < 10: → dead branch
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable x be 5.
Set x to x + 10.
If x is less than 10:
    Show "unreachable".
Show x.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("unreachable"),
        "Dead branch (x=15, x<10) should be eliminated. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "15");
}

// ---------------------------------------------------------------------------
// Phase 3: Loops + Widening
// ---------------------------------------------------------------------------

#[test]
fn range_for_loop_counter() {
    // For i from 1 to 100: i is in [1, 100]
    // Inside loop, if i > 200: → dead branch
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable total be 0.
Let mutable i be 1.
While i is at most 100:
    If i is greater than 200:
        Show "unreachable".
    Set total to total + i.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("unreachable"),
        "Dead branch inside loop (i<=100, i>200) should be eliminated. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "5050");
}

#[test]
fn range_for_loop_counter_variable_bound() {
    // For i from 1 to n: i is in [1, n]
    // After loop, counter value is preserved
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("10").
Let mutable total be 0.
Let mutable i be 1.
While i is at most n:
    Set total to total + i.
    Set i to i + 1.
Show total.
"#;
    common::assert_exact_output(source, "55");
}

#[test]
fn range_while_loop_widening() {
    // Widening should ensure convergence for unbounded loops
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("5").
Let mutable i be 0.
Let mutable sum be 0.
While i is less than n:
    Set sum to sum + i.
    Set i to i + 1.
Show sum.
"#;
    common::assert_exact_output(source, "10");
}

// ---------------------------------------------------------------------------
// Phase 4: Collection Lengths + Bounds Proofs
// ---------------------------------------------------------------------------

#[test]
fn range_push_increments_length() {
    // 3 pushes → length in [3, 3]
    // If length < 1: → dead branch
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable items be a new Seq of Int.
Push 10 to items.
Push 20 to items.
Push 30 to items.
If length of items is less than 1:
    Show "unreachable".
Show length of items.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("unreachable"),
        "Dead branch (3 pushes, length<1) should be eliminated. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "3");
}

#[test]
fn range_dead_branch_elimination_simple() {
    // Let x = 5. If x > 100: → eliminated by constant propagation + DCE
    // This tests that the existing pipeline handles it
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let x be 5.
If x is greater than 100:
    Show "unreachable".
Show "ok".
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("unreachable"),
        "Dead branch should be eliminated. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "ok");
}

// ---------------------------------------------------------------------------
// Phase 5: Edge Cases
// ---------------------------------------------------------------------------

#[test]
fn range_overflow_preserves_branch() {
    // Very large arithmetic → range goes to Top, branches preserved
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let x be parseInt("9223372036854775807").
If x is greater than 0:
    Show "big".
Show "done".
"#;
    common::assert_exact_output(source, "big\ndone");
}

#[test]
fn range_nested_loop() {
    // Nested loop counters tracked correctly
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable total be 0.
Let mutable i be 1.
While i is at most 3:
    Let mutable j be 1.
    While j is at most 3:
        Set total to total + 1.
        Set j to j + 1.
    Set i to i + 1.
Show total.
"#;
    common::assert_exact_output(source, "9");
}

#[test]
fn range_function_call_unknown_return() {
    // Function calls return unknown ranges → branches preserved
    let source = r#"## To native parseInt (s: Text) -> Int

## To double (n: Int) -> Int:
    Return n * 2.

## Main
Let x be double(21).
If x is greater than 100:
    Show "big".
Otherwise:
    Show x.
"#;
    common::assert_exact_output(source, "42");
}

#[test]
fn range_conditional_join_preserves() {
    // After an if-else, range is the join of both branches
    // Both branches are valid, so the outer condition is preserved
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("7").
Let mutable x be 0.
If n is greater than 5:
    Set x to 10.
Otherwise:
    Set x to 20.
Show x.
"#;
    common::assert_exact_output(source, "10");
}
