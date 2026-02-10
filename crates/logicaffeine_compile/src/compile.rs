//! LOGOS Compilation Pipeline
//!
//! This module provides the end-to-end compilation pipeline that transforms
//! LOGOS source code into executable Rust programs.
//!
//! # Pipeline Overview
//!
//! ```text
//! LOGOS Source (.md)
//!       │
//!       ▼
//! ┌───────────────────┐
//! │  1. Lexer         │ Tokenize source
//! └─────────┬─────────┘
//!           ▼
//! ┌───────────────────┐
//! │  2. Discovery     │ Type & policy definitions
//! └─────────┬─────────┘
//!           ▼
//! ┌───────────────────┐
//! │  3. Parser        │ Build AST
//! └─────────┬─────────┘
//!           ▼
//! ┌───────────────────┐
//! │  4. Analysis      │ Escape, ownership, verification
//! └─────────┬─────────┘
//!           ▼
//! ┌───────────────────┐
//! │  5. CodeGen       │ Emit Rust source
//! └─────────┬─────────┘
//!           ▼
//!     Rust Source
//! ```
//!
//! # Compilation Functions
//!
//! | Function | Analysis | Use Case |
//! |----------|----------|----------|
//! | [`compile_to_rust`] | Escape only | Basic compilation |
//! | [`compile_to_rust_checked`] | Escape + Ownership | Use with `--check` flag |
//! | `compile_to_rust_verified` | All + Z3 | Formal verification (requires `verification` feature) |
//! | [`compile_project`] | Multi-file | Projects with imports |
//! | [`compile_and_run`] | Full + Execute | Development workflow |
//!
//! # Examples
//!
//! ## Basic Compilation
//!
//! ```ignore
//! let source = "## Main\nLet x be 5.\nShow x to show.";
//! let rust_code = compile_to_rust(source)?;
//! // rust_code contains:
//! // fn main() {
//! //     let x = 5;
//! //     println!("{}", x);
//! // }
//! ```
//!
//! ## With Ownership Checking
//!
//! ```ignore
//! let source = "## Main\nLet x be 5.\nGive x to y.\nShow x to show.";
//! let result = compile_to_rust_checked(source);
//! // Returns Err: "x has already been given away"
//! ```

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;

// Runtime crates paths (relative to workspace root)
const CRATES_DATA_PATH: &str = "crates/logicaffeine_data";
const CRATES_SYSTEM_PATH: &str = "crates/logicaffeine_system";

use std::fmt::Write as FmtWrite;

use crate::analysis::{DiscoveryPass, EscapeChecker, OwnershipChecker, PolicyRegistry};
use crate::arena::Arena;
use crate::arena_ctx::AstContext;
use crate::ast::{Expr, Stmt, TypeExpr};
use crate::codegen::codegen_program;
use crate::diagnostic::{parse_rustc_json, translate_diagnostics, LogosError};
use crate::drs::WorldState;
use crate::error::ParseError;
use crate::intern::Interner;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::sourcemap::SourceMap;

/// A declared external crate dependency from a `## Requires` block.
#[derive(Debug, Clone)]
pub struct CrateDependency {
    pub name: String,
    pub version: String,
    pub features: Vec<String>,
}

/// Full compilation output including generated Rust code and extracted dependencies.
#[derive(Debug)]
pub struct CompileOutput {
    pub rust_code: String,
    pub dependencies: Vec<CrateDependency>,
}

/// Compile LOGOS source to Rust source code.
///
/// This is the basic compilation function that runs lexing, parsing, and
/// escape analysis before generating Rust code.
///
/// # Arguments
///
/// * `source` - LOGOS source code as a string
///
/// # Returns
///
/// Generated Rust source code on success.
///
/// # Errors
///
/// Returns [`ParseError`] if:
/// - Lexical analysis fails (invalid tokens)
/// - Parsing fails (syntax errors)
/// - Escape analysis fails (zone-local values escaping)
///
/// # Example
///
/// ```ignore
/// let source = "## Main\nLet x be 5.\nShow x to show.";
/// let rust_code = compile_to_rust(source)?;
/// assert!(rust_code.contains("let x = 5;"));
/// ```
pub fn compile_to_rust(source: &str) -> Result<String, ParseError> {
    compile_program_full(source).map(|o| o.rust_code)
}

