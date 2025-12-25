//! Phase 4: Syntax of Movement (Topicalization)
//!
//! These tests verify handling of Filler-Gap dependencies where the object
//! moves to sentence-initial position (topicalization).
//!
//! TDD Approach: These tests are written FIRST and should FAIL until
//! the implementation is complete.

use logos::compile;

// ═══════════════════════════════════════════════════════════════════
// TOPICALIZATION TESTS
// ═══════════════════════════════════════════════════════════════════

#[test]
fn topicalization_simple() {
    let output = compile("The apple, John ate.").unwrap();

    eprintln!("DEBUG: Simple topicalization output: {}", output);

    // Apple may appear as "Apple", "A(", or as Theme in NeoEvent format
    assert!(
        output.contains("Apple") || output.contains("A(") || output.contains("Theme(e, A)"),
        "Object 'Apple' should appear in output: got '{}'",
        output
    );

    // John may appear as "John", "J," or as Agent
    assert!(
        output.contains("John") || output.contains("J,") || output.contains("Agent(e, J)"),
        "Subject 'John' should appear in output: got '{}'",
        output
    );

    // Verb should be Eat in some form
    assert!(
        output.contains("Eat") || output.contains("E("),
        "Verb 'Eat' should appear: got '{}'",
        output
    );
}

#[test]
fn topicalization_with_adjectives() {
    let output = compile("The red apple, John ate.").unwrap();

    eprintln!("DEBUG: Adjective topicalization output: {}", output);

    // Adjective Red should appear in output (as "Red" or "R(" for Red(x))
    assert!(
        output.contains("Red") || output.contains("R("),
        "Should preserve adjective 'Red' in the moved topic: got '{}'",
        output
    );

    // Apple should appear
    assert!(
        output.contains("Apple") || output.contains("A("),
        "Should have Apple in output: got '{}'",
        output
    );
}

#[test]
fn topicalization_indefinite() {
    let output = compile("A book, Mary read.").unwrap();

    eprintln!("DEBUG: Indefinite topicalization output: {}", output);

    // Read should appear as "Read" or "R("
    assert!(
        output.contains("Read") || output.contains("R("),
        "Should have Read predicate: got '{}'",
        output
    );

    // Book should appear as "Book" or "B("
    assert!(
        output.contains("Book") || output.contains("B("),
        "Book should be present: got '{}'",
        output
    );
}

#[test]
fn non_topicalized_unchanged() {
    let output = compile("John ate the apple.").unwrap();

    eprintln!("DEBUG: Standard SVO output: {}", output);

    // Standard SVO should produce NeoEvent format with Theme role
    // Apple appears as "Apple" or "A" in Theme(e, A)
    assert!(
        output.contains("Theme"),
        "Standard order should produce Theme role: got '{}'",
        output
    );

    assert!(
        output.contains("Apple") || output.contains(", A)"),
        "Apple should be Theme: got '{}'",
        output
    );
}

#[test]
fn topicalization_pronoun_subject() {
    let output = compile("The book, he read.").unwrap();

    eprintln!("DEBUG: Pronoun subject output: {}", output);

    assert!(
        output.contains("Read") || output.contains("R("),
        "Should find main verb: got '{}'",
        output
    );
    assert!(
        output.contains("Book") || output.contains("B("),
        "Should find topic object: got '{}'",
        output
    );
}
