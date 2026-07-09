//! Theorem and proof block AST types.
//!
//! This module defines the AST for theorem blocks in the vernacular proof language:
//!
//! ```text
//! ## Theorem: Socrates_Mortality
//! Given: All men are mortal.
//! Given: Socrates is a man.
//! Prove: Socrates is mortal.
//! Proof: Auto.
//! ```
//!
//! # Key Types
//!
//! - **[`TheoremBlock`]**: Contains premises, goal, and proof strategy
//! - **[`ProofStrategy`]**: How to prove (Auto, Manual, By lemmas)

use super::logic::LogicExpr;

/// A theorem block containing premises, goal, and proof strategy.
#[derive(Debug, Clone)]
pub struct TheoremBlock<'a> {
    /// The name of the theorem (e.g., "Socrates_Mortality")
    pub name: String,

    /// Premises (Given statements) - logical expressions to assume true
    pub premises: Vec<&'a LogicExpr<'a>>,

    /// Optional names for the premises, parallel to `premises`. A `Given (h): …`
    /// names that premise `h`, so a `Proof:` script can refer to it as `cases h`
    /// rather than the positional `hp0`. `None` for an unnamed `Given:`.
    pub premise_names: Vec<Option<String>>,

    /// The goal to prove (Prove statement)
    pub goal: &'a LogicExpr<'a>,

    /// The proof strategy to use
    pub strategy: ProofStrategy,
}

/// Proof strategies for theorem verification.
#[derive(Debug, Clone, PartialEq)]
pub enum ProofStrategy {
    /// Automatic proof search using backward chaining.
    /// The prover will try all available inference rules.
    Auto,

    /// Induction on a variable (for inductive types like Nat, List).
    /// Example: `Proof: Induction on n.`
    Induction(String),

    /// Direct application of a specific rule.
    /// Example: `Proof: ModusPonens.`
    ByRule(String),

    /// An explicit tactic-script proof, written in the English-esque vernacular and
    /// run by the tactic framework. Example:
    /// `Proof: Assume h. By cases on h, right, by assumption. Left, by assumption.`
    Script(String),
}

impl Default for ProofStrategy {
    fn default() -> Self {
        ProofStrategy::Auto
    }
}
