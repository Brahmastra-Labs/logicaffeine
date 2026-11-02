//! A pronoun's gender is a constraint and a preference. "He" can be accommodated by a
//! referent whose gender is unknown — "a dog", "a farmer" — but a referent whose gender the
//! pronoun names exactly ("John" for "he", "Mary" for "she") is the better antecedent, however
//! recent the other one is. So in "If John sees a dog, he runs." it is John who runs; and when
//! nothing agrees exactly, "he" still finds the farmer.

use logicaffeine_compile::compile_theorem_for_ui;

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

/// The consequent of the formula the Studio shows for a conditional: what follows its arrow.
fn consequent(sentence: &str) -> String {
    let result = logicaffeine_compile::compile_for_ui(sentence);
    assert!(result.error.is_none(), "{sentence} did not parse: {:?}", result.error);
    let logic = result.logic.unwrap_or_default();
    let (_, after) = logic.split_once('→').unwrap_or_else(|| panic!("{sentence} shows no conditional: {logic}"));
    after.to_string()
}

#[test]
fn he_is_the_man_the_sentence_names_not_the_dog() {
    let premises = ["If John sees a dog, he runs.", "Fido is a dog.", "John sees Fido."];
    certifies("He_Is_John", &premises, "John runs.");
    refuses("He_Is_Not_The_Dog", &premises, "Fido runs.");
}

#[test]
fn she_is_the_woman_the_sentence_names_not_the_dog() {
    let premises = ["If Mary sees a dog, she runs.", "Fido is a dog.", "Mary sees Fido."];
    certifies("She_Is_Mary", &premises, "Mary runs.");
    refuses("She_Is_Not_The_Dog", &premises, "Fido runs.");
}

#[test]
fn the_formula_shows_the_named_man_running() {
    let after = consequent("If John sees a dog, he runs.");
    assert!(after.contains("Agent(e, John)"), "John is the one who runs: {after}");
    assert!(!after.contains("Agent(e, x)"), "the dog does not run: {after}");
}

#[test]
fn he_still_finds_a_referent_whose_gender_is_unknown() {
    certifies(
        "He_Is_The_Farmer",
        &["If a farmer runs, he smiles.", "Giles is a farmer.", "Giles runs."],
        "Giles smiles.",
    );
}

#[test]
fn the_farmer_still_beats_the_donkey() {
    certifies(
        "Farmer_Beats_Donkey",
        &["If a farmer owns a donkey, he beats it.", "Giles is a farmer.", "Benjy is a donkey.", "Giles owns Benjy."],
        "Giles beats Benjy.",
    );
}
