//! Phase 48: Sipping Protocol Tests
//!
//! Tests for zero-copy file chunking and manifest generation.
//! This is the foundation of LOGOS's resumable file transfer protocol.

mod common;
use common::compile_to_rust;

// =============================================================================
// FileSipper Expression Tests
// =============================================================================

#[test]
fn test_manifest_of_zone() {
    let source = r#"## Main
    Inside a zone called "Data" mapped from "test.bin":
        Let m be the manifest of "Data"."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("FileSipper"), "Should use FileSipper");
    assert!(rust.contains(".manifest()"), "Should call manifest()");
}

#[test]
fn test_chunk_at_index_in_zone() {
    let source = r#"## Main
    Inside a zone called "Data" mapped from "test.bin":
        Let c be the chunk at 1 in "Data"."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("FileSipper"), "Should use FileSipper");
    assert!(rust.contains(".get_chunk("), "Should call get_chunk()");
}

#[test]
fn test_chunk_with_variable_index() {
    let source = r#"## Main
    Let i be 5.
    Inside a zone called "Data" mapped from "test.bin":
        Let c be the chunk at i in "Data"."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains(".get_chunk("), "Should call get_chunk with variable");
}

#[test]
fn test_portable_file_manifest() {
    let source = r#"## Definition
A FileManifest is Portable and has:
    a file_id (Text).
    a total_size (Nat).
    a chunk_count (Nat).

## Main
    Let m be a new FileManifest."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("Serialize"), "FileManifest should be Portable");
}

// =============================================================================
// Integration with Phase 46 Agents
// =============================================================================

#[test]
fn test_sipping_with_agents() {
    let source = r#"## Main
    Spawn a FileServer called "server".
    Inside a zone called "Data" mapped from "test.bin":
        Let m be the manifest of "Data".
        Send m to "server"."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("tokio::spawn"), "Should spawn agent");
    assert!(rust.contains(".manifest()"), "Should get manifest");
    assert!(rust.contains(".send("), "Should send to agent");
}

// =============================================================================
// Error Cases
// =============================================================================

#[test]
fn test_manifest_requires_of() {
    let source = r#"## Main
    Inside a zone called "Data" mapped from "test.bin":
        Let m be the manifest "Data"."#;
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail without 'of' keyword");
}

#[test]
fn test_chunk_requires_at() {
    let source = r#"## Main
    Inside a zone called "Data" mapped from "test.bin":
        Let c be the chunk in "Data"."#;
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail without 'at index'");
}

#[test]
fn test_chunk_requires_in() {
    let source = r#"## Main
    Inside a zone called "Data" mapped from "test.bin":
        Let c be the chunk at 1 "Data"."#;
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail without 'in Zone'");
}
