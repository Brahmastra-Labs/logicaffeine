//! Compile-time analysis passes for the LOGOS compilation pipeline.
//!
//! This module provides static analysis that runs after parsing but before
//! code generation. These passes detect errors that would otherwise manifest
//! as confusing Rust borrow checker errors.
//!
//! # Analysis Passes
//!
//! | Pass | Module | Description |
//! |------|--------|-------------|
//! | Escape | [`escape`] | Detects zone-local values escaping their scope |
//! | Ownership | [`ownership`] | Linear type enforcement (use-after-move) |
//! | Discovery | [`discover_with_imports`] | Multi-file type discovery |
//!
//! # Pass Ordering
//!
//! ```text
//! Parser Output (AST)
//!        │
//!        ▼
//! ┌──────────────┐
//! │ Escape Check │ ← Catches zone violations (fast, simple)
//! └──────────────┘
//!        │
//!        ▼
//! ┌─────────────────┐
//! │ Ownership Check │ ← Catches use-after-move (control-flow aware)
//! └─────────────────┘
//!        │
//!        ▼
//!    Code Generation
//! ```
//!
//! # Re-exports
//!
//! This module re-exports types from `logicaffeine_language::analysis` for
//! convenience, including:
//! - [`TypeRegistry`], [`TypeDef`], [`FieldDef`] - Type definitions
//! - [`PolicyRegistry`], [`PredicateDef`], [`CapabilityDef`] - Security policies
//! - [`DiscoveryPass`] - Token-level type discovery

pub mod callgraph;
pub mod check;
pub mod escape;
pub mod liveness;
pub mod ownership;
pub mod readonly;
pub mod types;
pub mod unify;
mod discovery;

pub use escape::{EscapeChecker, EscapeError, EscapeErrorKind};
pub use ownership::{OwnershipChecker, OwnershipError, OwnershipErrorKind, VarState};
pub use discovery::discover_with_imports;
pub use types::{LogosType, TypeEnv, FnSig, RustNames};
pub use check::check_program;

// Re-export language analysis types with submodule aliases
pub mod registry {
    pub use logicaffeine_language::analysis::registry::*;
}

pub mod policy {
    pub use logicaffeine_language::analysis::policy::*;
}

pub mod dependencies {
    pub use logicaffeine_language::analysis::dependencies::*;
}

pub use logicaffeine_language::analysis::{
    TypeRegistry, TypeDef, FieldDef, FieldType, VariantDef,
    DiscoveryPass, DiscoveryResult,
    PolicyRegistry, PredicateDef, CapabilityDef, PolicyCondition,
    scan_dependencies, Dependency,
};
