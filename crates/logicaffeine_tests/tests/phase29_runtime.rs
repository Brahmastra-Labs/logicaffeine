//! Phase 29: Runtime Injection
//!
//! Tests that logicaffeine_data and logicaffeine_system are properly embedded
//! in the compiler binary and injected into the output directory when compiling LOGOS programs.

use logicaffeine_compile::compile::compile_to_dir;
use tempfile::TempDir;

#[test]
fn runtime_crates_are_injected() {
    let source = "## Main\nLet x: Text be \"Hello\".";
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    compile_to_dir(source, temp_dir.path()).expect("Compilation failed");

    // Verify runtime crates were written
    assert!(temp_dir.path().join("crates/logicaffeine_data/Cargo.toml").exists(), "Missing logicaffeine_data Cargo.toml");
    assert!(temp_dir.path().join("crates/logicaffeine_data/src/lib.rs").exists(), "Missing logicaffeine_data lib.rs");
    assert!(temp_dir.path().join("crates/logicaffeine_system/Cargo.toml").exists(), "Missing logicaffeine_system Cargo.toml");
    assert!(temp_dir.path().join("crates/logicaffeine_system/src/lib.rs").exists(), "Missing logicaffeine_system lib.rs");

    // Verify main.rs imports the crates
    let main_rs = std::fs::read_to_string(temp_dir.path().join("src/main.rs")).unwrap();
    assert!(main_rs.contains("use logicaffeine_data::*;") || main_rs.contains("logicaffeine_data"), "Missing logicaffeine_data import");
    assert!(main_rs.contains("use logicaffeine_system::*;") || main_rs.contains("logicaffeine_system"), "Missing logicaffeine_system import");
    // TempDir auto-cleans on drop
}

#[test]
fn runtime_has_type_aliases() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    compile_to_dir("## Main\nReturn.", temp_dir.path()).expect("Compilation failed");

    let types_rs = std::fs::read_to_string(temp_dir.path().join("crates/logicaffeine_data/src/types.rs")).unwrap();
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

    let io_rs = std::fs::read_to_string(temp_dir.path().join("crates/logicaffeine_system/src/io.rs")).unwrap();
    assert!(io_rs.contains("pub fn show"), "Missing show function");
    assert!(io_rs.contains("pub fn read_line"), "Missing read_line function");
    assert!(io_rs.contains("pub fn println"), "Missing println function");
    // TempDir auto-cleans on drop
}

#[test]
fn crates_export_required_modules() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    compile_to_dir("## Main\nReturn.", temp_dir.path()).expect("Compilation failed");

    // Check logicaffeine_data exports
    let data_lib = std::fs::read_to_string(temp_dir.path().join("crates/logicaffeine_data/src/lib.rs")).unwrap();
    assert!(data_lib.contains("pub mod types") || data_lib.contains("pub use types"), "logicaffeine_data missing types module");

    // Check logicaffeine_system exports
    let system_lib = std::fs::read_to_string(temp_dir.path().join("crates/logicaffeine_system/src/lib.rs")).unwrap();
    assert!(system_lib.contains("pub mod io") || system_lib.contains("pub use io"), "logicaffeine_system missing io module");
    // TempDir auto-cleans on drop
}
