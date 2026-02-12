#![cfg_attr(docsrs, feature(doc_cfg))]

//! # logicaffeine_compile
//!
//! The compilation pipeline for LOGOS, transforming natural language logic
//! into executable Rust code.
//!
//! ## Architecture
//!
//! ```text
//! LOGOS Source
//!      │
//!      ▼
//! ┌─────────┐     ┌───────────┐     ┌──────────┐
//! │  Lexer  │ ──▶ │  Parser   │ ──▶ │   AST    │
//! └─────────┘     └───────────┘     └──────────┘
//!                                         │
//!      ┌──────────────────────────────────┘
//!      ▼
//! ┌─────────────────────────────────────────────┐
//! │            Analysis Passes                   │
//! │  ┌─────────┐  ┌───────────┐  ┌───────────┐ │
//! │  │ Escape  │  │ Ownership │  │    Z3     │ │
//! │  └─────────┘  └───────────┘  └───────────┘ │
//! └─────────────────────────────────────────────┘
//!      │
//!      ▼
//! ┌──────────┐     ┌────────────┐
//! │ CodeGen  │ ──▶ │ Rust Code  │
//! └──────────┘     └────────────┘
//! ```
//!
//! ## Feature Flags
//!
//! | Feature | Description |
//! |---------|-------------|
//! | `codegen` | Rust code generation (default) |
//! | `verification` | Z3-based static verification |
//!
//! ## Modules
//!
//! - [`compile`]: Top-level compilation functions
//! - [`codegen`]: AST to Rust code generation (requires `codegen` feature)
//! - [`analysis`]: Static analysis passes (escape, ownership, discovery)
//! - [`extraction`]: Kernel term extraction to Rust
//! - [`interpreter`]: Tree-walking AST interpreter
//! - [`diagnostic`]: Rustc error translation to LOGOS-friendly messages
//! - [`sourcemap`]: Source location mapping for diagnostics
//! - [`loader`]: Multi-file module loading
//! - [`ui_bridge`]: Web interface integration
//! - `verification`: Z3-based static verification (requires `verification` feature)
//!
//! ## Getting Started
//!
//! ### Basic Compilation
//!
//! ```
//! use logicaffeine_compile::compile::compile_to_rust;
//! # use logicaffeine_compile::ParseError;
//! # fn main() -> Result<(), ParseError> {
//!
//! let source = "## Main\nLet x be 5.\nShow x.";
//! let rust_code = compile_to_rust(source)?;
//! # Ok(())
//! # }
//! ```
//!
//! ### With Ownership Checking
//!
//! ```
//! use logicaffeine_compile::compile::compile_to_rust_checked;
//!
//! let source = "## Main\nLet x be 5.\nGive x to y.\nShow x.";
//! // Returns error: use-after-move detected at check-time
//! let result = compile_to_rust_checked(source);
//! ```
//!
//! ### Interpretation
//!
//! ```no_run
//! use logicaffeine_compile::interpret_for_ui;
//!
//! # fn main() {}
//! # async fn example() {
//! let source = "## Main\nLet x be 5.\nShow x.";
//! let result = interpret_for_ui(source).await;
//! // result.lines contains ["5"]
//! # }
//! ```

// Re-export base types
pub use logicaffeine_base::{Arena, Interner, Symbol, SymbolEq};

// Re-export language types needed for compilation
pub use logicaffeine_language::{
    ast, drs, error, lexer, parser, token,
    analysis::{TypeRegistry, DiscoveryPass, PolicyRegistry, PolicyCondition},
    arena_ctx::AstContext,
    registry::SymbolRegistry,
    formatter,
    mwe,
    Lexer, Parser, ParseError,
};

// Re-export kernel for extraction
pub use logicaffeine_kernel as kernel;

// Module loading
pub mod loader;
pub use loader::{Loader, ModuleSource};

// Compile-time analysis
pub mod analysis;

// Code generation
#[cfg(feature = "codegen")]
pub mod codegen;

// Compilation pipeline
pub mod compile;
pub use compile::{CompileOutput, CrateDependency};

// Diagnostics
pub mod diagnostic;

// Source mapping
pub mod sourcemap;

// Extraction (proof term extraction)
pub mod extraction;

// Optimization passes
pub mod optimize;

// Interpreter
pub mod interpreter;

// UI Bridge - high-level compilation for web interface
pub mod ui_bridge;

// Verification pass (Z3-based, requires verification feature)
#[cfg(feature = "verification")]
pub mod verification;
#[cfg(feature = "verification")]
pub use verification::VerificationPass;

// Re-export UI types at crate root for convenience
pub use ui_bridge::{
    compile_for_ui, compile_for_proof, compile_theorem_for_ui, verify_theorem,
    interpret_for_ui, interpret_streaming, CompileResult, ProofCompileResult, TheoremCompileResult,
    AstNode, TokenInfo, TokenCategory,
};
#[cfg(feature = "codegen")]
pub use ui_bridge::generate_rust_code;

// Provide module aliases for internal code
pub mod intern {
    pub use logicaffeine_base::{Interner, Symbol, SymbolEq};
}

pub mod arena {
    pub use logicaffeine_base::Arena;
}

pub mod arena_ctx {
    pub use logicaffeine_language::arena_ctx::*;
}

pub mod registry {
    pub use logicaffeine_language::registry::*;
}

pub mod style {
    pub use logicaffeine_language::style::*;
}
