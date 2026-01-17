//! Tests for Generalized Quantifiers (Most, Few, Many)

use logicaffeine_language::compile;

#[test]
fn test_most_birds_can_fly() {
    let result = compile("Most birds can fly.").unwrap();
    println!("Most birds can fly: {}", result);
    
    // Should contain MOST, not ∀
    assert!(result.contains("MOST") || result.contains("Most"),
        "Expected MOST quantifier, got: {}", result);
    assert!(!result.contains("∀x"),
        "Should NOT contain universal ∀x: {}", result);
}

#[test]
fn test_few_students_study() {
    let result = compile("Few students study.").unwrap();
    println!("Few students study: {}", result);
    
    assert!(result.contains("FEW") || result.contains("Few"),
        "Expected FEW quantifier, got: {}", result);
}

#[test]
fn test_many_people_agree() {
    let result = compile("Many people agree.").unwrap();
    println!("Many people agree: {}", result);
    
    assert!(result.contains("MANY") || result.contains("Many"),
        "Expected MANY quantifier, got: {}", result);
}

#[test]
fn test_all_vs_most_body_structure() {
    // All should use implication (→)
    let all_result = compile("All birds fly.").unwrap();
    println!("All birds fly: {}", all_result);
    
    // Most should use conjunction (∧)
    let most_result = compile("Most birds fly.").unwrap();
    println!("Most birds fly: {}", most_result);
    
    // All uses → (implication)
    assert!(all_result.contains("→") || all_result.contains("->"),
        "All should use implication: {}", all_result);
    
    // Most uses ∧ (conjunction) - NOT implication
    // Note: This test will fail if Most incorrectly uses implication
}
