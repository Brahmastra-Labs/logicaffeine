use logos::{compile, compile_simple};

#[test]
fn quantified_subject_with_relative_clause_and_verb() {
    let output = compile("Every user who has a key enters the room.");
    assert!(output.is_ok(), "Should parse quantified subject with relative clause followed by verb");
    let result = output.unwrap();
    assert!(result.contains("User(x)"), "Should have User predicate");
    assert!(result.contains("Key("), "Should have Key predicate");
    assert!(result.contains("Have("), "Should have Have predicate");
    assert!(result.contains("Enter("), "Should have Enter predicate");
}

#[test]
fn simple_fol_output() {
    let output = compile_simple("Every user who has a key enters the room.");
    assert!(output.is_ok(), "Should parse");
    let result = output.unwrap();
    eprintln!("SimpleFOL: {}", result);

    // Also check the full FOL output for comparison
    let full = compile("Every user who has a key enters the room.").unwrap();
    eprintln!("Full FOL: {}", full);
    assert!(result.contains("User(x)"), "Should have full User predicate");
    assert!(result.contains("Key("), "Should have full Key predicate");
    assert!(result.contains("Have("), "Should have full Have predicate (lemma form)");
    assert!(result.contains("Enter("), "Should have full Enter predicate");
    assert!(result.contains("Room"), "Should have full Room constant");
}

#[test]
fn conditional_with_quantified_antecedent() {
    let output = compile("If a user enters the room, the alarm triggers.");
    assert!(output.is_ok(), "Should parse conditional with quantified antecedent");
}

#[test]
fn conditional_simple_fol() {
    let output = compile_simple("If a user enters the room, the alarm triggers.");
    assert!(output.is_ok(), "Should parse");
    let result = output.unwrap();
    eprintln!("SimpleFOL conditional: {}", result);
}

#[test]
fn negative_quantifier_with_relative_clause_and_modal() {
    let output = compile("No user who lacks a key can enter the room.");
    assert!(output.is_ok(), "Should parse negative quantifier with relative clause and modal");
    let result = output.unwrap();
    eprintln!("Full FOL negative: {}", result);
    assert!(!result.contains("?"), "Should not contain unknown marker");
    // "lacks" is an antonym of "have" so it compiles to ¬Have (canonical form)
    // NeoEvent format: ¬∃e(Have(e) ∧ Agent(e, x) ∧ Theme(e, y))
    assert!(result.contains("¬∃e(Have(e)"), "Should have negated Have event (canonical form of 'lacks')");
}

#[test]
fn negative_quantifier_simple_fol() {
    let output = compile_simple("No user who lacks a key can enter the room.");
    assert!(output.is_ok(), "Should parse");
    let result = output.unwrap();
    eprintln!("SimpleFOL negative: {}", result);
}

#[test]
fn all_three_marketing_sentences() {
    let input = "Every user who has a key enters the room. If a user enters the room, the alarm triggers. No user who lacks a key can enter the room.";
    let output = compile_simple(input);
    assert!(output.is_ok(), "Should parse all 3 sentences");
    let result = output.unwrap();
    eprintln!("All 3 sentences SimpleFOL: {}", result);

    // Should have all 3 sentences conjoined
    assert!(result.contains("User(x)"), "Should have sentence 1 User predicate");
    assert!(result.contains("Have("), "Should have sentence 1 Have predicate");
    assert!(result.contains("Trigger("), "Should have sentence 2 Trigger predicate");
    // "lacks" compiles to ¬Have (canonical form normalization)
    // In Simple mode, this is still ¬Have(
    assert!(result.contains("¬Have("), "Should have sentence 3 negated Have predicate (canonical form)");
}
