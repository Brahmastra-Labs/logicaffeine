//! A verb phrase coordinated after "and" shares the clause's subject: "Some man sees Mary and
//! sees Bill" is ONE man who does both — `∃x(Man(x) ∧ See(x, Mary) ∧ See(x, Bill))` — and
//! "John sees Mary and loves Sue" says John loves Sue. The second verb phrase is not a clause
//! whose subject is a word named after its verb (`Agent(e, See_Bill)`).

use logicaffeine_compile::{compile_for_ui, compile_theorem_for_ui};

fn theorem(name: &str, givens: &[&str], goal: &str) -> logicaffeine_compile::TheoremCompileResult {
    let mut source = format!("## Theorem: {name}\n");
    for given in givens {
        source.push_str(&format!("Given: {given}\n"));
    }
    source.push_str(&format!("Prove: {goal}\nProof: Auto.\n"));
    compile_theorem_for_ui(&source)
}

fn certifies(name: &str, givens: &[&str], goal: &str) {
    let result = theorem(name, givens, goal);
    assert!(result.error.is_none(), "{name} did not compile: {:?}", result.error);
    assert!(
        result.verified,
        "{name}: from {givens:?} the prover does not certify {goal:?} ({:?}); premises {:?}",
        result.verification_error, result.premises
    );
}

fn refuses(name: &str, givens: &[&str], goal: &str) {
    let result = theorem(name, givens, goal);
    assert!(result.error.is_none(), "{name} did not compile: {:?}", result.error);
    assert!(!result.verified, "{name}: from {givens:?} the prover certified {goal:?}; premises {:?}", result.premises);
}

fn shown(sentence: &str) -> String {
    let result = compile_for_ui(sentence);
    assert!(result.error.is_none(), "{sentence} did not parse: {:?}", result.error);
    result.logic.unwrap_or_default()
}

#[test]
fn a_named_subject_does_both() {
    certifies("Loves_Sue", &["John sees Mary and loves Sue."], "John loves Sue.");
    certifies("Sees_Mary", &["John sees Mary and loves Sue."], "John sees Mary.");
    refuses("Not_Mary_Loves", &["John sees Mary and loves Sue."], "Mary loves Sue.");
}

#[test]
fn one_indefinite_subject_does_both() {
    let logic = shown("Some man sees Mary and sees Bill.");
    assert_eq!(logic.matches("Man(").count(), 1, "one man, not one per verb phrase: {logic}");
    assert!(logic.contains("Theme(e, Bill)"), "{logic}");
    certifies("Some_Man_Sees_Bill", &["Some man sees Mary and sees Bill."], "Some man sees Bill.");
}

#[test]
fn every_subject_does_both() {
    certifies(
        "Every_Man_Loves_Sue",
        &["Every man sees Mary and loves Sue.", "Tom is a man."],
        "Tom loves Sue.",
    );
    refuses("Every_Man_Not_Ann", &["Every man sees Mary and loves Sue.", "Tom is a man."], "Tom loves Ann.");
}

#[test]
fn no_verb_phrase_becomes_a_subject_named_after_its_verb() {
    for sentence in ["Some man sees Mary and sees Bill.", "John sees Mary and loves Sue.", "Every man sees Mary and loves Sue."] {
        let logic = shown(sentence);
        assert!(!logic.contains("Agent(e, See_") && !logic.contains("Agent(e, Love_"), "{sentence}: {logic}");
    }
}
