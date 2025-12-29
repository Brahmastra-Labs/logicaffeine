//! E2E Test Harness
//!
//! Provides utilities for compiling LOGOS source and running the generated Rust.

use std::process::Command;
use logos::compile::compile_to_rust;

pub struct E2EResult {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
    pub rust_code: String,
}

/// Compile LOGOS source and run the generated Rust, returning result.
pub fn run_logos(source: &str) -> E2EResult {
    // 1. Compile LOGOS to Rust
    let rust_code = match compile_to_rust(source) {
        Ok(code) => code,
        Err(e) => {
            return E2EResult {
                stdout: String::new(),
                stderr: format!("LOGOS compile error: {:?}", e),
                success: false,
                rust_code: String::new(),
            };
        }
    };

    // 2. Create temp project
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let project_dir = temp_dir.path();

    // 3. Write Cargo.toml
    let cargo_toml = format!(
        r#"[package]
name = "logos_e2e"
version = "0.1.0"
edition = "2021"

[dependencies]
logos_core = {{ path = "{}" }}
"#,
        std::env::current_dir().unwrap().join("logos_core").display()
    );

    std::fs::create_dir_all(project_dir.join("src")).unwrap();
    std::fs::write(project_dir.join("Cargo.toml"), cargo_toml).unwrap();
    std::fs::write(project_dir.join("src/main.rs"), &rust_code).unwrap();

    // 4. Run
    let output = Command::new("cargo")
        .args(["run", "--quiet"])
        .current_dir(project_dir)
        .output()
        .expect("cargo run");

    E2EResult {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        success: output.status.success(),
        rust_code,
    }
}

/// Assert that LOGOS code runs and produces expected output.
pub fn assert_output(source: &str, expected: &str) {
    let result = run_logos(source);
    assert!(
        result.success,
        "Code should run.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        source,
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains(expected),
        "Expected '{}' in output.\nGot: '{}'\n\nSource:\n{}\n\nGenerated Rust:\n{}",
        expected,
        result.stdout.trim(),
        source,
        result.rust_code
    );
}

/// Assert that LOGOS code runs successfully (no output check).
#[allow(dead_code)]
pub fn assert_runs(source: &str) {
    let result = run_logos(source);
    assert!(
        result.success,
        "Code should run.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        source,
        result.rust_code,
        result.stderr
    );
}

/// Assert that LOGOS code panics at runtime (e.g., debug_assert! failure).
#[allow(dead_code)]
pub fn assert_panics(source: &str, expected_msg: &str) {
    let result = run_logos(source);
    assert!(
        !result.success,
        "Code should panic but succeeded.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstdout: {}",
        source,
        result.rust_code,
        result.stdout
    );
    assert!(
        result.stderr.contains(expected_msg),
        "Expected '{}' in panic message.\nGot stderr:\n{}\n\nSource:\n{}\n\nGenerated Rust:\n{}",
        expected_msg,
        result.stderr,
        source,
        result.rust_code
    );
}
