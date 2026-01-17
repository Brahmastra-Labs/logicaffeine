//! Static analysis passes for type and policy discovery.
//!
//! This module provides analysis passes that run before or alongside main parsing:
//!
//! | Pass | Purpose |
//! |------|---------|
//! | [`DiscoveryPass`] | Scans for type definitions (`## Definition` blocks) |
//! | [`TypeRegistry`] | Stores and looks up type definitions during parsing |
//! | [`PolicyRegistry`] | Stores security predicates and capability definitions |
//! | [`scan_dependencies`] | Finds module imports in the document abstract |
//!
//! # Usage Order
//!
//! 1. Lexer tokenizes the source
//! 2. [`DiscoveryPass`] scans tokens for type/policy definitions
//! 3. Parser receives populated [`TypeRegistry`] and [`PolicyRegistry`]
//! 4. Code generator uses registries for type-aware output

pub mod registry;
pub mod discovery;
pub mod dependencies;
pub mod policy;

pub use registry::{FieldDef, FieldType, TypeDef, TypeRegistry, VariantDef};
pub use discovery::{DiscoveryPass, DiscoveryResult};
pub use dependencies::{scan_dependencies, Dependency};
pub use policy::{PolicyRegistry, PredicateDef, CapabilityDef, PolicyCondition};
