use logos::{compile, compile_simple, compile_all_scopes};

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

// ═══════════════════════════════════════════════════════════════════
// LEXICALLY NEGATIVE VERB SCOPE TESTS ("lacks", "miss")
// ═══════════════════════════════════════════════════════════════════
//
// "lacks" is lexically negative (antonym of "have")
// "user who lacks a key" should allow scope ambiguity:
//   - Narrow scope ¬: ∃y(Key(y) ∧ ¬Have(x, y)) - "missing ANY key blocks entry"
//   - Wide scope ¬:  ¬∃y(Key(y) ∧ Have(x, y))  - "having NO keys blocks entry"
//
// ═══════════════════════════════════════════════════════════════════

#[test]
fn lacks_basic_transformation() {
    // Verify "lacks" → "¬Have" transformation
    let output = compile_simple("A user lacks a key.");
    assert!(output.is_ok(), "Should parse 'lacks' sentence");
    let result = output.unwrap();
    eprintln!("Basic lacks: {}", result);

    // Should transform "lacks" to ¬Have (canonical form)
    assert!(result.contains("¬Have(") || result.contains("¬Have "),
        "Should have negated Have predicate, got: {}", result);
}

#[test]
fn lacks_in_relative_clause() {
    // Test "lacks" in a relative clause
    let output = compile_simple("Every user who lacks a key enters the room.");
    assert!(output.is_ok(), "Should parse relative clause with 'lacks'");
    let result = output.unwrap();
    eprintln!("Relative clause lacks: {}", result);

    assert!(result.contains("¬Have("),
        "Should have negated Have predicate in relative clause, got: {}", result);
}

#[test]
fn lacks_scope_ambiguity() {
    // Test with universal quantifier where donkey binding processing is well-defined
    // "Every user who lacks a key enters the room" should produce both scope readings
    let results = logos::compile_forest("Every user who lacks a key enters the room.");

    eprintln!("Number of parse forest readings for 'lacks': {}", results.len());
    for (i, reading) in results.iter().enumerate() {
        eprintln!("Reading {}: {}", i + 1, reading);
    }

    // Should have at least 2 readings: narrow scope and wide scope
    assert!(
        results.len() >= 2,
        "Should have at least 2 readings (narrow and wide scope). Got: {}",
        results.len()
    );

    // Check for both narrow and wide scope readings
    // Narrow (NeoEvent): ∃y(Key(y) ∧ ¬∃e(Have(e) ∧ Agent(e, x) ∧ Theme(e, y))) - "missing some key"
    // Wide: ¬∃y(Key(y) ∧ Have(x,y)) - "has no keys"
    // In NeoEvent format, narrow scope shows up as ¬∃e(Have(e)
    let has_narrow = results.iter().any(|r| r.contains("Key(y)") && r.contains("¬∃e(Have(e)"));
    let has_wide = results.iter().any(|r| r.contains("¬∃y"));

    eprintln!("Has narrow scope (∃y...¬∃e(Have)): {}", has_narrow);
    eprintln!("Has wide scope (¬∃y): {}", has_wide);

    assert!(has_narrow, "Should have narrow scope reading (∃y...¬∃e(Have))");
    assert!(has_wide, "Should have wide scope reading (¬∃y)");
}

#[test]
fn no_user_lacks_key_marketing() {
    // Test the marketing sentence
    let output = compile_simple("No user who lacks a key can enter the room.");
    assert!(output.is_ok(), "Should parse marketing sentence");
    let result = output.unwrap();
    eprintln!("Marketing sentence: {}", result);

    // Basic checks
    assert!(result.contains("User("), "Should have User predicate");
    assert!(result.contains("Key("), "Should have Key predicate");
    assert!(result.contains("¬Have(") || result.contains("Have("),
        "Should have Have predicate (possibly negated)");
    assert!(result.contains("Enter("), "Should have Enter predicate");
    assert!(result.contains("Room"), "Should have Room constant");
}

// ═══════════════════════════════════════════════════════════════════
// UNACCUSATIVE VERB TESTS
// ═══════════════════════════════════════════════════════════════════
//
// Unaccusative verbs (trigger, break, melt, open, close, etc.) have
// Theme subjects when used intransitively:
//   "The alarm triggers" → Theme(e, Alarm), NOT Agent(e, Alarm)
//   "John triggers the alarm" → Agent(e, John), Theme(e, Alarm)
//
// ═══════════════════════════════════════════════════════════════════

