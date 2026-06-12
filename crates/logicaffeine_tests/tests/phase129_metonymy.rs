//! Phase 129 — §8.6 Metonymy (MISSING_ENGLISH.md).
//!
//! Reference by association: a place that conventionally denotes an institution
//! coerces to that institution when it acts:
//!   "The White House announced a plan." → Announce(GovernmentOf(white_house), …)
//! A place cannot announce; "the White House" coerces to its government. The same
//! place in a non-agent position ("I visited the White House") is literal.

use logicaffeine_language::compile;

#[test]
fn institution_metonym_coerces_in_agent_position() {
    let out = compile("The White House announced a plan.").unwrap();
    eprintln!("white-house: {out}");
    assert!(out.contains("Announce"), "the action predicate: {out}");
    assert!(
        out.contains("GovernmentOf"),
        "the place metonym coerces to its institution as agent: {out}"
    );
}

#[test]
fn place_in_object_position_is_literal() {
    // Regression: the same place as an object is NOT coerced (it is the building).
    let out = compile("I visited the White House.").unwrap();
    eprintln!("visited: {out}");
    assert!(out.contains("White_House") || out.contains("WhiteHouse"), "the literal place: {out}");
    assert!(!out.contains("GovernmentOf"), "object place is literal, not coerced: {out}");
}
