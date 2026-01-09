// =============================================================================
// THEOREM AST (PHASE 63)
// =============================================================================
//
// This module defines the AST for theorem blocks:
//
// ```logos
// ## Theorem: Name
// Given: Premise 1.
// Given: Premise 2.
// Prove: Goal.
// Proof: Auto.
// ```

use super::logic::LogicExpr;

/// A theorem block containing premises, goal, and proof strategy.
#[derive(Debug)]
pub struct TheoremBlock<'a> {
    /// The name of the theorem (e.g., "Socrates_Mortality")
    pub name: String,

    /// Premises (Given statements) - logical expressions to assume true
    pub premises: Vec<&'a LogicExpr<'a>>,

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
}

impl Default for ProofStrategy {
    fn default() -> Self {
        ProofStrategy::Auto
    }
}
