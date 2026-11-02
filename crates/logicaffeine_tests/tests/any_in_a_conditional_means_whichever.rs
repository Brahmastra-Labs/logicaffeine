//! "If any dog barks, John runs." — whichever dog barks, John runs: ∀x((Dog(x) ∧ Bark(x)) →
//! Run(John)), the universal over the whole conditional (equivalently, "if some dog barks").
//! Read as "if every dog barks, John runs" it says something else, and English's inferences
//! show which: one barking dog is enough to make John run. Each claim is a theorem the prover
//! must certify — or must refuse. In an if-clause "any" is the indefinite: it says what "a"
//! says, and a pronoun in the consequent finds the individual it introduces.

use logicaffeine_compile::compile_theorem_for_ui;

fn theorem(name: &str, givens: &[&str], goal: &str) -> logicaffeine_compile::TheoremCompileResult {
    let mut source = format!("## Theorem: {name}\n");
    for given in givens {
        source.push_str(&format!("Given: {given}\n"));
    }
    source.push_str(&format!("Prove: {goal}\nProof: Auto.\n"));
    compile_theorem_for_ui(&source)
}

#[test]
fn one_barking_dog_is_enough() {
    let result = theorem("One_Dog", &["If any dog barks, John runs.", "Fido is a dog.", "Fido barks."], "John runs.");
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(
        result.verified,
        "a barking dog does not make John run: {:?}; premises {:?}",
        result.verification_error, result.premises
    );
}

#[test]
fn a_quiet_dog_is_not_enough() {
    let result = theorem("Quiet_Dog", &["If any dog barks, John runs.", "Fido is a dog."], "John runs.");
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.verified, "John runs although no dog was said to bark; premises {:?}", result.premises);
}

#[test]
fn a_barking_cat_is_not_enough() {
    let result = theorem("Barking_Cat", &["If any dog barks, John runs.", "Tom is a cat.", "Tom barks."], "John runs.");
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.verified, "a cat set John running; premises {:?}", result.premises);
}

#[test]
fn one_dog_seen_is_enough() {
    let result = theorem(
        "One_Seen",
        &["If John sees any dog, John runs.", "Fido is a dog.", "John sees Fido."],
        "John runs.",
    );
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(
        result.verified,
        "seeing a dog does not make John run: {:?}; premises {:?}",
        result.verification_error, result.premises
    );
}

#[test]
fn an_unseen_dog_is_not_enough() {
    let result = theorem("Unseen", &["If John sees any dog, John runs.", "Fido is a dog."], "John runs.");
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!result.verified, "John runs although he saw no dog; premises {:?}", result.premises);
}

/// The dog "any" introduces is the one a pronoun in the consequent picks up.
#[test]
fn the_pronoun_finds_the_dog_that_barked() {
    let result = theorem("It_Runs", &["If any dog barks, it runs.", "Fido is a dog.", "Fido barks."], "Fido runs.");
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(
        result.verified,
        "the barking dog does not run: {:?}; premises {:?}",
        result.verification_error, result.premises
    );
}

fn shown(sentence: &str) -> (String, String) {
    let result = logicaffeine_compile::compile_for_ui(sentence);
    assert!(result.error.is_none(), "{sentence} did not parse: {:?}", result.error);
    (result.logic.unwrap_or_default(), result.simple_logic.unwrap_or_default())
}

/// In an if-clause "any" says what "a" says, in every position and in every view.
#[test]
fn any_in_an_if_clause_reads_as_a() {
    for (any, a) in [
        ("If any dog barks, John runs.", "If a dog barks, John runs."),
        ("If any dog barks, then John runs.", "If a dog barks, then John runs."),
        ("If John sees any dog, John runs.", "If John sees a dog, John runs."),
        ("If any farmer owns a donkey, he beats it.", "If a farmer owns a donkey, he beats it."),
        ("If a farmer owns any donkey, he beats it.", "If a farmer owns a donkey, he beats it."),
        ("If any dog barks and any cat meows, John runs.", "If a dog barks and a cat meows, John runs."),
        ("If John does not see any dog, John runs.", "If John does not see a dog, John runs."),
    ] {
        assert_eq!(shown(any), shown(a), "{any:?} does not say what {a:?} says");
    }
}

/// The if-clause ends at its comma: an "any" in the consequent is not the antecedent's.
#[test]
fn any_after_the_comma_is_not_the_antecedents() {
    assert_ne!(
        shown("If John runs, any dog barks."),
        shown("If John runs, a dog barks."),
        "the consequent's \"any\" was read as the if-clause's indefinite"
    );
}