#[test]
fn unaccusative_intransitive_theme_subject() {
    // "The alarm triggers" - alarm is undergoing the event, not causing it
    let output = logos::compile("The alarm triggers.");
    assert!(output.is_ok(), "Should parse intransitive trigger");
    let result = output.unwrap();
    eprintln!("Intransitive trigger: {}", result);

    // Should have Theme role (with quantified variable), NOT Agent
    // Output format: ∃x(((Alarm(x) ∧ ...) ∧ ...∃e(Trigger(e) ∧ Theme(e, x))))
    assert!(
        result.contains("Theme(e, x)") || result.contains("Theme(e,x)"),
        "Intransitive unaccusative should use Theme for subject. Got: {}",
        result
    );
    assert!(
        result.contains("Alarm(x)") || result.contains("Alarm(y)"),
        "Should have Alarm predicate. Got: {}",
        result
    );
    assert!(
        !result.contains("Agent(e, x)") && !result.contains("Agent(e,x)"),
        "Intransitive unaccusative should NOT use Agent for subject. Got: {}",
        result
    );
}

#[test]
fn unaccusative_transitive_agent_subject() {
    // "John triggers the alarm" - John is the agent causing the event
    let output = logos::compile("John triggers the alarm.");
    assert!(output.is_ok(), "Should parse transitive trigger");
    let result = output.unwrap();
    eprintln!("Transitive trigger: {}", result);

    // Should have Agent role for John (causative use)
    assert!(
        result.contains("Agent(e, John)") || result.contains("Agent(e,John)"),
        "Transitive unaccusative should use Agent for subject. Got: {}",
        result
    );
    // Should have Theme role for the alarm
    assert!(
        result.contains("Theme(e, Alarm)") || result.contains("Theme(e,Alarm)"),
        "Transitive unaccusative should use Theme for object. Got: {}",
        result
    );
}

#[test]
fn unaccusative_break_intransitive() {
    let output = logos::compile("The window breaks.");
    assert!(output.is_ok(), "Should parse intransitive break");
    let result = output.unwrap();
    eprintln!("Intransitive break: {}", result);

    // Output uses quantified variable: ∃x(((Window(x) ∧ ...) ∧ ...Theme(e, x)))
    assert!(
        result.contains("Theme(e, x)") || result.contains("Theme(e,x)"),
        "Intransitive 'break' should use Theme. Got: {}",
        result
    );
    assert!(
        result.contains("Window(x)") || result.contains("Window(y)"),
        "Should have Window predicate. Got: {}",
        result
    );
}

#[test]
fn unaccusative_melt_intransitive() {
    let output = logos::compile("The ice melts.");
    assert!(output.is_ok(), "Should parse intransitive melt");
    let result = output.unwrap();
    eprintln!("Intransitive melt: {}", result);

    // Output uses quantified variable: ∃x(((Ice(x) ∧ ...) ∧ ...Theme(e, x)))
    assert!(
        result.contains("Theme(e, x)") || result.contains("Theme(e,x)"),
        "Intransitive 'melt' should use Theme. Got: {}",
        result
    );
    assert!(
        result.contains("Ice(x)") || result.contains("Ice(y)"),
        "Should have Ice predicate. Got: {}",
        result
    );
}

// ═══════════════════════════════════════════════════════════════════
// MODAL POLYSEMY TESTS (Parse Forest Forking)
// ═══════════════════════════════════════════════════════════════════
//
// Polysemous modals produce multiple valid readings:
//   may: Permission (Deontic, Root) vs Possibility (Alethic, Epistemic)
//   can: Ability (Alethic, Root) vs Permission (Deontic, Root)
//   could: Past Ability (Alethic, Root) vs Conditional Possibility (Alethic, Epistemic)
//
// ═══════════════════════════════════════════════════════════════════

// ─────────────────────────────────────────────────────────────────
// MAY: Permission vs Possibility
// ─────────────────────────────────────────────────────────────────

#[test]
fn may_polysemy_permission_vs_possibility() {
    // "Students may leave" should produce both readings:
    // 1. Permission (Root): ∀x(Student(x) → P_{0.5}(Leave(x)))
    // 2. Possibility (Epistemic): ◇∀x(Student(x) → Leave(x))
    let results = logos::compile_forest("Students may leave.");

    eprintln!("May polysemy readings ({}):", results.len());
    for (i, r) in results.iter().enumerate() {
        eprintln!("  {}: {}", i + 1, r);
    }

    assert!(results.len() >= 2,
        "Should have at least 2 readings. Got: {:?}", results);

    // Permission reading: deontic P inside quantifier (narrow scope)
    let has_permission = results.iter().any(|r| r.contains("P_") && !r.starts_with("◇"));
    // Possibility reading: alethic ◇ outside quantifier (wide scope)
    let has_possibility = results.iter().any(|r| r.starts_with("◇"));

    assert!(has_permission, "Should have permission reading (P_). Got: {:?}", results);
    assert!(has_possibility, "Should have possibility reading (◇). Got: {:?}", results);
}

