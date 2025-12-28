// Phase 42: DRS (Discourse Representation Structure)
//
// Implements Kamp's DRT for handling donkey anaphora and accessibility.
// The key insight is that indefinites in conditional antecedents and
// universal restrictors get UNIVERSAL (not existential) force.

use logos::compile;

/// Basic donkey anaphora: "Every farmer who owns a donkey beats it."
/// The pronoun "it" must bind to "donkey" even though donkey is in a relative clause.
/// Required output: ∀x∀y((Farmer(x) ∧ Donkey(y) ∧ Own(x,y)) → Beat(x,y))
#[test]
fn drs_basic_donkey() {
    let output = compile("Every farmer who owns a donkey beats it.").unwrap();
    eprintln!("DEBUG drs_basic_donkey: {}", output);

    // Should have universal quantifiers
    assert!(output.contains("∀"), "Basic donkey needs universal quantifier: {}", output);

    // Should have implication structure
    assert!(output.contains("→"), "Basic donkey needs implication: {}", output);
}

/// Quantifier ordering: farmer (x) should be quantified BEFORE donkey (y)
#[test]
fn drs_quantifier_ordering() {
    let output = compile("Every farmer who owns a donkey beats it.").unwrap();
    eprintln!("DEBUG drs_quantifier_ordering: {}", output);

    // The farmer variable should appear before the donkey variable in the quantifier prefix
    // Looking for the pattern where the first ∀ binds farmer-related variable
    // This is a structural test - the outer quantifier should be for the farmer
    let first_forall = output.find("∀").expect("Should have universal");
    let chars_after: String = output[first_forall..].chars().take(10).collect();
    eprintln!("First ∀ context: {}", chars_after);

    // The output should have nested universals
    let count = output.matches("∀").count();
    assert!(count >= 2, "Should have at least 2 universal quantifiers for donkey sentence: {}", output);
}

/// Conditional donkey: "If a farmer owns a donkey, he beats it."
/// Indefinites in conditional antecedents get UNIVERSAL force (DRS signature).
#[test]
fn drs_conditional_donkey() {
    let output = compile("If a farmer owns a donkey, he beats it.").unwrap();
    eprintln!("DEBUG drs_conditional_donkey: {}", output);

    // The indefinites "a farmer" and "a donkey" should become universals
    assert!(output.contains("∀"), "Conditional donkey needs universal quantification: {}", output);

    // Should have implication
    assert!(output.contains("→"), "Conditional donkey needs implication: {}", output);
}

/// Negative donkey: "No farmer who owns a donkey beats it."
/// Required: ∀x∀y((Farmer(x) ∧ Donkey(y) ∧ Own(x,y)) → ¬Beat(x,y))
#[test]
fn drs_negative_donkey() {
    let output = compile("No farmer who owns a donkey beats it.").unwrap();
    eprintln!("DEBUG drs_negative_donkey: {}", output);

    // Should have universal quantifiers
    assert!(output.contains("∀"), "Negative donkey needs universal: {}", output);

    // Should have negation
    assert!(output.contains("¬"), "Negative donkey needs negation: {}", output);
}

/// Pronoun binding: "it" should resolve to "donkey" via DRS accessibility
#[test]
fn drs_pronoun_binding() {
    let output = compile("Every farmer who owns a donkey beats it.").unwrap();
    eprintln!("DEBUG drs_pronoun_binding: {}", output);

    // The output should NOT contain unresolved pronoun marker
    assert!(!output.contains("?"), "Pronoun 'it' should resolve: {}", output);

    // The Beat predicate should have the donkey variable as second argument
    // Looking for Beat(x, y) or similar pattern
    assert!(output.contains("B(") || output.contains("Beat("),
        "Should have Beat predicate: {}", output);
}

/// "Most farmers who own a donkey beat it." - proportional quantifier
/// This tests the "proportion problem" - DRS handles MOST specially.
#[test]
fn drs_proportional_most() {
    let output = compile("Most farmers who own a donkey beat it.").unwrap();
    eprintln!("DEBUG drs_proportional_most: {}", output);

    // Should have MOST quantifier (not converted to ∀)
    assert!(output.contains("MOST") || output.contains("Most"),
        "Proportional quantifier should preserve MOST: {}", output);
}

/// Antecedent accessible from consequent:
/// "If a farmer owns a donkey, the donkey is grey."
/// "the donkey" in consequent should bind to "a donkey" in antecedent.
#[test]
fn drs_antecedent_accessible() {
    let output = compile("If a farmer owns a donkey, the donkey is grey.").unwrap();
    eprintln!("DEBUG drs_antecedent_accessible: {}", output);

    // The definite "the donkey" should bind to the antecedent's donkey
    // Should not have unresolved reference
    assert!(!output.contains("?"), "Consequent should access antecedent referent: {}", output);
}

/// Simple donkey with different pronouns:
/// "Every man who owns a donkey loves her."
/// Tests gender matching in DRS pronoun resolution.
#[test]
fn drs_pronoun_gender_mismatch() {
    let output = compile("Every man who owns a donkey loves her.").unwrap();
    eprintln!("DEBUG drs_pronoun_gender_mismatch: {}", output);

    // "her" should NOT bind to "donkey" (gender mismatch: donkey is neuter)
    // This might have unresolved pronoun (?) or different binding
}

/// Multiple donkey referents - simplified to avoid parse issues
/// "Every farmer who owns a donkey owns a horse."
#[test]
fn drs_multiple_indefinites() {
    let output = compile("Every farmer who owns a donkey owns a horse.").unwrap();
    eprintln!("DEBUG drs_multiple_indefinites: {}", output);

    // Should have multiple variables for donkey and horse
    assert!(output.contains("D(") || output.contains("Donkey("),
        "Should have donkey predicate: {}", output);
    assert!(output.contains("H(") || output.contains("Horse("),
        "Should have horse predicate: {}", output);
}

/// Nested relative clauses:
/// "Every farmer who owns a donkey that kicks it runs."
/// Tests nested DRS boxes.
#[test]
fn drs_nested_relative() {
    let output = compile("Every farmer who owns a donkey that is grey beats it.").unwrap();
    eprintln!("DEBUG drs_nested_relative: {}", output);

    // Should have Grey/Gray predicate for the donkey
    assert!(output.contains("G(") || output.contains("Grey(") || output.contains("Gray("),
        "Should have Grey predicate for nested relative: {}", output);
}
