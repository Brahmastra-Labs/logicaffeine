//! Verification error types with Socratic error messages.

use std::fmt;

/// Result type for verification operations.
pub type VerificationResult<T = ()> = Result<T, VerificationError>;

/// A verification error with Socratic explanation.
#[derive(Debug)]
pub struct VerificationError {
    pub kind: VerificationErrorKind,
    pub span: Option<(usize, usize)>,
    pub explanation: String,
    pub counterexample: Option<CounterExample>,
}

/// The kind of verification error.
#[derive(Debug, Clone, PartialEq)]
pub enum VerificationErrorKind {
    /// An assertion that can never be true.
    ContradictoryAssertion,

    /// A variable violates its declared bounds.
    BoundsViolation {
        var: String,
        expected: String,
        found: String,
    },

    /// A refinement type predicate is not satisfied.
    RefinementViolation { type_name: String },

    /// Verification requires a license key.
    LicenseRequired,

    /// The license key is invalid or expired.
    LicenseInvalid { reason: String },

    /// The license plan doesn't include verification.
    LicenseInsufficientPlan { current: String },

    /// Z3 returned unknown (timeout or undecidable).
    SolverUnknown,

    /// Z3 initialization or internal error.
    SolverError { message: String },
}

/// A counter-example showing why verification failed.
#[derive(Debug, Clone)]
pub struct CounterExample {
    /// Variable assignments that make the assertion false.
    pub assignments: Vec<(String, String)>,
}

impl fmt::Display for CounterExample {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (var, val) in &self.assignments {
            write!(f, "{} = {}", var, val)?;
            if self.assignments.len() > 1 {
                write!(f, ", ")?;
            }
        }
        Ok(())
    }
}

impl fmt::Display for VerificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            VerificationErrorKind::ContradictoryAssertion => {
                writeln!(f, "This assertion can never be true.")?;
                writeln!(f)?;
                writeln!(f, "{}", self.explanation)?;
                if let Some(ce) = &self.counterexample {
                    writeln!(f)?;
                    writeln!(f, "Counter-example: {}", ce)?;
                }
            }
            VerificationErrorKind::BoundsViolation { var, expected, found } => {
                writeln!(f, "Value '{}' violates its constraint.", var)?;
                writeln!(f)?;
                writeln!(f, "Expected: {}", expected)?;
                writeln!(f, "But found possible value: {}", found)?;
            }
            VerificationErrorKind::RefinementViolation { type_name } => {
                writeln!(f, "Value does not satisfy refinement type '{}'.", type_name)?;
                writeln!(f)?;
                writeln!(f, "{}", self.explanation)?;
            }
            VerificationErrorKind::LicenseRequired => {
                writeln!(f, "Verification requires a license key.")?;
                writeln!(f)?;
                writeln!(f, "Use --license <key> or set the LOGOS_LICENSE environment variable.")?;
                writeln!(f, "Get a license at https://logicaffeine.com/pricing")?;
            }
            VerificationErrorKind::LicenseInvalid { reason } => {
                writeln!(f, "License validation failed: {}", reason)?;
            }
            VerificationErrorKind::LicenseInsufficientPlan { current } => {
                writeln!(f, "Verification requires Pro, Premium, Lifetime, or Enterprise plan.")?;
                writeln!(f)?;
                writeln!(f, "Current plan: {}", current)?;
                writeln!(f, "Upgrade at https://logicaffeine.com/pricing")?;
            }
            VerificationErrorKind::SolverUnknown => {
                writeln!(f, "The solver could not determine if the assertion is valid.")?;
                writeln!(f)?;
                writeln!(f, "This may be due to complexity or timeout.")?;
            }
            VerificationErrorKind::SolverError { message } => {
                writeln!(f, "Solver error: {}", message)?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for VerificationError {}

impl VerificationError {
    /// Create a license required error.
    pub fn license_required() -> Self {
        Self {
            kind: VerificationErrorKind::LicenseRequired,
            span: None,
            explanation: String::new(),
            counterexample: None,
        }
    }

    /// Create a license invalid error.
    pub fn license_invalid(reason: impl Into<String>) -> Self {
        Self {
            kind: VerificationErrorKind::LicenseInvalid {
                reason: reason.into(),
            },
            span: None,
            explanation: String::new(),
            counterexample: None,
        }
    }

    /// Create an insufficient plan error.
    pub fn insufficient_plan(current: impl Into<String>) -> Self {
        Self {
            kind: VerificationErrorKind::LicenseInsufficientPlan {
                current: current.into(),
            },
            span: None,
            explanation: String::new(),
            counterexample: None,
        }
    }

    /// Create a contradictory assertion error.
    pub fn contradiction(explanation: impl Into<String>, counterexample: Option<CounterExample>) -> Self {
        Self {
            kind: VerificationErrorKind::ContradictoryAssertion,
            span: None,
            explanation: explanation.into(),
            counterexample,
        }
    }

    /// Create a bounds violation error.
    pub fn bounds_violation(
        var: impl Into<String>,
        expected: impl Into<String>,
        found: impl Into<String>,
    ) -> Self {
        Self {
            kind: VerificationErrorKind::BoundsViolation {
                var: var.into(),
                expected: expected.into(),
                found: found.into(),
            },
            span: None,
            explanation: String::new(),
            counterexample: None,
        }
    }

    /// Create a solver unknown error.
    pub fn solver_unknown() -> Self {
        Self {
            kind: VerificationErrorKind::SolverUnknown,
            span: None,
            explanation: String::new(),
            counterexample: None,
        }
    }

    /// Create a solver error.
    pub fn solver_error(message: impl Into<String>) -> Self {
        Self {
            kind: VerificationErrorKind::SolverError {
                message: message.into(),
            },
            span: None,
            explanation: String::new(),
            counterexample: None,
        }
    }

    /// Set the span for this error.
    pub fn with_span(mut self, start: usize, end: usize) -> Self {
        self.span = Some((start, end));
        self
    }
}
