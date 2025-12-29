//! Phase 46: Agent System Tests
//!
//! Tests for actor-style agents with typed message passing.
//! This is the foundation of LOGOS's distributed operating system.

mod common;
use common::compile_to_rust;

// =============================================================================
// Agent Spawn Tests
// =============================================================================

#[test]
fn test_agent_spawn_basic() {
    let source = "## Main\nSpawn an EchoAgent called \"echo\".";
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("tokio::spawn"),
        "Should generate tokio::spawn for agent"
    );
}

#[test]
fn test_agent_spawn_with_article_a() {
    let source = "## Main\nSpawn a Worker called \"w1\".";
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("tokio::spawn"),
        "Should generate tokio::spawn for agent with 'a'"
    );
}

// =============================================================================
// Send Message Tests
// =============================================================================

#[test]
fn test_send_message_basic() {
    let source = "## Main\nSend Ping to \"echo\".";
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains(".send("),
        "Should generate channel send"
    );
}

#[test]
fn test_send_message_with_payload() {
    let source = "## Main\nLet msg be \"hello\".\nSend msg to \"echo\".";
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains(".send("),
        "Should generate channel send with payload"
    );
}

// =============================================================================
// Await Response Tests
// =============================================================================

#[test]
fn test_await_response_basic() {
    let source = "## Main\nAwait response from \"echo\" into result.";
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains(".recv()"), "Should generate channel recv");
    assert!(rust.contains(".await"), "Should be async");
}

#[test]
fn test_await_binds_to_variable() {
    let source = "## Main\nAwait response from \"worker\" into data.";
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("let data"),
        "Should bind result to variable"
    );
}

// =============================================================================
// Portable Struct Tests (Phase 47)
// =============================================================================

#[test]
fn test_portable_struct_derives_serde() {
    let source = r#"## Definition
A Message is Portable and has:
    a content (Text).

## Main
    Let m be a new Message with content "hello"."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("Serialize"),
        "Should derive Serialize for Portable"
    );
    assert!(
        rust.contains("Deserialize"),
        "Should derive Deserialize for Portable"
    );
}

#[test]
fn test_portable_enum_derives_serde() {
    let source = r#"## Definition
A Command is Portable and is either:
    a Start.
    a Stop.
    a Pause.

## Main
    Let cmd be a new Start."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("Serialize"),
        "Portable enum should derive Serialize"
    );
}

// =============================================================================
// Error Cases
// =============================================================================

#[test]
fn test_spawn_missing_called_fails() {
    let source = "## Main\nSpawn an EchoAgent \"echo\".";
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail without 'called' keyword");
}

#[test]
fn test_send_missing_to_fails() {
    let source = "## Main\nSend Ping \"echo\".";
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail without 'to' keyword");
}

#[test]
fn test_await_missing_from_fails() {
    let source = "## Main\nAwait response \"echo\" into result.";
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail without 'from' keyword");
}

#[test]
fn test_await_missing_into_fails() {
    let source = "## Main\nAwait response from \"echo\".";
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail without 'into' keyword");
}

// =============================================================================
// Integration Tests
// =============================================================================

#[test]
fn test_spawn_and_send_in_sequence() {
    let source = r#"## Main
    Spawn a Worker called "worker".
    Send Start to "worker".
    Await response from "worker" into result."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("tokio::spawn"), "Should have spawn");
    assert!(rust.contains(".send("), "Should have send");
    assert!(rust.contains(".recv()"), "Should have recv");
}

#[test]
fn test_agent_in_function() {
    let source = r#"## To start_worker:
    Spawn a Worker called "w".
    Send Init to "w".
    Await response from "w" into status.
    Return status.

## Main
    Call start_worker."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("fn start_worker"),
        "Should have function definition"
    );
    assert!(
        rust.contains("tokio::spawn"),
        "Should have agent spawn in function"
    );
}
