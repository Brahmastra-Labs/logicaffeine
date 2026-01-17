use logicaffeine_language::compile;

#[test]
fn test_analytic_entailment_bachelor() {
    let input = "John is a bachelor.";
    let result = compile(input).expect("Should compile");

    // Axiom expansion should add Unmarried and Male predicates
    // Transpiler abbreviates predicates (Bachelor->B, Unmarried->U, Male->M2)
    assert!(
        result.contains("B(") || result.contains("Bachelor("),
        "Should have bachelor predicate: {}", result
    );
    assert!(
        result.contains("Unmarried") || result.contains("U("),
        "Bachelor should imply Unmarried: {}", result
    );
    assert!(
        result.contains("Male") || result.contains("M("),
        "Bachelor should imply Male: {}", result
    );
}

#[test]
fn test_privative_adjective_fake() {
    // Use a sentence structure that parses correctly
    let input = "The fake gun is dangerous.";
    let result = compile(input).expect("Should compile");

    // Privative: Fake-Gun should expand to NOT Gun
    assert!(
        result.contains("¬") || result.contains("\\neg"),
        "Fake should negate the noun: {}", result
    );
    assert!(
        result.contains("Gun") || result.contains("G("),
        "Should reference Gun: {}", result
    );
}

#[test]
fn test_verbal_entailment_murder() {
    let input = "John murdered Mary.";
    let result = compile(input).expect("Should compile");

    // Preserve original verb, add entailed verb
    assert!(
        result.contains("Murder"),
        "Should preserve original verb: {}", result
    );
    assert!(
        result.contains("Kill") || result.contains("K("),
        "Murder implies Killing: {}", result
    );
}

#[test]
fn test_hypernym_expansion() {
    let input = "Fido is a dog.";
    let result = compile(input).expect("Should compile");

    // Dog predicate should be preserved, Animal added
    assert!(
        result.contains("D(") || result.contains("Dog("),
        "Should preserve dog predicate: {}", result
    );
    assert!(
        result.contains("Animal") || result.contains("A("),
        "Dog should imply Animal: {}", result
    );
}
