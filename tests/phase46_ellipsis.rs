//! Phase 46: Generalized Ellipsis (Template-Based Gapping)
//!
//! Tests for template-guided reconstruction of gapped clauses.
//! The current parse_gapped_clause() hardcodes Agent + Theme roles.
//! This phase generalizes to support ditransitives, PPs, and modifier override.

use logos::compile;

// ═══════════════════════════════════════════════════════════════════
// DITRANSITIVE GAPPING
// ═══════════════════════════════════════════════════════════════════

#[test]
fn ditransitive_gapping_theme_only() {
    // "John gave Mary a book, and Sue a pen."
    // Sue=Agent, Mary=Recipient (preserved), Pen=Theme (new)
    let output = compile("John gave Mary a book, and Sue a pen.").unwrap();
    assert!(
        output.matches("Give").count() >= 2,
        "Should have two Give events. Got: {}",
        output
    );
}

#[test]
fn ditransitive_gapping_both_args() {
    // "John gave Mary a book, and Sue Bob a pen."
    // Sue=Agent, Bob=Recipient (new), Pen=Theme (new)
    let output = compile("John gave Mary a book, and Sue Bob a pen.").unwrap();
    assert!(
        output.matches("Give").count() >= 2,
        "Should have two Give events. Got: {}",
        output
    );
}

// ═══════════════════════════════════════════════════════════════════
// PP GAPPING
// ═══════════════════════════════════════════════════════════════════

#[test]
fn pp_gapping_goal() {
    // "John walked to the park, and Mary to the school."
    let output = compile("John walked to the park, and Mary to the school.").unwrap();
    assert!(
        output.matches("Walk").count() >= 2,
        "Should have two Walk events. Got: {}",
        output
    );
}

#[test]
fn pp_gapping_with_np_and_pp() {
    // "John put the book on the table, and Mary the pen."
    // Mary=Agent, Pen=Theme (new), Table=Location (preserved)
    let output = compile("John put the book on the table, and Mary the pen.").unwrap();
    assert!(
        output.matches("Put").count() >= 2,
        "Should have two Put events. Got: {}",
        output
    );
}

// ═══════════════════════════════════════════════════════════════════
// MODIFIER OVERRIDE (Contrastive Gapping)
// ═══════════════════════════════════════════════════════════════════

#[test]
fn temporal_modifier_override() {
    // "John ran yesterday, and Mary today."
    // Mary gets new event with Time(e2, Today), NOT Time(e2, Yesterday)
    let output = compile("John ran yesterday, and Mary today.").unwrap();
    assert!(
        output.matches("Run").count() >= 2,
        "Should have two Run events. Got: {}",
        output
    );
}

// ═══════════════════════════════════════════════════════════════════
// REGRESSION TESTS
// ═══════════════════════════════════════════════════════════════════

#[test]
fn simple_transitive_gapping_preserved() {
    // Existing functionality: "John ate an apple, and Mary a banana."
    let output = compile("John ate an apple, and Mary a banana.").unwrap();
    assert!(
        output.matches("Eat").count() >= 2,
        "Should have two Eat events. Got: {}",
        output
    );
}

#[test]
fn intransitive_gapping_preserved() {
    // "John ran, and Mary."
    let output = compile("John ran, and Mary.").unwrap();
    assert!(
        output.matches("Run").count() >= 2,
        "Should have two Run events. Got: {}",
        output
    );
}
