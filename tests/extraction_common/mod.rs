//! E2E Test Harness for Kernel Extraction
//!
//! Compiles extracted Rust code and runs it.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

static SHARED_TARGET_DIR: OnceLock<PathBuf> = OnceLock::new();
static COMPILE_COUNTER: AtomicU64 = AtomicU64::new(0);
static RUN_ID: OnceLock<u64> = OnceLock::new();

fn get_run_id() -> u64 {
    *RUN_ID.get_or_init(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64
    })
}

fn get_shared_target_dir() -> &'static PathBuf {
    SHARED_TARGET_DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join("logos_extraction_e2e_cache");
        std::fs::create_dir_all(&dir).expect("Failed to create shared target dir");
        dir
    })
}

pub struct E2EResult {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
    pub rust_code: String,
}

/// Run extracted Rust code with a custom main function.
pub fn run_extracted(rust_code: &str, main_code: &str) -> E2EResult {
    // Create temp project
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let project_dir = temp_dir.path();

    // Write Cargo.toml with unique package name
    let pkg_id = COMPILE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pkg_name = format!("logos_extract_e2e_{}_{}", get_run_id(), pkg_id);
    let cargo_toml = format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"
"#,
        pkg_name
    );

    // Combine extracted code with main
    let full_code = format!("{}\n{}", rust_code, main_code);

    std::fs::create_dir_all(project_dir.join("src")).unwrap();
    std::fs::write(project_dir.join("Cargo.toml"), cargo_toml).unwrap();
    std::fs::write(project_dir.join("src/main.rs"), &full_code).unwrap();

    // Run with shared target dir for caching
    let output = Command::new("cargo")
        .args(["run", "--quiet"])
        .current_dir(project_dir)
        .env("CARGO_TARGET_DIR", get_shared_target_dir())
        .output()
        .expect("cargo run");

    E2EResult {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        success: output.status.success(),
        rust_code: full_code,
    }
}

/// Assert extracted code runs and produces expected output.
pub fn assert_extracted_output(rust_code: &str, main_code: &str, expected: &str) {
    let result = run_extracted(rust_code, main_code);
    assert!(
        result.success,
        "Extracted code should compile and run.\n\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code, result.stderr
    );
    assert!(
        result.stdout.trim().contains(expected),
        "Expected '{}' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        expected,
        result.stdout.trim(),
        result.rust_code
    );
}
