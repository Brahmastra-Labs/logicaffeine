//! Phase 111 — §4.4 Habitual / generic modality (MISSING_ENGLISH.md).
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
