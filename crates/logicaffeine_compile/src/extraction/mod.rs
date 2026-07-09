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
pub mod fol_model;
pub mod verilog;

pub use codegen::{emit_property_check, emit_value, primitive_rust_type};
pub use collector::{is_extractable, is_logical_type};
pub use error::ExtractError;

use logicaffeine_kernel::Context;
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
    extract_programs(ctx, &[entry])
}

/// Extract a program rooted at several entry points into one Rust module.
///
/// Returns Rust source code as a string. The transitive dependencies of every
/// entry are unioned, topologically sorted once, and emitted through a single
/// [`CodeGen`], so shared dependencies (e.g. a `Nat` enum used by both `add` and
/// `double`) are emitted exactly once.
///
/// # Arguments
///
/// * `ctx` - The kernel context containing definitions and inductives
/// * `entries` - The names of the entry points to extract
///
/// # Errors
///
/// Returns an error if any entry point is not found or cannot be extracted.
pub fn extract_programs(ctx: &Context, entries: &[&str]) -> Result<String, ExtractError> {
    // 1. Collect the union of all dependencies, validating each entry exists.
    let mut all_deps: HashSet<String> = HashSet::new();
    for entry in entries {
        if !ctx.is_inductive(entry) && !ctx.is_definition(entry) && !ctx.is_constructor(entry) {
            return Err(ExtractError::NotFound((*entry).to_string()));
        }
        all_deps.extend(collect_dependencies(ctx, entry));
    }

    // 2. Topologically sort (inductives first, then definitions)
    let sorted = topological_sort(ctx, &all_deps);

    // 3. Generate code through a single CodeGen so shared deps dedup.
    let mut codegen = CodeGen::new(ctx);
    for name in &sorted {
        if ctx.is_inductive(name) {
            // Opaque/primitive inductives (Int, Float, Text, Bool, Duration, …)
            // are declared with NO constructors; they have no enum form and are
            // mapped to Rust types at use sites, so skip them rather than
            // erroring with `NotFound`.
            if !ctx.get_constructors(name).is_empty() {
                codegen.emit_inductive(name)?;
            }
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

    // Inductives first, then definitions — and each group sorted by name so the
    // emitted module is DETERMINISTIC regardless of HashSet/HashMap iteration
    // order (every Compile builds a fresh kernel with new hash seeds). Rust
    // resolves top-level items order-independently, so sorting is sound.
    inductives.sort();
    definitions.sort();
    inductives.extend(definitions);
    inductives
}
