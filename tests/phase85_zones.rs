//! Phase 8.5 & 8.6: Zone System Tests
//!
//! Tests for region-based memory management (Arenas) and memory-mapped files.

mod common;
use common::compile_to_rust;

// =============================================================================
// Codegen Tests - Verify Rust Output
// =============================================================================

#[test]
fn test_zone_heap_basic() {
    let source = "## Main\nInside a new zone called \"Scratch\":\n    Let x be 5.";
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("Zone::new_heap"), "Should generate Zone::new_heap");
}

#[test]
fn test_zone_with_size_mb() {
    let source = "## Main\nInside a zone called \"Arena\" of size 2 MB:\n    Let x be 5.";
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("Zone::new_heap(2097152)"), "Should have 2MB = 2097152 bytes");
}

#[test]
fn test_zone_with_size_kb() {
    let source = "## Main\nInside a zone called \"Small\" of size 64 KB:\n    Let value be 1.";
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("Zone::new_heap(65536)"), "Should have 64KB = 65536 bytes");
}

#[test]
fn test_zone_mapped_from_file() {
    let source = "## Main\nInside a zone called \"Data\" mapped from \"input.bin\":\n    Let bytes be 0.";
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("Zone::new_mapped"), "Should generate Zone::new_mapped");
    assert!(rust.contains("input.bin"), "Should include file path");
}

#[test]
fn test_zone_creates_block_scope() {
    let source = "## Main\nLet x be 1.\nInside a zone called \"Work\":\n    Let y be 2.\nLet z be 3.";
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("let x = 1;"), "Should have x variable");
    assert!(rust.contains("let z = 3;"), "Should have z variable");
    assert!(rust.contains("{"), "Should have block scope");
}

#[test]
fn test_zone_default_capacity() {
    let source = "## Main\nInside a zone called \"Default\":\n    Let x be 1.";
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("Zone::new_heap(4096)"), "Default should be 4KB");
}

#[test]
fn test_zone_in_function() {
    let source = "## To process:\n    Inside a zone called \"Temp\":\n        Let x be 1.\n    Return.\n\n## Main\nCall process.";
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("fn process"), "Should have function");
    assert!(rust.contains("Zone::new_heap"), "Should have zone in function");
}

#[test]
fn test_zone_with_multiple_statements() {
    let source = "## Main\nInside a zone called \"Work\" of size 512 KB:\n    Let a be 1.\n    Let b be 2.\n    Let c be 3.";
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("Zone::new_heap(524288)"), "Should have 512KB zone");
    assert!(rust.contains("let a = 1;"), "Should have a");
    assert!(rust.contains("let b = 2;"), "Should have b");
    assert!(rust.contains("let c = 3;"), "Should have c");
}

#[test]
fn test_zone_with_control_flow() {
    let source = "## Main\nInside a zone called \"Arena\":\n    Let x be 5.\n    If x equals 5:\n        Let y be 10.";
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("Zone::new_heap"), "Should have zone");
    assert!(rust.contains("if"), "Should have if statement");
}

// =============================================================================
// Error Cases
// =============================================================================

#[test]
fn test_zone_missing_colon_fails() {
    let source = "## Main\nInside a zone called \"Test\"\n    Let x be 5.";
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail without colon");
}

#[test]
fn test_zone_missing_called_fails() {
    let source = "## Main\nInside a zone \"Test\":\n    Let x be 5.";
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail without 'called' keyword");
}

#[test]
fn test_zone_missing_name_fails() {
    let source = "## Main\nInside a zone called:\n    Let x be 5.";
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail without zone name");
}
