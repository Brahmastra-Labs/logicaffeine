//! Phase 132 — §2.1 Non-constituent coordination / gapping (MISSING_ENGLISH.md).
//!
//! A second conjunct that is not a constituent shares the verb AND the subject of
//! the first via a gap:
//!   "John gave Mary a book and Sue a pen."
//!     → ∃e(Give ∧ Ag=John ∧ Rec=Mary ∧ Th=book) ∧ ∃e'(Give ∧ Ag=John ∧ Rec=Sue ∧ Th=pen)
//! The gapped verb "gave" and agent "John" are reconstructed for the remnant "Sue a pen".

use logicaffeine_language::compile;

#[test]
fn gapped_ditransitive_reconstructs_verb_and_agent() {
    let out = compile("John gave Mary a book and Sue a pen.").unwrap();
    eprintln!("gapping: {out}");
    // Two giving events.
    assert!(out.matches("Give").count() >= 2, "two Give events (gap reconstructed): {out}");
    // First conjunct: Mary + book.
    assert!(out.contains("Mary") && out.contains("Book"), "first conjunct roles: {out}");
    // Second conjunct: Sue is the Recipient and pen the Theme.
    assert!(out.contains("Recipient(e, Sue)"), "Sue is the recipient in the gapped conjunct: {out}");
    assert!(out.contains("Pen"), "pen is the theme: {out}");
    // The agent John is shared into BOTH conjuncts (not Sue).
    assert!(out.matches("Agent(e, John)").count() >= 2, "agent John reconstructed in both: {out}");
    assert!(!out.contains("Agent(e, Sue)"), "Sue is not the agent: {out}");
}

#[test]
fn lone_remnant_is_object_coordination_not_new_agent() {
    use logicaffeine_language::compile;
    // "John saw himself and Mary." — Mary is a second THEME with the agent
    // shared, never a new agent inheriting the template's object.
    let out = compile("John saw himself and Mary.").unwrap();
    eprintln!("obj-coord: {out}");
    assert!(
        !out.contains("Agent(e, Mary)"),
        "Mary must not become the agent: {out}"
    );
    assert!(out.contains("Theme(e, Mary)"), "Mary is a theme: {out}");
    assert!(out.contains("Theme(e, John)"), "the reflexive theme stays: {out}");
}

#[test]
fn quantified_subject_keeps_possessive_object() {
    use logicaffeine_language::compile;
    // "Each student saw his dog." — the possessive object must survive.
    let out = compile("Each student saw his dog.").unwrap();
    eprintln!("quant-poss-obj: {out}");
    assert!(out.contains("Theme"), "the object must not be dropped: {out}");
    assert!(out.contains("Dog"), "the dog: {out}");
}

#[test]
fn trailing_particle_is_directional_modifier() {
    use logicaffeine_language::compile;
    // "He sat down." — an unclaimed trailing particle modifies the event;
    // it must never be left as a trailing token.
    let out = compile("He sat down.").unwrap();
    eprintln!("sat-down: {out}");
    assert!(out.contains("Sit"), "the verb: {out}");
    assert!(out.contains("Down(e"), "directional modifier: {out}");
    let out2 = compile("John fell down.").unwrap();
    assert!(out2.contains("Down(e"), "directional modifier: {out2}");
}
