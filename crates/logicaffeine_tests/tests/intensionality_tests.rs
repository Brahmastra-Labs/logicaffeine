//! Phase 2: Intensionality Tests (Types & Mereology)
//!
//! These tests verify the implementation of de re / de dicto readings
//! for opaque verbs (seek, want, believe, etc.) and mass noun handling.
//!
//! TDD Approach: These tests are written FIRST and should FAIL until
//! the implementation is complete.

use logicaffeine_language::compile;
use logicaffeine_language::compile_all_scopes;

// ═══════════════════════════════════════════════════════════════════
// DE RE / DE DICTO AMBIGUITY TESTS
// ═══════════════════════════════════════════════════════════════════

#[test]
fn seek_unicorn_produces_two_readings() {
    let readings = compile_all_scopes("John seeks a unicorn.");
    assert!(readings.is_ok(), "Should parse successfully");
    let readings = readings.unwrap();

    eprintln!("DEBUG: Got {} readings: {:?}", readings.len(), readings);

    assert_eq!(
        readings.len(), 2,
        "Should produce exactly 2 readings (de re and de dicto), got {}: {:?}",
        readings.len(), readings
    );

    // De Re: ∃x(U(x) ∧ Seek(j, x)) - a specific unicorn exists
    // Note: "U" is the abbreviated form of "Unicorn" from the symbol registry
    let has_de_re = readings.iter().any(|r| {
        r.contains("∃x") && r.contains("Seek") && r.contains("Theme(e, x)")
    });

    // De Dicto: Seek(j, ^U) - seeking the concept (Montague up-arrow)
    let has_de_dicto = readings.iter().any(|r| {
        r.contains("Seek") && r.contains("^")
    });

    assert!(has_de_re, "Should have de re reading (existential over unicorn). Got: {:?}", readings);
    assert!(has_de_dicto, "Should have de dicto reading (intension marker ^). Got: {:?}", readings);
}

#[test]
fn fear_produces_two_readings() {
    // Using 'fear' instead of 'want' because 'want' is a control verb
    // which gets different parsing treatment
    let readings = compile_all_scopes("Mary fears a monster.");
    assert!(readings.is_ok(), "Should parse successfully");
    let readings = readings.unwrap();

    assert!(
        readings.len() >= 2,
        "Opaque verb 'fear' should produce multiple readings, got {}: {:?}",
        readings.len(), readings
    );
}

#[test]
fn believe_exists_produces_two_readings() {
    let readings = compile_all_scopes("John believes a spy exists.");
    assert!(readings.is_ok(), "Should parse successfully");
    let readings = readings.unwrap();

    assert!(
        readings.len() >= 2,
        "Opaque verb 'believe' should produce multiple readings"
    );
}

#[test]
fn need_doctor_produces_two_readings() {
    let readings = compile_all_scopes("Mary needs a doctor.");
    assert!(readings.is_ok(), "Should parse successfully");
    let readings = readings.unwrap();

    assert!(
        readings.len() >= 2,
        "Opaque verb 'need' should produce multiple readings"
    );
}

// ═══════════════════════════════════════════════════════════════════
// NON-OPAQUE VERBS (EXTENSIONAL ONLY)
// ═══════════════════════════════════════════════════════════════════

#[test]
fn kick_ball_produces_single_reading() {
    let readings = compile_all_scopes("John kicks a ball.");
    assert!(readings.is_ok(), "Should parse successfully");
    let readings = readings.unwrap();

    // Non-opaque verb: only de re reading (no intensional ambiguity)
    assert_eq!(
        readings.len(), 1,
        "Non-opaque verb should have single extensional reading, got {}: {:?}",
        readings.len(), readings
    );
}

#[test]
fn love_produces_single_reading() {
    let readings = compile_all_scopes("John loves a woman.");
    assert!(readings.is_ok(), "Should parse successfully");
    let readings = readings.unwrap();

    // 'love' is not opaque - only extensional reading
    assert_eq!(
        readings.len(), 1,
        "Non-opaque verb 'love' should have single reading"
    );
}

// ═══════════════════════════════════════════════════════════════════
// MASS NOUN TESTS
// ═══════════════════════════════════════════════════════════════════

#[test]
fn mass_noun_water_uses_sigma() {
    let output = compile("John drank water.");
    assert!(output.is_ok(), "Should parse successfully");
    let output = output.unwrap();

    // Mass nouns should use sigma (part-of) rather than existential quantification
    // Should NOT have ∃x(Water(x) ∧ ...) pattern
    let has_existential_water = output.contains("∃") && output.contains("Water(");

    assert!(
        !has_existential_water || output.contains("σ"),
        "Mass noun 'water' should not be quantified like count noun. Got: {}",
        output
    );
}

#[test]
fn mass_noun_milk_uses_sigma() {
    let output = compile("Mary drank milk.");
    assert!(output.is_ok(), "Should parse successfully");
    let output = output.unwrap();

    let has_existential_milk = output.contains("∃") && output.contains("Milk(");

    assert!(
        !has_existential_milk || output.contains("σ"),
        "Mass noun 'milk' should not be existentially quantified. Got: {}",
        output
    );
}

// ═══════════════════════════════════════════════════════════════════
// INTENSION OPERATOR OUTPUT TESTS
// ═══════════════════════════════════════════════════════════════════

#[test]
fn de_dicto_uses_up_arrow_notation() {
    let readings = compile_all_scopes("John seeks a unicorn.");
    assert!(readings.is_ok());
    let readings = readings.unwrap();

    // At least one reading should use ^ notation
    let has_up_arrow = readings.iter().any(|r| r.contains("^"));

    assert!(
        has_up_arrow,
        "De dicto reading should use ^ (up-arrow) notation for intension. Got: {:?}",
        readings
    );
}

// ═══════════════════════════════════════════════════════════════════
// EDGE CASES
// ═══════════════════════════════════════════════════════════════════

#[test]
fn definite_with_opaque_is_de_re_only() {
    let readings = compile_all_scopes("John seeks the unicorn.");
    assert!(readings.is_ok());
    let readings = readings.unwrap();

    // Definite descriptions with opaque verbs are typically de re only
    // (the specific unicorn that exists and is known)
    assert_eq!(
        readings.len(), 1,
        "Definite NP with opaque verb should be de re only"
    );
}

#[test]
fn proper_name_with_opaque_is_de_re_only() {
    let readings = compile_all_scopes("John seeks Mary.");
    assert!(readings.is_ok());
    let readings = readings.unwrap();

    // Proper names are always de re (rigid designators)
    assert_eq!(
        readings.len(), 1,
        "Proper name with opaque verb should be de re only"
    );
}
