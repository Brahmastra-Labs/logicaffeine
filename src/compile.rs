//! LOGOS Compilation Pipeline
//!
//! This module provides the end-to-end compilation pipeline:
//! LOGOS source → Rust source → executable

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;

// Embed runtime at compile time
const LOGOS_CORE_TOML: &str = include_str!("../logos_core/Cargo.toml");
const LOGOS_CORE_LIB: &str = include_str!("../logos_core/src/lib.rs");
const LOGOS_CORE_TYPES: &str = include_str!("../logos_core/src/types.rs");
const LOGOS_CORE_IO: &str = include_str!("../logos_core/src/io.rs");
// Phase 38: Standard library modules
const LOGOS_CORE_FILE: &str = include_str!("../logos_core/src/file.rs");
const LOGOS_CORE_TIME: &str = include_str!("../logos_core/src/time.rs");
const LOGOS_CORE_RANDOM: &str = include_str!("../logos_core/src/random.rs");
const LOGOS_CORE_ENV: &str = include_str!("../logos_core/src/env.rs");
// Phase 8.5: Zone-based memory management
const LOGOS_CORE_MEMORY: &str = include_str!("../logos_core/src/memory.rs");

use crate::analysis::{DiscoveryPass, EscapeChecker, OwnershipChecker, PolicyRegistry};
use crate::arena::Arena;
use crate::arena_ctx::AstContext;
use crate::ast::{Expr, Stmt, TypeExpr};
use crate::codegen::codegen_program;
use crate::context::DiscourseContext;
use crate::diagnostic::{parse_rustc_json, translate_diagnostics, LogosError};
use crate::error::ParseError;
use crate::intern::Interner;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::sourcemap::SourceMap;

/// Compile LOGOS source to Rust source code.
pub fn compile_to_rust(source: &str) -> Result<String, ParseError> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    // Pass 1: Discovery - scan for type definitions and policies
    let (type_registry, policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };
    // Clone for codegen (parser takes ownership)
    let codegen_registry = type_registry.clone();
    let codegen_policies = policy_registry.clone();

    let mut ctx = DiscourseContext::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ast_ctx = AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_expr_arena,
    );

    // Pass 2: Parse with type context
    let mut parser = Parser::with_types(tokens, &mut ctx, &mut interner, ast_ctx, type_registry);
    // Note: Don't call process_block_headers() - parse_program handles blocks itself

    let stmts = parser.parse_program()?;

    // Pass 3: Escape analysis - check for zone escape violations
    // This catches obvious cases like returning zone-local variables
    let mut escape_checker = EscapeChecker::new(&interner);
    escape_checker.check_program(&stmts).map_err(|e| {
        // Convert EscapeError to ParseError for now
        // The error message is already Socratic from EscapeChecker
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(e.to_string()),
            span: e.span,
        }
    })?;

    // Note: Static verification (Phase 42) is available when the `verification`
    // feature is enabled, but must be explicitly invoked via compile_to_rust_verified().

    let rust_code = codegen_program(&stmts, &codegen_registry, &codegen_policies, &interner);

    Ok(rust_code)
}

/// Compile LOGOS source to Rust with ownership checking enabled.
///
/// This runs the lightweight ownership analysis pass (Phase 45) that catches
/// use-after-move errors with control flow awareness in milliseconds.
/// Use this with `--check` flag for instant feedback on ownership errors.
pub fn compile_to_rust_checked(source: &str) -> Result<String, ParseError> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    // Pass 1: Discovery - scan for type definitions and policies
    let (type_registry, policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };
    // Clone for codegen (parser takes ownership)
    let codegen_registry = type_registry.clone();
    let codegen_policies = policy_registry.clone();

    let mut ctx = DiscourseContext::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ast_ctx = AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_expr_arena,
    );

    // Pass 2: Parse with type context
    let mut parser = Parser::with_types(tokens, &mut ctx, &mut interner, ast_ctx, type_registry);
    let stmts = parser.parse_program()?;

    // Pass 3: Escape analysis
    let mut escape_checker = EscapeChecker::new(&interner);
    escape_checker.check_program(&stmts).map_err(|e| {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(e.to_string()),
            span: e.span,
        }
    })?;

    // Pass 4: Ownership analysis (Phase 45)
    // Catches use-after-move errors with control flow awareness
    let mut ownership_checker = OwnershipChecker::new(&interner);
    ownership_checker.check_program(&stmts).map_err(|e| {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(e.to_string()),
            span: e.span,
        }
    })?;

    let rust_code = codegen_program(&stmts, &codegen_registry, &codegen_policies, &interner);

    Ok(rust_code)
}

