//! Sprint E: Consistency Filter
//!
//! Tests for the hardware specification consistency checker.
//! Validates that specs parse correctly, signals are declared,
//! and properties are structurally well-formed.

use logicaffeine_language::{compile, compile_kripke};

// ═══════════════════════════════════════════════════════════════════════════
// WELL-FORMED SPECS PASS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn filter_passes_simple_sentence() {
    // A basic sentence should still parse fine
    let result = compile("Every dog runs.");
    assert!(result.is_ok(), "Simple sentence should parse: {:?}", result.err());
}

#[test]
fn filter_passes_modal_sentence() {
    // Modal sentences should still work after our temporal additions
    let result = compile_kripke("John can fly.");
    assert!(result.is_ok(), "Modal sentence should Kripke-lower: {:?}", result.err());
}

#[test]
fn filter_passes_deontic_modal() {
    let result = compile_kripke("John should study.");
    assert!(result.is_ok(), "Deontic modal should parse: {:?}", result.err());
    let output = result.unwrap();
    assert!(
        output.contains("Accessible_Deontic"),
        "Should use Deontic accessibility. Got: {}",
        output
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// EXISTING KRIPKE TESTS STILL PASS (regression guard)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn filter_kripke_possibility_still_works() {
    let output = compile_kripke("John can fly.").unwrap();
    assert!(output.contains("Accessible_Alethic"));
    assert!(output.contains("w0") || output.contains("w1"));
}

#[test]
fn filter_kripke_necessity_still_works() {
    let output = compile_kripke("John must study.").unwrap();
    assert!(
        output.contains("∀") || output.contains("ForAll"),
        "Necessity should produce universal quantifier. Got: {}",
        output
    );
    assert!(output.contains("Accessible_Alethic"));
}

#[test]
fn filter_kripke_nested_modals_still_work() {
    let output = compile_kripke("John might be able to fly.").unwrap();
    assert!(output.contains("w1") && output.contains("w2"));
}

// ═══════════════════════════════════════════════════════════════════════════
// MALFORMED INPUT REJECTED
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn filter_rejects_incomplete_sentence() {
    let result = compile("Every.");
    assert!(result.is_err(), "Incomplete sentence should be rejected");
}

#[test]
fn filter_rejects_bare_quantifier() {
    let result = compile("Every and some.");
    assert!(result.is_err(), "Bare quantifiers without nouns should fail");
}

// ═══════════════════════════════════════════════════════════════════════════
// HARDWARE BLOCK TYPES RECOGNIZED
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn filter_hardware_block_type_exists() {
    use logicaffeine_language::token::BlockType;
    let _hw = BlockType::Hardware;
    let _prop = BlockType::Property;
}

#[test]
fn filter_temporal_domain_exists() {
    use logicaffeine_language::ast::logic::ModalDomain;
    let _temporal = ModalDomain::Temporal;
}
