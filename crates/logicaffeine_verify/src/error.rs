//! Verification error types with Socratic error messages.
//!
//! ## Philosophy
//!
//! Errors in this module follow the Socratic method: they guide users toward
//! understanding rather than simply reporting failures. Each error type includes:
//!
//! - A clear description of what went wrong
//! - Context about why it matters
//! - Guidance on how to address the issue
//! - When available, a concrete counterexample
//!
//! ## Error Categories
//!
//! | Category | Error Types | User Action |
//! |----------|-------------|-------------|
//! | Logic | `ContradictoryAssertion`, `BoundsViolation`, `RefinementViolation` | Fix the logical issue |
//! | License | `LicenseRequired`, `LicenseInvalid`, `LicenseInsufficientPlan` | Provide valid license |
//! | Solver | `SolverUnknown`, `SolverError` | Simplify or restructure |
//! | Termination | `TerminationViolation` | Add decreasing variant |

use std::fmt;

/// Result type for verification operations.
pub type VerificationResult<T = ()> = Result<T, VerificationError>;

/// A verification error with Socratic explanation.
///
/// Each error includes contextual information to help users understand
/// and fix the issue.
///
/// # Fields
///
/// - `kind`: Categorizes the error type
/// - `span`: Source location (byte offsets) where the error occurred
/// - `explanation`: Human-readable context explaining why verification failed
/// - `counterexample`: Concrete variable assignments that demonstrate the failure
#[derive(Debug)]
pub struct VerificationError {
    /// The category of verification error.
    pub kind: VerificationErrorKind,
    /// Optional source span as `(start, end)` byte offsets.
    pub span: Option<(usize, usize)>,
    /// Human-readable explanation of why verification failed.
    pub explanation: String,
    /// Concrete witness showing a failing case, when available.
    pub counterexample: Option<CounterExample>,
}

/// The category of verification error.
///
/// Each variant represents a distinct failure mode with specific remediation steps.
#[derive(Debug, Clone, PartialEq)]
pub enum VerificationErrorKind {
    /// An assertion that can never be true.
    ///
    /// Z3 proved that no interpretation satisfies the assertion.
    ///
    /// **Common causes:**
    /// - Conflicting constraints (e.g., `x > 5` and `x < 3`)
    /// - Logical contradictions in premises
    /// - Impossible refinement type predicates
    ///
    /// **Action:** Review your assumptions for conflicts.
    ContradictoryAssertion,

    /// A variable violates its declared bounds.
    ///
    /// The verifier found a possible value that falls outside the
    /// declared constraint.
    ///
    /// **Action:** Either tighten the constraint or add assumptions
    /// that rule out the violating value.
    BoundsViolation {
        /// The variable that violated its bounds.
        var: String,
        /// The constraint that was expected.
        expected: String,
        /// The value that violated the constraint.
        found: String,
    },

    /// A refinement type predicate is not satisfied.
    ///
    /// The value being assigned does not satisfy the predicate
    /// in the refinement type definition.
    ///
    /// **Action:** Ensure the value meets the refinement predicate,
    /// or add assumptions that constrain the value appropriately.
    RefinementViolation {
        /// The name of the refinement type.
        type_name: String,
    },

    /// Verification requires a license key.
    ///
    /// **Action:** Provide a license key via `--license <key>` or the
    /// `LOGOS_LICENSE` environment variable.
    LicenseRequired,

    /// The license key is invalid or expired.
    ///
    /// **Action:** Check that your license key is correct and active.
    LicenseInvalid {
        /// The reason the license was rejected.
        reason: String,
    },

    /// The license plan doesn't include verification.
    ///
    /// Verification requires Pro, Premium, Lifetime, or Enterprise plan.
    ///
    /// **Action:** Upgrade your plan at <https://logicaffeine.com/pricing>.
    LicenseInsufficientPlan {
        /// The user's current plan.
        current: String,
    },

    /// Z3 returned unknown (timeout or undecidable).
    ///
    /// The solver could not determine validity within the timeout period,
    /// or the problem is undecidable.
    ///
    /// **Action:** Simplify the assertion or add more constraining assumptions.
    SolverUnknown,

