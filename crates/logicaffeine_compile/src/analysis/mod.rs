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

pub mod assemble;
pub mod callgraph;
pub mod check;
pub mod defined_real;
pub mod dimension_check;
mod discovery;
pub mod escape;
pub(crate) mod absence;
pub(crate) mod exactness;
pub mod import_hygiene;
pub mod known_calls;
pub mod liveness;
pub(crate) mod numbers;
pub mod ownership;
pub mod pipes;
pub mod readonly;
pub mod relay;
pub mod tower;
pub mod types;
pub mod unify;

pub use assemble::{
    assemble_project, assemble_project_with_deps, check_scopes, AssembledProgram, DefKind, DefSite,
    Located, Provenance, ScopeInfo, Segment, SourceMap,
};
pub use check::{
    check_program, check_program_collect, check_program_located, infer_instantiations,
    root_binding_types, CallInstance, IndexedTypeError, Instantiations, LocatedTypeError, Site,
};
pub use dimension_check::{DimensionChecker, DimensionError};
pub use discovery::discover_with_imports;
pub use escape::{EscapeChecker, EscapeError, EscapeErrorKind};
pub use import_hygiene::{
    lint_imports, lint_imports_with_deps, ImportLint, ImportLintKind, LintSeverity,
};
pub use known_calls::{
    check_unknown_calls, check_unknown_calls_with_exports, proven_module_exports,
    refuse_retired_calls,
};
pub use ownership::{
    MaybeGivenIn, OwnershipChecker, OwnershipError, OwnershipErrorKind, OwnershipFinding, VarState,
};
pub use types::{FnSig, LogosType, RustNames, TypeEnv};

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
    first_section_header, misplaced_imports, scan_dependencies, CapabilityDef, Dependency,
    DiscoveryPass, DiscoveryResult, FieldDef, FieldType, PolicyCondition, PolicyRegistry,
    PredicateDef, TypeDef, TypeRegistry, VariantDef,
};
