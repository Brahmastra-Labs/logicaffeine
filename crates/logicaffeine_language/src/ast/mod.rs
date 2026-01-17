//! Abstract Syntax Tree types for both logical expressions and imperative statements.
//!
//! This module defines the core AST types produced by the parser and consumed by
//! the transpiler, interpreter, and verifier. It is split into three submodules:
//!
//! - [`logic`]: First-order logic expressions (predicates, quantifiers, connectives)
//! - [`stmt`]: Imperative statements (let bindings, if/match, loops, function defs)
//! - [`theorem`]: Theorem and proof blocks for the vernacular proof language
//!
//! # Logic Expressions ([`LogicExpr`])
//!
//! The logical fragment includes:
//! - Predicates with terms: `Predicate { name, args, world }`
//! - Quantifiers: `∀x`, `∃x` with island tracking for scope
//! - Connectives: `∧`, `∨`, `→`, `↔`, `¬`
//! - Modal operators: `□`, `◇` with Kripke semantics
//! - Lambda terms: `λx.body` for compositional semantics
//! - Neo-Davidsonian events: `NeoEvent` with thematic roles
//!
//! # Imperative Statements ([`Stmt`])
//!
//! The imperative fragment (LOGOS mode) includes:
//! - Let bindings with optional type annotations
//! - Control flow: if/else, match, while/for
//! - Function definitions with refinement types
//! - Assert/require/ensure for specification
//!
//! # Arena Allocation
//!
//! All AST nodes are arena-allocated using `bumpalo` for efficient memory management.
//! The `'a` lifetime parameter tracks the arena lifetime.

pub mod logic;
pub mod stmt;
pub mod theorem;

pub use logic::*;
pub use stmt::{Stmt, Expr, Literal, Block, BinaryOpKind, TypeExpr, MatchArm};
pub use theorem::{TheoremBlock, ProofStrategy};
