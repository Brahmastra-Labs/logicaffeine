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
//! # License Requirement
//!
//! Verification is a premium feature. License keys are Stripe subscription IDs
//! (`sub_*` format) validated against `api.logicaffeine.com/validate`.

pub mod error;
pub mod license;
pub mod solver;

pub use error::{VerificationError, VerificationErrorKind, VerificationResult};
pub use license::{LicensePlan, LicenseValidator};
pub use solver::Verifier;
