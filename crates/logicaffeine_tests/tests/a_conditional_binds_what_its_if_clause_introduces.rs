//! A conditional is two clauses: the if-clause ends at its comma, and whatever the if-clause
//! introduces, the consequent's pronouns pick up. "If a farmer owns a donkey, he beats it" says
//! that every farmer beats every donkey he owns — "he" and "it" are the farmer and the donkey the
//! antecedent quantifies over, not constants named Farmer and E. And in "If a dog barks, John
//! runs", John is the consequent's subject, not a second thing the dog barks. Each claim is a
//! theorem the prover must certify — or must refuse — and the formula the Studio shows must be
//! the one the prover reads.

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

fn shown(sentence: &str) -> (String, String) {
    let result = logicaffeine_compile::compile_for_ui(sentence);
    assert!(result.error.is_none(), "{sentence} did not parse: {:?}", result.error);
    (result.logic.unwrap_or_default(), result.simple_logic.unwrap_or_default())
}

#[test]
fn a_named_subject_after_the_comma_starts_the_consequent() {
    certifies("Dog_Barks_John_Runs", &["If a dog barks, John runs.", "Fido is a dog.", "Fido barks."], "John runs.");
    certifies("John_Barks_Mary_Runs", &["If John barks, Mary runs.", "John barks."], "Mary runs.");
    certifies("Dog_Barks_John_Is_Happy", &["If a dog barks, John is happy.", "Fido is a dog.", "Fido barks."], "John is happy.");
    certifies(
        "Sees_A_Dog",
        &["If John sees a dog, John runs.", "Fido is a dog.", "John sees Fido."],
        "John runs.",
    );
    certifies("Sees_Fido", &["If John sees Fido, John runs.", "John sees Fido."], "John runs.");
}

#[test]
fn the_consequent_needs_its_antecedent() {
    refuses("No_Bark", &["If a dog barks, John runs.", "Fido is a dog."], "John runs.");
    refuses("No_Sight", &["If John sees a dog, John runs.", "Fido is a dog."], "John runs.");
}

/// The comma ends the if-clause exactly as "then" does.
#[test]
fn the_comma_ends_the_if_clause_as_then_does() {
    for (comma, then) in [
        ("If a dog barks, John runs.", "If a dog barks, then John runs."),
        ("If John barks, Mary runs.", "If John barks, then Mary runs."),
        ("If a dog barks, John is happy.", "If a dog barks, then John is happy."),
        ("If John sees a dog, John runs.", "If John sees a dog, then John runs."),
        ("If John sees Fido, Mary runs.", "If John sees Fido, then Mary runs."),
    ] {
        assert_eq!(shown(comma), shown(then), "{comma:?} does not say what {then:?} says");
    }
}

/// A name opens a clause only when it opens the coordinate: a comma list of objects keeps
/// every member, and the name inside "the friend Simon went with" is not a new subject.
#[test]
fn a_comma_list_of_objects_is_still_one_list() {
    let (full, _) = shown("Determine each trip's activity, state and year, as well as the friend Simon went with.");
    for member in ["Activity", "State", "Year", "Friend"] {
        assert!(full.contains(member), "the list lost {member}: {full}");
    }
}

#[test]
fn the_farmer_beats_the_donkey_he_owns() {
    let donkey = "If a farmer owns a donkey, he beats it.";
    certifies(
        "Pedro_Beats_Chiquita",
        &[donkey, "Pedro is a farmer.", "Chiquita is a donkey.", "Pedro owns Chiquita."],
        "Pedro beats Chiquita.",
    );
    refuses(
        "Pedro_Owns_Nothing",
        &[donkey, "Pedro is a farmer.", "Chiquita is a donkey."],
        "Pedro beats Chiquita.",
    );
}

#[test]
fn the_dog_that_barks_is_the_one_that_runs() {
    certifies("It_Runs", &["If a dog barks, it runs.", "Fido is a dog.", "Fido barks."], "Fido runs.");
    refuses("Rex_Is_Quiet", &["If a dog barks, it runs.", "Fido is a dog.", "Rex is a dog.", "Fido barks."], "Rex runs.");
}

/// Each referent of the if-clause is bound once. A referent registered on a parse path that was
/// then abandoned goes with the path, so the formula never quantifies over it.
#[test]
fn every_referent_of_the_if_clause_is_bound_once() {
    for sentence in [
        "If a dog barks, John runs.",
        "If any dog barks, John runs.",
        "If a dog barks, it runs.",
        "If a farmer owns a donkey, he beats it.",
    ] {
        let (full, simple) = shown(sentence);
        for (view, formula) in [("Full", &full), ("Simple", &simple)] {
            for binder in ["∀e(∀e(", "∀x(∀x(", "∀y(∀y("] {
                assert!(!formula.contains(binder), "{sentence} ({view}) binds a referent twice: {formula}");
            }
        }
    }
}

/// An indefinite under negation stays inside it: "If John does not own a donkey" denies that a
/// donkey exists — ¬∃ — rather than quantifying over the donkey it denies.
#[test]
fn an_indefinite_under_negation_stays_existential() {
    for sentence in ["If John does not own a donkey, John is sad.", "If John does not own a donkey, he is sad."] {
        let (full, simple) = shown(sentence);
        for (view, formula) in [("Full", &full), ("Simple", &simple)] {
            assert!(formula.contains("¬∃x"), "{sentence} ({view}) lost the negated existential: {formula}");
            assert!(!formula.contains("∀x"), "{sentence} ({view}) quantifies over the donkey it denies: {formula}");
        }
    }
    refuses(
        "Owns_A_Donkey",
        &["If John does not own a donkey, John is sad.", "Eeyore is a donkey.", "John owns Eeyore."],
        "John is sad.",
    );
}

/// The pronouns the Studio shows are the antecedent's bound variables: no common noun and no
/// event stands in the consequent as a constant of its own, no indefinite of the if-clause is
/// bound twice, and none is named after its own noun — `Dog(dog)` reads to the prover as a
/// definite description's self-guard, which it drops from the premise it searches.
#[test]
fn a_pronoun_in_the_consequent_is_the_antecedents_variable() {
    for (sentence, stray) in [
        ("If a farmer owns a donkey, he beats it.", ", Farmer)"),
        ("If a farmer owns a donkey, he beats it.", ", Donkey)"),
        ("If a farmer owns a donkey, he beats it.", "∃x"),
        ("If John sees a dog, John runs.", "∃x"),
        ("If a dog barks, it runs.", ", E)"),
        ("If a dog barks, it runs.", ", Dog)"),
        ("If a dog barks, John runs.", "Dog(dog)"),
        ("If a farmer owns a donkey, he beats it.", "Farmer(farmer)"),
    ] {
        let (full, simple) = shown(sentence);
        for (view, formula) in [("Full", &full), ("Simple", &simple)] {
            assert!(!formula.contains(stray), "{sentence} ({view}) binds the if-clause's referents wrongly ({stray:?}): {formula}");
        }
    }
}
