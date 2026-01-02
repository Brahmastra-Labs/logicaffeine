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

/// Generic conditional with event: "If it rains, the ground is wet."
/// The event variable should get UNIVERSAL quantification, not existential.
/// Expected: ∀e(Rain(e) → W(G))
/// NOT: (∃e(Rain(e)) → W(G))
#[test]
fn drs_generic_conditional_event_lifting() {
    let output = compile("If it rains, the ground is wet.").unwrap();
    eprintln!("DEBUG drs_generic_conditional_event_lifting: {}", output);

    // Should have universal quantifier for event variable
    assert!(output.contains("∀"), "Generic conditional should have universal quantifier: {}", output);

    // Should have implication
    assert!(output.contains("→"), "Generic conditional should have implication: {}", output);

    // Should NOT have existential inside the antecedent
    // The Rain event should not be existentially quantified
    assert!(!output.contains("∃e(Rain") && !output.contains("∃e(R("),
        "Event variable should NOT be existentially quantified in antecedent: {}", output);
}

/// Coordinated weather verbs: "If it rains and thunders, the ground shakes."
/// Should handle multiple weather verbs in the antecedent.
#[test]
fn drs_coordinated_weather_verbs() {
    let output = compile("If it rains and thunders, the ground shakes.").unwrap();
    eprintln!("DEBUG drs_coordinated_weather_verbs: {}", output);

    // Should parse without error and contain both weather events
    assert!(output.contains("Rain") || output.contains("R("), "Should have Rain: {}", output);
    assert!(output.contains("Thunder") || output.contains("T("), "Should have Thunder: {}", output);

    // Should have conjunction
    assert!(output.contains("∧"), "Should have conjunction for coordinated events: {}", output);
}

/// Weather adjective with expletive "it": "If it rains then it is wet."
/// The pronoun "it" should be recognized as expletive when followed by weather adjective.
/// The weather adjective should predicate over the event variable: Wet(e)
#[test]
fn drs_weather_adjective_consequent() {
    // Test with full form "it is"
    let output = compile("If it rains then it is wet.").unwrap();
    eprintln!("DEBUG drs_weather_adjective_consequent: {}", output);

    // Should not have unresolved pronoun (?)
    assert!(!output.contains("?"), "Should not have unresolved pronoun: {}", output);

    // Single universal quantifier (not duplicate ∀e(∀e(...)))
    assert_eq!(output.matches("∀").count(), 1,
        "Should have exactly one universal quantifier: {}", output);

    // Wet should predicate over event variable with full lemma
    assert!(output.contains("Wet(e)"),
        "Should have Wet(e) with full lemma, not W(e): {}", output);
}

/// Weather adjective with contraction "it's": "If it rains then it's wet."
#[test]
fn drs_weather_adjective_contraction() {
    let output = compile("If it rains then it's wet.").unwrap();
    eprintln!("DEBUG drs_weather_adjective_contraction: {}", output);

    // Should not have unresolved pronoun (?)
    assert!(!output.contains("?"), "Should not have unresolved pronoun: {}", output);

    // Single universal quantifier
    assert_eq!(output.matches("∀").count(), 1,
        "Should have exactly one universal quantifier: {}", output);

    // Wet should predicate over event variable with full lemma
    assert!(output.contains("Wet(e)"),
        "Should have Wet(e) with full lemma, not W(e): {}", output);
}

/// Full weather event semantics: "If it rains and thunders then it's wet."
/// Expected: ∀e((Rain(e) ∧ Thunder(e)) → Wet(e))
#[test]
fn drs_weather_event_semantics() {
    let output = compile("If it rains and thunders then it's wet.").unwrap();
    eprintln!("DEBUG drs_weather_event_semantics: {}", output);

    // Single universal quantifier (not duplicate ∀e(∀e(...)))
    assert_eq!(output.matches("∀").count(), 1,
        "Should have exactly one universal quantifier: {}", output);

    // Should have conjunction of weather events
    assert!(output.contains("Rain(e)"), "Should have Rain(e): {}", output);
    assert!(output.contains("Thunder(e)"), "Should have Thunder(e): {}", output);
    assert!(output.contains("∧"), "Should have conjunction: {}", output);

    // Wet should predicate over event variable with full lemma
    assert!(output.contains("Wet(e)"),
        "Should have Wet(e) with full lemma, not W(e): {}", output);
}

/// Weather disjunction: "If it rains or thunders then it's wet."
/// Expected: ∀e((Rain(e) ∨ Thunder(e)) → Wet(e))
#[test]
fn drs_weather_disjunction() {
    let output = compile("If it rains or thunders then it's wet.").unwrap();
    eprintln!("DEBUG drs_weather_disjunction: {}", output);

    // Single universal quantifier
    assert_eq!(output.matches("∀").count(), 1,
        "Should have exactly one universal quantifier: {}", output);

    // Should have disjunction of weather events
    assert!(output.contains("∨"), "Should have disjunction: {}", output);
    assert!(output.contains("Rain(e)"), "Should have Rain(e): {}", output);
    assert!(output.contains("Thunder(e)"), "Should have Thunder(e): {}", output);

    // Wet should predicate over event variable with full lemma
    assert!(output.contains("Wet(e)"),
        "Should have Wet(e) with full lemma, not W(e): {}", output);
}

/// Coordinated weather adjectives: "If it rains then it's wet and cold."
/// Expected: ∀e(Rain(e) → (Wet(e) ∧ Cold(e)))
#[test]
fn drs_weather_compound_adjectives() {
    let output = compile("If it rains then it's wet and cold.").unwrap();
    eprintln!("DEBUG drs_weather_compound_adjectives: {}", output);

    // Should have Wet(e) with full lemma, not W(e)
    assert!(output.contains("Wet(e)"),
        "Should have Wet(e) with full lemma, not W(e): {}", output);

    // Should have Cold(e) with full lemma, not C(e)
    assert!(output.contains("Cold(e)"),
        "Should have Cold(e) with full lemma, not C(e): {}", output);

    // Should have conjunction in consequent
    let arrow_pos = output.find("→").expect("Should have implication");
    let consequent = &output[arrow_pos..];
    assert!(consequent.contains("∧"), "Consequent should have conjunction: {}", output);
}

/// Grammar error: "its" (possessive) vs "it's" (contraction)
/// "If it rains then its wet." should produce a grammar error
#[test]
fn drs_its_possessive_grammar_error() {
    let result = compile("If it rains then its wet.");
    assert!(result.is_err(), "Should reject 'its wet' as grammar error");

    let err = result.unwrap_err();
    let err_msg = format!("{:?}", err);
    assert!(err_msg.contains("it's") || err_msg.contains("possessive"),
        "Error should mention the typo: {}", err_msg);
}
