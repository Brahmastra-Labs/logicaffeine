//! Gapping leaves the verb out of a second clause: "John loves Mary and Bill Susan" says Bill
//! loves Susan. The remnant after "and" is two names side by side, and a name parser that reads
//! "Ray Ricardo" as one person would read "Bill Susan" as one person too — one more object John
//! loves. Two names the lexicon knows as people are two people when the verb they borrow needs a
//! subject and an object and the first contrasts with the clause's own subject; a name the
//! lexicon does not know as a person ("Bill Smith") keeps the multi-word reading, and a remnant
//! that repeats the subject ("…and John Paul") has nothing to contrast and stays one name. Each
//! claim is a theorem the prover must certify — or must refuse.

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
fn the_second_name_is_what_the_first_one_does_it_to() {
    let logic = shown("John loves Mary and Bill Susan.");
    assert!(logic.contains("Agent(e, Bill)") && logic.contains("Theme(e, Susan)"), "Bill loves Susan: {logic}");
    assert!(!logic.contains("Bill_Susan"), "Bill and Susan are two people: {logic}");
    assert!(logic.contains("Agent(e, John)") && logic.contains("Theme(e, Mary)"), "John still loves Mary: {logic}");
}

#[test]
fn the_gapped_clause_is_a_claim_of_its_own() {
    certifies("Bill_Loves_Susan", &["John loves Mary and Bill Susan."], "Bill loves Susan.");
    certifies("John_Loves_Mary", &["John loves Mary and Bill Susan."], "John loves Mary.");
    refuses("John_Loves_Susan", &["John loves Mary and Bill Susan."], "John loves Susan.");
    refuses("Susan_Loves_Bill", &["John loves Mary and Bill Susan."], "Susan loves Bill.");
}

#[test]
fn a_trailing_adverb_does_not_hide_the_two_names() {
    let logic = shown("John saw Mary yesterday and Bill Susan today.");
    assert!(logic.contains("Agent(e, Bill)") && logic.contains("Theme(e, Susan)"), "Bill saw Susan: {logic}");
    assert!(!logic.contains("Bill_Susan"), "{logic}");
}

#[test]
fn a_surname_the_lexicon_does_not_know_stays_part_of_the_name() {
    let logic = shown("John loves Mary and Bill Smith.");
    assert!(logic.contains("Bill_Smith"), "Bill Smith is one person: {logic}");
    assert!(!logic.contains("Agent(e, Bill)"), "no one named Bill does the loving: {logic}");
}

#[test]
fn a_remnant_that_repeats_the_subject_is_not_a_new_clause() {
    let logic = shown("John loves Mary and John Paul.");
    assert!(logic.contains("John_Paul"), "John Paul is one more person John loves: {logic}");
}

#[test]
fn a_verb_without_an_object_leaves_the_names_together() {
    let logic = shown("John left and Bill Susan.");
    assert!(!logic.contains("Theme(e, Susan)"), "left takes no object: {logic}");
}
