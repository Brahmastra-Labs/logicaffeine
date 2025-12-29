//! LOGOS Static Verification
//!
//! Z3-based static verification for LOGOS programs.
//! Requires a Pro, Premium, Lifetime, or Enterprise license.
//!
//! # Overview
//!
//! This crate provides compile-time verification of LOGOS assertions using
//! the Z3 SMT solver. It can detect contradictions, prove bounds, and
//! generate counter-examples when verification fails.
//!
//! # Architecture
//!
//! The verification system uses a lightweight IR (Intermediate Representation)
//! to avoid circular dependencies. The main `logos` crate translates its AST
//! into this IR before passing it to the verifier.
//!
//! **Smart Full Mapping Strategy:**
//! - `Int`, `Bool` → direct Z3 sorts
//! - `Object` → uninterpreted sort for entities
//! - Predicates, Modals, Temporals → `Apply` (uninterpreted functions)
//! - Z3 reasons structurally without semantic knowledge
//!
//! # License Requirement
//!
//! Verification is a premium feature. License keys are Stripe subscription IDs
//! (`sub_*` format) validated against `api.logicaffeine.com/validate`.

pub mod error;
pub mod ir;
pub mod license;
pub mod solver;

pub use error::{VerificationError, VerificationErrorKind, VerificationResult};
pub use ir::{VerifyExpr, VerifyOp, VerifyType};
pub use license::{LicensePlan, LicenseValidator};
pub use solver::{Verifier, VerificationSession};
