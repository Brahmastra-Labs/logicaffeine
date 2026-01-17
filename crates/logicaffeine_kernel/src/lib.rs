#![cfg_attr(docsrs, feature(doc_cfg))]

//! The Kernel: Calculus of Constructions
//!
//! A unified type system where terms and types are the same syntactic category,
//! based on the Calculus of Inductive Constructions (CIC).
//!
//! # Core Insight
//!
//! Everything is a [`Term`]:
//! - Types are Terms: `Nat : Type 0`
//! - Values are Terms: `zero : Nat`
//! - Functions are Terms: `λx:Nat. x`
//! - Proofs are Terms: `refl : a = a`
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                        Interface                            │
//! │  (term_parser, literate_parser, command)                    │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │                      Type Checker                           │
//! │  infer_type, is_subtype, substitute                         │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!               ┌──────────────┴──────────────┐
//!               ▼                             ▼
//! ┌─────────────────────────┐   ┌─────────────────────────────┐
//! │      Reduction          │   │         Prelude             │
//! │  normalize, reduce      │   │  standard library types     │
//! └─────────────────────────┘   └─────────────────────────────┘
//!                                             │
//!               ┌─────────────┬───────────────┼───────────────┐
//!               ▼             ▼               ▼               ▼
//! ┌───────────────┐ ┌───────────────┐ ┌───────────────┐ ┌───────────┐
//! │     ring      │ │     lia       │ │      cc       │ │   omega   │
//! │  polynomial   │ │    linear     │ │  congruence   │ │  integer  │
//! │   equality    │ │  arithmetic   │ │   closure     │ │ arithmetic│
//! └───────────────┘ └───────────────┘ └───────────────┘ └───────────┘
//! ```
//!
//! # Public API
//!
//! ## Core Types
//! - [`Term`] - The unified representation of terms, types, and proofs
//! - [`Context`] - Typing context with local and global bindings
//! - [`KernelError`] - Error types for type checking failures
//!
//! ## Type Checking
//! - [`infer_type`] - Infer the type of a term
//! - [`is_subtype`] - Check subtyping with cumulativity
//! - [`normalize`] - Reduce a term to normal form
//!
//! ## Decision Procedures
//! - [`ring`] - Polynomial equality by normalization
//! - [`lia`] - Linear arithmetic by Fourier-Motzkin
//! - [`cc`] - Congruence closure for uninterpreted functions
//! - [`omega`] - Integer arithmetic with exact semantics
//! - [`simp`] - General simplification with rewriting
//!
//! # Milner Invariant
//!
//! This crate has NO path to the lexicon. Adding words to the English
//! vocabulary never triggers a recompile of the type checker. The kernel
//! is purely logical and language-agnostic.

mod context;
mod error;
pub mod interface;
pub mod positivity;
pub mod prelude;
mod reduction;
pub mod ring;
pub mod lia;
pub mod cc;
pub mod simp;
pub mod omega;
pub mod termination;
mod term;
mod type_checker;

pub use context::Context;
pub use error::{KernelError, KernelResult};
pub use reduction::normalize;
pub use term::{Literal, Term, Universe};
pub use type_checker::{infer_type, is_subtype};