/// Compile LOGOS source and return full output including dependency metadata.
///
/// This is the primary compilation entry point that returns both the generated
/// Rust code and any crate dependencies declared in `## Requires` blocks.
pub fn compile_program_full(source: &str) -> Result<CompileOutput, ParseError> {
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

    let mut world_state = WorldState::new();
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
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, type_registry);
    // Note: Don't call process_block_headers() - parse_program handles blocks itself

    let stmts = parser.parse_program()?;

    // Extract dependencies before escape analysis
    let dependencies = extract_dependencies(&stmts, &interner);

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

    // Note: Static verification is available when the `verification` feature is enabled,
    // but must be explicitly invoked via compile_to_rust_verified().

    let rust_code = codegen_program(&stmts, &codegen_registry, &codegen_policies, &interner);

    Ok(CompileOutput { rust_code, dependencies })
}

/// Extract crate dependencies from `Stmt::Require` nodes.
fn extract_dependencies(stmts: &[Stmt], interner: &Interner) -> Vec<CrateDependency> {
    stmts.iter().filter_map(|stmt| {
        if let Stmt::Require { crate_name, version, features, .. } = stmt {
            Some(CrateDependency {
                name: interner.resolve(*crate_name).to_string(),
                version: interner.resolve(*version).to_string(),
                features: features.iter().map(|f| interner.resolve(*f).to_string()).collect(),
            })
        } else {
            None
        }
    }).collect()
}

/// Compile LOGOS source to Rust with ownership checking enabled.
///
/// This runs the lightweight ownership analysis pass that catches use-after-move
/// errors with control flow awareness. The analysis is fast enough to run on
/// every keystroke in an IDE.
///
/// # Arguments
///
/// * `source` - LOGOS source code as a string
///
/// # Returns
///
/// Generated Rust source code on success.
///
/// # Errors
///
/// Returns [`ParseError`] if:
/// - Any error from [`compile_to_rust`] occurs
/// - Ownership analysis detects use-after-move
/// - Ownership analysis detects use-after-borrow violations
///
/// # Example
///
/// ```ignore
/// // This will fail ownership checking
/// let source = "## Main\nLet x be 5.\nGive x to y.\nShow x to show.";
/// let result = compile_to_rust_checked(source);
/// assert!(result.is_err()); // "x has already been given away"
/// ```
///
/// # Use Case
///
/// Use this function with the `--check` CLI flag for instant feedback on
/// ownership errors before running the full Rust compilation.
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

    let mut world_state = WorldState::new();
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
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, type_registry);
    let stmts = parser.parse_program()?;

    // Pass 3: Escape analysis
    let mut escape_checker = EscapeChecker::new(&interner);
    escape_checker.check_program(&stmts).map_err(|e| {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(e.to_string()),
            span: e.span,
        }
    })?;

    // Pass 4: Ownership analysis
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

/// Compile LOGOS source to Rust with full Z3 static verification.
///
/// This runs the Z3-based verifier on Assert statements before code generation,
/// proving that assertions hold for all possible inputs. This is the most
/// thorough compilation mode, suitable for high-assurance code.
///
/// # Arguments
///
/// * `source` - LOGOS source code as a string
///
/// # Returns
///
/// Generated Rust source code on success.
///
/// # Errors
///
/// Returns [`ParseError`] if:
/// - Any error from [`compile_to_rust`] occurs
/// - Z3 cannot prove an Assert statement
/// - Refinement type constraints cannot be satisfied
/// - Termination cannot be proven for loops with `decreasing`
///
/// # Example
///
/// ```ignore
/// let source = r#"
/// ## Main
/// Let x: { it: Int | it > 0 } be 5.
/// Assert that x > 0.
/// "#;
/// let rust_code = compile_to_rust_verified(source)?;
/// ```
///
/// # Feature Flag
///
/// This function requires the `verification` feature to be enabled:
///
/// ```toml
/// [dependencies]
/// logicaffeine_compile = { version = "...", features = ["verification"] }
/// ```
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

    let mut world_state = WorldState::new();
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
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, type_registry);
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

