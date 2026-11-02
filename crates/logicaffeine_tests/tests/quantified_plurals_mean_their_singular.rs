//! Which predicate does a quantified plural translate to — `Dogs(x)` or `Dog(x)`? The one under
//! which English's valid inferences go through. "All dogs bark" and "Every dog barks" say the
//! same thing; a dog named Fido falls under "all dogs"; "no dogs" excludes each dog. Each claim
//! below is a theorem the prover must certify: if the plural named a predicate of its own, the
//! premises about `Dogs` and the premise about `Dog` would never meet.

use logicaffeine_compile::compile_theorem_for_ui;

fn certifies(name: &str, givens: &[&str], goal: &str) {
    let mut source = format!("## Theorem: {name}\n");
    for given in givens {
        source.push_str(&format!("Given: {given}\n"));
    }
    source.push_str(&format!("Prove: {goal}\nProof: Auto.\n"));
    let result = compile_theorem_for_ui(&source);
    assert!(result.error.is_none(), "{name} did not compile: {:?}", result.error);
    assert!(
        result.verified,
        "{name}: from {givens:?} the prover does not certify {goal:?} ({:?}); premises {:?}, goal {:?}",
        result.verification_error, result.premises, result.goal_string
    );
}

#[test]
fn all_dogs_and_every_dog_say_the_same() {
    certifies("All_To_Every", &["All dogs bark."], "Every dog barks.");
    certifies("Every_To_All", &["Every dog barks."], "All dogs bark.");
}

#[test]
fn a_named_dog_falls_under_all_dogs() {
    certifies("Fido_Barks", &["All dogs bark.", "Fido is a dog."], "Fido barks.");
}

#[test]
fn no_dogs_excludes_each_dog() {
    certifies("Fido_Is_Quiet", &["No dogs bark.", "Fido is a dog."], "Fido does not bark.");
}

#[test]
fn a_barking_dog_is_some_dog_barking() {
    certifies("Some_Dogs_Bark", &["Fido is a dog.", "Fido barks."], "Some dogs bark.");
}

/// The singular claim, beside the plural one: an existential claim is proved by its witness.
#[test]
fn a_barking_dog_is_a_dog_that_barks() {
    certifies("Some_Dog_Barks", &["Fido is a dog.", "Fido barks."], "Some dog barks.");
}

#[test]
fn all_men_and_every_man_say_the_same() {
    certifies("Men_Are_Mortal", &["All men are mortal.", "Socrates is a man."], "Socrates is mortal.");
}

/// The prover already reads a quantified plural as its singular: `proof_convert` lemmatizes
/// "dogs" to `dog` so that "All men are mortal" meets "Socrates is a man" — which is why the
/// theorems above certify. The formula the Studio shows must be the formula that is proved:
/// its restrictor names the same predicate the prover does.
#[test]
fn the_formula_shown_names_the_predicate_the_prover_uses() {
    for (sentence, noun) in [
        ("All dogs bark.", "Dog"),
        ("Some cats sleep.", "Cat"),
        ("No students laugh.", "Student"),
        ("All men are mortal.", "Man"),
        ("John did not see any dogs.", "Dog"),
    ] {
        let shown = logicaffeine_compile::compile_for_ui(sentence);
        let full = shown.logic.clone().unwrap_or_default();
        let simple = shown.simple_logic.clone().unwrap_or_default();
        for (view, formula) in [("Full", &full), ("Simple", &simple)] {
            assert!(
                formula.contains(&format!("{noun}(")),
                "{sentence} ({view}) shows {formula}, not the predicate {noun} the prover uses"
            );
        }
    }
}
