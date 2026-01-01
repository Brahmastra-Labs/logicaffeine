//! Phase 57B: Map E2E Tests
//!
//! Verifies Map operations work end-to-end.

mod common;
use common::run_logos;

#[test]
fn map_create_empty() {
    let source = r#"
## Main
Let prices be a new Map of Text to Int.
Show "ok".
"#;
    let output = run_logos(source);
    assert!(output.stdout.contains("ok"), "Map creation should work. stdout: {}, stderr: {}", output.stdout, output.stderr);
}

#[test]
fn map_set_and_get() {
    let source = r#"
## Main
Let mut prices be a new Map of Text to Int.
Set item "iron" of prices to 100.
Let cost be item "iron" of prices.
Show cost.
"#;
    let output = run_logos(source);
    assert!(output.stdout.contains("100"), "Should get 100. stdout: {}, stderr: {}", output.stdout, output.stderr);
}

#[test]
fn map_multiple_keys() {
    let source = r#"
## Main
Let mut inventory be a new Map of Text to Int.
Set item "iron" of inventory to 50.
Set item "copper" of inventory to 30.
Set item "gold" of inventory to 10.
Let total be item "iron" of inventory + item "copper" of inventory + item "gold" of inventory.
Show total.
"#;
    let output = run_logos(source);
    assert!(output.stdout.contains("90"), "Should sum to 90. stdout: {}, stderr: {}", output.stdout, output.stderr);
}

#[test]
fn map_overwrite_key() {
    let source = r#"
## Main
Let mut prices be a new Map of Text to Int.
Set item "iron" of prices to 100.
Set item "iron" of prices to 200.
Let cost be item "iron" of prices.
Show cost.
"#;
    let output = run_logos(source);
    assert!(output.stdout.contains("200"), "Should overwrite to 200. stdout: {}, stderr: {}", output.stdout, output.stderr);
}

#[test]
fn map_with_text_values() {
    let source = r#"
## Main
Let mut names be a new Map of Text to Text.
Set item "player1" of names to "Alice".
Let name be item "player1" of names.
Show name.
"#;
    let output = run_logos(source);
    assert!(output.stdout.contains("Alice"), "Should get Alice. stdout: {}, stderr: {}", output.stdout, output.stderr);
}
