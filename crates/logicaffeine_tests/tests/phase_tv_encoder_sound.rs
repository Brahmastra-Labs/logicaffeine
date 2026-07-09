//! Phase 0 — encoder soundness (the meta-oracle).
//!
//! The translation validator's trust rests on one property: the symbolic LOGOS encoder
//! must agree with the tree-walking interpreter (the de-facto semantics). Every test
//! here runs a program through *both* and proves with Z3 that they produce the same
//! observable behavior. Without this anchor, a buggy encoder could "prove" two wrong
//! programs equivalent — so this is tested first, before anything is validated against
//! it (`work/PE_IMPROVE.md §4.2`). Requires the `verification` feature (Z3).

#![cfg(feature = "verification")]

use logicaffeine_tv::{check_encoder_sound, SoundnessReport};

/// Assert the encoder provably matches the interpreter on `src`.
fn agree(src: &str) {
    match check_encoder_sound(src) {
        SoundnessReport::Agrees => {}
        other => panic!("encoder/interpreter disagree on:\n{src}\n=> {other:?}"),
    }
}

/// Assert the program is soundly excluded (out of the Verifiable Core) — never silently
/// reported as agreement.
fn punted(src: &str) {
    match check_encoder_sound(src) {
        SoundnessReport::Unsupported { .. } => {}
        other => panic!("expected sound punt (Unsupported) on:\n{src}\n=> {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Integer arithmetic (modeled as 64-bit bitvectors — exact i64 semantics).
// ---------------------------------------------------------------------------

#[test]
fn arith_precedence() {
    agree("## Main\nLet x be 2 + 3 * 4.\nShow x.");
}

#[test]
fn arith_subtraction() {
    agree("## Main\nLet x be 10.\nLet y be 4.\nShow x - y.");
}

#[test]
fn arith_multiplication() {
    agree("## Main\nShow 7 * 6.");
}

#[test]
fn arith_negative_result() {
    agree("## Main\nShow 10 - 20.");
}

#[test]
fn arith_nested() {
    agree("## Main\nLet a be 3.\nLet b be 4.\nLet c be 5.\nShow (a + b) * c - a.");
}

// ---------------------------------------------------------------------------
// Mutation and sequencing.
// ---------------------------------------------------------------------------

#[test]
fn set_mutation() {
    agree("## Main\nLet x be 5.\nSet x to x + 1.\nShow x.");
}

#[test]
fn set_then_read_in_arith() {
    agree("## Main\nLet x be 2.\nSet x to x * x.\nSet x to x + 1.\nShow x.");
}

#[test]
fn multiple_outputs_in_order() {
    agree("## Main\nShow 1.\nShow 2.\nShow 3.");
}

// ---------------------------------------------------------------------------
// Comparisons (signed) and booleans.
// ---------------------------------------------------------------------------

#[test]
fn compare_less_true() {
    agree("## Main\nShow 3 < 5.");
}

#[test]
fn compare_less_false() {
    agree("## Main\nShow 5 < 3.");
}

#[test]
fn compare_gte_boundary() {
    agree("## Main\nLet x be 7.\nShow x >= 7.");
}

#[test]
fn compare_equality_ints() {
    agree("## Main\nShow 4 == 4.");
}

#[test]
fn compare_inequality_ints() {
    agree("## Main\nShow 4 != 5.");
}

#[test]
fn bool_and() {
    agree("## Main\nLet a be true.\nLet b be false.\nShow a and b.");
}

#[test]
fn bool_or() {
    agree("## Main\nShow true or false.");
}

#[test]
fn bool_not() {
    agree("## Main\nShow not true.");
}

// ---------------------------------------------------------------------------
// Sound punt: constructs outside the Phase 0 fragment must be excluded, never
// reported as agreement (property 4).
// ---------------------------------------------------------------------------

#[test]
fn punt_collections() {
    punted("## Main\nLet xs be [1, 2, 3].\nShow length of xs.");
}

// ---------------------------------------------------------------------------
// Division and modulo (bvsdiv/bvsrem) — including the observable error model.
// ---------------------------------------------------------------------------

#[test]
fn division_exact() {
    agree("## Main\nShow 10 / 2.");
}

#[test]
fn division_truncates_toward_zero() {
    agree("## Main\nShow 7 / 2.");
}

#[test]
fn division_negative_truncates_toward_zero() {
    // -7 / 2 == -3 (toward zero), matching Rust i64 and bvsdiv.
    agree("## Main\nLet a be 0 - 7.\nShow a / 2.");
}

#[test]
fn modulo_basic() {
    agree("## Main\nShow 17 % 5.");
}

#[test]
fn modulo_negative_sign_of_dividend() {
    // -7 % 2 == -1 (sign of dividend), matching Rust i64 and bvsrem.
    agree("## Main\nLet a be 0 - 7.\nShow a % 2.");
}

#[test]
fn division_by_zero_is_an_observable_error() {
    // The interpreter raises "Division by zero"; the encoder must prove `errored`.
    agree("## Main\nShow 10 / 0.");
}

#[test]
fn modulo_by_zero_is_an_observable_error() {
    agree("## Main\nShow 10 % 0.");
}
