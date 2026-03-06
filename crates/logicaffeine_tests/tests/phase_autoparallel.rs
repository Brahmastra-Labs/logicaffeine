mod common;

use common::compile_to_rust;

// =============================================================================
// Camp 8: Automatic Parallelization
// =============================================================================
//
// Detect commutative reduction patterns in Repeat loops and emit par_iter()
// for automatic parallelization using rayon.

#[test]
fn par_reduction_sum() {
    // Sum reduction: Set total to total + x → par_iter().sum()
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("100000").
Let mutable items be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push (i % 100) + 1 to items.
    Set i to i + 1.
Let mutable total be 0.
Repeat for x in items:
    Set total to total + x.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("par_iter"),
        "Sum reduction should use par_iter. Got:\n{}",
        rust
    );
    // Sum = (1+2+...+100) * 1000 = 5050 * 1000 = 5050000
    common::assert_exact_output(source, "5050000");
}

#[test]
fn par_no_parallel_io() {
    // Loop body has IO (Show) — NOT parallelizable
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable items be a new Seq of Int.
Push 1 to items.
Push 2 to items.
Push 3 to items.
Repeat for x in items:
    Show x.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("par_iter"),
        "Loop with IO (Show) should NOT use par_iter. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "1\n2\n3");
}

#[test]
fn par_no_parallel_push() {
    // Loop body has Push — order-dependent, NOT parallelizable
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable items be a new Seq of Int.
Push 1 to items.
Push 2 to items.
Push 3 to items.
Let mutable result be a new Seq of Int.
Repeat for x in items:
    Push x * 2 to result.
Show length of result.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("par_iter"),
        "Loop with Push should NOT use par_iter. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "3");
}

#[test]
fn par_no_parallel_multiple_writes() {
    // Body writes to two accumulators — complex dependency, NOT parallelizable
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable items be a new Seq of Int.
Push 1 to items.
Push 2 to items.
Push 3 to items.
Let mutable total be 0.
Let mutable count be 0.
Repeat for x in items:
    Set total to total + x.
    Set count to count + 1.
Show total.
Show count.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("par_iter"),
        "Loop with multiple writes should NOT use par_iter. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "6\n3");
}

#[test]
fn par_correctness_large_sum() {
    // Large input — verify parallel sum matches sequential
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("50000").
Let mutable items be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i + 1 to items.
    Set i to i + 1.
Let mutable total be 0.
Repeat for x in items:
    Set total to total + x.
Show total.
"#;
    // Sum of 1..50000 = 50000 * 50001 / 2 = 1250025000
    common::assert_exact_output(source, "1250025000");
}

#[test]
fn par_correctness_sum_with_init() {
    // Accumulator starts at non-zero — parallel result must include init
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("10000").
Let mutable items be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push 1 to items.
    Set i to i + 1.
Let mutable total be 100.
Repeat for x in items:
    Set total to total + x.
Show total.
"#;
    // 100 + 10000 = 10100
    common::assert_exact_output(source, "10100");
}
