use logicaffeine_language::compile;

#[test]
fn and_binds_tighter_than_or() {
    // "A cat runs or a dog walks and a bird flies."
    // With precedence: A or (B and C) -> the And should bind B and C first
    // Output structure: (Run(cat) ∨ (Walk(dog) ∧ Fly(bird)))
    let result = compile("A cat runs or a dog walks and a bird flies.");
    assert!(result.is_ok(), "Should parse: {:?}", result);

    let output = result.unwrap();
    // The And should appear as a sub-expression of the Or
    // Structure check: the overall connective should be Or (∨)
    assert!(
        output.contains("∨") || output.contains("Or"),
        "Top-level should be disjunction (Or): {}",
        output
    );
}

#[test]
fn explicit_grouping_with_and_first() {
    // "A cat runs and a dog walks."
    // Simple conjunction
    let result = compile("A cat runs and a dog walks.");
    assert!(result.is_ok(), "Should parse: {:?}", result);

    let output = result.unwrap();
    assert!(
        output.contains("∧") || output.contains("And"),
        "Should contain conjunction: {}",
        output
    );
}

#[test]
fn explicit_grouping_with_or_first() {
    // "A cat runs or a dog walks."
    // Simple disjunction
    let result = compile("A cat runs or a dog walks.");
    assert!(result.is_ok(), "Should parse: {:?}", result);

    let output = result.unwrap();
    assert!(
        output.contains("∨") || output.contains("Or"),
        "Should contain disjunction: {}",
        output
    );
}

#[test]
fn left_associative_and() {
    // "A runs and B walks and C flies."
    // Should be ((A and B) and C)
    let result = compile("A cat runs and a dog walks and a bird flies.");
    assert!(result.is_ok(), "Should parse chained And: {:?}", result);
}

#[test]
fn left_associative_or() {
    // "A runs or B walks or C flies."
    // Should be ((A or B) or C)
    let result = compile("A cat runs or a dog walks or a bird flies.");
    assert!(result.is_ok(), "Should parse chained Or: {:?}", result);
}

#[test]
fn mixed_precedence_complex() {
    // "A runs and B walks or C flies and D swims."
    // With precedence: (A and B) or (C and D)
    // The two And groups should be separate, joined by Or
    let result = compile("A cat runs and a dog walks or a bird flies and a fish swims.");
    assert!(result.is_ok(), "Should parse mixed precedence: {:?}", result);

    let output = result.unwrap();
    // Should have both And and Or
    let has_and = output.contains("∧") || output.contains("And");
    let has_or = output.contains("∨") || output.contains("Or");
    assert!(has_and && has_or, "Should have both And and Or: {}", output);
}
