#![cfg_attr(docsrs, feature(doc_cfg))]

//! # logicaffeine-base
//!
//! Pure structural atoms for the logicaffeine ecosystem.
//!
//! This crate provides the foundational types used throughout logicaffeine:
//!
//! - [`Arena`] — Bump allocation for stable AST references
//! - [`Interner`]/[`Symbol`] — String interning for O(1) equality
//! - [`Span`] — Source location tracking
//! - [`SpannedError`]/[`Result`] — Errors with source positions
//!
//! # Design Principles
//!
//! This crate has **no knowledge of English vocabulary or I/O**. It provides
//! only generic, reusable infrastructure that higher-level crates build upon.
//!
//! # Example
//!
//! ```
//! use logicaffeine_base::{Arena, Interner, Span};
//!
//! let arena: Arena<&str> = Arena::new();
//! let mut interner = Interner::new();
//!
//! let hello = interner.intern("hello");
//! let span = Span::new(0, 5);
//!
//! let allocated = arena.alloc("hello");
//! assert_eq!(*allocated, "hello");
//! ```

pub mod arena;
pub mod intern;
pub mod span;
pub mod error;

pub use arena::Arena;
pub use intern::{Interner, Symbol, SymbolEq};
pub use span::Span;
pub use error::{SpannedError, Result};
