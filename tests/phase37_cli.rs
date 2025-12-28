//! Phase 37: The LOGOS Build Tool (CLI)
//!
//! Tests for project creation, building, and execution.
//!
//! These tests require the `cli` feature to be enabled.

#![cfg(feature = "cli")]

use std::fs;
use tempfile::tempdir;
use toml;

use logos::project::build::{build, find_project_root, BuildConfig};
use logos::project::manifest::Manifest;

#[test]
fn test_manifest_parse_minimal() {
    let toml_str = r#"
[package]
name = "myproject"
"#;
    let manifest: Manifest = toml::from_str(toml_str).expect("Should parse minimal manifest");
    assert_eq!(manifest.package.name, "myproject");
    assert_eq!(manifest.package.version, "0.1.0"); // default
    assert_eq!(manifest.package.entry, "src/main.lg"); // default
}

#[test]
fn test_manifest_parse_full() {
    let toml_str = r#"
[package]
name = "myproject"
version = "1.0.0"
description = "A test project"
entry = "src/app.lg"
authors = ["Test Author"]

[dependencies]
std = "logos:std"
"#;
    let manifest: Manifest = toml::from_str(toml_str).expect("Should parse full manifest");
    assert_eq!(manifest.package.name, "myproject");
    assert_eq!(manifest.package.version, "1.0.0");
    assert_eq!(manifest.package.entry, "src/app.lg");
    assert!(manifest.package.description.is_some());
    assert_eq!(manifest.package.authors.len(), 1);
}

#[test]
fn test_manifest_new() {
    let manifest = Manifest::new("testproject");
    assert_eq!(manifest.package.name, "testproject");
    let toml = manifest.to_toml().expect("Should serialize");
    assert!(toml.contains("name = \"testproject\""));
}

#[test]
fn test_find_project_root() {
    let temp = tempdir().unwrap();
    let sub = temp.path().join("a/b/c");
    fs::create_dir_all(&sub).unwrap();
    fs::write(temp.path().join("Largo.toml"), "[package]\nname=\"test\"\n").unwrap();

    let found = find_project_root(&sub);
    assert!(found.is_some());
    assert_eq!(found.unwrap(), temp.path());
}

#[test]
fn test_find_project_root_not_found() {
    let temp = tempdir().unwrap();
    let found = find_project_root(temp.path());
    assert!(found.is_none());
}

#[test]
fn test_build_simple_project() {
    let temp = tempdir().unwrap();
    let src_dir = temp.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();

    // Create Largo.toml
    fs::write(
        temp.path().join("Largo.toml"),
        r#"
[package]
name = "test_project"
"#,
    )
    .unwrap();

    // Create src/main.lg with minimal code
    fs::write(
        src_dir.join("main.lg"),
        r#"# Main

## Main
Let x be 42.
"#,
    )
    .unwrap();

    let config = BuildConfig {
        project_dir: temp.path().to_path_buf(),
        release: false,
    };

    let result = build(config);
    assert!(result.is_ok(), "Build should succeed: {:?}", result);

    let result = result.unwrap();
    assert!(result.binary_path.exists(), "Binary should exist");
}

#[test]
fn test_build_with_md_fallback() {
    let temp = tempdir().unwrap();
    let src_dir = temp.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();

    // Create Largo.toml pointing to .lg (which won't exist)
    fs::write(
        temp.path().join("Largo.toml"),
        r#"
[package]
name = "test_md"
entry = "src/main.lg"
"#,
    )
    .unwrap();

    // Create src/main.md instead (fallback)
    fs::write(
        src_dir.join("main.md"),
        r#"# Main

## Main
Let x be 1.
"#,
    )
    .unwrap();

    let config = BuildConfig {
        project_dir: temp.path().to_path_buf(),
        release: false,
    };

    let result = build(config);
    assert!(
        result.is_ok(),
        "Build should succeed with .md fallback: {:?}",
        result
    );
}

#[test]
fn test_build_missing_entry() {
    let temp = tempdir().unwrap();

    // Create Largo.toml but no source files
    fs::write(
        temp.path().join("Largo.toml"),
        r#"
[package]
name = "empty"
"#,
    )
    .unwrap();

    let config = BuildConfig {
        project_dir: temp.path().to_path_buf(),
        release: false,
    };

    let result = build(config);
    assert!(result.is_err(), "Build should fail with missing entry");
}

#[test]
fn test_manifest_load() {
    let temp = tempdir().unwrap();

    fs::write(
        temp.path().join("Largo.toml"),
        r#"
[package]
name = "loaded_project"
version = "2.0.0"
"#,
    )
    .unwrap();

    let manifest = Manifest::load(temp.path());
    assert!(manifest.is_ok(), "Should load manifest: {:?}", manifest);
    let manifest = manifest.unwrap();
    assert_eq!(manifest.package.name, "loaded_project");
    assert_eq!(manifest.package.version, "2.0.0");
}

#[test]
fn test_path_dependency_parsing() {
    let toml_str = r#"
[package]
name = "with_deps"

[dependencies]
math = { path = "./math" }
"#;
    let manifest: Manifest = toml::from_str(toml_str).expect("Should parse path deps");
    assert!(!manifest.dependencies.is_empty());
}
