//! Phase 52 Part 2: E2E tests for GossipSub CRDT replication.
//!
//! These tests verify that CRDTs automatically synchronize between nodes
//! using libp2p GossipSub.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::run_logos;

/// Test that two nodes can synchronize a GCounter via GossipSub.
///
/// Node A increments its counter, and Node B should see the update
/// through automatic CRDT replication - no explicit Send required.
///
/// This test runs two separate processes to ensure proper mDNS discovery.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_crdt_gossip_convergence() {
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::Duration;

    // Node A: Publishes after discovering peers and mesh formation
    // Listen on 0.0.0.0 with random port to avoid conflicts
    let source_a = r#"## Definition
A GameState is Shared and has:
    clicks: ConvergentCount.

## Main
Listen on "/ip4/0.0.0.0/tcp/0".
Let state be a new GameState.
Sync state on "game_sync".
Sleep 8000.
Increase state's clicks by 5.
Show "NODE_A: published 5".
Sleep 4000.
"#;

    // Node B: Waits and reads the synced value
    let source_b = r#"## Definition
A GameState is Shared and has:
    clicks: ConvergentCount.

## Main
Listen on "/ip4/0.0.0.0/tcp/0".
Let state be a new GameState.
Sync state on "game_sync".
Sleep 14000.
Show state's clicks.
If state's clicks equals 5:
    Show "SYNC_SUCCESS".
"#;

    // Compile both programs
    let result_a = common::compile_logos(source_a);
    assert!(result_a.success, "Node A should compile: {}", result_a.stderr);

    let result_b = common::compile_logos(source_b);
    assert!(result_b.success, "Node B should compile: {}", result_b.stderr);

    // Run both as separate processes
    let mut child_a = Command::new(&result_a.binary_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start Node A");

    // Small delay to let A start listening
    thread::sleep(Duration::from_millis(500));

    let mut child_b = Command::new(&result_b.binary_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start Node B");

    // Wait for both with timeout
    let output_a = child_a.wait_with_output().expect("Failed to wait for Node A");
    let output_b = child_b.wait_with_output().expect("Failed to wait for Node B");

    let stdout_a = String::from_utf8_lossy(&output_a.stdout);
    let stderr_a = String::from_utf8_lossy(&output_a.stderr);
    let stdout_b = String::from_utf8_lossy(&output_b.stdout);
    let stderr_b = String::from_utf8_lossy(&output_b.stderr);

    eprintln!("=== Node A stdout ===\n{}", stdout_a);
    eprintln!("=== Node A stderr ===\n{}", stderr_a);
    eprintln!("=== Node B stdout ===\n{}", stdout_b);
    eprintln!("=== Node B stderr ===\n{}", stderr_b);

    assert!(
        stdout_b.contains("SYNC_SUCCESS"),
        "Node B should see synced value. stdout: {}", stdout_b
    );
}

/// Test basic GossipSub subscription without network - just compilation.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_gossip_sync_compiles() {
    let source = r#"## Definition
A Counter is Shared and has:
    value: ConvergentCount.

## Main
Listen on "/ip4/127.0.0.1/tcp/0".
Let c be a new Counter.
Sync c on "test-topic".
Show "Subscribed to topic".
"#;

    let result = run_logos(source);
    assert!(result.success, "Sync should compile and run: {}", result.stderr);
    assert!(
        result.stdout.contains("Subscribed to topic"),
        "Should reach subscription. stdout: {}", result.stdout
    );
}
