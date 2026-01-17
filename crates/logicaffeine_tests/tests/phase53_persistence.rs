//! Phase 53: VFS + Persistent<T> Journaling
//!
//! Tests for platform-agnostic file operations and crash-resilient CRDT persistence.
//! Enables durable state with automatic replay on restart.

mod common;
use common::compile_to_rust;

// =============================================================================
// Mount Statement Parsing
// =============================================================================

#[test]
fn test_mount_basic_parsing() {
    let source = r#"## Definition
A Counter is Shared and has:
    a points, which is ConvergentCount.

## Main
Mount counter at "data/counter.journal"."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("Persistent") && rust.contains("mount"),
        "Should generate Persistent::mount call. Got:\n{}",
        rust
    );
}

#[test]
fn test_mount_with_variable_path() {
    let source = r#"## Definition
A Counter is Shared and has:
    a value, which is ConvergentCount.

## Main
Let path be "journals/state.journal".
Mount state at path."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("mount") && rust.contains("path"),
        "Should generate mount with variable path. Got:\n{}",
        rust
    );
}

#[test]
fn test_mount_lowercase_variable() {
    let source = r#"## Definition
A Score is Shared and has:
    a points, which is ConvergentCount.

## Main
Mount myScore at "score.journal"."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("my_score") || rust.contains("myScore"),
        "Should handle camelCase variable names. Got:\n{}",
        rust
    );
}

// =============================================================================
// Persistent Type Annotation
// =============================================================================

#[test]
fn test_persistent_type_in_definition() {
    let source = r#"## Definition
A Counter is Shared and has:
    a points, which is ConvergentCount.

## Main
Let counter: Persistent Counter be mounted at "counter.journal"."#;

    let rust = compile_to_rust(source).expect("Should compile");
    // The "Let x: Persistent T be mounted at path" syntax is sugar for Mount
    // Rust infers the Persistent<Counter> type from the mount call
    assert!(
        rust.contains("Persistent") && rust.contains("mount") && rust.contains("counter"),
        "Should generate Persistent::mount call. Got:\n{}",
        rust
    );
}

// =============================================================================
// Async Detection
// =============================================================================

#[test]
fn test_mount_requires_async_main() {
    let source = r#"## Definition
A Counter is Shared and has:
    a points, which is ConvergentCount.

## Main
Mount c at "counter.journal"."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("#[tokio::main]") && rust.contains("async fn main"),
        "Mount should require async main. Got:\n{}",
        rust
    );
}

#[test]
fn test_file_read_requires_async_main() {
    let source = r#"## Main
Read data from file "input.txt".
Show data."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("#[tokio::main]") && rust.contains("async fn main"),
        "File read should require async main. Got:\n{}",
        rust
    );
}

#[test]
fn test_file_write_requires_async_main() {
    let source = r#"## Main
Write "hello world" to file "output.txt"."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("#[tokio::main]") && rust.contains("async fn main"),
        "File write should require async main. Got:\n{}",
        rust
    );
}

// =============================================================================
// VFS Injection
// =============================================================================

#[test]
fn test_vfs_injection_for_mount() {
    let source = r#"## Definition
A Counter is Shared and has:
    a points, which is ConvergentCount.

## Main
Mount x at "data.journal"."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("NativeVfs") || rust.contains("Vfs"),
        "Should inject VFS when Mount is used. Got:\n{}",
        rust
    );
}

#[test]
fn test_vfs_injection_for_file_read() {
    let source = r#"## Main
Read data from file "input.txt".
Show data."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("vfs") && rust.contains("read"),
        "ReadFrom File should use VFS. Got:\n{}",
        rust
    );
}

#[test]
fn test_vfs_injection_for_file_write() {
    let source = r#"## Main
Write "hello" to file "output.txt"."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("vfs") && rust.contains("write"),
        "WriteFile should use VFS. Got:\n{}",
        rust
    );
}

#[test]
fn test_no_vfs_for_console_read() {
    let source = r#"## Main
Read name from console.
Show name."#;

    let rust = compile_to_rust(source).expect("Should compile");
    // Console read should NOT require VFS (uses logicaffeine_system::io::read_line)
    assert!(
        rust.contains("read_line"),
        "Console read should use read_line. Got:\n{}",
        rust
    );
}

