//! Phase 29: Runtime Injection
//!
//! Tests that logos_core is properly embedded in the compiler binary
//! and injected into the output directory when compiling LOGOS programs.

use logos::compile::compile_to_dir;
use tempfile::TempDir;

#[test]
fn runtime_files_are_injected() {
    let source = "## Main\nLet x: Text be \"Hello\".";
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    compile_to_dir(source, temp_dir.path()).expect("Compilation failed");

    // Verify logos_core was written
    assert!(temp_dir.path().join("logos_core/Cargo.toml").exists(), "Missing Cargo.toml");
    assert!(temp_dir.path().join("logos_core/src/lib.rs").exists(), "Missing lib.rs");
    assert!(temp_dir.path().join("logos_core/src/types.rs").exists(), "Missing types.rs");
    assert!(temp_dir.path().join("logos_core/src/io.rs").exists(), "Missing io.rs");

    // Verify main.rs imports prelude
    let main_rs = std::fs::read_to_string(temp_dir.path().join("src/main.rs")).unwrap();
    assert!(main_rs.contains("use logos_core::prelude::*;"), "Missing prelude import");
    // TempDir auto-cleans on drop
}

#[test]
fn runtime_has_type_aliases() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    compile_to_dir("## Main\nReturn.", temp_dir.path()).expect("Compilation failed");

    let types_rs = std::fs::read_to_string(temp_dir.path().join("logos_core/src/types.rs")).unwrap();
    assert!(types_rs.contains("pub type Nat = u64;"), "Missing Nat type");
    assert!(types_rs.contains("pub type Int = i64;"), "Missing Int type");
    assert!(types_rs.contains("pub type Text = String;"), "Missing Text type");
    assert!(types_rs.contains("pub type Bool = bool;"), "Missing Bool type");
    assert!(types_rs.contains("pub type Real = f64;"), "Missing Real type");
    assert!(types_rs.contains("pub type Unit = ();"), "Missing Unit type");
    // TempDir auto-cleans on drop
}

#[test]
fn runtime_has_io_functions() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    compile_to_dir("## Main\nReturn.", temp_dir.path()).expect("Compilation failed");

    let io_rs = std::fs::read_to_string(temp_dir.path().join("logos_core/src/io.rs")).unwrap();
    assert!(io_rs.contains("pub fn show"), "Missing show function");
    assert!(io_rs.contains("pub fn read_line"), "Missing read_line function");
    assert!(io_rs.contains("pub fn println"), "Missing println function");
    // TempDir auto-cleans on drop
}

#[test]
fn prelude_exports_all() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    compile_to_dir("## Main\nReturn.", temp_dir.path()).expect("Compilation failed");

    let lib_rs = std::fs::read_to_string(temp_dir.path().join("logos_core/src/lib.rs")).unwrap();

    // Check prelude re-exports
    assert!(lib_rs.contains("pub mod prelude"), "Missing prelude module");
    assert!(lib_rs.contains("pub mod io"), "Missing io module");
    assert!(lib_rs.contains("pub mod types"), "Missing types module");
    // TempDir auto-cleans on drop
}
