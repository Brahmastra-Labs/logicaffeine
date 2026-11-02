//! Flattening a Davidsonian event to Simple FOL must not silently drop an
//! adverbial modifier. `∃e(Bark(e) ∧ Agent(e,y) ∧ Loudly(e))` reduces to
//! `Bark(y) ∧ Loudly(y)` — never a bare `Bark(y)` that loses "loudly".

use logicaffeine_language::compile_simple;

#[test]
fn simple_fol_keeps_intransitive_manner_adverb() {
    let fol = compile_simple("Some dogs bark loudly.").expect("compiles");
    assert!(
        fol.contains("Bark"),
        "expected the verb predicate in: {fol}"
    );
    assert!(
        fol.contains("Loudly"),
        "Simple FOL silently dropped the manner adverb: {fol}"
    );
}

#[test]
fn simple_fol_keeps_transitive_manner_adverb() {
    let fol = compile_simple("John eats quickly.").expect("compiles");
    assert!(
        fol.contains("Quickly"),
        "Simple FOL silently dropped the manner adverb: {fol}"
    );
}

/// Flattening puts the event's participants in its place, but a tense is not a property of a
/// participant: "John ran quickly." must not say that John is past. The tense wraps the
/// flattened formula, as it already does for "John did not see any dogs." (`¬Past(∃x(…))`).
#[test]
fn simple_fol_wraps_a_past_event_in_its_tense() {
    let fol = compile_simple("John ran quickly.").expect("compiles");
    assert!(!fol.contains("Past(John)"), "the agent is not past: {fol}");
    assert!(
        fol.starts_with("Past("),
        "the tense does not wrap the formula: {fol}"
    );
    assert!(
        fol.contains("Run(John)") && fol.contains("Quickly(John)"),
        "{fol}"
    );
}

#[test]
fn simple_fol_wraps_a_past_transitive_event_in_its_tense() {
    let fol = compile_simple("Mary sliced the bread carefully.").expect("compiles");
    assert!(!fol.contains("Past(Mary)"), "the agent is not past: {fol}");
    assert!(
        fol.starts_with("Past("),
        "the tense does not wrap the formula: {fol}"
    );
    assert!(
        fol.contains("Slice(Mary, Bread)") && fol.contains("Carefully(Mary)"),
        "{fol}"
    );
}

#[test]
fn simple_fol_wraps_a_future_event_in_its_tense() {
    let fol = compile_simple("John will run.").expect("compiles");
    assert!(
        !fol.contains("Future(John)"),
        "the agent is not future: {fol}"
    );
    assert!(fol.contains("Run(John)"), "{fol}");
}