// =============================================================================
// Integration with CRDT Operations
// =============================================================================

#[test]
fn test_persistent_with_increase() {
    let source = r#"## Definition
A Counter is Shared and has:
    a points, which is ConvergentCount.

## Main
Mount c at "counter.journal".
Increase c's points by 10."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("mutate"),
        "CRDT operations on Persistent should use mutate(). Got:\n{}",
        rust
    );
}

#[test]
fn test_persistent_with_merge() {
    let source = r#"## Definition
A Counter is Shared and has:
    a points, which is ConvergentCount.

## Main
Mount local at "local.journal".
Let remote be a new Counter.
Merge remote into local."#;

    let rust = compile_to_rust(source).expect("Should compile");
    // Merge into persistent should also use mutate
    assert!(
        rust.contains("mutate") || rust.contains("merge"),
        "Merge into Persistent should persist changes. Got:\n{}",
        rust
    );
}

#[test]
fn test_persistent_field_access() {
    let source = r#"## Definition
A Counter is Shared and has:
    a points, which is ConvergentCount.

## Main
Mount c at "counter.journal".
Show c's points."#;

    let rust = compile_to_rust(source).expect("Should compile");
    // Reading from persistent should use get() or similar
    assert!(
        rust.contains("get()") || rust.contains(".points"),
        "Should be able to read persistent fields. Got:\n{}",
        rust
    );
}

// =============================================================================
// Error Cases
// =============================================================================

#[test]
fn test_mount_missing_at_fails() {
    let source = r#"## Definition
A Counter is Shared and has:
    a points, which is ConvergentCount.

## Main
Mount x "path.journal"."#;

    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail without 'at' keyword");
}

#[test]
fn test_mount_missing_path_fails() {
    let source = r#"## Definition
A Counter is Shared and has:
    a points, which is ConvergentCount.

## Main
Mount x at."#;

    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail without path expression");
}

#[test]
fn test_mount_missing_variable_fails() {
    let source = r#"## Main
Mount at "path.journal"."#;

    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail without variable name");
}

// =============================================================================
// Complex Scenarios
// =============================================================================

#[test]
fn test_multiple_mounts() {
    let source = r#"## Definition
A Counter is Shared and has:
    a points, which is ConvergentCount.

A Profile is Shared and has:
    a name, which is LastWriteWins of Text.

## Main
Mount counter at "counter.journal".
Mount profile at "profile.journal".
Increase counter's points by 5."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("counter") && rust.contains("profile"),
        "Should handle multiple mounts. Got:\n{}",
        rust
    );
}

#[test]
fn test_mount_in_conditional() {
    let source = r#"## Definition
A Counter is Shared and has:
    a points, which is ConvergentCount.

## Main
Let load be true.
If load:
    Mount c at "counter.journal".
    Show c's points."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("mount") && rust.contains("if"),
        "Should allow Mount in conditional blocks. Got:\n{}",
        rust
    );
}

#[test]
fn test_mount_with_string_concat_path() {
    let source = r#"## Definition
A Counter is Shared and has:
    a points, which is ConvergentCount.

## Main
Let base be "data/".
Let filename be "counter.journal".
Mount c at base combined with filename."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("mount"),
        "Should allow dynamic path construction. Got:\n{}",
        rust
    );
}

// =============================================================================
// VFS Await Semantics
// =============================================================================

#[test]
fn test_mount_generates_await() {
    let source = r#"## Definition
A Counter is Shared and has:
    a points, which is ConvergentCount.

## Main
Mount c at "counter.journal"."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains(".await"),
        "Mount should generate .await for async operation. Got:\n{}",
        rust
    );
}

#[test]
fn test_file_read_generates_await() {
    let source = r#"## Main
Read data from file "input.txt".
Show data."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains(".await"),
        "File read should generate .await. Got:\n{}",
        rust
    );
}

#[test]
fn test_file_write_generates_await() {
    let source = r#"## Main
Write "hello" to file "output.txt"."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains(".await"),
        "File write should generate .await. Got:\n{}",
        rust
    );
}
