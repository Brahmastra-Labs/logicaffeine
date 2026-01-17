#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

//! English vocabulary types and runtime lexicon loading.
//!
//! This crate provides the core lexicon infrastructure for the logicaffeine
//! English-to-First-Order-Logic transpiler. It defines the type system for
//! representing linguistic knowledge and optionally provides runtime lexicon
//! loading for faster development iteration.
//!
//! # Core Types
//!
//! The [`types`] module defines the fundamental linguistic categories:
//!
//! - **Verb classification**: [`VerbClass`] (Vendler's Aktionsart), [`Time`], [`Aspect`]
//! - **Noun properties**: [`Number`], [`Gender`], [`Case`], [`Definiteness`]
//! - **Semantic typing**: [`Sort`] (type hierarchy for entities)
//! - **Lexical features**: [`Feature`] (27 grammatical and semantic properties)
//! - **Metadata structs**: [`VerbMetadata`], [`NounMetadata`], [`AdjectiveMetadata`]
//!
//! # Architecture
//!
//! The lexicon supports two modes of operation:
//!
//! 1. **Compile-time** (default): The main `logicaffeine_language` crate generates
//!    Rust code from `lexicon.json` at build time, providing type-safe lookups
//!    with zero runtime parsing overhead.
//!
//! 2. **Runtime** (feature `dynamic-lexicon`): The [`runtime`] module loads and
//!    parses `lexicon.json` at runtime, trading compile-time safety for faster
//!    edit-compile cycles during development.
//!
//! # Feature Flags
//!
//! | Feature | Description |
//! |---------|-------------|
//! | `dynamic-lexicon` | Enable runtime JSON lexicon loading via the [`runtime`] module |
//!
//! # Example
//!
//! ```
//! use logicaffeine_lexicon::{VerbClass, Feature, Sort};
//!
//! // Check verb aspectual properties
//! let class = VerbClass::Activity;
//! assert!(!class.is_stative());
//! assert!(class.is_durative());
//!
//! // Parse features from strings
//! let feature = Feature::from_str("Transitive");
//! assert_eq!(feature, Some(Feature::Transitive));
//!
//! // Check sort compatibility
//! assert!(Sort::Human.is_compatible_with(Sort::Animate));
//! ```

/// Lexicon type definitions for grammatical and semantic categories.
pub mod types;
pub use types::*;

/// Runtime JSON-based lexicon loading (requires `dynamic-lexicon` feature).
///
/// This module provides dynamic lexicon loading as an alternative to compile-time
/// code generation. It defines its own entry types (`runtime::VerbEntry`, etc.)
/// for JSON deserialization, distinct from the compile-time types in [`types`].
#[cfg(feature = "dynamic-lexicon")]
#[cfg_attr(docsrs, doc(cfg(feature = "dynamic-lexicon")))]
pub mod runtime;
