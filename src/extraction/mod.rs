//! Program extraction from kernel terms to Rust.
//!
//! This module compiles verified kernel terms into executable Rust source code.
//!
//! # Extraction Rules
//!
//! | Kernel Term | Rust Output |
//! |-------------|-------------|
//! | Inductive | `enum Name { Ctor(Args...) }` |
//! | Fix + Lambdas | `fn name(args) -> Ret { body }` |
//! | Match | `match disc { Name::Ctor(vars) => body }` |
//! | App | `func(arg)` |
//!
//! # Structural Honesty
//!
//! We extract types exactly as defined, with no optimization.
//! For example, `Nat` becomes `enum Nat { Zero, Succ(Box<Nat>) }`.
//! This guarantees that the extracted code is isomorphic to the verified logic.

mod codegen;
mod collector;
mod error;

pub use error::ExtractError;

use crate::kernel::Context;
use codegen::CodeGen;
use collector::collect_dependencies;
use std::collections::HashSet;

/// Extract a program rooted at the given entry point.
///
/// Returns Rust source code as a string.
///
/// # Arguments
///
/// * `ctx` - The kernel context containing definitions and inductives
/// * `entry` - The name of the entry point to extract
///
/// # Errors
///
/// Returns an error if the entry point is not found or cannot be extracted.
pub fn extract_program(ctx: &Context, entry: &str) -> Result<String, ExtractError> {
    // Check if entry exists
    if !ctx.is_inductive(entry) && !ctx.is_definition(entry) && !ctx.is_constructor(entry) {
        return Err(ExtractError::NotFound(entry.to_string()));
    }

    // 1. Collect all dependencies
    let deps = collect_dependencies(ctx, entry);

    // 2. Topologically sort (inductives first, then definitions)
    let sorted = topological_sort(ctx, &deps);

    // 3. Generate code
    let mut codegen = CodeGen::new(ctx);
    for name in &sorted {
        if ctx.is_inductive(name) {
            codegen.emit_inductive(name)?;
        } else if ctx.is_definition(name) {
            codegen.emit_definition(name)?;
        }
        // Skip constructors (emitted with their inductive)
    }

    Ok(codegen.finish())
}

/// Topologically sort dependencies.
///
/// Inductives come first, then definitions in dependency order.
fn topological_sort(ctx: &Context, deps: &HashSet<String>) -> Vec<String> {
    let mut inductives = Vec::new();
    let mut definitions = Vec::new();

    for name in deps {
        if ctx.is_inductive(name) {
            inductives.push(name.clone());
        } else if ctx.is_definition(name) {
            definitions.push(name.clone());
        }
        // Skip constructors - they're emitted with their inductive
    }

    // Simple topological ordering: inductives first, then definitions
    // A more sophisticated implementation would do proper dependency analysis
    inductives.extend(definitions);
    inductives
}
