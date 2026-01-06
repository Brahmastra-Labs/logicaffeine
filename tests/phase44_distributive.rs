use logos::compile_forest;
use logos::lexer::Lexer;

/// Phase 44: Link's Logic of Plurals - Explicit Distributive Feature
///
/// This phase tests the explicit tripartite verb plurality classification:
/// - Distributive: verbs that must apply to individuals (sleep, eat, die)
/// - Collective: verbs that must apply to groups (gather, assemble, meet)
/// - Mixed: verbs that can apply to either (lift, carry, push)

#[test]
fn test_explicit_distributive_verb() {
    // "Sleep" is explicitly Distributive - cannot be performed collectively
    // Expected: single reading with * (distributive) operator
    let input = "The boys slept.";
    let readings = compile_forest(input);

    assert_eq!(
        readings.len(),
        1,
        "Distributive verb should have exactly 1 reading, got: {:?}",
        readings
    );

    // Should have the * operator for distributive reading
    assert!(
        readings[0].contains("*"),
        "Should have distributive marker (*), got: {}",
        readings[0]
    );
}

#[test]
fn test_implicit_distributive_unchanged() {
    // Verbs without any plurality feature default to distributive (backward compat)
    // "bark" has no Collective/Mixed/Distributive feature
    let input = "The dogs barked.";
    let readings = compile_forest(input);

    assert_eq!(
        readings.len(),
        1,
        "Verb without plurality feature should default to distributive (1 reading), got: {:?}",
        readings
    );
}

#[test]
fn test_mixed_verb_still_forks() {
    // "lift" is Mixed - should produce both collective and distributive readings
    let input = "The boys lifted the piano.";
    let readings = compile_forest(input);

    assert!(
        readings.len() >= 2,
        "Mixed verb should produce multiple readings, got: {:?}",
        readings
    );

    // Should have both readings
    let has_distributive = readings.iter().any(|r| r.contains("*"));
    let has_collective = readings.iter().any(|r| !r.contains("*"));

    assert!(
        has_distributive && has_collective,
        "Should have both distributive and collective readings, got: {:?}",
        readings
    );
}

#[test]
fn test_collective_verb_no_fork() {
    // "gather" is Collective - should produce single collective reading
    let input = "The students gathered.";
    let readings = compile_forest(input);

    assert_eq!(
        readings.len(),
        1,
        "Collective verb should have exactly 1 reading, got: {:?}",
        readings
    );

    // Should NOT have distributive marker
    assert!(
        !readings[0].contains("*"),
        "Collective reading should not have distributive marker, got: {}",
        readings[0]
    );
}

#[test]
fn test_distributive_verb_eat() {
    // "eat" is explicitly Distributive - you cannot eat collectively as a group entity
    let input = "The children ate.";
    let readings = compile_forest(input);

    assert_eq!(
        readings.len(),
        1,
        "Distributive verb 'eat' should have exactly 1 reading, got: {:?}",
        readings
    );

    assert!(
        readings[0].contains("*"),
        "Should have distributive marker for 'eat', got: {}",
        readings[0]
    );
}

#[test]
fn test_distributive_verb_die() {
    // "die" is explicitly Distributive - death is fundamentally individual
    let input = "The soldiers died.";
    let readings = compile_forest(input);

    assert_eq!(
        readings.len(),
        1,
        "Distributive verb 'die' should have exactly 1 reading, got: {:?}",
        readings
    );

    assert!(
        readings[0].contains("*"),
        "Should have distributive marker for 'die', got: {}",
        readings[0]
    );
}

#[test]
fn test_sigma_operator_in_output() {
    // Definite plurals should use the sigma operator (maximal sum)
    let input = "The boys slept.";
    let readings = compile_forest(input);

    // The output should reference the sigma term or the predicate over boys
    assert!(
        readings[0].contains("Boy") || readings[0].contains("boy"),
        "Should reference the predicate, got: {}",
        readings[0]
    );
}

#[test]
fn test_is_distributive_verb_predicate() {
    // Test the explicit is_distributive_verb() predicate exists and works
    // This tests the explicit tripartite classification
    assert!(
        Lexer::is_distributive_verb("sleep"),
        "sleep should be explicitly distributive"
    );
    assert!(
        Lexer::is_distributive_verb("eat"),
        "eat should be explicitly distributive"
    );
    assert!(
        Lexer::is_distributive_verb("die"),
        "die should be explicitly distributive"
    );

    // Collective verbs should NOT be distributive
    assert!(
        !Lexer::is_distributive_verb("gather"),
        "gather should NOT be distributive"
    );

    // Mixed verbs should NOT be distributive (they're polymorphic)
    assert!(
        !Lexer::is_distributive_verb("lift"),
        "lift should NOT be distributive (it's mixed)"
    );
}

#[test]
fn test_tripartite_classification_mutually_exclusive() {
    // Verify the three classes are mutually exclusive
    // A verb should be in exactly one category

    // Distributive verbs
    assert!(Lexer::is_distributive_verb("sleep"));
    assert!(!Lexer::is_collective_verb("sleep"));
    assert!(!Lexer::is_mixed_verb("sleep"));

    // Collective verbs
    assert!(Lexer::is_collective_verb("gather"));
    assert!(!Lexer::is_distributive_verb("gather"));
    assert!(!Lexer::is_mixed_verb("gather"));

    // Mixed verbs
    assert!(Lexer::is_mixed_verb("lift"));
    assert!(!Lexer::is_distributive_verb("lift"));
    assert!(!Lexer::is_collective_verb("lift"));
}