/// Compile LOGOS source and write output to a directory as a Cargo project.
///
/// Creates a complete Cargo project structure with:
/// - `src/main.rs` containing the generated Rust code
/// - `Cargo.toml` with runtime dependencies
/// - `crates/` directory with runtime crate copies
///
/// # Arguments
///
/// * `source` - LOGOS source code as a string
/// * `output_dir` - Directory to create the Cargo project in
///
/// # Errors
///
/// Returns [`CompileError`] if:
/// - Compilation fails (wrapped as `CompileError::Parse`)
/// - File system operations fail (wrapped as `CompileError::Io`)
///
/// # Example
///
/// ```ignore
/// let source = "## Main\nShow \"Hello\" to show.";
/// compile_to_dir(source, Path::new("/tmp/my_project"))?;
/// // Now /tmp/my_project is a buildable Cargo project
/// ```
pub fn compile_to_dir(source: &str, output_dir: &Path) -> Result<(), CompileError> {
    let output = compile_program_full(source).map_err(CompileError::Parse)?;

    // Create output directory structure
    let src_dir = output_dir.join("src");
    fs::create_dir_all(&src_dir).map_err(|e| CompileError::Io(e.to_string()))?;

    // Write main.rs (codegen already includes the use statements)
    let main_path = src_dir.join("main.rs");
    let mut file = fs::File::create(&main_path).map_err(|e| CompileError::Io(e.to_string()))?;
    file.write_all(output.rust_code.as_bytes()).map_err(|e| CompileError::Io(e.to_string()))?;

    // Write Cargo.toml with runtime crate dependencies
    let mut cargo_toml = String::from(r#"[package]
name = "logos_output"
version = "0.1.0"
edition = "2021"

[dependencies]
logicaffeine-data = { path = "./crates/logicaffeine_data" }
logicaffeine-system = { path = "./crates/logicaffeine_system", features = ["full"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
"#);

    // Append user-declared dependencies from ## Requires blocks
    for dep in &output.dependencies {
        if dep.features.is_empty() {
            let _ = writeln!(cargo_toml, "{} = \"{}\"", dep.name, dep.version);
        } else {
            let feats = dep.features.iter()
                .map(|f| format!("\"{}\"", f))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(
                cargo_toml,
                "{} = {{ version = \"{}\", features = [{}] }}",
                dep.name, dep.version, feats
            );
        }
    }

    let cargo_path = output_dir.join("Cargo.toml");
    let mut file = fs::File::create(&cargo_path).map_err(|e| CompileError::Io(e.to_string()))?;
    file.write_all(cargo_toml.as_bytes()).map_err(|e| CompileError::Io(e.to_string()))?;

    // Copy runtime crates to output directory
    copy_runtime_crates(output_dir)?;

    Ok(())
}

/// Copy the runtime crates to the output directory.
/// Copies logicaffeine_data and logicaffeine_system.
pub fn copy_runtime_crates(output_dir: &Path) -> Result<(), CompileError> {
    let crates_dir = output_dir.join("crates");
    fs::create_dir_all(&crates_dir).map_err(|e| CompileError::Io(e.to_string()))?;

    // Find workspace root
    let workspace_root = find_workspace_root()?;

    // Copy logicaffeine_data
    let data_src = workspace_root.join(CRATES_DATA_PATH);
    let data_dest = crates_dir.join("logicaffeine_data");
    copy_dir_recursive(&data_src, &data_dest)?;

    // Copy logicaffeine_system
    let system_src = workspace_root.join(CRATES_SYSTEM_PATH);
    let system_dest = crates_dir.join("logicaffeine_system");
    copy_dir_recursive(&system_src, &system_dest)?;

    // Also need to copy logicaffeine_base since both crates depend on it
    let base_src = workspace_root.join("crates/logicaffeine_base");
    let base_dest = crates_dir.join("logicaffeine_base");
    copy_dir_recursive(&base_src, &base_dest)?;

    Ok(())
}

/// Find the workspace root directory.
fn find_workspace_root() -> Result<std::path::PathBuf, CompileError> {
    // Try CARGO_MANIFEST_DIR first
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        // Go up from crates/logicaffeine_compile to workspace root
        let path = Path::new(&manifest_dir);
        if let Some(parent) = path.parent().and_then(|p| p.parent()) {
            if parent.join("Cargo.toml").exists() {
                return Ok(parent.to_path_buf());
            }
        }
    }

    // Fallback to current directory traversal
    let mut current = std::env::current_dir()
        .map_err(|e| CompileError::Io(e.to_string()))?;

    loop {
        if current.join("Cargo.toml").exists() && current.join("crates").exists() {
            return Ok(current);
        }
        if !current.pop() {
            return Err(CompileError::Io("Could not find workspace root".to_string()));
        }
    }
}

/// Recursively copy a directory.
/// Skips files that disappear during copy (race condition with parallel builds).
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), CompileError> {
    fs::create_dir_all(dst).map_err(|e| CompileError::Io(e.to_string()))?;

    for entry in fs::read_dir(src).map_err(|e| CompileError::Io(e.to_string()))? {
        let entry = entry.map_err(|e| CompileError::Io(e.to_string()))?;
        let src_path = entry.path();
        let file_name = entry.file_name();
        let dst_path = dst.join(&file_name);

        // Skip target directory, build artifacts, and lock files
        if file_name == "target"
            || file_name == ".git"
            || file_name == "Cargo.lock"
            || file_name == ".DS_Store"
        {
            continue;
        }

        // Skip files that start with a dot (hidden files)
        if file_name.to_string_lossy().starts_with('.') {
            continue;
        }

        // Check if path still exists (race condition protection)
        if !src_path.exists() {
            continue;
        }

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if file_name == "Cargo.toml" {
            // Special handling for Cargo.toml: remove [workspace] line
            // which can interfere with nested crate dependencies
            match fs::read_to_string(&src_path) {
                Ok(content) => {
                    let filtered: String = content
                        .lines()
                        .filter(|line| !line.trim().starts_with("[workspace]"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    fs::write(&dst_path, filtered)
                        .map_err(|e| CompileError::Io(e.to_string()))?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(CompileError::Io(e.to_string())),
            }
        } else {
            match fs::copy(&src_path, &dst_path) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(CompileError::Io(e.to_string())),
            }
        }
    }

    Ok(())
}

/// Compile and run a LOGOS program end-to-end.
///
/// This function performs the full compilation workflow:
/// 1. Compile LOGOS to Rust via [`compile_to_dir`]
/// 2. Run `cargo build` with JSON diagnostics
/// 3. Translate any rustc errors to LOGOS-friendly messages
/// 4. Run the compiled program via `cargo run`
///
/// # Arguments
///
/// * `source` - LOGOS source code as a string
/// * `output_dir` - Directory to create the temporary Cargo project in
///
/// # Returns
///
/// The stdout output of the executed program.
///
/// # Errors
///
/// Returns [`CompileError`] if:
/// - Compilation fails (see [`compile_to_dir`])
/// - Rust compilation fails (`CompileError::Build` or `CompileError::Ownership`)
/// - The program crashes at runtime (`CompileError::Runtime`)
///
/// # Diagnostic Translation
///
/// When rustc reports errors (e.g., E0382 for use-after-move), this function
/// uses the [`diagnostic`](crate::diagnostic) module to translate them into
/// LOGOS-friendly Socratic error messages.
///
/// # Example
///
/// ```ignore
/// let source = "## Main\nShow \"Hello, World!\" to show.";
/// let output = compile_and_run(source, Path::new("/tmp/run"))?;
/// assert_eq!(output.trim(), "Hello, World!");
/// ```
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

/// Compile a multi-file LOGOS project with dependency resolution.
///
/// This function:
/// 1. Reads the entry file
/// 2. Scans for dependencies in the abstract (Markdown links)
/// 3. Recursively loads and discovers types from dependencies
/// 4. Compiles with the combined type registry
///
/// # Arguments
/// * `entry_file` - The main entry file to compile (root is derived from parent directory)
///
/// # Example
/// ```ignore
/// let result = compile_project(Path::new("/project/main.md"));
/// ```
pub fn compile_project(entry_file: &Path) -> Result<CompileOutput, CompileError> {
    use crate::loader::Loader;
    use crate::analysis::discover_with_imports;

    let root_path = entry_file.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut loader = Loader::new(root_path);
    let mut interner = Interner::new();

    // Read the entry file
    let source = fs::read_to_string(entry_file)
        .map_err(|e| CompileError::Io(format!("Failed to read entry file: {}", e)))?;

    // Discover types from entry file and all imports
    let type_registry = discover_with_imports(entry_file, &source, &mut loader, &mut interner)
        .map_err(|e| CompileError::Io(e))?;

    // Now compile with the discovered types
    compile_to_rust_with_registry_full(&source, type_registry, &mut interner)
        .map_err(CompileError::Parse)
}

/// Compile LOGOS source with a pre-populated type registry, returning full output.
/// Returns both generated Rust code and extracted dependencies.
fn compile_to_rust_with_registry_full(
    source: &str,
    type_registry: crate::analysis::TypeRegistry,
    interner: &mut Interner,
) -> Result<CompileOutput, ParseError> {
    let mut lexer = Lexer::new(source, interner);
    let tokens = lexer.tokenize();

    // Discovery pass for policies (types already discovered)
    let policy_registry = {
        let mut discovery = DiscoveryPass::new(&tokens, interner);
        discovery.run_full().policies
    };

    let codegen_registry = type_registry.clone();
    let codegen_policies = policy_registry.clone();

    let mut world_state = WorldState::new();
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

    let mut parser = Parser::new(tokens, &mut world_state, interner, ast_ctx, type_registry);
    let stmts = parser.parse_program()?;

    // Extract dependencies before escape analysis
    let dependencies = extract_dependencies(&stmts, interner);

    let mut escape_checker = EscapeChecker::new(interner);
    escape_checker.check_program(&stmts).map_err(|e| {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(e.to_string()),
            span: e.span,
        }
    })?;

    let rust_code = codegen_program(&stmts, &codegen_registry, &codegen_policies, interner);

    Ok(CompileOutput { rust_code, dependencies })
}

/// Errors that can occur during the LOGOS compilation pipeline.
///
/// This enum represents the different stages where compilation can fail,
/// from parsing through to runtime execution.
///
/// # Error Hierarchy
///
/// ```text
/// CompileError
/// ├── Parse      ← Lexing, parsing, or static analysis
/// ├── Io         ← File system operations
/// ├── Build      ← Rust compilation (cargo build)
/// ├── Ownership  ← Translated borrow checker errors
/// └── Runtime    ← Program execution failure
/// ```
///
/// # Error Translation
///
/// The `Ownership` variant contains LOGOS-friendly error messages translated
/// from rustc's borrow checker errors (E0382, E0505, E0597) using the
/// [`diagnostic`](crate::diagnostic) module.
#[derive(Debug)]
pub enum CompileError {
    /// Parsing or static analysis failed.
    ///
    /// This includes lexer errors, syntax errors, escape analysis failures,
    /// ownership analysis failures, and Z3 verification failures.
    Parse(ParseError),

    /// File system operation failed.
    ///
    /// Typically occurs when reading source files or writing output.
    Io(String),

    /// Rust compilation failed (`cargo build`).
    ///
    /// Contains the raw stderr output from rustc when diagnostic translation
    /// was not possible.
    Build(String),

    /// Runtime execution failed.
    ///
    /// Contains stderr output from the executed program.
    Runtime(String),

    /// Translated ownership/borrow checker error with LOGOS-friendly message.
    ///
    /// This variant is used when rustc reports errors like E0382 (use after move)
    /// and we can translate them into natural language error messages that
    /// reference the original LOGOS source.
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
