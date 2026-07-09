//! `## Define` block AST — a vernacular-logic predicate definition (Rung 0a).
//!
//! A definition introduces a new predicate (the *definiendum*) as an
//! abbreviation for a formula over its parameters (the *definiens*), written as
//! a biconditional sentence:
//!
//! ```text
//! ## Define
//! x is a bachelor if and only if x is unmarried and x is a man.
//! ```
//!
//! The parser splits the biconditional: the LHS `Predicate` supplies the
//! definiendum name and its parameter symbols; the RHS is the definiens. The
//! proof layer registers this as a δ-unfoldable kernel definition, so the
//! predicate becomes a reusable, citable node — not an inlined expansion.

use super::logic::LogicExpr;
use logicaffeine_base::Symbol;

/// A `## Define` block: `<definiendum>(params) if and only if <definiens>`.
#[derive(Debug, Clone)]
pub struct DefinitionBlock<'a> {
    /// The definiendum predicate name (the LHS predicate, e.g. `"bachelor"`).
    pub name: String,
    /// The parameter symbols the definiendum binds (its LHS arguments).
    pub params: Vec<Symbol>,
    /// The definiendum application — the LHS `Predicate` of the biconditional.
    pub definiendum: &'a LogicExpr<'a>,
    /// The definiens — the RHS of the biconditional.
    pub definiens: &'a LogicExpr<'a>,
}
