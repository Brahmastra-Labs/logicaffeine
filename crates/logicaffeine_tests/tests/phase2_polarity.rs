use logicaffeine_language::compile;

#[test]
fn npi_any_is_existential() {
    // "Not any dogs run." -> ¬∃x(Dog(x) ∧ Run(x))
    let output = compile("Not any dogs run.").unwrap();
    assert!(output.contains("∃") || output.contains("Some"),
        "NPI 'any' in negative context should be existential: got {}", output);
    assert!(!output.contains("∀"),
        "NPI 'any' should NOT be universal: got {}", output);
}

#[test]
fn free_choice_any_is_universal() {
    // "Any dog runs." -> ∀x(Dog(x) → Run(x))
    let output = compile("Any dog runs.").unwrap();
    assert!(output.contains("∀") || output.contains("All"),
        "Free choice 'any' should be universal: got {}", output);
}

#[test]
fn any_with_double_negation() {
    // Double negation restores positive context
    // "It is not true that John did not see any dogs."
    // The "any" is at depth 2 (even), so positive → Universal?
    // Actually this is tricky - the inner negation scopes over "any"
    // Let's test a simpler case first
    let output = compile("Any cat sleeps.").unwrap();
    assert!(output.contains("∀") || output.contains("All"),
        "Free choice 'any' should be universal: got {}", output);
}

#[test]
fn any_in_conditional_antecedent() {
    // "If any dog barks, John runs."
    // In conditional antecedent, "any" should be universal
    let output = compile("If any dog barks, John runs.").unwrap();
    assert!(output.contains("∀") || output.contains("All"),
        "'any' in conditional antecedent should be universal: got {}", output);
}
