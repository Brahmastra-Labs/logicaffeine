use logos::compile_forest;

#[test]
fn test_mixed_verb_ambiguity() {
    // "The boys lifted the piano."
    // Ambiguous: Together (Collective) or Separately (Distributive)?
    // Using definite plural for both subject and object
    let input = "The boys lifted the piano.";
    let readings = compile_forest(input);

    // Expect: 2 distinct readings
    assert!(
        readings.len() >= 2,
        "Should return multiple readings for mixed verb, got {} readings: {:?}",
        readings.len(),
        readings
    );

    // Collective reading: no * operator (group lifted together)
    // Distributive reading: has * operator (each boy lifted individually)
    let has_collective = readings.iter().any(|r| !r.contains("*"));
    let has_distributive = readings.iter().any(|r| r.contains("*"));

    assert!(has_collective, "Should have Collective reading (no *)");
    assert!(has_distributive, "Should have Distributive reading (*)");
}

#[test]
fn test_forced_collective() {
    // "The boys gathered."
    // "Gather" is strictly Collective. Distributive reading is nonsense.
    let input = "The boys gathered.";
    let readings = compile_forest(input);

    // Expect: 1 reading, NO distributive operator
    assert_eq!(
        readings.len(),
        1,
        "Collective verb should have only 1 reading, got: {:?}",
        readings
    );
    assert!(
        !readings[0].contains("*"),
        "Should not be distributive, got: {}",
        readings[0]
    );
}

#[test]
fn test_forced_distributive() {
    // "The boys slept."
    // "Sleep" is strictly Distributive. You cannot "sleep" as a group entity.
    let input = "The boys slept.";
    let readings = compile_forest(input);

    // Expect: 1 reading (distributive is the default for non-collective verbs)
    assert_eq!(
        readings.len(),
        1,
        "Distributive verb should have 1 reading, got: {:?}",
        readings
    );
}

#[test]
fn test_mixed_verb_with_definite_plural() {
    // "The boys lifted the piano."
    let input = "The boys lifted the piano.";
    let readings = compile_forest(input);

    assert!(
        readings.len() >= 2,
        "Should return multiple readings for mixed verb with definite plural, got: {:?}",
        readings
    );
}

#[test]
fn test_mixed_verb_singular_no_fork() {
    // "A boy lifted a rock."
    // Singular subject - no ambiguity, should not fork
    let input = "A boy lifted a rock.";
    let readings = compile_forest(input);

    assert_eq!(
        readings.len(),
        1,
        "Singular subject should not fork, got: {:?}",
        readings
    );
}