/// Compile LOGOS source to Rust source code with static verification.
///
/// This runs the Z3-based verifier on Assert statements before codegen.
/// Requires the `verification` feature to be enabled.
#[cfg(feature = "verification")]
pub fn compile_to_rust_verified(source: &str) -> Result<String, ParseError> {
    use crate::verification::VerificationPass;

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    // Pass 1: Discovery - scan for type definitions and policies
    let (type_registry, policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };
    // Clone for codegen (parser takes ownership)
    let codegen_registry = type_registry.clone();
    let codegen_policies = policy_registry.clone();

    let mut ctx = DiscourseContext::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ast_ctx = AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_expr_arena,
    );

    // Pass 2: Parse with type context
    let mut parser = Parser::with_types(tokens, &mut ctx, &mut interner, ast_ctx, type_registry);
    let stmts = parser.parse_program()?;

    // Pass 3: Escape analysis
    let mut escape_checker = EscapeChecker::new(&interner);
    escape_checker.check_program(&stmts).map_err(|e| {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(e.to_string()),
            span: e.span,
        }
    })?;

    // Pass 4: Static verification
    let mut verifier = VerificationPass::new(&interner);
    verifier.verify_program(&stmts).map_err(|e| {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(format!(
                "Verification Failed:\n\n{}",
                e
            )),
            span: crate::token::Span::default(),
        }
    })?;

    let rust_code = codegen_program(&stmts, &codegen_registry, &codegen_policies, &interner);

    Ok(rust_code)
}

/// Compile LOGOS source and write output to a directory.
/// Creates a Cargo project with logos_core dependency.
pub fn compile_to_dir(source: &str, output_dir: &Path) -> Result<(), CompileError> {
    let rust_code = compile_to_rust(source).map_err(CompileError::Parse)?;

    // Create output directory structure
    let src_dir = output_dir.join("src");
    fs::create_dir_all(&src_dir).map_err(|e| CompileError::Io(e.to_string()))?;

    // Write main.rs with logos_core import
    let main_rs = format!(
        "use logos_core::prelude::*;\n\n{}",
        rust_code
    );
    let main_path = src_dir.join("main.rs");
    let mut file = fs::File::create(&main_path).map_err(|e| CompileError::Io(e.to_string()))?;
    file.write_all(main_rs.as_bytes()).map_err(|e| CompileError::Io(e.to_string()))?;

    // Write Cargo.toml
    let cargo_toml = format!(
        r#"[package]
name = "logos_output"
version = "0.1.0"
edition = "2021"

[dependencies]
logos_core = {{ path = "./logos_core" }}
"#
    );
    let cargo_path = output_dir.join("Cargo.toml");
    let mut file = fs::File::create(&cargo_path).map_err(|e| CompileError::Io(e.to_string()))?;
    file.write_all(cargo_toml.as_bytes()).map_err(|e| CompileError::Io(e.to_string()))?;

    // Copy logos_core to output directory
    copy_logos_core(output_dir)?;

    Ok(())
}

/// Copy the embedded logos_core crate to the output directory.
pub fn copy_logos_core(output_dir: &Path) -> Result<(), CompileError> {
    let core_dir = output_dir.join("logos_core");
    let src_dir = core_dir.join("src");

    fs::create_dir_all(&src_dir).map_err(|e| CompileError::Io(e.to_string()))?;

    fs::write(core_dir.join("Cargo.toml"), LOGOS_CORE_TOML)
        .map_err(|e| CompileError::Io(e.to_string()))?;
    fs::write(src_dir.join("lib.rs"), LOGOS_CORE_LIB)
        .map_err(|e| CompileError::Io(e.to_string()))?;
    fs::write(src_dir.join("types.rs"), LOGOS_CORE_TYPES)
        .map_err(|e| CompileError::Io(e.to_string()))?;
    fs::write(src_dir.join("io.rs"), LOGOS_CORE_IO)
        .map_err(|e| CompileError::Io(e.to_string()))?;
    // Phase 38: Write standard library modules
    fs::write(src_dir.join("file.rs"), LOGOS_CORE_FILE)
        .map_err(|e| CompileError::Io(e.to_string()))?;
    fs::write(src_dir.join("time.rs"), LOGOS_CORE_TIME)
        .map_err(|e| CompileError::Io(e.to_string()))?;
    fs::write(src_dir.join("random.rs"), LOGOS_CORE_RANDOM)
        .map_err(|e| CompileError::Io(e.to_string()))?;
    fs::write(src_dir.join("env.rs"), LOGOS_CORE_ENV)
        .map_err(|e| CompileError::Io(e.to_string()))?;
    // Phase 8.5: Zone-based memory management
    fs::write(src_dir.join("memory.rs"), LOGOS_CORE_MEMORY)
        .map_err(|e| CompileError::Io(e.to_string()))?;

    Ok(())
}

