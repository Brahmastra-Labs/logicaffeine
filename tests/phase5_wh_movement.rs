//! Phase 5: Long-Distance Dependencies (Wh-Movement)
//!
//! These tests verify handling of unbounded wh-movement where the gap
//! can be extracted across multiple clause boundaries.
//!
//! TDD Approach: These tests are written FIRST and should FAIL until
//! the implementation is complete.

use logos::compile;

// ═══════════════════════════════════════════════════════════════════
// WH-MOVEMENT TESTS
// ═══════════════════════════════════════════════════════════════════

#[test]
fn wh_simple_still_works() {
    let output = compile("Who did John see?").unwrap();

    eprintln!("DEBUG: Simple wh output: {}", output);

    assert!(
        output.contains("x") || output.contains("?"),
        "Should have wh-variable: got '{}'",
        output
    );
    assert!(
        output.contains("J") || output.contains("John"),
        "Should have John: got '{}'",
        output
    );
}

#[test]
fn wh_embedded_clause() {
    let output = compile("Who did John say Mary loves?").unwrap();

    eprintln!("DEBUG: Embedded wh output: {}", output);

    // Embedded clause should have Love with the gap variable
    assert!(
        output.contains("Love") || output.contains("L("),
        "Should contain Love predicate: got '{}'",
        output
    );

    // The gap variable x should be argument of Love
    assert!(
        output.contains("Theme(e, x)") || output.contains("Love(M, x)") || output.contains("L(M, x)"),
        "Gap should fill object of embedded 'loves': got '{}'",
        output
    );

    // Say should take the embedded clause as argument, NOT conjunction
    // The bug produces "S(J) ∧ ∃e(Love..." - conjunction between Say and Love
    // Correct would be "S(J, [Love...])" - Love as argument of Say
    let has_say_with_one_arg = output.contains("S(J)") || output.contains("Say(J)");
    let has_conjunction_after_say = output.contains("S(J) ∧") || output.contains("Say(J) ∧");

    assert!(
        !has_conjunction_after_say,
        "Bug detected: Say and Love are conjoined instead of Love being argument of Say: got '{}'",
        output
    );

    // Say should have two arguments: subject and the embedded clause
    assert!(
        output.contains("S(J, ") || output.contains("Say(J, ") ||
        output.contains("Say(J, [") || output.contains("S(J, [") ||
        output.contains("Say(John, ") || output.contains("Say(John, ["),
        "Say should have embedded clause as second argument: got '{}'",
        output
    );
}

#[test]
fn wh_double_embedding() {
    let output = compile("Who did John think Mary said Bill saw?").unwrap();

    eprintln!("DEBUG: Double embedding output: {}", output);

    // The gap should reach the deepest verb "saw"
    assert!(
        output.contains("Saw") || output.contains("S(") || output.contains("See"),
        "Should contain See/Saw predicate: got '{}'",
        output
    );
}

#[test]
fn wh_subject_extraction() {
    let output = compile("Who saw Mary?").unwrap();

    eprintln!("DEBUG: Subject extraction output: {}", output);

    assert!(
        output.contains("x") || output.contains("?") || output.contains("Saw"),
        "Should have wh-binding or Saw predicate: got '{}'",
        output
    );
}

#[test]
fn non_wh_unchanged() {
    let output = compile("John said Mary ran.").unwrap();

    eprintln!("DEBUG: Non-wh output: {}", output);

    assert!(output.len() > 5, "Should produce output: got '{}'", output);
}
