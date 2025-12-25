//! LOGOS Compilation Pipeline
//!
//! This module provides the end-to-end compilation pipeline:
//! LOGOS source → Rust source → executable

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;

use crate::arena::Arena;
use crate::arena_ctx::AstContext;
use crate::ast::{Expr, Stmt};
use crate::codegen::codegen_program;
use crate::context::DiscourseContext;
use crate::error::ParseError;
use crate::intern::Interner;
use crate::lexer::Lexer;
use crate::parser::Parser;

/// Compile LOGOS source to Rust source code.
pub fn compile_to_rust(source: &str) -> Result<String, ParseError> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
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

    let ast_ctx = AstContext::with_imperative(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
    );

    let mut parser = Parser::with_context(tokens, &mut ctx, &mut interner, ast_ctx);
    parser.process_block_headers();

    let stmts = parser.parse_program()?;
    let rust_code = codegen_program(&stmts, &interner);

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

/// Copy the logos_core crate to the output directory.
fn copy_logos_core(output_dir: &Path) -> Result<(), CompileError> {
    let core_dir = output_dir.join("logos_core");
    let core_src_dir = core_dir.join("src");
    fs::create_dir_all(&core_src_dir).map_err(|e| CompileError::Io(e.to_string()))?;

    // Write logos_core/Cargo.toml
    let cargo_toml = r#"[package]
name = "logos_core"
version = "0.1.0"
edition = "2021"

[dependencies]
"#;
    let cargo_path = core_dir.join("Cargo.toml");
    let mut file = fs::File::create(&cargo_path).map_err(|e| CompileError::Io(e.to_string()))?;
    file.write_all(cargo_toml.as_bytes()).map_err(|e| CompileError::Io(e.to_string()))?;

    // Write logos_core/src/lib.rs
    let lib_rs = r#"//! LOGOS Runtime Library

pub mod io {
    pub fn println<T: std::fmt::Display>(x: T) {
        println!("{}", x);
    }
    pub fn eprintln<T: std::fmt::Display>(x: T) {
        eprintln!("{}", x);
    }
    pub fn print<T: std::fmt::Display>(x: T) {
        print!("{}", x);
    }
}

pub fn panic_with(reason: &str) -> ! {
    panic!("{}", reason);
}

pub mod fmt {
    pub fn format<T: std::fmt::Display>(x: T) -> String {
        format!("{}", x)
    }
}

pub mod prelude {
    pub use super::io::{println, eprintln, print};
    pub use super::panic_with;
    pub use super::fmt::format;
}
"#;
    let lib_path = core_src_dir.join("lib.rs");
    let mut file = fs::File::create(&lib_path).map_err(|e| CompileError::Io(e.to_string()))?;
    file.write_all(lib_rs.as_bytes()).map_err(|e| CompileError::Io(e.to_string()))?;

    Ok(())
}

/// Compile and run a LOGOS program.
pub fn compile_and_run(source: &str, output_dir: &Path) -> Result<String, CompileError> {
    compile_to_dir(source, output_dir)?;

    // Run cargo build
    let build_output = Command::new("cargo")
        .arg("build")
        .current_dir(output_dir)
        .output()
        .map_err(|e| CompileError::Io(e.to_string()))?;

    if !build_output.status.success() {
        let stderr = String::from_utf8_lossy(&build_output.stderr);
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
pub fn compile_file(path: &Path) -> Result<String, CompileError> {
    let source = fs::read_to_string(path).map_err(|e| CompileError::Io(e.to_string()))?;
    compile_to_rust(&source).map_err(CompileError::Parse)
}

/// Errors that can occur during compilation.
#[derive(Debug)]
pub enum CompileError {
    Parse(ParseError),
    Io(String),
    Build(String),
    Runtime(String),
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::Parse(e) => write!(f, "Parse error: {:?}", e),
            CompileError::Io(e) => write!(f, "IO error: {}", e),
            CompileError::Build(e) => write!(f, "Build error: {}", e),
            CompileError::Runtime(e) => write!(f, "Runtime error: {}", e),
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