/// Compile and run a LOGOS program.
pub fn compile_and_run(source: &str, output_dir: &Path) -> Result<String, CompileError> {
    compile_to_dir(source, output_dir)?;

    // Run cargo build with JSON message format for structured error parsing
    let build_output = Command::new("cargo")
        .arg("build")
        .arg("--message-format=json")
        .current_dir(output_dir)
        .output()
        .map_err(|e| CompileError::Io(e.to_string()))?;

    if !build_output.status.success() {
        let stderr = String::from_utf8_lossy(&build_output.stderr);
        let stdout = String::from_utf8_lossy(&build_output.stdout);

        // Try to parse JSON diagnostics and translate them
        let diagnostics = parse_rustc_json(&stdout);

        if !diagnostics.is_empty() {
            // Create a basic source map with the LOGOS source
            let source_map = SourceMap::new(source.to_string());
            let interner = Interner::new();

            if let Some(logos_error) = translate_diagnostics(&diagnostics, &source_map, &interner) {
                return Err(CompileError::Ownership(logos_error));
            }
        }

        // Fallback to raw error if translation fails
        return Err(CompileError::Build(stderr.to_string()));
    }

    // Run the compiled program
    let run_output = Command::new("cargo")
        .arg("run")
        .arg("--quiet")
        .current_dir(output_dir)
        .output()
        .map_err(|e| CompileError::Io(e.to_string()))?;

    if !run_output.status.success() {
        let stderr = String::from_utf8_lossy(&run_output.stderr);
        return Err(CompileError::Runtime(stderr.to_string()));
    }

    let stdout = String::from_utf8_lossy(&run_output.stdout);
    Ok(stdout.to_string())
}

/// Compile a LOGOS source file.
/// For single-file compilation without dependencies.
pub fn compile_file(path: &Path) -> Result<String, CompileError> {
    let source = fs::read_to_string(path).map_err(|e| CompileError::Io(e.to_string()))?;
    compile_to_rust(&source).map_err(CompileError::Parse)
}

/// Phase 36: Compile a LOGOS project with dependencies.
/// Scans the Abstract for [Alias](URI) links and loads dependencies recursively.
pub fn compile_project(path: &Path) -> Result<String, CompileError> {
    use crate::analysis::discover_with_imports;
    use crate::project::Loader;

    let source = fs::read_to_string(path).map_err(|e| CompileError::Io(e.to_string()))?;
    let root_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();

    let mut interner = Interner::new();
    let mut loader = Loader::new(root_dir);

    // Pass 1: Recursive discovery with imports
    let type_registry = discover_with_imports(path, &source, &mut loader, &mut interner)
        .map_err(|e| CompileError::Io(e))?;
    let codegen_registry = type_registry.clone();

    // Phase 50: Also discover policies from the main file
    // (discover_with_imports doesn't handle policies yet, so we do a separate pass)
    let mut lexer = Lexer::new(&source, &mut interner);
    let tokens = lexer.tokenize();
    let policy_registry = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        discovery.run_full().policies
    };
    let codegen_policies = policy_registry.clone();

    // Re-tokenize for parsing (interner may have been modified)
    let mut lexer = Lexer::new(&source, &mut interner);
    let tokens = lexer.tokenize();

    let mut ctx = DiscourseContext::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ast_ctx = AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_expr_arena,
    );

    // Pass 2: Parse with type context (includes imported types)
    let mut parser = Parser::with_types(tokens, &mut ctx, &mut interner, ast_ctx, type_registry);
    let stmts = parser.parse_program().map_err(CompileError::Parse)?;
    let rust_code = codegen_program(&stmts, &codegen_registry, &codegen_policies, &interner);

    Ok(rust_code)
}

/// Errors that can occur during compilation.
#[derive(Debug)]
pub enum CompileError {
    Parse(ParseError),
    Io(String),
    Build(String),
    Runtime(String),
    /// Translated ownership/borrow checker error with friendly LOGOS message
    Ownership(LogosError),
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::Parse(e) => write!(f, "Parse error: {:?}", e),
            CompileError::Io(e) => write!(f, "IO error: {}", e),
            CompileError::Build(e) => write!(f, "Build error: {}", e),
            CompileError::Runtime(e) => write!(f, "Runtime error: {}", e),
            CompileError::Ownership(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for CompileError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compile_let_statement() {
        let source = "## Main\nLet x be 5.";
        let result = compile_to_rust(source);
        assert!(result.is_ok(), "Should compile: {:?}", result);
        let rust = result.unwrap();
        assert!(rust.contains("fn main()"));
        assert!(rust.contains("let x = 5;"));
    }

    #[test]
    fn test_compile_return_statement() {
        let source = "## Main\nReturn 42.";
        let result = compile_to_rust(source);
        assert!(result.is_ok(), "Should compile: {:?}", result);
        let rust = result.unwrap();
        assert!(rust.contains("return 42;"));
    }
}
