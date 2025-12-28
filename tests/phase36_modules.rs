//! Phase 36: The Hyperlink Module System
//!
//! Tests for dependency scanning, module loading, and recursive discovery.

use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;

use logos::analysis::dependencies::scan_dependencies;
use logos::project::Loader;
use logos::analysis::discover_with_imports;
use logos::intern::Interner;

#[test]
fn test_scan_dependencies_in_abstract() {
    let source = r#"
# My Game

This uses [Geometry](file:./geo.md) and [Physics](logos:std).

## Main
Let x be 1.
"#;
    let deps = scan_dependencies(source);
    assert_eq!(deps.len(), 2, "Should find 2 dependencies: {:?}", deps);
    assert_eq!(deps[0].alias, "Geometry");
    assert_eq!(deps[0].uri, "file:./geo.md");
    assert_eq!(deps[1].alias, "Physics");
    assert_eq!(deps[1].uri, "logos:std");
}

#[test]
fn test_ignore_links_after_abstract() {
    let source = r#"
# Header

This is the abstract with [Dep1](file:a.md).

This second paragraph has [Dep2](file:b.md).

## Main
Let x be 1.
"#;
    let deps = scan_dependencies(source);
    assert_eq!(deps.len(), 1, "Should only find 1 dependency in abstract: {:?}", deps);
    assert_eq!(deps[0].alias, "Dep1");
}

#[test]
fn test_loader_file_scheme() {
    let temp_dir = tempdir().unwrap();
    let geo_path = temp_dir.path().join("geo.md");
    fs::write(&geo_path, "## Definition\nA Point has:\n    an x, which is Int.\n").unwrap();

    let mut loader = Loader::new(temp_dir.path().to_path_buf());
    let result = loader.resolve(&temp_dir.path().join("main.md"), "file:./geo.md");

    assert!(result.is_ok(), "Should resolve file: scheme: {:?}", result);
    assert!(result.unwrap().content.contains("Point"), "Should contain Point definition");
}

#[test]
fn test_recursive_discovery_imports_types() {
    let temp_dir = tempdir().unwrap();
    // A proper LOGOS file has a # Header before ## Definition
    fs::write(temp_dir.path().join("geo.md"), r#"# Geometry

## Definition
A Point has:
    an x, which is Int.
    a y, which is Int.
"#).unwrap();

    let main_source = r#"
# Main

Uses [Geometry](file:./geo.md) for math.

## Main
Let p be a new Point from Geometry.
"#;

    let mut interner = Interner::new();
    let mut loader = Loader::new(temp_dir.path().to_path_buf());
    let registry = discover_with_imports(
        &temp_dir.path().join("main.md"),
        main_source,
        &mut loader,
        &mut interner
    ).expect("Should discover imports");

    let qualified = interner.intern("Geometry::Point");
    assert!(registry.is_type(qualified), "Should have Geometry::Point in registry");
}

#[test]
fn test_logos_std_import() {
    let source = r#"
# App

Uses the [Standard Library](logos:std).

## Main
Let x be 1.
"#;
    let deps = scan_dependencies(source);
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0].alias, "Standard Library");
    assert_eq!(deps[0].uri, "logos:std");

    let mut loader = Loader::new(PathBuf::from("."));
    let result = loader.resolve(&PathBuf::from("main.md"), "logos:std");
    assert!(result.is_ok(), "Should resolve logos:std scheme: {:?}", result);
}

#[test]
fn test_no_dependencies_without_abstract() {
    let source = r#"
# Module

## Main
Let x be 1.
"#;
    let deps = scan_dependencies(source);
    assert_eq!(deps.len(), 0, "Should find no dependencies without abstract");
}

#[test]
fn test_multiline_abstract() {
    let source = r#"
# My Project

This project uses [Math](file:./math.md) for calculations
and [IO](file:./io.md) for input/output operations.

## Main
Let x be 1.
"#;
    let deps = scan_dependencies(source);
    assert_eq!(deps.len(), 2, "Should find dependencies across multiline abstract: {:?}", deps);
    assert_eq!(deps[0].alias, "Math");
    assert_eq!(deps[1].alias, "IO");
}

#[test]
fn test_from_module_lexer() {
    // Verify "from" is tokenized correctly
    use logos::{Lexer, Interner, TokenType};

    let mut interner = Interner::new();
    let mut lexer = Lexer::new("Point from Geometry", &mut interner);
    let tokens = lexer.tokenize();

    // Should have: ProperName(Point), From, ProperName(Geometry), EOF
    assert!(tokens.len() >= 3, "Should have at least 3 tokens: {:?}", tokens);
    assert!(matches!(tokens[1].kind, TokenType::From), "Second token should be From: {:?}", tokens[1].kind);
}

#[test]
fn test_compile_project_with_imports() {
    use logos::compile::compile_project;

    let temp_dir = tempdir().unwrap();

    // Create dependency module
    fs::write(temp_dir.path().join("geo.md"), r#"# Geometry

## Definition
A Point has:
    an x, which is Int.
    a y, which is Int.
"#).unwrap();

    // Create main module that imports it
    let main_path = temp_dir.path().join("main.md");
    fs::write(&main_path, r#"# Main

Uses [Geometry](file:./geo.md) for points.

## Main
Let p be a new Point from Geometry.
"#).unwrap();

    let result = compile_project(&main_path);
    assert!(result.is_ok(), "Should compile project: {:?}", result);
    let rust_code = result.unwrap();
    // The codegen should produce struct usage
    assert!(rust_code.contains("fn main()"), "Should have main function");
}
