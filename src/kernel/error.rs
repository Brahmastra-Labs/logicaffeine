//! Error types for the kernel type checker.

use std::fmt;

/// Errors that can occur during type checking.
#[derive(Debug)]
pub enum KernelError {
    /// Reference to an undefined variable.
    UnboundVariable(String),

    /// Attempted to apply a non-function term.
    NotAFunction(String),

    /// Expected a type (something with type Sort), got something else.
    NotAType(String),

    /// Type mismatch: expected one type, found another.
    TypeMismatch { expected: String, found: String },

    /// Attempted to match on a non-inductive type.
    NotAnInductive(String),

    /// Invalid motive in match expression.
    InvalidMotive(String),

    /// Wrong number of cases in match expression.
    WrongNumberOfCases { expected: usize, found: usize },

    /// Error during proof certification.
    CertificationError(String),

    /// Recursive call does not decrease structurally.
    ///
    /// This error prevents infinite loops in proofs.
    /// A fixpoint must recurse on a structurally smaller argument.
    TerminationViolation { fix_name: String, reason: String },

    /// Inductive type appears in negative position in constructor.
    ///
    /// This error prevents logical paradoxes (Russell's paradox, etc).
    /// An inductive must appear strictly positively in its constructors.
    PositivityViolation {
        inductive: String,
        constructor: String,
        reason: String,
    },
}

impl fmt::Display for KernelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KernelError::UnboundVariable(v) => {
                write!(f, "Unbound variable: {}", v)
            }
            KernelError::NotAFunction(t) => {
                write!(f, "Not a function: {}", t)
            }
            KernelError::NotAType(t) => {
                write!(f, "Not a type: {}", t)
            }
            KernelError::TypeMismatch { expected, found } => {
                write!(f, "Type mismatch: expected {}, found {}", expected, found)
            }
            KernelError::NotAnInductive(t) => {
                write!(f, "Not an inductive type: {}", t)
            }
            KernelError::InvalidMotive(t) => {
                write!(f, "Invalid motive: {}", t)
            }
            KernelError::WrongNumberOfCases { expected, found } => {
                write!(
                    f,
                    "Wrong number of cases: expected {}, found {}",
                    expected, found
                )
            }
            KernelError::CertificationError(msg) => {
                write!(f, "Certification error: {}", msg)
            }
            KernelError::TerminationViolation { fix_name, reason } => {
                write!(
                    f,
                    "Termination violation in '{}': {}",
                    fix_name, reason
                )
            }
            KernelError::PositivityViolation {
                inductive,
                constructor,
                reason,
            } => {
                write!(
                    f,
                    "Positivity violation: constructor '{}' of '{}': {}",
                    constructor, inductive, reason
                )
            }
        }
    }
}

impl std::error::Error for KernelError {}

/// Result type for kernel operations.
pub type KernelResult<T> = Result<T, KernelError>;
