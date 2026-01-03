/// Phase XX: Privation × Modal Interpretation Matrix
///
/// Tests that "lacks" + modal verbs generate all 4 semantic interpretations:
/// 1. Alethic + Partial: "lacks SOME key" → physically impossible to enter
/// 2. Alethic + Total: "has NO keys" → physically impossible to enter
/// 3. Deontic + Partial: "lacks SOME key" → not permitted to enter
/// 4. Deontic + Total: "has NO keys" → not permitted to enter

#[test]
fn lacks_can_four_interpretations() {
    let readings = logos::compile_forest("No user who lacks a key can enter the room.");

    // Full tier: 4 readings (2 modal × 2 scope)
    assert!(
        readings.len() >= 4,
        "Expected 4+ readings (2 modal × 2 scope), got {}: {:?}",
        readings.len(),
        readings
    );

    // Check for both scope patterns
    // Partial (narrow): ∃y((Key(y) ∧ ¬∃e(Have(e)...))) - negation INSIDE the existential
    // Total (wide): ¬∃y((Key(y) ∧ Have(...))) - negation OUTSIDE the existential
    let has_partial = readings.iter().any(|r| {
        // Pattern: Key(y) followed by ¬ (negation) before Have
        r.contains("Key(") && r.contains("¬∃e(Have(")
    });
    let has_total = readings.iter().any(|r| {
        // Pattern: ¬∃y((Key( - negation before the existential over key
        r.contains("¬∃y((Key(")
    });

    assert!(has_partial, "Missing Partial scope reading (∃y(Key(y) ∧ ¬Have))");
    assert!(has_total, "Missing Total scope reading (¬∃y(Key(y) ∧ Have))");

    // Check modal distribution
    // In Unicode format: ◇_{0.5} for Alethic, P_{0.5} for Deontic
    // In Kripke format: Accessible_Alethic / Accessible_Deontic
    let has_alethic = readings.iter().any(|r| {
        r.contains("◇_{") || r.contains("Accessible_Alethic")
    });
    let has_deontic = readings.iter().any(|r| {
        r.contains("P_{") || r.contains("Accessible_Deontic")
    });

    assert!(has_alethic, "Missing Alethic modal reading (◇)");
    assert!(has_deontic, "Missing Deontic modal reading (P)");
}

#[test]
fn all_readings_use_neo_davidsonian_for_have() {
    // Full resolution should use Neo-Davidsonian event syntax for ALL interpretations
    let readings = logos::compile_forest("No user who lacks a key can enter the room.");

    for (i, reading) in readings.iter().enumerate() {
        // Simple Have looks like: Have(x, y) or Have(w, v)
        // Neo-Davidsonian looks like: ∃e(Have(e) ∧ Agent(e, x) ∧ Theme(e, y))
        //
        // Check for simple predicate pattern: "Have(" followed by a variable, comma, variable
        // This regex-like check: if Have( is followed by anything other than "e" it's Simple
        if reading.contains("Have(") {
            // Find all occurrences of "Have("
            for (idx, _) in reading.match_indices("Have(") {
                let after_paren = &reading[idx + 5..];
                // In Neo-Davidsonian, next char should be 'e' (the event variable)
                // In Simple, it would be a subject variable like 'x', 'w', etc.
                if let Some(first_char) = after_paren.chars().next() {
                    if first_char != 'e' && first_char.is_alphabetic() {
                        panic!(
                            "Reading {} uses Simple syntax for Have (found Have({}, should use Have(e, ...)): {}",
                            i, first_char, reading
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn simple_tier_has_two_scope_readings() {
    // Simple tier strips modals, so Alethic/Deontic collapse
    // But scope difference (Partial vs Total) should still produce 2 distinct outputs
    let readings = logos::compile_forest_with_options(
        "No user who lacks a key can enter the room.",
        logos::CompileOptions {
            format: logos::OutputFormat::SimpleFOL,
        },
    );

    // Deduplicated readings should be at least 2 (Partial vs Total)
    // May be fewer if modals collapse scope differences too
    assert!(
        readings.len() >= 2,
        "Simple tier should have 2+ readings (Partial vs Total scope), got {}: {:?}",
        readings.len(),
        readings
    );
}
