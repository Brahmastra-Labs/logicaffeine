//! Flattening a Davidsonian event to Simple FOL must not silently drop an
//! adverbial modifier. `∃e(Bark(e) ∧ Agent(e,y) ∧ Loudly(e))` reduces to
//! `Bark(y) ∧ Loudly(y)` — never a bare `Bark(y)` that loses "loudly".

use logicaffeine_language::compile_simple;

#[test]
fn simple_fol_keeps_intransitive_manner_adverb() {
    let fol = compile_simple("Some dogs bark loudly.").expect("compiles");
    assert!(fol.contains("Bark"), "expected the verb predicate in: {fol}");
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
