//! Phase 111 — §4.4 Habitual / generic modality (work/MISSING_ENGLISH.md).
//!
//! Characterizing statements carry a GEN/HAB operator, NOT a bare existential:
//!   "John smokes."         → HAB(∃e(Smoke(e) ∧ Agent(e, John)))
//!   "John usually smokes." → HAB(…)            (adverb of quantification)
//!   "Dogs bark."           → GEN x(Dog(x) → Bark(x))   (bare-plural subject)
//!
//! Operator-level semantics (the FOL export). The defeasible/non-monotonic
//! reasoning layer (P4: a counter-instance cancels the default without
//! contradiction) is a separate reasoning subsystem; here we lock that the
//! characterizing OPERATOR is produced rather than a strict ∃/∀.

use logicaffeine_language::compile;

#[test]
fn bare_present_eventive_is_habitual() {
    let out = compile("John smokes.").unwrap();
    eprintln!("smokes: {out}");
    assert!(out.contains("HAB") || out.contains("Gen"), "characterizing operator, not bare ∃: {out}");
    assert!(out.contains("Smoke"), "the verb: {out}");
}

#[test]
fn adverb_usually_yields_habitual() {
    let out = compile("John usually smokes.").unwrap();
    eprintln!("usually: {out}");
    assert!(out.contains("HAB") || out.contains("Gen"), "usually ⇒ habitual: {out}");
    assert!(out.contains("Smoke"), "verb survives the adverb: {out}");
    assert!(out != "John", "the clause must not collapse to the bare subject: {out}");
}

#[test]
fn adverb_always_yields_habitual() {
    let out = compile("John always runs.").unwrap();
    eprintln!("always: {out}");
    assert!(out.contains("HAB") || out.contains("Gen"), "always ⇒ habitual: {out}");
    assert!(out.contains("Run"), "verb survives: {out}");
}

#[test]
fn adverb_often_yields_habitual() {
    let out = compile("Mary often reads.").unwrap();
    eprintln!("often: {out}");
    assert!(out.contains("HAB") || out.contains("Gen"), "often ⇒ habitual: {out}");
    assert!(out.contains("Read"), "verb survives: {out}");
}

#[test]
fn bare_plural_subject_is_generic() {
    let out = compile("Dogs bark.").unwrap();
    eprintln!("dogs-bark: {out}");
    assert!(out.contains("Gen"), "bare plural ⇒ generic quantifier: {out}");
    assert!(out.contains("Bark"), "the verb: {out}");
    assert!(out.contains('→'), "generic is a restricted (implicational) quantification: {out}");
}

/// A trailing manner adverb after an intransitive main-clause verb must be
/// captured as an event modifier, never silently dropped (nor left as a stray
/// token that trips a TrailingTokens parse error). The existential-subject
/// path ("Some dogs …") already conjoins it; the bare-plural generic path
/// used to return before consuming the adverb.
#[test]
fn manner_adverb_survives_trailing_intransitive_verb() {
    // Existential subject — the task-named sentence.
    let some = compile("Some dogs bark loudly.").expect("existential + manner adverb must parse");
    eprintln!("some-dogs-bark-loudly: {some}");
    assert!(some.contains("Bark"), "verb survives: {some}");
    assert!(some.contains("Loudly"), "manner adverb must not be silently dropped: {some}");

    // Transitive main clause with a trailing manner adverb.
    let john = compile("John eats quickly.").expect("transitive + manner adverb must parse");
    eprintln!("john-eats-quickly: {john}");
    assert!(john.contains("Eat"), "verb survives: {john}");
    assert!(john.contains("Quickly"), "manner adverb must survive: {john}");

    // Bare-plural generic subject — the regression this test guards.
    let dogs = compile("Dogs bark loudly.").expect("bare-plural + manner adverb must parse");
    eprintln!("dogs-bark-loudly: {dogs}");
    assert!(dogs.contains("Gen"), "still a generic quantifier: {dogs}");
    assert!(dogs.contains("Bark"), "verb survives: {dogs}");
    assert!(dogs.contains("Loudly"), "manner adverb must not be silently dropped: {dogs}");

    // Generalizes to any manner adverb, not hardcoded to "loudly".
    let cats = compile("Cats sleep peacefully.").expect("bare-plural + manner adverb must parse");
    eprintln!("cats-sleep-peacefully: {cats}");
    assert!(cats.contains("Peacefully"), "any manner adverb survives: {cats}");

    let birds = compile("Birds fly gracefully.").expect("bare-plural + manner adverb must parse");
    eprintln!("birds-fly-gracefully: {birds}");
    assert!(birds.contains("Gracefully"), "any manner adverb survives: {birds}");
}
