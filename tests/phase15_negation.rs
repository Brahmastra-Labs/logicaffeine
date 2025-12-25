use logos::compile;

// === Core NPI Tests ===

#[test]
fn test_free_choice_any() {
    let input = "Any cat hunts.";
    let result = compile(input).expect("Should compile");
    assert!(result.contains("∀"), "Free choice 'any' should be Universal: got {}", result);
    assert!(result.contains("→"), "Universal requires implication: got {}", result);
}

#[test]
fn test_npi_any_with_negation() {
    let input = "John did not see any cat.";
    let result = compile(input).expect("Should compile");
    assert!(result.contains("¬"), "Should be negated: got {}", result);
    assert!(result.contains("∃"), "NPI 'any' should be Existential: got {}", result);
}

#[test]
fn test_not_all_scope() {
    let input = "Not all birds fly.";
    let result = compile(input).expect("Should compile");
    let not_idx = result.find("¬").expect(&format!("Should contain negation: got {}", result));
    let all_idx = result.find("∀").expect(&format!("Should contain universal: got {}", result));
    assert!(not_idx < all_idx, "Negation must scope over Universal: got {}", result);
}

#[test]
fn test_no_licenses_npi() {
    let input = "No dog saw anything.";
    let result = compile(input).expect("Should compile");
    assert!(result.contains("∀"), "'No' should produce Universal: got {}", result);
    assert!(result.contains("¬"), "'No' should produce negation: got {}", result);
    assert!(result.contains("T(") || result.contains("Thing"), "'anything' implies Thing restriction: got {}", result);
}

// === Full NPI Vocabulary Tests ===

#[test]
fn test_anything_standalone() {
    let input = "John did not see anything.";
    let result = compile(input).expect("Should compile");
    assert!(result.contains("¬"), "Should be negated: got {}", result);
    assert!(result.contains("T(") || result.contains("Thing"), "'anything' implies Thing restriction: got {}", result);
}

#[test]
fn test_anyone_npi() {
    let input = "Mary did not meet anyone.";
    let result = compile(input).expect("Should compile");
    assert!(result.contains("¬"), "Should be negated: got {}", result);
    assert!(result.contains("P(") || result.contains("Person"), "'anyone' implies Person restriction: got {}", result);
}

#[test]
fn test_nobody_negative_quantifier() {
    let input = "Nobody runs.";
    let result = compile(input).expect("Should compile");
    assert!(result.contains("∀"), "'nobody' should produce Universal: got {}", result);
    assert!(result.contains("¬"), "'nobody' should produce negation: got {}", result);
    assert!(result.contains("P(") || result.contains("Person"), "'nobody' implies Person restriction: got {}", result);
}

#[test]
fn test_nothing_negative_quantifier() {
    let input = "Nothing happened.";
    let result = compile(input).expect("Should compile");
    assert!(result.contains("∀"), "'nothing' should produce Universal: got {}", result);
    assert!(result.contains("¬"), "'nothing' should produce negation: got {}", result);
}

#[test]
fn test_no_one_mwe() {
    let input = "No one saw anything.";
    let result = compile(input).expect("Should compile");
    assert!(result.contains("P(") || result.contains("Person"), "'no one' implies Person restriction: got {}", result);
    assert!(result.contains("T(") || result.contains("Thing"), "'anything' implies Thing restriction: got {}", result);
}

#[test]
fn test_ever_temporal_npi() {
    let input = "John did not ever run.";
    let result = compile(input).expect("Should compile");
    assert!(result.contains("¬"), "Should be negated: got {}", result);
}

#[test]
fn test_never_negative_temporal() {
    let input = "John never runs.";
    let result = compile(input).expect("Should compile");
    assert!(result.contains("¬"), "'never' should produce negation: got {}", result);
}
