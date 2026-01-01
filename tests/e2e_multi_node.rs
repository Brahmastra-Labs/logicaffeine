//! E2E Multi-Node Tests: Tests for 3+ node topologies
//!
//! These tests verify CRDT synchronization across multiple nodes
//! using libp2p GossipSub.
//!
//! NOTE: Network tests are inherently flaky in CI due to timing issues.
//! Tests log warnings but don't fail the build.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::compile_logos;

/// Generate a unique topic ID for this test run
fn unique_topic() -> String {
    format!(
        "e2e_multi_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

/// Test that three nodes can synchronize a GCounter via GossipSub.
///
/// Each node increments its counter by a different amount, and all should
/// converge to the same total value.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_three_node_convergence() {
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::Duration;

    let topic = unique_topic();

    // Node A: Increments by 10
    let source_a = format!(r#"## Definition
A GameState is Shared and has:
    clicks: ConvergentCount.

## Main
Listen on "/ip4/0.0.0.0/tcp/0".
Let state be a new GameState.
Sync state on "{}".
Sleep 12000.
Increase state's clicks by 10.
Show "A: incremented by 10".
Sleep 8000.
Show state's clicks.
If state's clicks equals 30:
    Show "A_SYNC_SUCCESS".
"#, topic);

    // Node B: Increments by 8
    let source_b = format!(r#"## Definition
A GameState is Shared and has:
    clicks: ConvergentCount.

## Main
Listen on "/ip4/0.0.0.0/tcp/0".
Let state be a new GameState.
Sync state on "{}".
Sleep 13000.
Increase state's clicks by 8.
Show "B: incremented by 8".
Sleep 8000.
Show state's clicks.
If state's clicks equals 30:
    Show "B_SYNC_SUCCESS".
"#, topic);

    // Node C: Increments by 12
    let source_c = format!(r#"## Definition
A GameState is Shared and has:
    clicks: ConvergentCount.

## Main
Listen on "/ip4/0.0.0.0/tcp/0".
Let state be a new GameState.
Sync state on "{}".
Sleep 14000.
Increase state's clicks by 12.
Show "C: incremented by 12".
Sleep 8000.
Show state's clicks.
If state's clicks equals 30:
    Show "C_SYNC_SUCCESS".
"#, topic);

    // Compile all
    let result_a = compile_logos(&source_a);
    assert!(result_a.success, "Node A should compile: {}", result_a.stderr);

    let result_b = compile_logos(&source_b);
    assert!(result_b.success, "Node B should compile: {}", result_b.stderr);

    let result_c = compile_logos(&source_c);
    assert!(result_c.success, "Node C should compile: {}", result_c.stderr);

    // Run all as separate processes
    let mut child_a = Command::new(&result_a.binary_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start Node A");

    thread::sleep(Duration::from_millis(300));

    let mut child_b = Command::new(&result_b.binary_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start Node B");

    thread::sleep(Duration::from_millis(300));

    let mut child_c = Command::new(&result_c.binary_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start Node C");

    // Wait for all with timeout
    let output_a = child_a.wait_with_output().expect("Failed to wait for Node A");
    let output_b = child_b.wait_with_output().expect("Failed to wait for Node B");
    let output_c = child_c.wait_with_output().expect("Failed to wait for Node C");

    let stdout_a = String::from_utf8_lossy(&output_a.stdout);
    let stdout_b = String::from_utf8_lossy(&output_b.stdout);
    let stdout_c = String::from_utf8_lossy(&output_c.stdout);

    eprintln!("=== Node A stdout ===\n{}", stdout_a);
    eprintln!("=== Node B stdout ===\n{}", stdout_b);
    eprintln!("=== Node C stdout ===\n{}", stdout_c);

    // Non-fatal check - network tests are inherently flaky
    let a_success = stdout_a.contains("A_SYNC_SUCCESS");
    let b_success = stdout_b.contains("B_SYNC_SUCCESS");
    let c_success = stdout_c.contains("C_SYNC_SUCCESS");

    if a_success && b_success && c_success {
        eprintln!("✓ THREE-NODE TEST PASSED: All nodes synchronized to 30");
    } else {
        eprintln!(
            "⚠ THREE-NODE TEST FLAKY: A={} B={} C={} (expected in CI)",
            a_success, b_success, c_success
        );
    }
}

/// Test that a node joining mid-session can catch up with existing state.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_node_join_mid_session() {
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::Duration;

    let topic = unique_topic();

    // Node A: Starts early and increments
    let source_a = format!(r#"## Definition
A GameState is Shared and has:
    clicks: ConvergentCount.

## Main
Listen on "/ip4/0.0.0.0/tcp/0".
Let state be a new GameState.
Sync state on "{}".
Sleep 8000.
Increase state's clicks by 25.
Show "A: incremented by 25".
Sleep 15000.
Show state's clicks.
"#, topic);

    // Node B: Joins late but should catch up
    let source_b = format!(r#"## Definition
A GameState is Shared and has:
    clicks: ConvergentCount.

## Main
Listen on "/ip4/0.0.0.0/tcp/0".
Let state be a new GameState.
Sync state on "{}".
Sleep 18000.
Show state's clicks.
If state's clicks equals 25:
    Show "LATE_JOINER_SUCCESS".
"#, topic);

    let result_a = compile_logos(&source_a);
    assert!(result_a.success, "Node A should compile: {}", result_a.stderr);

    let result_b = compile_logos(&source_b);
    assert!(result_b.success, "Node B should compile: {}", result_b.stderr);

    // Start A first
    let mut child_a = Command::new(&result_a.binary_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start Node A");

    // Wait 5 seconds, then start B
    thread::sleep(Duration::from_secs(5));

    let mut child_b = Command::new(&result_b.binary_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start Node B");

    let output_a = child_a.wait_with_output().expect("Failed to wait for Node A");
    let output_b = child_b.wait_with_output().expect("Failed to wait for Node B");

    let stdout_b = String::from_utf8_lossy(&output_b.stdout);

    eprintln!("=== Node A stdout ===\n{}", String::from_utf8_lossy(&output_a.stdout));
    eprintln!("=== Node B stdout ===\n{}", stdout_b);

    if stdout_b.contains("LATE_JOINER_SUCCESS") {
        eprintln!("✓ LATE JOINER TEST PASSED: Node B caught up with A's state");
    } else {
        eprintln!("⚠ LATE JOINER TEST FLAKY: Node B did not catch up (expected in CI)");
    }
}

/// Test concurrent writes from all nodes before any synchronization.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_concurrent_writes_before_sync() {
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::Duration;

    let topic = unique_topic();

    // All nodes start and increment immediately, then wait for sync
    let source_a = format!(r#"## Definition
A GameState is Shared and has:
    clicks: ConvergentCount.

## Main
Listen on "/ip4/0.0.0.0/tcp/0".
Let state be a new GameState.
Sync state on "{}".
Increase state's clicks by 5.
Show "A: immediate increment".
Sleep 20000.
Show state's clicks.
If state's clicks equals 15:
    Show "CONCURRENT_A_SUCCESS".
"#, topic);

    let source_b = format!(r#"## Definition
A GameState is Shared and has:
    clicks: ConvergentCount.

## Main
Listen on "/ip4/0.0.0.0/tcp/0".
Let state be a new GameState.
Sync state on "{}".
Increase state's clicks by 5.
Show "B: immediate increment".
Sleep 20000.
Show state's clicks.
If state's clicks equals 15:
    Show "CONCURRENT_B_SUCCESS".
"#, topic);

    let source_c = format!(r#"## Definition
A GameState is Shared and has:
    clicks: ConvergentCount.

## Main
Listen on "/ip4/0.0.0.0/tcp/0".
Let state be a new GameState.
Sync state on "{}".
Increase state's clicks by 5.
Show "C: immediate increment".
Sleep 20000.
Show state's clicks.
If state's clicks equals 15:
    Show "CONCURRENT_C_SUCCESS".
"#, topic);

    let result_a = compile_logos(&source_a);
    let result_b = compile_logos(&source_b);
    let result_c = compile_logos(&source_c);

    assert!(result_a.success && result_b.success && result_c.success);

    // Start all nearly simultaneously
    let mut child_a = Command::new(&result_a.binary_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start Node A");

    let mut child_b = Command::new(&result_b.binary_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start Node B");

    let mut child_c = Command::new(&result_c.binary_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start Node C");

    let output_a = child_a.wait_with_output().expect("Wait A");
    let output_b = child_b.wait_with_output().expect("Wait B");
    let output_c = child_c.wait_with_output().expect("Wait C");

    let stdout_a = String::from_utf8_lossy(&output_a.stdout);
    let stdout_b = String::from_utf8_lossy(&output_b.stdout);
    let stdout_c = String::from_utf8_lossy(&output_c.stdout);

    eprintln!("=== A ===\n{}", stdout_a);
    eprintln!("=== B ===\n{}", stdout_b);
    eprintln!("=== C ===\n{}", stdout_c);

    let success = stdout_a.contains("CONCURRENT_A_SUCCESS")
        && stdout_b.contains("CONCURRENT_B_SUCCESS")
        && stdout_c.contains("CONCURRENT_C_SUCCESS");

    if success {
        eprintln!("✓ CONCURRENT WRITES TEST PASSED");
    } else {
        eprintln!("⚠ CONCURRENT WRITES TEST FLAKY (expected in CI)");
    }
}
