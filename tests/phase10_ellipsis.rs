use logos::compile;

#[test]
fn vp_ellipsis_does_too() {
    // "John runs. Mary does too."
    // Should parse as: Run(John) ∧ Run(Mary)
    let output = compile("John runs. Mary does too.").unwrap();

    assert!(output.matches("Run").count() >= 2 || output.matches("R(").count() >= 2,
        "Should reconstruct 'Run' for Mary. Output: {}", output);
    assert!(output.contains("Mary") || output.contains("M"), "Mary should be in the output");
}

#[test]
fn modal_ellipsis_can_too() {
    // "John can swim. Mary can too."
    // Should parse as: ◇Swim(John) ∧ ◇Swim(Mary)
    let output = compile("John can swim. Mary can too.").unwrap();

    let modal_count = output.matches("◇").count() + output.matches("Can").count();
    assert!(modal_count >= 2, "Both clauses should have modal operators. Output: {}", output);

    let swim_count = output.matches("Swim").count() + output.matches("S(").count();
    assert!(swim_count >= 2, "Swim predicate should be reconstructed. Output: {}", output);
}

#[test]
fn negative_ellipsis_does_not() {
    // "John runs. Mary does not."
    // Should parse as: Run(John) ∧ ¬Run(Mary)
    let output = compile("John runs. Mary does not.").unwrap();

    assert!(output.contains("¬") || output.contains("Not"),
        "Second clause should be negated. Output: {}", output);
    assert!(output.matches("Run").count() >= 2 || output.matches("R(").count() >= 2,
        "Run predicate should be reconstructed for Mary. Output: {}", output);
}

#[test]
fn ellipsis_with_object() {
    // "John eats an apple. Mary does too."
    // Strict identity: Mary eats an apple (same predicate template)
    let output = compile("John eats an apple. Mary does too.").unwrap();

    assert!(output.matches("Eat").count() >= 2 || output.matches("E(").count() >= 2,
        "Eat predicate should be reconstructed. Output: {}", output);
    assert!(output.contains("Apple") || output.contains("A("), "Apple should persist in context");
}
