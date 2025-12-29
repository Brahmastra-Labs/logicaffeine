//! E2E Tests: Integration Tests
//!
//! Complex algorithms that combine multiple language features.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_output;

// === FIBONACCI ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_fibonacci() {
    assert_output(
        r#"## To fib (n: Int) -> Int:
    If n is less than 2:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Show fib(10).
"#,
        "55",
    );
}

// === ACCUMULATOR PATTERN ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_accumulator_sum() {
    assert_output(
        r#"## Main
Let items be [1, 2, 3, 4, 5].
Let sum be 0.
Repeat for x in items:
    Set sum to sum + x.
Show sum.
"#,
        "15",
    );
}

// === FILTER PATTERN ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_filter_pattern() {
    assert_output(
        r#"## Main
Let items be [1, 2, 3, 4, 5, 6].
Let evens be a new Seq of Int.
Repeat for x in items:
    If x / 2 * 2 equals x:
        Push x to evens.
Show evens.
"#,
        "[2, 4, 6]",
    );
}

// === NESTED LOOPS (2D ITERATION) ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_nested_loops_sum() {
    // Sum of 1*1 + 1*2 + 2*1 + 2*2 = 1 + 2 + 2 + 4 = 9
    assert_output(
        r#"## Main
Let sum be 0.
Repeat for i from 1 to 2:
    Repeat for j from 1 to 2:
        Set sum to sum + i * j.
Show sum.
"#,
        "9",
    );
}

// === BUBBLE SORT ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_bubble_sort() {
    assert_output(
        r#"## To bubbleSort (items: Seq of Int) -> Seq of Int:
    Let result be copy of items.
    Let n be length of result.
    Let i be 1.
    While i is less than n:
        Let j be 1.
        While j is at most n - i:
            Let a be item j of result.
            Let b be item (j + 1) of result.
            If a is greater than b:
                Let temp be a.
                Set item j of result to b.
                Set item (j + 1) of result to temp.
            Set j to j + 1.
        Set i to i + 1.
    Return result.

## Main
Let nums be [5, 2, 8, 1, 9].
Let sorted be bubbleSort(nums).
Show sorted.
"#,
        "[1, 2, 5, 8, 9]",
    );
}

// === GCD (EUCLIDEAN ALGORITHM) ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_gcd() {
    assert_output(
        r#"## To gcd (a: Int) and (b: Int) -> Int:
    If b equals 0:
        Return a.
    Return gcd(b, a - (a / b) * b).

## Main
Show gcd(48, 18).
"#,
        "6",
    );
}
