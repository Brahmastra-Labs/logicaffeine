//! Code generation from LOGOS AST to Rust source code.
//!
//! This module transforms the parsed and type-checked LOGOS program into
//! idiomatic Rust code. The generated code uses `logicaffeine_data` types
//! for runtime values and integrates with the kernel for proof verification.
//!
//! # Pipeline Position
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │  LOGOS Source → Lexer → Parser → AST → Analysis → HERE │
//! └─────────────────────────────────────────────────────────┘
//!                                                      ↓
//!                                               Rust Source
//! ```
//!
//! # Code Generation Rules
//!
//! | LOGOS Statement | Rust Output |
//! |-----------------|-------------|
//! | `Let x be 5.` | `let x = 5;` |
//! | `Set x to 10.` | `x = 10;` |
//! | `Give x to y.` | `let y = x;` (move) |
//! | `Show x to show.` | `println!("{}", x);` |
//! | `If x > 0 then...` | `if x > 0 { ... }` |
//! | `Repeat while x > 0...` | `while x > 0 { ... }` |
//! | `Zone "name"...` | `{ /* zone scope */ }` |
//! | `Mount x at "path".` | `x.mount(vfs, "path").await;` |
//! | `Sync x on "topic".` | `x.subscribe("topic").await;` |
//!
//! # Key Features
//!
//! - **Refinement Types**: Generates `debug_assert!` for type predicates
//! - **Policy Enforcement**: Emits capability checks for access control
//! - **Zone Safety**: Translates memory zones to Rust scopes
//! - **CRDT Mutability**: Uses `.set()` for LWWRegister/MVRegister fields
//! - **Async Detection**: Adds `#[tokio::main]` when async operations are present
//! - **VFS Detection**: Injects `NativeVfs::new()` for file operations
//! - **Mount+Sync Detection**: Uses `Distributed<T>` for combined persistence/sync
//!
//! # Refinement Context
//!
//! The [`RefinementContext`] tracks type predicates across scopes:
//!
//! ```text
//! Let x: { it: Int | it > 0 } be 5.   ←  Register constraint
//!      ↓
//! debug_assert!(5 > 0);               ←  Check at definition
//!      ↓
//! Set x to 10.                        ←  Re-check on mutation
//!      ↓
//! debug_assert!(10 > 0);              ←  Re-emit assertion
//! ```
//!
//! - When a variable with a refinement type is defined, its constraint is registered
//! - When that variable is mutated, the assertion is re-emitted
//! - Variable types are tracked for capability resolution
//!
//! # Entry Point
//!
//! The main entry point is [`codegen_program`], which generates a complete Rust
//! program from a list of statements.

// Module declarations
pub(crate) mod context;
pub(crate) mod detection;
pub(crate) mod types;
mod policy;
pub(crate) mod ffi;
pub(crate) mod bindings;
pub(crate) mod tce;
pub(crate) mod marshal;
pub(crate) mod peephole;
pub(crate) mod stmt;
pub(crate) mod expr;
pub(crate) mod program;

// ─── External API re-exports ────────────────────────────────────────────────
// These preserve the public API used by compile.rs, ui_bridge.rs, and test files.

pub use context::{RefinementContext, VariableCapabilities, empty_var_caps};
pub use detection::{collect_async_functions, collect_pipe_sender_params, collect_pipe_vars};
pub use program::codegen_program;
pub use ffi::generate_c_header;
pub use bindings::{generate_python_bindings, generate_typescript_bindings};
pub use expr::{codegen_expr, codegen_assertion, codegen_term};
pub use stmt::codegen_stmt;

// ─── Internal cross-module re-exports ───────────────────────────────────────
// These allow sibling submodules to use `use super::item` for items
// that moved out of mod.rs into submodule files.

pub(crate) use ffi::{
    CAbiClass, classify_type_for_c_abi, has_wasm_exports, has_c_exports, has_c_exports_with_text,
    codegen_c_accessors, collect_c_export_ref_structs, collect_c_export_reference_types,
    collect_c_export_value_type_structs, codegen_logos_runtime_preamble, mangle_type_for_c,
    map_type_to_c_header, map_field_type_to_c,
};
pub(crate) use expr::{
    codegen_expr_with_async, codegen_expr_boxed, codegen_expr_boxed_with_strings,
    codegen_expr_boxed_with_types, codegen_interpolated_string, codegen_literal,
    codegen_expr_with_async_and_strings, is_definitely_string_expr_with_vars,
    is_definitely_string_expr, is_definitely_numeric_expr, collect_string_concat_operands,
};
pub(crate) use stmt::{get_root_identifier, is_copy_type, has_copy_element_type, has_copy_value_type};
pub(crate) use peephole::{
    try_emit_for_range_pattern, try_emit_vec_fill_pattern, try_emit_swap_pattern,
    try_emit_seq_copy_pattern, try_emit_rotate_left_pattern,
    body_mutates_collection, exprs_equal,
};
pub(crate) use types::is_recursive_field;

/// Check if a name is a Rust keyword (needs `r#` escaping in generated code).
pub(crate) fn is_rust_keyword(name: &str) -> bool {
    matches!(name,
        "as" | "async" | "await" | "break" | "const" | "continue" | "crate" |
        "dyn" | "else" | "enum" | "extern" | "false" | "fn" | "for" | "if" |
        "impl" | "in" | "let" | "loop" | "match" | "mod" | "move" | "mut" |
        "pub" | "ref" | "return" | "self" | "Self" | "static" | "struct" |
        "super" | "trait" | "true" | "type" | "unsafe" | "use" | "where" |
        "while" | "abstract" | "become" | "box" | "do" | "final" | "macro" |
        "override" | "priv" | "try" | "typeof" | "unsized" | "virtual" | "yield"
    )
}

/// Escape a name as a Rust raw identifier if it's a keyword.
/// e.g., "move" → "r#move", "add" → "add"
pub(crate) fn escape_rust_ident(name: &str) -> String {
    if is_rust_keyword(name) {
        format!("r#{}", name)
    } else {
        name.to_string()
    }
}