#[test]
fn may_permission_narrow_scope() {
    // Permission reading: "Students are allowed to leave"
    // ∀x(Student(x) → P_{0.5}(Leave(x)))
    let output = logos::compile("Students may leave.").unwrap();
    eprintln!("May permission: {}", output);

    // Default should be permission (Root/Narrow)
    assert!(output.contains("P_"),
        "Default 'may' should be permission (P_). Got: {}", output);

    // Quantifier before modal (narrow scope)
    if let (Some(q_pos), Some(p_pos)) = (output.find('∀'), output.find("P_")) {
        assert!(q_pos < p_pos,
            "Permission 'may' should have narrow scope. Got: {}", output);
    }
}

// ─────────────────────────────────────────────────────────────────
// CAN: Ability vs Permission
// ─────────────────────────────────────────────────────────────────

#[test]
fn can_polysemy_ability_vs_permission() {
    // "Students can leave" should produce both readings:
    // 1. Ability (Alethic): ∀x(Student(x) → ◇_{0.5}(Leave(x)))
    // 2. Permission (Deontic): ∀x(Student(x) → P_{0.5}(Leave(x)))
    let results = logos::compile_forest("Students can leave.");

    eprintln!("Can polysemy readings ({}):", results.len());
    for (i, r) in results.iter().enumerate() {
        eprintln!("  {}: {}", i + 1, r);
    }

    assert!(results.len() >= 2,
        "Should have at least 2 readings. Got: {:?}", results);

    // Ability reading: alethic ◇
    let has_ability = results.iter().any(|r| r.contains("◇"));
    // Permission reading: deontic P
    let has_permission = results.iter().any(|r| r.contains("P_"));

    assert!(has_ability, "Should have ability reading (◇). Got: {:?}", results);
    assert!(has_permission, "Should have permission reading (P_). Got: {:?}", results);
}

#[test]
fn can_ability_narrow_scope() {
    // Default "can" = ability (Alethic, Root/Narrow)
    // Use quantified subject to test scope
    let output = logos::compile("Some birds can fly.").unwrap();
    eprintln!("Can ability: {}", output);

    // Should use ◇ (alethic possibility)
    assert!(output.contains("◇"),
        "Default 'can' should be ability (◇). Got: {}", output);

    // Quantifier before modal (narrow scope)
    let q_pos = output.find('∃').expect("Should have ∃ quantifier");
    let m_pos = output.find('◇').expect("Should have ◇ modal");
    assert!(q_pos < m_pos, "Ability 'can' should have narrow scope. Got: {}", output);
}

// ─────────────────────────────────────────────────────────────────
// COULD: Past Ability vs Conditional Possibility
// ─────────────────────────────────────────────────────────────────
// Note: "could" polysemy differs from "may"/"can" because both readings
// use Alethic domain with force 0.5. The ONLY difference is SCOPE:
//   - Default (Root): narrow scope - quantifier BEFORE modal
//   - Epistemic: wide scope - modal BEFORE quantifier
// This means we MUST use quantified subjects to observe the difference.

#[test]
fn could_polysemy_narrow_vs_wide_scope() {
    // "Some students could pass" should produce both readings:
    // 1. Past Ability (Root/Narrow): ∃x(Student(x) ∧ ◇Pass(x))
    // 2. Conditional Possibility (Epistemic/Wide): ◇∃x(Student(x) ∧ Pass(x))
    let results = logos::compile_forest("Some students could pass.");

    eprintln!("Could polysemy readings ({}):", results.len());
    for (i, r) in results.iter().enumerate() {
        eprintln!("  {}: {}", i + 1, r);
    }

    assert!(results.len() >= 2,
        "Should have at least 2 readings. Got: {:?}", results);

    // Narrow scope: quantifier (∃) comes before modal (◇)
    let has_narrow = results.iter().any(|r| {
        if let (Some(q), Some(m)) = (r.find('∃'), r.find('◇')) {
            q < m
        } else {
            false
        }
    });

    // Wide scope: modal (◇) comes before quantifier (∃)
    let has_wide = results.iter().any(|r| {
        if let (Some(q), Some(m)) = (r.find('∃'), r.find('◇')) {
            m < q
        } else {
            false
        }
    });

    assert!(has_narrow, "Should have narrow scope reading (∃...◇). Got: {:?}", results);
    assert!(has_wide, "Should have wide scope reading (◇...∃). Got: {:?}", results);
}

#[test]
fn could_default_narrow_scope() {
    // Default "could" = past ability (Alethic, Root/Narrow)
    // Use quantified subject to test scope
    let output = logos::compile("Some students could pass.").unwrap();
    eprintln!("Could past ability: {}", output);

    // Should have ◇ (alethic possibility)
    assert!(output.contains("◇"),
        "Default 'could' should use ◇. Got: {}", output);

    // Quantifier before modal (narrow scope)
    let q_pos = output.find('∃').expect("Should have ∃ quantifier");
    let m_pos = output.find('◇').expect("Should have ◇ modal");
    assert!(q_pos < m_pos, "Default 'could' should have narrow scope. Got: {}", output);
}
