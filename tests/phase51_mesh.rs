//! Phase 51: The Mesh (P2P Networking)
//!
//! Tests for P2P networking primitives:
//! - Listen on [address] - bind to network address
//! - Connect to [address] - dial a peer
//! - Let x be a PeerAgent at [address] - create remote handle
//! - Result-returning Send for network operations
//!
//! Architecture decisions (AAA Council):
//! - Transport: libp2p (QUIC-first)
//! - Serialization: bincode via LogosWire trait
//! - Error handling: Result types (Success/Failure)
//! - E2E testing: In-process tokio tasks

mod common;
use common::compile_to_rust;

// =============================================================================
// Listen Statement Tests
// =============================================================================

#[test]
fn test_listen_basic() {
    let source = r#"## Main
Listen on "/ip4/127.0.0.1/tcp/8000"."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("logos_core::network") && rust.contains("listen"),
        "Should generate listen call. Got:\n{}",
        rust
    );
}

#[test]
fn test_listen_with_variable() {
    let source = r#"## Main
Let addr be "/ip4/0.0.0.0/tcp/9000".
Listen on addr."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("listen") && rust.contains("addr"),
        "Should generate listen with variable"
    );
}

#[test]
fn test_listen_with_concatenation() {
    let source = r#"## Main
Let port be "8000".
Listen on "/ip4/127.0.0.1/tcp/" + port."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("listen"), "Should generate listen with concatenated address");
}

// =============================================================================
// Connect Statement Tests
// =============================================================================

#[test]
fn test_connect_basic() {
    let source = r#"## Main
Connect to "/ip4/127.0.0.1/tcp/8000"."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("logos_core::network") && rust.contains("connect"),
        "Should generate connect call. Got:\n{}",
        rust
    );
}

#[test]
fn test_connect_with_variable() {
    let source = r#"## Main
Let peer_addr be "/ip4/192.168.1.5/tcp/8000".
Connect to peer_addr."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("connect") && rust.contains("peer_addr"),
        "Should generate connect with variable"
    );
}

// =============================================================================
// PeerAgent at Address Tests
// =============================================================================

#[test]
fn test_peer_agent_at_address() {
    let source = r#"## Main
Let remote be a PeerAgent at "/ip4/127.0.0.1/tcp/8000"."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("PeerAgent") && rust.contains("new"),
        "Should create PeerAgent. Got:\n{}",
        rust
    );
}

#[test]
fn test_peer_agent_at_variable() {
    let source = r#"## Main
Let addr be "/ip4/127.0.0.1/tcp/8000".
Let remote be a PeerAgent at addr."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("PeerAgent") && rust.contains("addr"),
        "Should create PeerAgent with variable address"
    );
}

// =============================================================================
// Send to Remote Agent Tests
// =============================================================================

#[test]
fn test_send_to_peer_agent() {
    let source = r#"## Definition
A Ping is Portable and has:
    a sender_id (Text).

## Main
Let remote be a PeerAgent at "/ip4/127.0.0.1/tcp/8000".
Let msg be a new Ping with sender_id "client".
Send msg to remote."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("send"), "Should generate send call");
}

// =============================================================================
// Combined Listen + Connect Workflow
// =============================================================================

#[test]
fn test_server_client_workflow() {
    let source = r#"## Definition
A Message is Portable and has:
    a content (Text).

## Main
Listen on "/ip4/0.0.0.0/tcp/8000".
Connect to "/ip4/127.0.0.1/tcp/9000".
Let remote be a PeerAgent at "/ip4/127.0.0.1/tcp/9000".
Let msg be a new Message with content "hello".
Send msg to remote."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("listen"), "Should have listen");
    assert!(rust.contains("connect"), "Should have connect");
    assert!(rust.contains("PeerAgent"), "Should have PeerAgent");
    assert!(rust.contains("send"), "Should have send");
}

// =============================================================================
// Error Cases
// =============================================================================

#[test]
fn test_listen_missing_on_fails() {
    let source = r#"## Main
Listen "/ip4/127.0.0.1/tcp/8000"."#;
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail without 'on' keyword");
}

#[test]
fn test_connect_missing_to_fails() {
    let source = r#"## Main
Connect "/ip4/127.0.0.1/tcp/8000"."#;
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail without 'to' keyword");
}

#[test]
fn test_peer_agent_missing_at_fails() {
    let source = r#"## Main
Let remote be a PeerAgent "/ip4/127.0.0.1/tcp/8000"."#;
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail without 'at' keyword");
}

// =============================================================================
// Integration with Portable Types (Phase 47)
// =============================================================================

#[test]
fn test_mesh_with_portable_types() {
    // Simpler test without struct initialization complexity
    let source = r#"## Main
Listen on "/ip4/0.0.0.0/tcp/8000".
Let remote be a PeerAgent at "/ip4/127.0.0.1/tcp/9000".
Let msg be "hello world".
Send msg to remote."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("listen"), "Should have listen call");
    assert!(rust.contains("PeerAgent"), "Should have PeerAgent");
    assert!(rust.contains("send"), "Should have send call");
}
