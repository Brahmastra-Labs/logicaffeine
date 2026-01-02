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
    assert!(!result.contains("?"), "Should not contain unknown marker");
    assert!(result.contains("Lack("), "Should have Lack predicate");
}

#[test]
fn negative_quantifier_simple_fol() {
    let output = compile_simple("No user who lacks a key can enter the room.");
    assert!(output.is_ok(), "Should parse");
    let result = output.unwrap();
    eprintln!("SimpleFOL negative: {}", result);
}