    /// Z3 initialization or internal error.
    ///
    /// An unexpected error occurred in the Z3 solver.
    ///
    /// **Action:** Check that Z3 is properly installed and configured.
    SolverError {
        /// The error message from Z3.
        message: String,
    },

    /// Loop termination cannot be proven.
    ///
    /// The verifier could not prove that the loop variant strictly decreases
    /// on each iteration while remaining non-negative.
    ///
    /// **Action:** Ensure your variant expression decreases by at least 1
    /// on each iteration and has a lower bound.
    TerminationViolation {
        /// The variant expression that should decrease.
        variant: String,
        /// Why termination could not be proven.
        reason: String,
    },
}

/// A counterexample showing concrete values that falsify an assertion.
///
/// When Z3 finds that an assertion is not valid, it produces a model
/// (set of variable assignments) that makes the negation of the assertion true.
/// This counterexample helps users understand exactly why verification failed.
///
/// # Example Interpretation
///
/// If verifying `x > 5` fails with counterexample `x = 3`, this means:
/// - The solver found that `x = 3` is a possible value
/// - With `x = 3`, the assertion `x > 5` is false
/// - Therefore the assertion is not universally valid
#[derive(Debug, Clone)]
pub struct CounterExample {
    /// Variable assignments that make the assertion false.
    ///
    /// Each tuple contains `(variable_name, value_as_string)`.
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
            VerificationErrorKind::TerminationViolation { variant, reason } => {
                writeln!(f, "Cannot prove loop terminates.")?;
                writeln!(f)?;
                writeln!(f, "Variant '{}' does not strictly decrease: {}", variant, reason)?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for VerificationError {}

impl VerificationError {
    // ---- License Errors ----

    /// Create a license required error.
    ///
    /// Use when verification is attempted without providing a license key.
    pub fn license_required() -> Self {
        Self {
            kind: VerificationErrorKind::LicenseRequired,
            span: None,
            explanation: String::new(),
            counterexample: None,
        }
    }

    /// Create a license invalid error.
    ///
    /// Use when the provided license key fails validation.
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
    ///
    /// Use when the license is valid but the plan doesn't include verification.
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

    // ---- Logic Errors ----

    /// Create a contradictory assertion error.
    ///
    /// Use when Z3 proves the negation of an assertion is satisfiable.
    pub fn contradiction(explanation: impl Into<String>, counterexample: Option<CounterExample>) -> Self {
        Self {
            kind: VerificationErrorKind::ContradictoryAssertion,
            span: None,
            explanation: explanation.into(),
            counterexample,
        }
    }

    /// Create a bounds violation error.
    ///
    /// Use when a variable's value falls outside its declared constraint.
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

    /// Create a refinement type violation error.
    ///
    /// Use when a value does not satisfy its refinement type predicate.
    pub fn refinement_violation(type_name: impl Into<String>, explanation: impl Into<String>) -> Self {
        Self {
            kind: VerificationErrorKind::RefinementViolation {
                type_name: type_name.into(),
            },
            span: None,
            explanation: explanation.into(),
            counterexample: None,
        }
    }

    // ---- Solver Errors ----

    /// Create a solver unknown error.
    ///
    /// Use when Z3 cannot determine satisfiability (timeout or undecidable).
    pub fn solver_unknown() -> Self {
        Self {
            kind: VerificationErrorKind::SolverUnknown,
            span: None,
            explanation: String::new(),
            counterexample: None,
        }
    }

    /// Create a solver error.
    ///
    /// Use when Z3 encounters an internal error or configuration issue.
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

    // ---- Termination Errors ----

    /// Create a termination violation error.
    ///
    /// Use when loop termination cannot be proven via the variant expression.
    pub fn termination_violation(variant: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            kind: VerificationErrorKind::TerminationViolation {
                variant: variant.into(),
                reason: reason.into(),
            },
            span: None,
            explanation: String::new(),
            counterexample: None,
        }
    }

    // ---- Builder Methods ----

    /// Set the source span for this error.
    ///
    /// Spans are byte offsets `(start, end)` into the source text.
    pub fn with_span(mut self, start: usize, end: usize) -> Self {
        self.span = Some((start, end));
        self
    }
}
