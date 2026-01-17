//! Error types for proof search and unification.
//!
//! This module defines [`ProofError`], which captures all failure modes
//! during proof construction. Errors are designed to be informative for
//! debugging and educational feedback.
//!
//! # Error Categories
//!
//! | Category | Variants | Meaning |
//! |----------|----------|---------|
//! | Search | `NoProofFound`, `DepthExceeded` | Proof search terminated |
//! | Unification | `OccursCheck`, `UnificationFailed`, `ExprUnificationFailed` | Terms cannot be unified |
//! | Matching | `SymbolMismatch`, `ArityMismatch` | Structural incompatibility |
//! | Higher-Order | `PatternNotDistinct`, `NotAPattern`, `ScopeViolation` | Miller pattern failures |
//!
//! # Example
//!
//! ```
//! use logicaffeine_proof::ProofError;
//!
//! fn check_result(result: Result<(), ProofError>) {
//!     match result {
//!         Ok(()) => println!("Proof found"),
//!         Err(ProofError::NoProofFound) => println!("No proof exists"),
//!         Err(ProofError::DepthExceeded) => println!("Search too deep"),
//!         Err(e) => println!("Error: {}", e),
//!     }
//! }
//! ```

use crate::{ProofExpr, ProofTerm};
use std::fmt;

/// Errors that can occur during proof search.
#[derive(Debug, Clone)]
pub enum ProofError {
    /// No proof was found for the given goal.
    NoProofFound,

    /// The proof search exceeded the maximum depth limit.
    DepthExceeded,

    /// Unification failed due to occurs check.
    /// This prevents infinite types like x = f(x).
    OccursCheck {
        variable: String,
        term: ProofTerm,
    },

    /// Unification failed because terms could not be unified.
    UnificationFailed {
        left: ProofTerm,
        right: ProofTerm,
    },

    /// Expression-level unification failed.
    ExprUnificationFailed {
        left: ProofExpr,
        right: ProofExpr,
    },

    /// Symbol mismatch during unification.
    SymbolMismatch {
        left: String,
        right: String,
    },

    /// Arity mismatch during unification.
    ArityMismatch {
        expected: usize,
        found: usize,
    },

    // --- Higher-Order Pattern Unification ---

    /// The pattern has non-distinct arguments (e.g., ?F(x, x))
    /// Miller patterns require distinct bound variables.
    PatternNotDistinct(String),

    /// Expression is not a valid Miller pattern.
    /// Pattern arguments must be Term(BoundVarRef(...)).
    NotAPattern(ProofExpr),

    /// RHS uses variables not in pattern scope.
    /// In ?P(x) = Body, Body must only use variables that appear in the pattern.
    ScopeViolation {
        var: String,
        allowed: Vec<String>,
    },
}

impl fmt::Display for ProofError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProofError::NoProofFound => {
                write!(f, "No proof found: the goal could not be derived from the knowledge base")
            }
            ProofError::DepthExceeded => {
                write!(f, "Proof search exceeded maximum depth limit")
            }
            ProofError::OccursCheck { variable, term } => {
                write!(
                    f,
                    "Occurs check failed: variable '{}' appears in term '{}' (would create infinite type)",
                    variable, term
                )
            }
            ProofError::UnificationFailed { left, right } => {
                write!(f, "Unification failed: cannot unify '{}' with '{}'", left, right)
            }
            ProofError::ExprUnificationFailed { left, right } => {
                write!(f, "Expression unification failed: cannot unify '{}' with '{}'", left, right)
            }
            ProofError::SymbolMismatch { left, right } => {
                write!(f, "Symbol mismatch: '{}' vs '{}'", left, right)
            }
            ProofError::ArityMismatch { expected, found } => {
                write!(f, "Arity mismatch: expected {} arguments, found {}", expected, found)
            }
            ProofError::PatternNotDistinct(var) => {
                write!(f, "Pattern has duplicate variable '{}': Miller patterns require distinct bound variables", var)
            }
            ProofError::NotAPattern(expr) => {
                write!(f, "Not a valid Miller pattern: '{}' (arguments must be bound variable references)", expr)
            }
            ProofError::ScopeViolation { var, allowed } => {
                write!(f, "Scope violation: variable '{}' not in pattern scope {:?}", var, allowed)
            }
        }
    }
}

impl std::error::Error for ProofError {}

/// Result type for proof operations.
pub type ProofResult<T> = Result<T, ProofError>;
