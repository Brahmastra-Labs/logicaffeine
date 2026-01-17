#![cfg_attr(docsrs, feature(doc_cfg))]

//! # logicaffeine_verify
//!
//! Z3-based static verification for Logicaffeine programs.
//!
//! ## Quick Start
//!
//! ```ignore
//! use logicaffeine_verify::{VerificationSession, VerifyExpr, VerifyType};
//!
//! let mut session = VerificationSession::new();
//! session.declare("x", VerifyType::Int);
//! session.assume(&VerifyExpr::eq(VerifyExpr::var("x"), VerifyExpr::int(10)));
//! assert!(session.verify(&VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(5))).is_ok());
//! ```
//!
//! ## Architecture
//!
//! The crate is organized around the **Tarski Invariant**: the verification IR
//! has no dependency on the main AST. This enables:
//!
//! - Clean separation between parsing and verification
//! - Reusable verification logic across frontends
//! - Avoidance of circular crate dependencies
//!
//! ## Encoding Strategy
//!
//! | Logicaffeine Type | Z3 Encoding |
//! |-------------------|-------------|
//! | `Int` | Z3 `IntSort` |
//! | `Bool` | Z3 `BoolSort` |
//! | `Object` | Uninterpreted sort |
//! | Predicates | Uninterpreted functions returning `Bool` |
//! | Modals/Temporals | Uninterpreted functions (structural reasoning) |
//!
//! ## Modules
//!
//! - [`ir`] - Verification intermediate representation
//! - [`solver`] - Z3 wrapper and verification sessions
//! - [`error`] - Error types with Socratic explanations
//! - [`license`] - Stripe-based license validation
//!
//! ## License Requirement
//!
//! Verification requires Pro, Premium, Lifetime, or Enterprise plan.
//! License keys are Stripe subscription IDs (`sub_*` format).

pub mod error;
pub mod ir;
pub mod license;
pub mod solver;

pub use error::{VerificationError, VerificationErrorKind, VerificationResult};
pub use ir::{VerifyExpr, VerifyOp, VerifyType};
pub use license::{LicensePlan, LicenseValidator};
pub use solver::{Verifier, VerificationSession};
