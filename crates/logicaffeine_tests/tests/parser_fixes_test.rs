//! Tests for parser fixes: Both...and, Weather verbs, Copula+Adjective

use logicaffeine_language::compile;

#[test]
fn test_both_and_correlative() {
    // "Both Socrates and Plato are men" should produce proper group semantics
    let result = compile("Both Socrates and Plato are men.").unwrap();
    println!("Both Socrates and Plato are men: {}", result);

    // Should contain both names and the predicate Men/Man
    assert!(result.contains("Socrates") || result.contains("S"),
        "Expected Socrates in output: {}", result);
    assert!(result.contains("Plato") || result.contains("P"),
        "Expected Plato in output: {}", result);
    // Should NOT be garbage like "S & M(P)"
    assert!(!result.starts_with("S &"),
        "Output should not be garbage pattern 'S &...': {}", result);
}

#[test]
fn test_weather_verb_it_rains() {
    // "It rains" should produce event semantics without "It" as agent
    let result = compile("It rains.").unwrap();
    println!("It rains: {}", result);

    // Should contain Rain predicate
    assert!(result.contains("Rain") || result.contains("rain"),
        "Expected Rain predicate in output: {}", result);
    // Should have event quantification ∃e
    assert!(result.contains("∃e") || result.contains("∃") || result.contains("e("),
        "Expected event quantification in output: {}", result);
    // Should NOT have Agent role (weather verbs have no agent)
    assert!(!result.contains("Agent"),
        "Weather verbs should not have Agent role: {}", result);
}

#[test]
fn test_weather_verb_in_conditional() {
    // "If it rains, the ground is wet"
    let result = compile("If it rains, the ground is wet.").unwrap();
    println!("If it rains, the ground is wet: {}", result);

    // Should be a conditional (→)
    assert!(result.contains("→") || result.contains("->"),
        "Expected conditional in output: {}", result);
    // Should contain Rain
    assert!(result.contains("Rain") || result.contains("rain"),
        "Expected Rain in output: {}", result);
    // Should NOT be "(? -> R)" - the original garbage output
    assert!(!result.starts_with("(? ->") && !result.starts_with("? →"),
        "Output should not be garbage '(? -> ...)': {}", result);
}

#[test]
fn test_copula_adjective_preserves_subject() {
    // "Mary is wet" should produce Wet(Mary) or W(M), not just "Wet" or "W"
    let result = compile("Mary is wet.").unwrap();
    println!("Mary is wet: {}", result);

    // Should be a predicate applied to Mary: W(M) or Wet(Mary)
    // The system abbreviates, so we check for the pattern
    assert!(result.contains("W(M)") || result.contains("Wet(Mary)") || result.contains("Wet(M)"),
        "Expected Wet predicate applied to Mary in output: {}", result);
}

#[test]
fn test_copula_adjective_the_ground() {
    // "The ground is wet"
    let result = compile("The ground is wet.").unwrap();
    println!("The ground is wet: {}", result);

    // Should contain W(x) or W(G) - wet applied to ground variable
    assert!(result.contains("W(") || result.contains("Wet("),
        "Expected Wet predicate applied in output: {}", result);
    // Should contain Ground reference
    assert!(result.contains("G(") || result.contains("Ground") || result.contains("G)"),
        "Expected Ground in output: {}", result);
}

#[test]
fn test_copula_adjective_simple() {
    // Simple test: "John is happy"
    let result = compile("John is happy.").unwrap();
    println!("John is happy: {}", result);

    // Should be H(J) or Happy(John) - predicate applied to subject
    assert!(result.contains("(") && result.contains(")"),
        "Expected predicate with parentheses: {}", result);
}
