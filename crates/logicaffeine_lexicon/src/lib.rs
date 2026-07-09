#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
#![doc = include_str!("../README.md")]

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
