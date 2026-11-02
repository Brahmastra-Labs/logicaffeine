//! Coordinated objects each get the verb: "John sees Mary and Bill" says John sees Mary and John
//! sees Bill, and "John sees Mary, Bill and Sue" says it of all three. The coordination lives
//! inside the subject's scope, so "A man sees Mary and Bill" is one man seeing both, and each
//! object keeps its own determiner — "a cat and a dog" is some cat and some dog, not a cat and
//! a constant named Dog. Nothing in the list is dropped, and a bare name after "and" is an
//! object here, not the subject of a gapped clause. Each claim is a theorem the prover must
//! certify — or must refuse.

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

/// The formula the Studio shows for `sentence` (a top-level conjunction is listed as numbered
/// lines — one per conjunct — which is display, not ambiguity).
fn shown(sentence: &str) -> String {
    let result = compile_for_ui(sentence);
    assert!(result.error.is_none(), "{sentence} did not parse: {:?}", result.error);
    result.logic.unwrap_or_default()
}

#[test]
fn both_named_objects_are_seen() {
    certifies("Sees_Mary", &["John sees Mary and Bill."], "John sees Mary.");
    certifies("Sees_Bill", &["John sees Mary and Bill."], "John sees Bill.");
    refuses("Sees_Only_Them", &["John sees Mary and Bill."], "John sees Sue.");
}

#[test]
fn a_comma_list_gives_the_verb_to_every_member() {
    for (goal, name) in [("John sees Mary.", "Mary"), ("John sees Bill.", "Bill"), ("John sees Sue.", "Sue")] {
        certifies(&format!("List_{name}"), &["John sees Mary, Bill and Sue."], goal);
        certifies(&format!("Oxford_{name}"), &["John sees Mary, Bill, and Sue."], goal);
    }
}

#[test]
fn every_member_of_the_list_is_in_the_formula() {
    for sentence in ["John sees Mary and Bill.", "John sees Mary, Bill and Sue.", "John loves Mary, Bill and Sue."] {
        let logic = shown(sentence);
        for name in ["Mary", "Bill"] {
            assert!(logic.contains(&format!("Theme(e, {name})")), "{sentence} lost {name}: {logic}");
        }
        assert!(!logic.contains("Agent(e, Bill)"), "{sentence}: Bill is seen, he does not see: {logic}");
    }
}

#[test]
fn every_subject_gives_the_verb_to_each_object() {
    certifies("Every_Man_Bill", &["Every man sees Mary and Bill.", "Tom is a man."], "Tom sees Bill.");
    certifies("Every_Man_Mary", &["Every man sees Mary and Bill.", "Tom is a man."], "Tom sees Mary.");
    certifies("Every_Man_List", &["Every man sees Mary, Bill and Sue.", "Tom is a man."], "Tom sees Sue.");
    refuses("Every_Man_Not_Sue", &["Every man sees Mary and Bill.", "Tom is a man."], "Tom sees Sue.");
}

#[test]
fn an_indefinite_subject_is_one_individual_seeing_both() {
    let logic = shown("A man sees Mary and Bill.");
    assert_eq!(logic.matches("Man(").count(), 1, "one man, not one per object: {logic}");
    certifies("A_Man_Sees_Bill", &["A man sees Mary and Bill."], "Some man sees Bill.");
    certifies("A_Man_Sees_Mary", &["A man sees Mary and Bill."], "Some man sees Mary.");
    refuses("A_Man_Not_Sue", &["A man sees Mary and Bill."], "Some man sees Sue.");
}

#[test]
fn each_object_keeps_its_own_determiner() {
    certifies("Sees_A_Dog", &["John sees a cat and a dog."], "John sees a dog.");
    certifies("Sees_A_Cat", &["John sees a cat and a dog."], "John sees a cat.");
    let logic = shown("John sees a cat and a dog.");
    assert!(!logic.contains("Theme(e, Dog)"), "a dog is some dog, not a constant named Dog: {logic}");
}

#[test]
fn definite_objects_each_get_the_verb() {
    let logic = shown("John sees the dog and the cat.");
    for theme in ["Theme(e, Dog)", "Theme(e, Cat)"] {
        assert!(logic.contains(theme), "John sees each: {logic}");
    }
    assert!(!logic.contains("Agent(e, Dog)") && !logic.contains("Agent(e, Cat)"), "{logic}");
}

#[test]
fn gapping_still_reads_its_second_subject_and_object() {
    let result = compile_for_ui("John loves Mary and Bill Susan.");
    assert!(result.error.is_none(), "{:?}", result.error);
    let logic = result.logic.unwrap_or_default();
    assert!(logic.contains("Agent(e, Bill)") && logic.contains("Theme(e, Susan)"), "Bill loves Susan: {logic}");
}

#[test]
fn a_name_that_starts_a_new_clause_is_not_an_object() {
    certifies("New_Clause", &["John sees Mary and Bill runs."], "Bill runs.");
    refuses("New_Clause_Not_Seen", &["John sees Mary and Bill runs."], "John sees Bill.");
}
