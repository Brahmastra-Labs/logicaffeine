//! User's exact concurrency code test

mod common;
use common::run_logos;

#[test]
fn run_user_code() {
    let source = r#"# Concurrency
-- Guide Section 12: Simultaneously and Attempt all
-- These work in the browser!

## Main

## To double (x: Int):
    Return x * 2.

## To add (a: Int) and (b: Int):
    Return a + b.

## To isEven (n: Int) -> Bool:
    Return n / 2 * 2 equals n.


Show "Parallel computation:".
Simultaneously:
    Let a be 100.
    Let b be 200.

Show "a = " + a.
Show "b = " + b.
Show "Product: " + (a * b).

Show "Async concurrent:".
Attempt all of the following:
    Let x be 10.
    Let y be 20.
    Let z be double(x).

Show "Sum: " + (x + y + z).
"#;
    let result = run_logos(source);
    println!("\n=== GENERATED RUST ===\n{}", result.rust_code);
    println!("\n=== OUTPUT ===\n{}", result.stdout);
    if !result.stderr.is_empty() {
        println!("\n=== STDERR ===\n{}", result.stderr);
    }
    assert!(result.success, "Failed: {}", result.stderr);
}
