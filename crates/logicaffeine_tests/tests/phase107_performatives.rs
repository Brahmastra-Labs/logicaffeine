//! Phase 107 — §1.3 Performatives / speech acts (work/MISSING_ENGLISH.md).
//!
//! First-person present utterances whose saying is the doing:
//!   "I promise to call you." → SpeechAct(promise, speaker, ⟨Call(speaker,hearer)⟩)
//!   "I hereby resign."       → SpeechAct(resign,  speaker, ⟨Resign(speaker)⟩)
//!
//! The `SpeechAct{performer, act_type, content}` node + the `Performative`
//! lexicon feature already exist and a basic "I promise to come." path works;
//! this phase closes the real gaps: (1) the infinitive complement must capture
//! its OBJECT ("call you" → the hearer), (2) the `hereby` marker and bare
//! performatives ("I hereby resign.") must parse, (3) the content is a structured
//! proposition (P3 — the complement's own structure is preserved).

use logicaffeine_language::{compile, compile_kripke, compile_simple};

#[test]
fn promise_infinitive_captures_object_hearer() {
    let out = compile("I promise to call you.").unwrap();
    eprintln!("promise-call-you: {out}");
    assert!(out.contains("SpeechAct("), "performative ⇒ SpeechAct node: {out}");
    assert!(out.to_lowercase().contains("promise"), "act type promise: {out}");
    assert!(out.contains("Speaker"), "performer is the speaker: {out}");
    assert!(out.contains("Call("), "content verb Call: {out}");
    // The crux: the object "you" must survive as the hearer/addressee.
    assert!(
        out.contains("Addressee") || out.contains("Hearer"),
        "the infinitive object 'you' must be captured as the hearer: {out}"
    );
}

#[test]
fn promise_intransitive_infinitive_still_works() {
    // Regression: the pre-existing basic path must keep working.
    let out = compile("I promise to come.").unwrap();
    eprintln!("promise-come: {out}");
    assert!(out.contains("SpeechAct("), "SpeechAct node: {out}");
    assert!(out.to_lowercase().contains("promise"), "act type: {out}");
    assert!(out.contains("Come(") || out.to_lowercase().contains("come"), "content verb: {out}");
}

#[test]
fn hereby_bare_performative_resign() {
    let out = compile("I hereby resign.").unwrap();
    eprintln!("hereby-resign: {out}");
    assert!(out.contains("SpeechAct("), "bare performative ⇒ SpeechAct: {out}");
    assert!(out.to_lowercase().contains("resign"), "act type resign: {out}");
    assert!(out.contains("Speaker"), "performer is the speaker: {out}");
    // The saying is the doing: the content is the resigning by the speaker.
    assert!(out.contains("Resign("), "content is the act Resign(speaker): {out}");
}

#[test]
fn bare_transitive_performative_thank_you() {
    let out = compile("I thank you.").unwrap();
    eprintln!("thank-you: {out}");
    assert!(out.contains("SpeechAct("), "bare transitive performative ⇒ SpeechAct: {out}");
    assert!(out.to_lowercase().contains("thank"), "act type thank: {out}");
    assert!(
        out.contains("Addressee") || out.contains("Hearer"),
        "the object 'you' is the addressee: {out}"
    );
}

#[test]
fn performative_renders_in_simple_and_kripke() {
    let simple = compile_simple("I promise to call you.").unwrap();
    eprintln!("promise(simple): {simple}");
    assert!(simple.contains("SpeechAct("), "SimpleFOL keeps SpeechAct: {simple}");

    let kripke = compile_kripke("I promise to call you.").unwrap();
    eprintln!("promise(kripke): {kripke}");
    assert!(kripke.contains("SpeechAct("), "Kripke keeps SpeechAct: {kripke}");
}
