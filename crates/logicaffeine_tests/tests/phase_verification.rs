//! Phase 42: Z3 Static Verification Tests
//!
//! Tests for the Z3-based static verification system.
//! Requires the `verification` feature to be enabled.

#![cfg(feature = "verification")]

use logos_verification::{LicensePlan, Verifier};

#[test]
fn test_verifier_tautology() {
    let verifier = Verifier::new();
    assert!(verifier.check_bool(true).is_ok());
}

#[test]
fn test_verifier_contradiction() {
    let verifier = Verifier::new();
    let result = verifier.check_bool(false);
    assert!(result.is_err());
}

#[test]
fn test_integer_greater_than_valid() {
    let verifier = Verifier::new();
    // 10 > 5 is valid
    assert!(verifier.check_int_greater_than(10, 5).is_ok());
}

#[test]
fn test_integer_greater_than_invalid() {
    let verifier = Verifier::new();
    // 3 > 5 is not valid
    let result = verifier.check_int_greater_than(3, 5);
    assert!(result.is_err());
}

#[test]
fn test_integer_less_than_valid() {
    let verifier = Verifier::new();
    // 3 < 5 is valid
    assert!(verifier.check_int_less_than(3, 5).is_ok());
}

#[test]
fn test_integer_less_than_invalid() {
    let verifier = Verifier::new();
    // 10 < 5 is not valid
    let result = verifier.check_int_less_than(10, 5);
    assert!(result.is_err());
}

#[test]
fn test_integer_equals_valid() {
    let verifier = Verifier::new();
    // 42 == 42 is valid
    assert!(verifier.check_int_equals(42, 42).is_ok());
}

#[test]
fn test_integer_equals_invalid() {
    let verifier = Verifier::new();
    // 1 == 2 is not valid
    let result = verifier.check_int_equals(1, 2);
    assert!(result.is_err());
}

#[test]
fn test_license_plan_verification_access() {
    // Pro, Premium, Lifetime, Enterprise can verify
    assert!(LicensePlan::Pro.can_verify());
    assert!(LicensePlan::Premium.can_verify());
    assert!(LicensePlan::Lifetime.can_verify());
    assert!(LicensePlan::Enterprise.can_verify());

    // Free, Supporter cannot verify
    assert!(!LicensePlan::None.can_verify());
    assert!(!LicensePlan::Free.can_verify());
    assert!(!LicensePlan::Supporter.can_verify());
}

#[test]
fn test_edge_cases() {
    let verifier = Verifier::new();

    // Boundary: 5 > 5 is not valid
    assert!(verifier.check_int_greater_than(5, 5).is_err());

    // Boundary: 5 < 5 is not valid
    assert!(verifier.check_int_less_than(5, 5).is_err());

    // Negative numbers
    assert!(verifier.check_int_greater_than(-1, -5).is_ok());
    assert!(verifier.check_int_less_than(-10, -5).is_ok());

    // Zero
    assert!(verifier.check_int_equals(0, 0).is_ok());
    assert!(verifier.check_int_greater_than(0, -1).is_ok());
}
