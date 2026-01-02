use logos::compile;

// ═══════════════════════════════════════════════════════════════════
// MODAL SCOPE TESTS (De Re vs De Dicto)
// ═══════════════════════════════════════════════════════════════════
//
// Root modals (can, must, should) → Narrow Scope (De Re)
//   "Some birds can fly" → ∃x(Bird(x) ∧ ◇Fly(x))
//
// Epistemic modals (might) → Wide Scope (De Dicto)
//   "Some unicorns might exist" → ◇∃x(Unicorn(x) ∧ Exist(x))
//
// ═══════════════════════════════════════════════════════════════════

// ─────────────────────────────────────────────────────────────────
// Root Modal → Narrow Scope (De Re)
// ─────────────────────────────────────────────────────────────────

#[test]
fn modal_scope_root_can_narrow() {
    // "Some birds can fly" → ∃x(Bird(x) ∧ ◇Fly(x))
    // NOT ◇∃x(Bird(x) ∧ Fly(x))
    let output = compile("Some birds can fly.").unwrap();

    // Quantifier should come BEFORE modal (narrow scope)
    let quant_pos = output.find('∃').expect(&format!("Should have ∃. Got: {}", output));
    let modal_pos = output.find('◇').expect(&format!("Should have ◇. Got: {}", output));
    assert!(
        quant_pos < modal_pos,
        "Root modal 'can' should have narrow scope (de re). Quantifier should come before modal. Got: {}",
        output
    );
}

#[test]
fn modal_scope_root_must_narrow() {
    // "All students must study" → ∀x(Student(x) → □Study(x))
    // NOT □∀x(Student(x) → Study(x))
    let output = compile("All students must study.").unwrap();

    let quant_pos = output.find('∀').expect(&format!("Should have ∀. Got: {}", output));
    let modal_pos = output.find('□').expect(&format!("Should have □. Got: {}", output));
    assert!(
        quant_pos < modal_pos,
        "Root modal 'must' should have narrow scope (de re). Quantifier should come before modal. Got: {}",
        output
    );
}

#[test]
fn modal_scope_root_should_narrow() {
    // "Every person should vote" → ∀x(Person(x) → O(Vote(x)))
    // "should" outputs as O (deontic obligation), not □
    let output = compile("Every person should vote.").unwrap();

    // Deontic modal outputs as O_{force}
    assert!(
        !output.starts_with("O_"),
        "Root modal 'should' should have narrow scope (de re). Output should NOT start with O_. Got: {}",
        output
    );

    let quant_pos = output.find('∀').expect(&format!("Should have ∀. Got: {}", output));
    let modal_pos = output.find("O_").expect(&format!("Should have O_ (deontic). Got: {}", output));
    assert!(
        quant_pos < modal_pos,
        "Quantifier should come before modal. Got: {}",
        output
    );
}

// ─────────────────────────────────────────────────────────────────
// Epistemic Modal → Wide Scope (De Dicto)
// ─────────────────────────────────────────────────────────────────

#[test]
fn modal_scope_epistemic_might_wide() {
    // "Some unicorns might exist" → ◇∃x(Unicorn(x) ∧ Exist(x))
    // NOT ∃x(Unicorn(x) ∧ ◇Exist(x)) - that wrongly asserts unicorns exist!
    let output = compile("Some unicorns might exist.").unwrap();

    // Modal should come BEFORE quantifier (wide scope)
    let modal_pos = output.find('◇').expect(&format!("Should have ◇. Got: {}", output));
    let quant_pos = output.find('∃').expect(&format!("Should have ∃. Got: {}", output));
    assert!(
        modal_pos < quant_pos,
        "Epistemic modal 'might' should have wide scope (de dicto). Modal should come before quantifier. Got: {}",
        output
    );
}

// ─────────────────────────────────────────────────────────────────
// Generalized Quantifiers + Root Modal
// ─────────────────────────────────────────────────────────────────

#[test]
fn modal_scope_most_can_narrow() {
    // "Most birds can fly" → MOST x(Bird(x) ∧ ◇Fly(x))
    let output = compile("Most birds can fly.").unwrap();

    assert!(output.contains("MOST"), "Should have MOST quantifier. Got: {}", output);
    assert!(
        !output.starts_with("◇"),
        "Root modal should have narrow scope. Output should NOT start with ◇. Got: {}",
        output
    );
}

#[test]
fn modal_scope_no_can_narrow() {
    // "No bird can fly" → ∀x(Bird(x) → ¬◇Fly(x))
    let output = compile("No bird can fly.").unwrap();

    assert!(output.contains("∀"), "Should have ∀ (no = ∀...→¬). Got: {}", output);
    assert!(
        !output.starts_with("◇"),
        "Root modal should have narrow scope. Output should NOT start with ◇. Got: {}",
        output
    );
}

#[test]
fn modal_scope_few_can_narrow() {
    // "Few birds can fly" → FEW x(Bird(x) ∧ ◇Fly(x))
    let output = compile("Few birds can fly.").unwrap();

    assert!(output.contains("FEW"), "Should have FEW quantifier. Got: {}", output);
    assert!(
        !output.starts_with("◇"),
        "Root modal should have narrow scope. Got: {}",
        output
    );
}

// ─────────────────────────────────────────────────────────────────
// Edge Cases
// ─────────────────────────────────────────────────────────────────

#[test]
fn modal_scope_cannot_narrow() {
    // "Some birds cannot fly" → ∃x(Bird(x) ∧ □₀Fly(x))
    // Cannot uses □ (necessity) with force 0, meaning impossibility
    let output = compile("Some birds cannot fly.").unwrap();

    let quant_pos = output.find('∃').expect(&format!("Should have ∃. Got: {}", output));
    // Cannot produces □ with force 0
    let modal_pos = output.find('□').expect(&format!("Should have □. Got: {}", output));
    assert!(
        quant_pos < modal_pos,
        "Root modal 'cannot' should have narrow scope. Got: {}",
        output
    );
}

#[test]
fn modal_scope_could_narrow() {
    // "Some students could pass" → ∃x(Student(x) ∧ ◇Pass(x))
    let output = compile("Some students could pass.").unwrap();

    let quant_pos = output.find('∃').expect(&format!("Should have ∃. Got: {}", output));
    let modal_pos = output.find('◇').expect(&format!("Should have ◇. Got: {}", output));
    assert!(
        quant_pos < modal_pos,
        "Root modal 'could' should have narrow scope. Got: {}",
        output
    );
}
