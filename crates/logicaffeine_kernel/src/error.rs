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

    /// A checked declaration violates a semantic kernel invariant.
    ///
    /// Unlike CertificationError, this is a rejection, not a resource limit or
    /// unsupported operation. Callers must not classify inconclusive reduction
    /// failures as invalid declarations.
    InvalidDeclaration(String),

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

    /// Cannot infer the type of a hole without context.
    ///
    /// Holes (implicit arguments) need to be checked against an expected type,
    /// not inferred standalone.
    CannotInferHole,

    /// A declaration would change what a registered name means while declarations outside the
    /// name's own declaration unit depend on that meaning.
    ///
    /// Every dependent was checked against the current meaning, so the change would leave each
    /// one certified for a statement it no longer states. Lean's environment is append-only;
    /// LOGOS accepts a redeclaration while nothing relies on the name, and an identical one
    /// always. `dependents` names every dependent by the declaration it belongs to (a derived
    /// equation lemma by its definition, a constructor by its inductive), sorted.
    StableMeaningViolation {
        name: String,
        change: MeaningChange,
        dependents: Vec<String>,
    },

    /// The independent kernel ([`crate::recheck`](mod@crate::recheck)) did not confirm a declaration this kernel
    /// accepted: it refuted it, or reached no verdict — [`crate::recheck::Incomplete`] says why,
    /// a construct it does not cover or a resource bound that stopped it. `subject` names what
    /// was being certified.
    IndependentRecheck {
        subject: String,
        error: crate::recheck::ReCheckError,
    },
}

/// How a refused declaration would have changed a registered name's meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeaningChange {
    /// A different type, body, sort, parameter list or constructor list under the same name, or
    /// the name taken over by a declaration of another kind.
    Redeclared,
    /// A constructor added to an inductive whose constructor set something already relies on.
    Extended,
}

impl fmt::Display for MeaningChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            MeaningChange::Redeclared => "redeclared with a different meaning",
            MeaningChange::Extended => "extended with a constructor",
        })
    }
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
            KernelError::InvalidDeclaration(msg) => {
                write!(f, "Invalid declaration: {}", msg)
            }
            KernelError::TerminationViolation { fix_name, reason } => {
                write!(f, "Termination violation in '{}': {}", fix_name, reason)
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
            KernelError::CannotInferHole => {
                write!(
                    f,
                    "Cannot infer type of implicit argument (_) without context"
                )
            }
            KernelError::IndependentRecheck { subject, error } => {
                write!(f, "Certification error: {subject}: {error:?}")
            }
            KernelError::StableMeaningViolation {
                name,
                change,
                dependents,
            } => {
                const SHOWN: usize = 3;
                let shown = dependents
                    .iter()
                    .take(SHOWN)
                    .map(|dependent| format!("`{dependent}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let more = dependents.len().saturating_sub(SHOWN);
                let tail = if more == 0 {
                    String::new()
                } else {
                    format!(" and {more} more")
                };
                let verb = if dependents.len() == 1 { "was" } else { "were" };
                write!(
                    f,
                    "Stable meaning violation: `{name}` cannot be {change}: {shown}{tail} {verb} \
                     checked against its current meaning, and a declared name never changes \
                     meaning under what depends on it (declare a new name instead)"
                )
            }
        }
    }
}

impl std::error::Error for KernelError {}

/// Result type for kernel operations.
pub type KernelResult<T> = Result<T, KernelError>;
