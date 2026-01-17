//! Phase 52: The Sync (GossipSub & Automatic Replication)
//!
//! Tests for automatic CRDT synchronization over GossipSub:
//! - Sync [var] on [topic] - subscribe and auto-replicate
//! - Delta broadcast on mutation
//! - Auto-merge on incoming messages

mod common;
use common::compile_to_rust;

// =============================================================================
// Basic Sync Statement Parsing
// =============================================================================

#[test]
fn test_sync_basic_parsing() {
    let source = r#"## Definition
A Counter is Shared and has:
    points: ConvergentCount.

## Main
Let mutable c be a new Counter.
Sync c on "counters"."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("Synced"),
        "Should generate Synced wrapper. Got:\n{}",
        rust
    );
}

#[test]
fn test_sync_with_variable_topic() {
    let source = r#"## Definition
A State is Shared and has:
    value: ConvergentCount.

## Main
Let topic be "my-room".
Let mutable s be a new State.
Sync s on topic."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("Synced::new") && rust.contains("topic"),
        "Should generate Synced with variable topic. Got:\n{}",
        rust
    );
}

// =============================================================================
// Async Detection
// =============================================================================

#[test]
fn test_sync_requires_async_main() {
    let source = r#"## Definition
A Counter is Shared and has:
    points: ConvergentCount.

## Main
Let mutable c be a new Counter.
Sync c on "test"."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("#[tokio::main]") && rust.contains("async fn main"),
        "Sync should require async main. Got:\n{}",
        rust
    );
}

// =============================================================================
// Integration with CRDT Operations
// =============================================================================

#[test]
fn test_sync_with_increase() {
    let source = r#"## Definition
A Counter is Shared and has:
    points: ConvergentCount.

## Main
Let mutable c be a new Counter.
Sync c on "game-scores".
Increase c's points by 10."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("Synced"), "Should have Synced wrapper");
    assert!(rust.contains("increment"), "Should still support increment");
}

#[test]
fn test_sync_with_lww_register() {
    let source = r#"## Definition
A Profile is Shared and has:
    username: LastWriteWins of Text.

## Main
Let mutable p be a new Profile.
Sync p on "profiles"."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("Synced"), "Should wrap in Synced");
}

// =============================================================================
// Error Cases
// =============================================================================

#[test]
fn test_sync_missing_on_fails() {
    let source = r#"## Main
Let mutable c be 0.
Sync c "topic"."#;

    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail without 'on' keyword");
}

#[test]
fn test_sync_missing_topic_fails() {
    let source = r#"## Main
Let mutable c be 0.
Sync c on."#;

    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail without topic");
}
